"""View handles: the six canonical verbs over protocol v2 subscriptions.

Mirror of ``typescript/core/src/views.ts`` + ``stream.ts`` as Python async
iterators. Every verb takes keyword-only query options (``filters``, ``take``,
``skip``, ``partition``, ``after``, ``snapshot_limit``, ``with_snapshot``,
``parser``); ``get`` / ``get_one`` additionally take ``timeout``. Unknown
keywords raise ``TypeError`` (fail-closed).

- ``use``       → stream of merged entities (patches applied; remove/delete
                  filtered out of yields)
- ``watch``     → stream of :class:`arete.wire.Update` (upsert|patch|remove|delete)
- ``watch_rich``→ stream of :class:`arete.wire.RichUpdate` (before/after diffs)
- ``get``       → one-shot read awaiting an equivalent lease's snapshot,
                  bounded by ``timeout`` (Rust's ``initial_data_timeout``)
- ``get_sync``  → non-blocking; ``arete.UNSET`` when no equivalent active
                  subscription exists (absent ≠ empty)
- ``get_one``   → first element of a list view, or None

Breaking a stream (or ``aclose()``) releases the refcounted lease.
"""

from __future__ import annotations

import asyncio
from collections import deque
from dataclasses import dataclass
from typing import (
    Any,
    AsyncIterator,
    Callable,
    Dict,
    List,
    Mapping,
    Optional,
    Tuple,
)

from arete import UNSET
from arete.errors import AreteError
from arete.subscription import QueryLease, SubscriptionRegistry
from arete.wire import RichUpdate, Update

_OPTION_KEYS = (
    "filters",
    "take",
    "skip",
    "partition",
    "after",
    "snapshot_limit",
    "with_snapshot",
    "parser",
    "timeout",
)

#: Verbs that block on the initial snapshot and therefore accept ``timeout``.
_AWAITING_VERBS = ("get", "get_one")

#: Default bound on the initial snapshot of ``get`` / ``get_one``, mirroring
#: the Rust SDK's ``AreteConfig::initial_data_timeout`` (5s).
DEFAULT_INITIAL_DATA_TIMEOUT = 5.0

_MAX_QUEUE_SIZE = 1000

Parser = Callable[[Any], Any]


class InitialDataTimeoutError(AreteError):
    """Raised when ``get`` / ``get_one`` gives up waiting for a snapshot.

    The socket never delivered the initial data for the view's subscription
    (no connection, a reconnect storm, a server that never answers). Sibling
    of :class:`arete.errors.ProcessedSlotTimeoutError`; the one-shot read's
    lease is always released before it is raised.
    """

    def __init__(self, view: str, timeout: float) -> None:
        super().__init__(
            f"Timed out after {timeout}s waiting for the initial snapshot of "
            f"view '{view}'",
            "INITIAL_DATA_TIMEOUT",
        )
        self.view = view
        self.timeout = timeout


class StreamGapError(AreteError):
    """Raised when a stream's bounded queue dropped a replayable update.

    The consumer fell behind far enough that an update carrying a replay
    cursor was evicted, so continuing would present the loss as a continuous
    tape. ``recover_from`` is the last cursor actually delivered before the
    gap (``None`` when nothing had been delivered yet): resubscribe with
    ``after=recover_from`` to pick the tape back up without a hole.
    """

    def __init__(self, recover_from: Optional[str]) -> None:
        super().__init__(
            "Stream fell behind and skipped replayable updates"
            + (f"; recover from cursor {recover_from}" if recover_from else ""),
            "STREAM_GAP",
        )
        self.recover_from = recover_from


@dataclass(frozen=True)
class ViewDef:
    """Binding of one server view used to build a handle.

    ``mode`` is ``'state'`` or ``'list'``; ``view`` is the wire id
    (``"Entity/view"``); ``key_fields`` names the typed key kwargs of state
    views; ``parser`` is the default entity converter.
    """

    mode: str
    view: str
    key_fields: Tuple[str, ...] = ()
    parser: Optional[Parser] = None


