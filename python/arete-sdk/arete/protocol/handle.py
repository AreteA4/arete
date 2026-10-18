"""Live instance handles: an instance's detail kept current from its event
stream, plus the protocol's live stack view row when one is wired.

Mirror of ``client.instances.use(id)`` in ``@usearete/sdk``.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, AsyncIterator, Callable, Generator, List, Optional, Set

from arete.errors import AreteError
from arete.protocol.types import ProtocolInstanceDetail

if TYPE_CHECKING:
    from arete.protocol.client import ProtocolClient

__all__ = ["ProtocolInstanceHandle", "ProtocolInstanceSnapshot", "live_view_rows"]

_CLOSED = object()


@dataclass(frozen=True)
class ProtocolInstanceSnapshot:
    """One reading of an instance handle."""

    instance: ProtocolInstanceDetail
    """The instance as last read from the hosted API."""
    live: Any = None
    """The live stack view row, once the view has sent it; ``None`` when the
    protocol has no live stack or the client no connected stacks."""


def live_view_rows(member: Any, view: str, key: str) -> Optional[AsyncIterator[Any]]:
    """Follow the row ``key`` of ``view`` (``"<Entity>/<mode>"``) on a
    connected stack client, or ``None`` when the member has no such view.

    Generated Python stacks group views by snake_case entity name, so an
    :class:`arete.client.Arete` member is matched by the view's wire id;
    anything else is read as ``member.views.<Entity>.<mode>.use(key)``, the
    TypeScript shape."""
    from arete.views import ViewsNamespace, state_view_by_path

    views = getattr(member, "views", None)
    if views is None:
        return None
    if isinstance(views, ViewsNamespace):
        return state_view_by_path(views, view).use(key)
    entity, _, mode = view.partition("/")
    if not entity or not mode:
        return None
    handle = getattr(getattr(views, entity, None), mode, None)
    use = getattr(handle, "use", None)
    return use(key) if callable(use) else None


class ProtocolInstanceHandle:
    """A live instance from :meth:`ProtocolClient.instance_handle`::

        async with client.instance_handle(instance_id) as handle:
            print(handle.current.instance["state"])
            async for snapshot in handle:
                ...

    or ``handle = await client.instance_handle(instance_id)`` and
    ``await handle.aclose()`` when done. Starting reads the instance (and
    raises when that read fails); after that the instance is re-read after
    every event of its stream, events arriving together causing one read.
    When the definition names a live stack and the client has connected
    stacks (a client from a session, or ``stacks=`` on the client), the row
    of that view keyed by the instance's key account is followed too, as
    :attr:`ProtocolInstanceSnapshot.live`. Failures after the first read
    land on :attr:`error` until the next successful read.
    """

    def __init__(
        self, client: "ProtocolClient", instance_id: str, *, wallet: Optional[str] = None
    ) -> None:
        self._client = client
        self.instance_id = instance_id
        self._wallet = wallet
        self._snapshot: Optional[ProtocolInstanceSnapshot] = None
        self._error: Optional[Exception] = None
        self._listeners: List[Callable[[], None]] = []
        self._queues: Set["asyncio.Queue[Any]"] = set()
        self._tasks: List["asyncio.Task[Any]"] = []
        self._in_flight: Optional["asyncio.Future[None]"] = None
        self._queued = False
        self._live_key: Optional[str] = None
        self._live_task: Optional["asyncio.Task[Any]"] = None
        self._live_rows: Optional[AsyncIterator[Any]] = None
        self._started = False
        self._closed = False

    # ------------------------------------------------------------------
    # State

    @property
    def current(self) -> ProtocolInstanceSnapshot:
        """The latest snapshot."""
        if self._snapshot is None:
            raise AreteError(
                "the instance handle has not started; await it or use it with 'async with'",
                "INVALID_STATE",
            )
        return self._snapshot

    @property
    def error(self) -> Optional[Exception]:
        """The last refresh or stream failure, cleared by the next successful read."""
        return self._error

    @property
    def closed(self) -> bool:
        return self._closed

    def subscribe(self, listener: Callable[[], None]) -> Callable[[], None]:
        """Call ``listener`` after every change; returns the unsubscribe."""
        self._listeners.append(listener)

        def unsubscribe() -> None:
            if listener in self._listeners:
                self._listeners.remove(listener)

        return unsubscribe

    def __aiter__(self) -> AsyncIterator[ProtocolInstanceSnapshot]:
        """The current snapshot, then one per change until the handle closes."""
        return self._iterate()

    async def _iterate(self) -> AsyncIterator[ProtocolInstanceSnapshot]:
        queue: "asyncio.Queue[Any]" = asyncio.Queue()
        self._queues.add(queue)
        try:
            if self._snapshot is not None:
                yield self._snapshot
            while not self._closed:
                item = await queue.get()
                # Only the newest of several pending snapshots matters.
                while not queue.empty():
                    item = queue.get_nowait()
                if item is _CLOSED:
                    return
                yield item
        finally:
            self._queues.discard(queue)

    def _emit(self) -> None:
        for queue in list(self._queues):
            queue.put_nowait(self._snapshot)
        for listener in list(self._listeners):
            try:
                listener()
            except Exception:  # noqa: BLE001 - a throwing listener must not stop the handle
                pass

    # ------------------------------------------------------------------
    # Lifecycle

    def __await__(self) -> Generator[Any, None, "ProtocolInstanceHandle"]:
        return self.start().__await__()

    async def __aenter__(self) -> "ProtocolInstanceHandle":
        return await self.start()

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def start(self) -> "ProtocolInstanceHandle":
        """Read the instance and start following it; idempotent."""
        if self._closed:
            raise AreteError("the instance handle is closed", "INVALID_STATE")
        if self._started:
            return self
        instance = await self._client.get_instance(self.instance_id, wallet=self._wallet)
        self._started = True
        self._snapshot = ProtocolInstanceSnapshot(instance=instance)
        self._client._handles.add(self)
        self._tasks.append(asyncio.ensure_future(self._follow_events()))
        self._follow_live(instance)
        return self

    async def aclose(self) -> None:
        """Stop following the instance and its live row."""
        if self._closed:
            return
        self._closed = True
        self._client._handles.discard(self)
        tasks = [
            *self._tasks,
            *([self._live_task] if self._live_task else []),
            *([self._in_flight] if self._in_flight else []),
        ]
        self._tasks = []
        self._live_task = None
        self._in_flight = None
        for task in tasks:
            task.cancel()
        for task in tasks:
            try:
                await task
            except (asyncio.CancelledError, Exception):  # noqa: BLE001 - shutting down
                pass
        await self._close_live_rows()
        for queue in list(self._queues):
            queue.put_nowait(_CLOSED)

    # ------------------------------------------------------------------
    # Reads

    async def refresh(self) -> ProtocolInstanceSnapshot:
        """Re-read the instance now. Reads requested while one is running
        coalesce into one more read after it. Raises the read's failure,
        which is also :attr:`error` until the next successful read."""
        if self._closed:
            raise AreteError("the instance handle is closed", "INVALID_STATE")
        await asyncio.shield(self._schedule_refresh())
        return self.current

    def _schedule_refresh(self) -> "asyncio.Future[None]":
        if self._in_flight is not None:
            self._queued = True
            return self._in_flight
        in_flight = asyncio.ensure_future(self._run_loads())
        # A failure is on ``error``; refresh() callers get it raised.
        in_flight.add_done_callback(
            lambda done: done.cancelled() or done.exception()
        )
        self._in_flight = in_flight
        return in_flight

    async def _run_loads(self) -> None:
        try:
            await self._load()
            while self._queued:
                self._queued = False
                await self._load()
        finally:
            self._queued = False
            self._in_flight = None

    async def _load(self) -> None:
        try:
            instance = await self._client.get_instance(self.instance_id, wallet=self._wallet)
        except Exception as error:  # noqa: BLE001 - surfaced on the handle and raised
            self._error = error
            self._emit()
            raise
        live = self._snapshot.live if self._snapshot is not None else None
        self._snapshot = ProtocolInstanceSnapshot(instance=instance, live=live)
        self._error = None
        self._emit()
        self._follow_live(instance)

    async def _follow_events(self) -> None:
        def on_error(error: Exception) -> None:
            self._error = error
            self._emit()

        stream = self._client.events(self.instance_id, on_error=on_error)
        try:
            async for _event in stream:
                self._schedule_refresh()
        except asyncio.CancelledError:
            raise
        except Exception as error:  # noqa: BLE001 - the hosted API refused the stream for good
            self._error = error
            self._emit()
        finally:
            await stream.aclose()

    # ------------------------------------------------------------------
    # Live stack row

    def _follow_live(self, instance: ProtocolInstanceDetail) -> None:
        live = self._client.definition.live_stack
        if live is None or self._closed:
            return
        key = (instance.get("accounts") or {}).get(live.key_account)
        if not key or key == self._live_key:
            return
        member = self._client._live_stacks.get(live.stack)
        if member is None:
            return
        rows = live_view_rows(member, live.view, key)
        if rows is None:
            return
        previous = self._live_task
        if previous is not None:
            previous.cancel()
        self._live_key = key
        self._live_task = asyncio.ensure_future(self._consume_live(rows, previous))

    async def _close_live_rows(self) -> None:
        rows, self._live_rows = self._live_rows, None
        close = getattr(rows, "aclose", None)
        if close is not None:
            try:
                await close()
            except Exception:  # noqa: BLE001 - shutting down
                pass

    async def _consume_live(
        self, rows: AsyncIterator[Any], previous: Optional["asyncio.Task[Any]"]
    ) -> None:
        if previous is not None:
            try:
                await previous
            except (asyncio.CancelledError, Exception):  # noqa: BLE001 - replaced
                pass
        await self._close_live_rows()
        self._live_rows = rows
        try:
            async for row in rows:
                if self._snapshot is not None:
                    self._snapshot = replace(self._snapshot, live=row)
                    self._emit()
        except asyncio.CancelledError:
            raise
        except Exception as error:  # noqa: BLE001 - surfaced on the handle
            self._error = error
            self._emit()
