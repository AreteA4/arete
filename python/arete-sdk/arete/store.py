import asyncio
import bisect
import copy
import logging
from collections import OrderedDict
from typing import (
    Any,
    Dict,
    Optional,
    TypeVar,
    AsyncIterator,
    Generic,
    Callable,
    List,
    Tuple,
    Union,
)
from dataclasses import dataclass
from enum import Enum

from arete.types import SortConfig, SortOrder, SnapshotEntity

logger = logging.getLogger(__name__)

T = TypeVar("T")

DEFAULT_MAX_ENTRIES_PER_VIEW = 10_000


class Mode(Enum):
    STATE = "state"
    LIST = "list"
    APPEND = "append"


def _extract_sort_value(data: Any, field_path: List[str]) -> Any:
    current = data
    for segment in field_path:
        if isinstance(current, dict):
            current = current.get(segment)
            if current is None:
                return None
        else:
            return None
    return current


def _make_sort_key(value: Any, key: str, order: SortOrder) -> Tuple[Any, str]:
    if value is None:
        sort_val: Any = (0, None)
    elif isinstance(value, bool):
        bool_val = not value if order == SortOrder.DESC else value
        sort_val = (1, bool_val)
    elif isinstance(value, (int, float)):
        num_val = -value if order == SortOrder.DESC else value
        sort_val = (2, num_val)
    elif isinstance(value, str):
        sort_val = (3, value) if order == SortOrder.ASC else (3, value)
    else:
        sort_val = (4, str(value))

    if order == SortOrder.DESC and isinstance(value, str):
        return (sort_val, key)

    return (sort_val, key)


@dataclass
class Update(Generic[T]):
    key: str
    data: T
    op: str = "upsert"


@dataclass
class RichUpdate(Generic[T]):
    type: str  # "created" | "updated" | "deleted"
    key: str
    before: Optional[T]
    after: Optional[T]
    patch: Optional[Dict[str, Any]] = None


