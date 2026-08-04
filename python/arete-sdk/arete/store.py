"""Internal protocol v2 store engine.

Mirror of the TS ``query-store.ts`` + ``frame-processor.ts`` pair (and Rust's
``SharedStore``): shared entity storage per view plus per-subscription query
records. Not part of the public API — the six view verbs are the public read
surface.

Semantics:
- Snapshot batches sharing a ``snapshotId`` are staged; on the final
  ``complete: true`` batch, ``authoritative: true`` replaces membership,
  ``authoritative: false`` merges.
- Patches deep-merge with ``append``-path array concatenation.
- ``remove`` evicts a key from one query only; ``delete`` removes the entity
  from the source view globally.
- Ordering follows the server-declared ``sort`` from the ``subscribed`` ack.
  String comparison and the entity-key tie-break use
  :func:`arete.subscription.locale_compare` — the shared
  ``String.prototype.localeCompare`` equivalent TS uses at ``query-store.ts``
  lines 64 and 387 — so key order matches TS for mixed-case base58 keys.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from functools import cmp_to_key
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence, Set, Tuple

from arete.errors import AreteConnectionError, AreteError, SubscriptionError
from arete.subscription import Subscription, locale_compare
from arete.wire import (
    EntityFrame,
    ErrorFrame,
    Frame,
    RichUpdate,
    SnapshotFrame,
    SortConfig,
    SubscribedFrame,
    UnsubscribedFrame,
    Update,
    compare_seq,
)

_MISSING = object()


@dataclass(frozen=True)
class QueryResult:
    """Materialized state of one subscription's query window."""

    subscription_id: str
    query: Mapping[str, Any]
    keys: Tuple[str, ...]
    data: Tuple[Any, ...]
    is_loading: bool
    is_refreshing: bool
    error: Optional[AreteError] = None


@dataclass
class _StagedSnapshot:
    snapshot_id: str
    authoritative: bool
    keys: List[str] = field(default_factory=list)


@dataclass
class _Record:
    subscription: Subscription
    query_key: str
    keys: List[str] = field(default_factory=list)
    sequences: Dict[str, str] = field(default_factory=dict)
    is_loading: bool = True
    is_refreshing: bool = False
    resolved: bool = False
    error: Optional[AreteError] = None
    mode: Optional[str] = None
    sort: Optional[SortConfig] = None
    staged: Optional[_StagedSnapshot] = None
    refresh_future: Optional["asyncio.Future[None]"] = None
    change_listeners: Set[Callable[[], None]] = field(default_factory=set)
    update_listeners: Set[Callable[[Update], None]] = field(default_factory=set)
    rich_update_listeners: Set[Callable[[RichUpdate], None]] = field(default_factory=set)


def _get_nested(value: Any, path: Sequence[str]) -> Any:
    current = value
    for segment in path:
        if not isinstance(current, Mapping):
            return _MISSING
        current = current.get(segment, _MISSING)
        if current is _MISSING:
            return _MISSING
    return current


def _compare_values(left: Any, right: Any) -> int:
    if left is None or left is _MISSING:
        return 0 if (right is None or right is _MISSING) else -1
    if right is None or right is _MISSING:
        return 1
    if isinstance(left, bool) and isinstance(right, bool):
        return int(left) - int(right)
    if isinstance(left, (int, float)) and isinstance(right, (int, float)) \
            and not isinstance(left, bool) and not isinstance(right, bool):
        return -1 if left < right else (1 if left > right else 0)
    # TS query-store.ts:64 falls through to String(left).localeCompare(...)
    # for every remaining pair, strings included.
    if isinstance(left, str) and isinstance(right, str):
        return locale_compare(left, right)
    return locale_compare(str(left), str(right))


def _compare_sequences(left: Any, right: Any) -> int:
    if not isinstance(left, str) or not isinstance(right, str):
        return _compare_values(left, right)
    return compare_seq(left, right)


def _is_object(value: Any) -> bool:
    return isinstance(value, Mapping)