class _StreamQueue:
    """Bounded push queue for async iteration.

    Latest-state updates beyond the cap are dropped (a projection has no
    per-event identity to lose), but dropping an update that carried a replay
    cursor is data loss on a tape: the queue fails closed with
    :class:`StreamGapError` instead of silently continuing.
    """

    def __init__(self, maxsize: int = _MAX_QUEUE_SIZE) -> None:
        self._items: "deque[Tuple[Any, Optional[str]]]" = deque()
        self._maxsize = maxsize
        self._waiter: Optional["asyncio.Future[None]"] = None
        self._error: Optional[BaseException] = None
        self._delivered_cursor: Optional[str] = None

    def push(self, item: Any, cursor: Optional[str] = None) -> None:
        if self._error is not None:
            return
        if len(self._items) >= self._maxsize:
            _dropped, dropped_cursor = self._items.popleft()
            if dropped_cursor is not None:
                self.fail(StreamGapError(self._delivered_cursor), discard_queued=True)
                return
        self._items.append((item, cursor))
        self._wake()

    def fail(self, error: BaseException, discard_queued: bool = False) -> None:
        """End the stream at ``error``, after whatever is already queued.

        ``discard_queued`` is for a local overflow only: there the hole is at
        the front of the queue, so everything behind it is no longer contiguous
        with what the consumer has read. A server refusal is the opposite — it
        reports records skipped *after* everything already sent, so the queue
        holds real data that arrived before the gap. Dropping it would lose
        records the consumer was never told about and leave ``recover_from``
        pointing past them.
        """
        if self._error is not None:
            return
        self._error = error
        if discard_queued:
            self._items.clear()
        self._wake()

    async def get(self) -> Any:
        while not self._items:
            if self._error is not None:
                raise self._error
            self._waiter = asyncio.get_running_loop().create_future()
            try:
                await self._waiter
            finally:
                self._waiter = None
        item, cursor = self._items.popleft()
        if cursor is not None:
            self._delivered_cursor = cursor
        return item

    def _wake(self) -> None:
        if self._waiter is not None and not self._waiter.done():
            self._waiter.set_result(None)


def _split_options(
    options: Mapping[str, Any], context: str, verb: Optional[str] = None
) -> Dict[str, Any]:
    allowed = (
        _OPTION_KEYS
        if verb is None or verb in _AWAITING_VERBS
        else tuple(name for name in _OPTION_KEYS if name != "timeout")
    )
    for name in options:
        if name not in allowed:
            raise TypeError(f"{context} got an unexpected keyword argument '{name}'")
    return dict(options)


def _resolve_timeout(options: Mapping[str, Any], default: Optional[float]) -> Optional[float]:
    """Per-call ``timeout`` (``None`` waits forever) over the handle default."""
    timeout = options["timeout"] if "timeout" in options else default
    if timeout is None:
        return None
    if not isinstance(timeout, (int, float)) or isinstance(timeout, bool):
        raise TypeError("timeout must be a number of seconds or None")
    if timeout <= 0:
        raise ValueError("timeout must be greater than 0")
    return float(timeout)


def _build_query(view: str, key: Optional[str], options: Mapping[str, Any]) -> Dict[str, Any]:
    query: Dict[str, Any] = {"view": view}
    if key is not None:
        query["key"] = key
    if options.get("partition") is not None:
        query["partition"] = options["partition"]
    if options.get("filters") is not None:
        query["filters"] = options["filters"]
    if options.get("take") is not None:
        query["take"] = options["take"]
    if options.get("skip") is not None:
        query["skip"] = options["skip"]
    if options.get("after") is not None:
        query["after"] = options["after"]
    if options.get("snapshot_limit") is not None:
        query["snapshotLimit"] = options["snapshot_limit"]
    return query


def _snapshot_enabled(options: Mapping[str, Any]) -> bool:
    with_snapshot = options.get("with_snapshot")
    return True if with_snapshot is None else bool(with_snapshot)


def _serialize_key_value(value: Any, view: str, field: Optional[str] = None) -> str:
    location = f"view '{view}'" if field is None else f"key field '{field}' for view '{view}'"
    if isinstance(value, str):
        return value
    if isinstance(value, int) and not isinstance(value, bool):
        return str(value)
    raise TypeError(f"{location} must be a string or integer")