class Store(Generic[T]):
    def __init__(
        self,
        mode: Mode = Mode.LIST,
        parser: Optional[Callable[[Dict[str, Any]], T]] = None,
        view: Optional[str] = None,
        max_entries: Optional[int] = DEFAULT_MAX_ENTRIES_PER_VIEW,
        sort_config: Optional[SortConfig] = None,
    ):
        self.mode = mode
        self.parser = parser
        self.view = view
        self.max_entries = max_entries
        self.sort_config = sort_config
        self._lock = asyncio.Lock()
        self._callbacks: List[Callable[[Update[T]], None]] = []
        self._rich_callbacks: List[Callable[[RichUpdate[T]], None]] = []
        self._update_queue: asyncio.Queue[Update[T]] = asyncio.Queue()
        self._ready = asyncio.Event()
        self._last_seq: Optional[str] = None

        if mode in (Mode.LIST, Mode.STATE):
            self._data: Union[OrderedDict[str, T], List[T]] = OrderedDict()
        else:
            self._data: Union[OrderedDict[str, T], List[T]] = []

        self._sorted_keys: List[Tuple[Any, str]] = []
        self._key_to_sort_key: Dict[str, Tuple[Any, str]] = {}

    def set_sort_config(self, config: SortConfig) -> None:
        if self.sort_config is not None:
            return
        self.sort_config = config
        self._rebuild_sorted_keys()

    def _rebuild_sorted_keys(self) -> None:
        self._sorted_keys.clear()
        self._key_to_sort_key.clear()
        if self.sort_config is None or not isinstance(self._data, OrderedDict):
            return

        for key, value in self._data.items():
            raw_data = value if isinstance(value, dict) else {}
            sort_value = _extract_sort_value(raw_data, self.sort_config.field)
            sort_key = _make_sort_key(sort_value, key, self.sort_config.order)
            self._key_to_sort_key[key] = sort_key
            bisect.insort(self._sorted_keys, sort_key)

    def _insert_sorted(self, key: str, data: Any) -> None:
        if self.sort_config is None:
            return

        if key in self._key_to_sort_key:
            old_sort_key = self._key_to_sort_key[key]
            idx = bisect.bisect_left(self._sorted_keys, old_sort_key)
            if idx < len(self._sorted_keys) and self._sorted_keys[idx] == old_sort_key:
                self._sorted_keys.pop(idx)

        raw_data = data if isinstance(data, dict) else {}
        sort_value = _extract_sort_value(raw_data, self.sort_config.field)
        sort_key = _make_sort_key(sort_value, key, self.sort_config.order)
        self._key_to_sort_key[key] = sort_key
        bisect.insort(self._sorted_keys, sort_key)

    def _remove_sorted(self, key: str) -> None:
        if self.sort_config is None or key not in self._key_to_sort_key:
            return

        old_sort_key = self._key_to_sort_key.pop(key)
        idx = bisect.bisect_left(self._sorted_keys, old_sort_key)
        if idx < len(self._sorted_keys) and self._sorted_keys[idx] == old_sort_key:
            self._sorted_keys.pop(idx)

    def _enforce_max_entries(self) -> None:
        if self.max_entries is None:
            return
        if isinstance(self._data, OrderedDict):
            while len(self._data) > self.max_entries:
                if self.sort_config is not None and self._sorted_keys:
                    last_sort_key = self._sorted_keys.pop()
                    evicted_key = last_sort_key[1]
                    self._key_to_sort_key.pop(evicted_key, None)
                    self._data.pop(evicted_key, None)
                else:
                    self._data.popitem(last=False)

    def get(self, key: Optional[str] = None) -> Optional[T]:
        if isinstance(self._data, OrderedDict):
            if key is None:
                if self.sort_config is not None and self._sorted_keys:
                    first_key = self._sorted_keys[0][1]
                    return self._data.get(first_key)
                return next(iter(self._data.values()), None)
            if key in self._data and self.sort_config is None:
                self._data.move_to_end(key)
            return self._data.get(key)
        else:
            return self._data[0] if self._data else None

    def list_sorted(self) -> List[T]:
        if not isinstance(self._data, OrderedDict):
            return list(self._data)
        if self.sort_config is not None and self._sorted_keys:
            return [
                self._data[sort_key[1]]
                for sort_key in self._sorted_keys
                if sort_key[1] in self._data
            ]
        return list(self._data.values())

    def keys_sorted(self) -> List[str]:
        if not isinstance(self._data, OrderedDict):
            return []
        if self.sort_config is not None and self._sorted_keys:
            return [sort_key[1] for sort_key in self._sorted_keys]
        return list(self._data.keys())

    async def get_async(self, key: Optional[str] = None) -> Optional[T]:
        async with self._lock:
            return self.get(key)

    async def list_sorted_async(self) -> List[T]:
        async with self._lock:
            return self.list_sorted()

    def subscribe(self, callback: Callable[[Update[T]], None]) -> None:
        if callback not in self._callbacks:
            self._callbacks.append(callback)

    def unsubscribe(self, callback: Callable[[Update[T]], None]) -> None:
        if callback in self._callbacks:
            self._callbacks.remove(callback)

    def subscribe_rich(self, callback: Callable[[RichUpdate[T]], None]) -> Callable[[], None]:
        self._rich_callbacks.append(callback)
        def unsub() -> None:
            try:
                self._rich_callbacks.remove(callback)
            except ValueError:
                pass
        return unsub

    async def rich_updates(self) -> AsyncIterator[RichUpdate[T]]:
        queue: asyncio.Queue[RichUpdate[T]] = asyncio.Queue()
        unsub = self.subscribe_rich(queue.put_nowait)
        try:
            while True:
                yield await queue.get()
        finally:
            unsub()

    async def wait_until_ready(self, timeout: float = 5.0) -> bool:
        """Block until initial snapshot completes. Returns False on timeout."""
        try:
            await asyncio.wait_for(self._ready.wait(), timeout=timeout)
            return True
        except asyncio.TimeoutError:
            return False

    @property
    def last_seq(self) -> Optional[str]:
        return self._last_seq

    def clear(self) -> None:
        if isinstance(self._data, OrderedDict):
            self._data.clear()
        else:
            self._data.clear()
        self._sorted_keys.clear()
        self._key_to_sort_key.clear()
        self._ready.clear()
        self._last_seq = None

    async def apply_snapshot(self, entities: List[SnapshotEntity], complete: bool) -> None:
        async with self._lock:
            if complete:
                if isinstance(self._data, OrderedDict):
                    self._data.clear()
                else:
                    self._data.clear()
                self._sorted_keys.clear()
                self._key_to_sort_key.clear()
            for entity in entities:
                parsed = self._parse_data(entity.data)
                if isinstance(self._data, OrderedDict):
                    self._data[entity.key] = parsed
                    if self.sort_config is not None:
                        self._insert_sorted(entity.key, entity.data)
                else:
                    self._data.append(parsed)
        for entity in entities:
            parsed = self._parse_data(entity.data)
            update = Update(key=entity.key, data=parsed, op="snapshot")
            await self._update_queue.put(update)
            for cb in list(self._callbacks):
                try:
                    result = cb(update)
                    if asyncio.iscoroutine(result):
                        await result
                except Exception as e:
                    logger.error("Callback error: %s", e)
        self._ready.set()

    def _parse_data(self, data: Any) -> T:
        """Parse raw data using the custom parser if provided."""
        if self.parser:
            return self.parser(data)
        return data

    async def __aiter__(self) -> AsyncIterator[Update[T]]:
        while True:
            update = await self._update_queue.get()
            yield update

    async def apply_patch(
        self, key: str, patch: Dict[str, Any], append_paths: Optional[List[str]] = None
    ) -> None:
        if append_paths is None:
            append_paths = []
        async with self._lock:
            if isinstance(self._data, OrderedDict):
                current = self._data.get(key, {})
                if isinstance(current, dict):
                    merged = self._deep_merge_with_append(current, patch, append_paths)
                else:
                    merged = patch
                parsed_data = self._parse_data(merged)
                self._data[key] = parsed_data
                if self.sort_config is not None:
                    self._insert_sorted(key, merged)
                else:
                    self._data.move_to_end(key)
                self._enforce_max_entries()
                await self._notify_update(key, parsed_data)
            else:
                parsed_data = self._parse_data(patch)
                self._data.append(parsed_data)
                await self._notify_update(key, parsed_data)

    def _deep_merge_with_append(
        self,
        target: Dict[str, Any],
        source: Dict[str, Any],
        append_paths: List[str],
        current_path: str = "",
    ) -> Dict[str, Any]:
        result = {**target}
        for key, source_value in source.items():
            field_path = f"{current_path}.{key}" if current_path else key
            target_value = result.get(key)

            if isinstance(source_value, list) and isinstance(target_value, list):
                if field_path in append_paths:
                    result[key] = target_value + source_value
                else:
                    result[key] = source_value
            elif isinstance(source_value, dict) and isinstance(target_value, dict):
                result[key] = self._deep_merge_with_append(
                    target_value, source_value, append_paths, field_path
                )
            else:
                result[key] = source_value
        return result

    async def apply_upsert(self, key: str, value: T) -> None:
        async with self._lock:
            before = copy.deepcopy(self._data.get(key)) if isinstance(self._data, OrderedDict) else None
            parsed_data = self._parse_data(value)
            if isinstance(self._data, OrderedDict):
                self._data[key] = parsed_data
                if self.sort_config is not None:
                    self._insert_sorted(key, value)
                else:
                    self._data.move_to_end(key)
                self._enforce_max_entries()
            else:
                self._data.append(parsed_data)
        await self._notify_update(key, parsed_data, op="upsert")
        rich = RichUpdate(
            type="created" if before is None else "updated",
            key=key, before=before, after=parsed_data,
        )
        await self._notify_rich(rich)

    async def apply_delete(self, key: str) -> None:
        before = None
        async with self._lock:
            if isinstance(self._data, OrderedDict) and key in self._data:
                before = copy.deepcopy(self._data[key])
                self._remove_sorted(key)
                del self._data[key]
        if before is not None:
            await self._notify_update(key, None, op="delete")  # type: ignore[arg-type]
            await self._notify_rich(RichUpdate(type="deleted", key=key, before=before, after=None))

    async def handle_frame(self, frame) -> None:
        if frame.op == "subscribed":
            self._ready.set()
            return
        if hasattr(frame, "seq") and frame.seq:
            self._last_seq = frame.seq
        if frame.op == "upsert" or frame.op == "create":
            await self.apply_upsert(frame.key, frame.data)
        elif frame.op == "patch":
            append_paths = getattr(frame, "append", []) or []
            await self.apply_patch(frame.key, frame.data, append_paths)
        elif frame.op == "delete":
            await self.apply_delete(frame.key)
        else:
            await self.apply_upsert(frame.key, frame.data)

    async def _notify_rich(self, rich: RichUpdate[T]) -> None:
        for cb in list(self._rich_callbacks):
            try:
                result = cb(rich)
                if asyncio.iscoroutine(result):
                    await result
            except Exception as e:
                logger.error("Rich callback error: %s", e)

    async def _notify_update(self, key: str, data: T, op: str = "upsert") -> None:
        update = Update(key=key, data=data, op=op)
        await self._update_queue.put(update)
        for callback in list(self._callbacks):
            try:
                result = callback(update)
                if asyncio.iscoroutine(result):
                    await result
            except Exception as e:
                logger.error("Callback error: %s", e)