def deep_merge_with_append(
    target: Any,
    source: Any,
    append_paths: Sequence[str],
    current_path: str = "",
) -> Any:
    """Deep-merge ``source`` into ``target``; arrays at ``append_paths``
    (dot paths) concatenate instead of replacing."""
    if not _is_object(target) or not _is_object(source):
        return source

    result = dict(target)
    for key, source_value in source.items():
        target_value = result.get(key)
        field_path = f"{current_path}.{key}" if current_path else key

        if isinstance(source_value, list) and isinstance(target_value, list):
            if field_path in append_paths:
                result[key] = [*target_value, *source_value]
            else:
                result[key] = source_value
        elif _is_object(source_value) and _is_object(target_value):
            result[key] = deep_merge_with_append(
                target_value, source_value, append_paths, field_path
            )
        else:
            result[key] = source_value
    return result


def _extract_seq(data: Any) -> Optional[str]:
    if not isinstance(data, Mapping):
        return None
    seq = data.get("_seq")
    if isinstance(seq, str):
        return seq
    if isinstance(seq, (int, float)) and not isinstance(seq, bool):
        return str(seq)
    return None


class Store:
    """Internal engine: entity storage + per-subscription query records."""

    def __init__(self) -> None:
        self._entities: Dict[str, Dict[str, Any]] = {}
        self._seqs: Dict[str, Dict[str, str]] = {}
        self._records: Dict[str, _Record] = {}

    # -- registration ------------------------------------------------------

    def register(self, subscription: Subscription, query_key: str) -> None:
        if subscription.subscription_id in self._records:
            return
        self._records[subscription.subscription_id] = _Record(
            subscription=subscription,
            query_key=query_key,
            is_loading=subscription.snapshot_enabled,
            resolved=not subscription.snapshot_enabled,
        )

    def unregister(self, subscription_id: str) -> None:
        record = self._records.pop(subscription_id, None)
        if record is None:
            return
        self._reject_refresh(record, SubscriptionError(
            "Subscription was released while refreshing", "SUBSCRIPTION_NOT_FOUND"
        ))

    def get_subscription(self, subscription_id: str) -> Optional[Subscription]:
        record = self._records.get(subscription_id)
        return record.subscription if record else None

    # -- reads -------------------------------------------------------------

    def get_entity(self, view: str, key: str) -> Any:
        return self._entities.get(view, {}).get(key)

    def get_result(self, subscription_id: str) -> Optional[QueryResult]:
        record = self._records.get(subscription_id)
        if record is None:
            return None
        view = record.subscription.query.get("view")
        entities = self._entities.get(view, {})
        data = tuple(entities[key] for key in record.keys if key in entities)
        return QueryResult(
            subscription_id=subscription_id,
            query=record.subscription.query,
            keys=tuple(record.keys),
            data=data,
            is_loading=record.is_loading,
            is_refreshing=record.is_refreshing,
            error=record.error,
        )

    # -- listeners ---------------------------------------------------------

    def on_change(self, subscription_id: str, callback: Callable[[], None]) -> Callable[[], None]:
        listeners = self._require_record(subscription_id).change_listeners
        listeners.add(callback)
        return lambda: listeners.discard(callback)

    def on_update(
        self, subscription_id: str, callback: Callable[[Update], None]
    ) -> Callable[[], None]:
        listeners = self._require_record(subscription_id).update_listeners
        listeners.add(callback)
        return lambda: listeners.discard(callback)

    def on_rich_update(
        self, subscription_id: str, callback: Callable[[RichUpdate], None]
    ) -> Callable[[], None]:
        listeners = self._require_record(subscription_id).rich_update_listeners
        listeners.add(callback)
        return lambda: listeners.discard(callback)

    # -- frame handling ----------------------------------------------------

    def handle_frame(self, frame: Frame) -> None:
        if isinstance(frame, ErrorFrame):
            if frame.subscription_id is not None:
                self._fail(frame.subscription_id, SubscriptionError(
                    frame.message or frame.error or frame.code, frame.code, frame
                ))
            return
        if isinstance(frame, UnsubscribedFrame):
            return  # Local lease state owns cancellation; the ack is informational.
        if isinstance(frame, SubscribedFrame):
            self._acknowledge(frame)
            return
        if isinstance(frame, SnapshotFrame):
            self._handle_snapshot(frame)
            return
        if isinstance(frame, EntityFrame):
            self._handle_entity(frame)

    def _acknowledge(self, frame: SubscribedFrame) -> None:
        record = self._records.get(frame.subscription_id)
        if record is None:
            return
        record.mode = frame.mode
        record.sort = frame.sort
        record.error = None
        if not record.subscription.snapshot_enabled:
            record.is_loading = False
            record.is_refreshing = False
            record.resolved = True
        if frame.query.get("view") != record.subscription.query.get("view"):
            self._fail(frame.subscription_id, SubscriptionError(
                "Server acknowledged a different view for the subscription", "INVALID_FRAME"
            ))
            return
        self._touch(record)

    def _handle_snapshot(self, frame: SnapshotFrame) -> None:
        view = frame.entity
        accepted: List[str] = []
        for entity in frame.data:
            self._set_entity(view, entity.key, entity.data, _extract_seq(entity.data))
            accepted.append(entity.key)
        self._stage_snapshot(frame, accepted)

    def _stage_snapshot(self, frame: SnapshotFrame, keys: Sequence[str]) -> None:
        record = self._records.get(frame.subscription_id)
        if record is None:
            return
        if record.staged is None or record.staged.snapshot_id != frame.snapshot_id:
            record.staged = _StagedSnapshot(
                snapshot_id=frame.snapshot_id,
                authoritative=frame.authoritative,
            )
        staged = record.staged
        if staged.authoritative != frame.authoritative:
            self._fail(frame.subscription_id, SubscriptionError(
                "Snapshot batches disagree on authoritative mode", "INVALID_FRAME"
            ))
            record.staged = None
            return
        for key in keys:
            if key not in staged.keys:
                staged.keys.append(key)
        if not frame.complete:
            return

        staged_keys = staged.keys
        view = record.subscription.query.get("view")
        if frame.authoritative:
            retained = set(staged_keys)
            for key in [k for k in record.sequences if k not in retained]:
                del record.sequences[key]
        for key in staged_keys:
            sequence = self._seqs.get(view, {}).get(key)
            if sequence is not None:
                record.sequences[key] = sequence
        if frame.authoritative:
            record.keys = list(staged_keys)
        else:
            merged = list(record.keys)
            for key in staged_keys:
                if key not in merged:
                    merged.append(key)
            record.keys = merged
        record.staged = None
        record.is_loading = False
        record.is_refreshing = False
        record.resolved = True
        record.error = None
        self._touch(record)
        self._resolve_refresh(record)

        entities = self._entities.get(view, {})
        for key in staged_keys:
            if key in entities:
                data = entities[key]
                self._emit_update(record, Update(op="upsert", key=key, data=data))
                self._emit_rich_update(record, RichUpdate(type="created", key=key, data=data))

    def _handle_entity(self, frame: EntityFrame) -> None:
        view = frame.entity
        previous = self._entities.get(view, {}).get(frame.key, _MISSING)
        previous_value = None if previous is _MISSING else previous
        previous_seq = self._seqs.get(view, {}).get(frame.key)
        stale = (
            frame.seq is not None
            and previous_seq is not None
            and compare_seq(frame.seq, previous_seq) <= 0
        )

        if frame.op == "upsert":
            if frame.data is None:
                return
            if stale and previous is not _MISSING:
                update = Update(op="upsert", key=frame.key, data=previous_value)
                rich = RichUpdate(type="created", key=frame.key, data=previous_value)
                self._apply_live(frame.subscription_id, frame.key, update, rich, frame.seq)
                return
            seq = frame.seq or _extract_seq(frame.data)
            self._set_entity(view, frame.key, frame.data, seq)
            update = Update(op="upsert", key=frame.key, data=frame.data)
            rich = self._make_rich(frame.key, previous, frame.data)
            self._apply_live(frame.subscription_id, frame.key, update, rich, seq)
            return

        if frame.op == "patch":
            if frame.data is None:
                return
            if stale and previous is not _MISSING:
                update = Update(op="patch", key=frame.key, data=frame.data)
                rich = RichUpdate(
                    type="updated", key=frame.key,
                    before=previous_value, after=previous_value, patch=frame.data,
                )
                self._apply_live(frame.subscription_id, frame.key, update, rich, frame.seq)
                return
            merged = (
                deep_merge_with_append(previous_value, frame.data, list(frame.append))
                if previous is not _MISSING
                else frame.data
            )
            seq = frame.seq or _extract_seq(frame.data) or previous_seq
            self._set_entity(view, frame.key, merged, seq)
            update = Update(op="patch", key=frame.key, data=frame.data)
            rich = self._make_rich(frame.key, previous, merged, patch=frame.data)
            self._apply_live(
                frame.subscription_id, frame.key, update, rich,
                frame.seq or _extract_seq(frame.data),
            )
            return

        if frame.op == "remove":
            self._apply_live(
                frame.subscription_id, frame.key, Update(op="remove", key=frame.key)
            )
            return

        if frame.op == "delete":
            self._entities.get(view, {}).pop(frame.key, None)
            self._seqs.get(view, {}).pop(frame.key, None)
            self._delete_global(view, frame.key, previous_value)

    # -- live application --------------------------------------------------

    def _apply_live(
        self,
        subscription_id: str,
        key: str,
        update: Update,
        rich: Optional[RichUpdate] = None,
        seq: Optional[str] = None,
    ) -> None:
        record = self._records.get(subscription_id)
        if record is None:
            return
        view = record.subscription.query.get("view")

        if update.op == "remove":
            last_known = self._entities.get(view, {}).get(key)
            record.keys = [entry for entry in record.keys if entry != key]
            record.sequences.pop(key, None)
            self._touch(record)
            self._emit_update(record, update)
            self._emit_rich_update(
                record, RichUpdate(type="removed", key=key, last_known=last_known)
            )
            return

        if key not in record.keys:
            record.keys = [*record.keys, key]
        next_seq = seq if seq is not None else self._seqs.get(view, {}).get(key)
        if next_seq is not None:
            record.sequences[key] = next_seq
        self._sort_keys(record)
        record.is_loading = False
        if record.refresh_future is None or record.refresh_future.done():
            record.is_refreshing = False
        record.resolved = True
        record.error = None
        self._touch(record)
        self._emit_update(record, update)
        if rich is not None:
            self._emit_rich_update(record, rich)

    def _delete_global(self, view: str, key: str, last_known: Any) -> None:
        for record in list(self._records.values()):
            if record.subscription.query.get("view") != view or key not in record.keys:
                continue
            record.keys = [entry for entry in record.keys if entry != key]
            record.sequences.pop(key, None)
            self._touch(record)
            self._emit_update(record, Update(op="delete", key=key))
            self._emit_rich_update(
                record, RichUpdate(type="deleted", key=key, last_known=last_known)
            )

    def _sort_keys(self, record: _Record) -> None:
        if len(record.keys) < 2:
            return
        if record.sort is None and record.mode not in ("list", "append"):
            return
        sort_field = record.sort.field if record.sort is not None else None
        order = (
            record.sort.order
            if record.sort is not None
            else ("asc" if record.subscription.query.get("after") is not None else "desc")
        )
        view = record.subscription.query.get("view")
        entities = self._entities.get(view, {})

        def compare(left_key: str, right_key: str) -> int:
            if sort_field is not None:
                left = _get_nested(entities.get(left_key), sort_field)
                right = _get_nested(entities.get(right_key), sort_field)
                compared = _compare_values(left, right)
            else:
                compared = _compare_sequences(
                    record.sequences.get(left_key), record.sequences.get(right_key)
                )
            if order == "desc":
                compared = -compared
            if compared == 0:
                # TS query-store.ts:387 — leftKey.localeCompare(rightKey).
                return locale_compare(left_key, right_key)
            return compared

        record.keys = sorted(record.keys, key=cmp_to_key(compare))

    # -- refresh / reconnect ----------------------------------------------

    def begin_refresh(
        self, subscription_id: str, wait_for_snapshot: bool = False
    ) -> "asyncio.Future[None]":
        record = self._require_record(subscription_id)
        self._mark_refreshing(record)

        if not wait_for_snapshot or not record.subscription.snapshot_enabled:
            future: "asyncio.Future[None]" = asyncio.get_event_loop().create_future()
            future.set_result(None)
            return future
        if record.refresh_future is not None and not record.refresh_future.done():
            return record.refresh_future
        record.refresh_future = asyncio.get_event_loop().create_future()
        return record.refresh_future

    def fail_refresh(self, subscription_id: str, error: AreteError) -> None:
        self._fail(subscription_id, error)

    def begin_reconnect(self) -> None:
        for record in self._records.values():
            self._mark_refreshing(record)

    def fail_refreshing(self, error: AreteError) -> None:
        for subscription_id, record in list(self._records.items()):
            pending = record.refresh_future is not None and not record.refresh_future.done()
            if record.is_refreshing or pending:
                self._fail(subscription_id, error)

    def clear(self) -> None:
        error = AreteConnectionError(
            "Subscription store was cleared while refreshing", "CONNECTION_CANCELLED"
        )
        for record in self._records.values():
            self._reject_refresh(record, error)
        self._records.clear()
        self._entities.clear()
        self._seqs.clear()

    # -- internals ---------------------------------------------------------

    def _mark_refreshing(self, record: _Record) -> None:
        record.error = None
        record.is_loading = (not record.resolved) and record.subscription.snapshot_enabled
        record.is_refreshing = record.resolved and record.subscription.snapshot_enabled
        record.staged = None
        self._touch(record)

    def _set_entity(self, view: str, key: str, data: Any, seq: Optional[str]) -> None:
        self._entities.setdefault(view, {})[key] = data
        if seq is not None:
            self._seqs.setdefault(view, {})[key] = seq

    def _make_rich(self, key: str, previous: Any, after: Any, patch: Any = None) -> RichUpdate:
        if previous is _MISSING:
            return RichUpdate(type="created", key=key, data=after)
        return RichUpdate(type="updated", key=key, before=previous, after=after, patch=patch)

    def _fail(self, subscription_id: str, error: AreteError) -> None:
        record = self._records.get(subscription_id)
        if record is None:
            return
        record.error = error
        record.is_loading = False
        record.is_refreshing = False
        record.staged = None
        self._touch(record)
        self._reject_refresh(record, error)

    def _resolve_refresh(self, record: _Record) -> None:
        future = record.refresh_future
        record.refresh_future = None
        if future is not None and not future.done():
            future.set_result(None)

    def _reject_refresh(self, record: _Record, error: AreteError) -> None:
        future = record.refresh_future
        record.refresh_future = None
        if future is not None and not future.done():
            future.set_exception(error)
            future.exception()  # mark retrieved so abandonment never warns

    def _require_record(self, subscription_id: str) -> _Record:
        record = self._records.get(subscription_id)
        if record is None:
            raise SubscriptionError(
                f"Unknown local subscription '{subscription_id}'", "SUBSCRIPTION_NOT_FOUND"
            )
        return record

    def _touch(self, record: _Record) -> None:
        for listener in list(record.change_listeners):
            listener()

    def _emit_update(self, record: _Record, update: Update) -> None:
        for listener in list(record.update_listeners):
            listener(update)

    def _emit_rich_update(self, record: _Record, update: RichUpdate) -> None:
        for listener in list(record.rich_update_listeners):
            listener(update)