async def _wait_resolved(
    lease: QueryLease, timeout: Optional[float], view: str
) -> None:
    """Wait until the lease's snapshot has resolved (or its query failed).

    Bounded by ``timeout`` seconds (``None`` waits forever): a socket that
    never delivers must not hang the caller. Raises
    :class:`InitialDataTimeoutError` on expiry; the change listener is
    detached on the timeout and cancellation paths alike.
    """
    result = lease.get_result()
    if result.error is not None:
        raise result.error
    if not result.is_loading:
        return

    future: "asyncio.Future[None]" = asyncio.get_running_loop().create_future()

    def on_change() -> None:
        if future.done():
            return
        state = lease.get_result()
        if state.error is not None:
            future.set_exception(state.error)
        elif not state.is_loading:
            future.set_result(None)

    unsubscribe = lease.on_change(on_change)
    try:
        if timeout is None:
            await future
        else:
            try:
                await asyncio.wait_for(future, timeout)
            except asyncio.TimeoutError:
                raise InitialDataTimeoutError(view, timeout) from None
    finally:
        unsubscribe()


def _fail_stream_on_query_error(lease: QueryLease, queue: _StreamQueue) -> Callable[[], None]:
    """Terminate ``queue`` whenever the lease's query fails.

    A server refusal (a replay cursor the view cannot serve, a lagged
    subscription whose delivery has stopped) must reach the consumer as a
    raised :class:`~arete.errors.SubscriptionError` carrying the wire code,
    not leave it awaiting an update that will never come.
    """

    def check() -> None:
        error = lease.get_result().error
        if error is not None:
            queue.fail(error)

    check()
    return lease.on_change(check)


async def _entity_stream(
    registry: SubscriptionRegistry,
    query: Mapping[str, Any],
    snapshot_enabled: bool,
    parser: Optional[Parser],
    key_filter: Optional[str] = None,
) -> AsyncIterator[Any]:
    lease = registry.subscribe(query, snapshot_enabled)
    queue = _StreamQueue()

    def on_rich(update: RichUpdate) -> None:
        if key_filter is not None and update.key != key_filter:
            return
        if update.type == "created":
            queue.push(update.data, update.cursor)
        elif update.type == "updated":
            queue.push(update.after, update.cursor)

    unsubscribe = lease.on_rich_update(on_rich)
    stop_failing = _fail_stream_on_query_error(lease, queue)
    try:
        result = lease.get_result()
        for key, entity in zip(result.keys, result.data):
            if key_filter is None or key == key_filter:
                yield parser(entity) if parser else entity
        while True:
            value = await queue.get()
            yield parser(value) if parser else value
    finally:
        stop_failing()
        unsubscribe()
        lease.release()


async def _update_stream(
    registry: SubscriptionRegistry,
    query: Mapping[str, Any],
    snapshot_enabled: bool,
    key_filter: Optional[str] = None,
) -> AsyncIterator[Update]:
    lease = registry.subscribe(query, snapshot_enabled)
    queue = _StreamQueue()

    def on_update(update: Update) -> None:
        if key_filter is None or update.key == key_filter:
            queue.push(update, update.cursor)

    unsubscribe = lease.on_update(on_update)
    stop_failing = _fail_stream_on_query_error(lease, queue)
    try:
        while True:
            yield await queue.get()
    finally:
        stop_failing()
        unsubscribe()
        lease.release()


async def _rich_update_stream(
    registry: SubscriptionRegistry,
    query: Mapping[str, Any],
    snapshot_enabled: bool,
    key_filter: Optional[str] = None,
) -> AsyncIterator[RichUpdate]:
    lease = registry.subscribe(query, snapshot_enabled)
    queue = _StreamQueue()

    def on_rich(update: RichUpdate) -> None:
        if key_filter is None or update.key == key_filter:
            queue.push(update, update.cursor)

    unsubscribe = lease.on_rich_update(on_rich)
    stop_failing = _fail_stream_on_query_error(lease, queue)
    try:
        while True:
            yield await queue.get()
    finally:
        stop_failing()
        unsubscribe()
        lease.release()


class ListViewHandle:
    """The six verbs on an ordered list view."""

    def __init__(
        self,
        view: str,
        registry: SubscriptionRegistry,
        parser: Optional[Parser] = None,
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
    ) -> None:
        self._view = view
        self._registry = registry
        self._parser = parser
        self._initial_data_timeout = initial_data_timeout

    @property
    def view(self) -> str:
        return self._view

    def _prepare(self, options: Mapping[str, Any], verb: str):
        opts = _split_options(options, f"{self._view}.{verb}()", verb)
        query = _build_query(self._view, None, opts)
        parser = opts.get("parser") or self._parser
        return query, _snapshot_enabled(opts), parser, opts

    def use(self, **options: Any) -> AsyncIterator[Any]:
        query, snapshot_enabled, parser, _opts = self._prepare(options, "use")
        return _entity_stream(self._registry, query, snapshot_enabled, parser)

    def watch(self, **options: Any) -> AsyncIterator[Update]:
        query, snapshot_enabled, _parser, _opts = self._prepare(options, "watch")
        return _update_stream(self._registry, query, snapshot_enabled)

    def watch_rich(self, **options: Any) -> AsyncIterator[RichUpdate]:
        query, snapshot_enabled, _parser, _opts = self._prepare(options, "watch_rich")
        return _rich_update_stream(self._registry, query, snapshot_enabled)

    async def get(self, **options: Any) -> List[Any]:
        """Await the snapshot of an equivalent lease, bounded by ``timeout``
        seconds (default: the handle's ``initial_data_timeout``; ``None``
        waits forever). Raises :class:`InitialDataTimeoutError` on expiry."""
        query, snapshot_enabled, parser, opts = self._prepare(options, "get")
        timeout = _resolve_timeout(opts, self._initial_data_timeout)
        lease = self._registry.subscribe(query, snapshot_enabled)
        try:
            await _wait_resolved(lease, timeout, self._view)
            data = lease.get_result().data
            return [parser(entity) if parser else entity for entity in data]
        finally:
            lease.release()

    def get_sync(self, **options: Any) -> Any:
        """List data of an existing equivalent subscription, or ``arete.UNSET``."""
        query, snapshot_enabled, parser, _opts = self._prepare(options, "get_sync")
        result = self._registry.get_query_result(query, snapshot_enabled)
        if result is None:
            return UNSET
        return [parser(entity) if parser else entity for entity in result.data]

    async def get_one(self, **options: Any) -> Optional[Any]:
        data = await self.get(**options)
        return data[0] if data else None


class StateViewHandle:
    """The six verbs on a keyed state view.

    The key is passed as typed keyword arguments named after the view's key
    fields (``state.use(round_id=42, take=1)``); views without generated key
    fields take one positional scalar key.
    """

    def __init__(
        self,
        view: str,
        registry: SubscriptionRegistry,
        key_fields: Tuple[str, ...] = (),
        parser: Optional[Parser] = None,
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
    ) -> None:
        self._view = view
        self._registry = registry
        self._key_fields = tuple(key_fields)
        self._parser = parser
        self._initial_data_timeout = initial_data_timeout

    @property
    def view(self) -> str:
        return self._view

    def _wire_key(self, key: Any, kwargs: Dict[str, Any]) -> str:
        if not self._key_fields:
            if key is None:
                raise TypeError(f"View '{self._view}' requires a key")
            return _serialize_key_value(key, self._view)
        if key is not None:
            raise TypeError(
                f"View '{self._view}' takes its key as keyword arguments "
                f"({', '.join(self._key_fields)})"
            )
        if len(self._key_fields) != 1:
            raise TypeError(
                f"View '{self._view}' has an unsupported composite key with fields "
                f"[{', '.join(self._key_fields)}]"
            )
        field = self._key_fields[0]
        if field not in kwargs:
            raise TypeError(f"View '{self._view}' key is missing field '{field}'")
        return _serialize_key_value(kwargs.pop(field), self._view, field)

    def _prepare(self, key: Any, options: Dict[str, Any], verb: str):
        wire_key = self._wire_key(key, options)
        opts = _split_options(options, f"{self._view}.{verb}()", verb)
        query = _build_query(self._view, wire_key, opts)
        parser = opts.get("parser") or self._parser
        return query, _snapshot_enabled(opts), parser, wire_key, opts

    def use(self, key: Any = None, **options: Any) -> AsyncIterator[Any]:
        query, snapshot_enabled, parser, wire_key, _opts = self._prepare(key, options, "use")
        return _entity_stream(self._registry, query, snapshot_enabled, parser, wire_key)

    def watch(self, key: Any = None, **options: Any) -> AsyncIterator[Update]:
        query, snapshot_enabled, _parser, wire_key, _opts = self._prepare(key, options, "watch")
        return _update_stream(self._registry, query, snapshot_enabled, wire_key)

    def watch_rich(self, key: Any = None, **options: Any) -> AsyncIterator[RichUpdate]:
        query, snapshot_enabled, _parser, wire_key, _opts = self._prepare(
            key, options, "watch_rich"
        )
        return _rich_update_stream(self._registry, query, snapshot_enabled, wire_key)

    async def get(self, key: Any = None, **options: Any) -> Optional[Any]:
        """Await the snapshot of an equivalent lease, bounded by ``timeout``
        seconds (default: the handle's ``initial_data_timeout``; ``None``
        waits forever). Raises :class:`InitialDataTimeoutError` on expiry."""
        query, snapshot_enabled, parser, _wire_key, opts = self._prepare(
            key, options, "get"
        )
        timeout = _resolve_timeout(opts, self._initial_data_timeout)
        lease = self._registry.subscribe(query, snapshot_enabled)
        try:
            await _wait_resolved(lease, timeout, self._view)
            data = lease.get_result().data
            if not data:
                return None
            return parser(data[0]) if parser else data[0]
        finally:
            lease.release()

    def get_sync(self, key: Any = None, **options: Any) -> Any:
        """Entity (or None when empty) of an existing equivalent subscription,
        or ``arete.UNSET`` when none is active."""
        query, snapshot_enabled, parser, _wire_key, _opts = self._prepare(
            key, options, "get_sync"
        )
        result = self._registry.get_query_result(query, snapshot_enabled)
        if result is None:
            return UNSET
        if not result.data:
            return None
        return parser(result.data[0]) if parser else result.data[0]

    async def get_one(self, key: Any = None, **options: Any) -> Optional[Any]:
        return await self.get(key, **options)


def create_view_handle(
    view_def: ViewDef,
    registry: SubscriptionRegistry,
    initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
):
    if view_def.mode == "state":
        return StateViewHandle(
            view_def.view,
            registry,
            view_def.key_fields,
            view_def.parser,
            initial_data_timeout,
        )
    if view_def.mode == "list":
        return ListViewHandle(
            view_def.view, registry, view_def.parser, initial_data_timeout
        )
    raise TypeError(f"Unknown view mode '{view_def.mode}' for view '{view_def.view}'")


class ViewGroupHandle:
    """One entity's views (``a4.views.ore_round.state`` / ``.latest`` / ...)."""

    def __init__(
        self,
        name: str,
        defs: Mapping[str, ViewDef],
        registry: SubscriptionRegistry,
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
    ):
        self._name = name
        self._defs = dict(defs)
        self._registry = registry
        self._initial_data_timeout = initial_data_timeout
        self._handles: Dict[str, Any] = {}

    def __getattr__(self, name: str) -> Any:
        if name.startswith("_"):
            raise AttributeError(name)
        handle = self._handles.get(name)
        if handle is None:
            view_def = self._defs.get(name)
            if view_def is None:
                raise AttributeError(
                    f"Entity '{self._name}' has no view '{name}' "
                    f"(available: {', '.join(sorted(self._defs)) or 'none'})"
                )
            handle = create_view_handle(
                view_def, self._registry, self._initial_data_timeout
            )
            self._handles[name] = handle
        return handle

    def __dir__(self):
        return [*super().__dir__(), *self._defs]


class ViewsNamespace:
    """``client.views``: attribute access over the stack's view groups."""

    def __init__(
        self,
        registry: SubscriptionRegistry,
        view_defs: Mapping[str, Mapping[str, ViewDef]],
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
    ) -> None:
        self._registry = registry
        self._defs = {name: dict(group) for name, group in view_defs.items()}
        self._initial_data_timeout = initial_data_timeout
        self._groups: Dict[str, ViewGroupHandle] = {}

    def __getattr__(self, name: str) -> ViewGroupHandle:
        if name.startswith("_"):
            raise AttributeError(name)
        group = self._groups.get(name)
        if group is None:
            defs = self._defs.get(name)
            if defs is None:
                raise AttributeError(
                    f"Stack has no view group '{name}' "
                    f"(available: {', '.join(sorted(self._defs)) or 'none'})"
                )
            group = ViewGroupHandle(
                name, defs, self._registry, self._initial_data_timeout
            )
            self._groups[name] = group
        return group

    def __dir__(self):
        return [*super().__dir__(), *self._defs]
