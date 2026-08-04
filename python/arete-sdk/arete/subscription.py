"""Canonical protocol v2 subscription identity and the refcounted registry.

Mirror of ``typescript/core/src/subscription.ts``. The canonical query key is
the compact JSON of ``{"query": ..., "snapshot": ...}`` with the exact TS
field order (``view, key, partition, filters, take, skip, after,
snapshotLimit``; filter keys sorted) so identities are byte-identical across
SDKs. Equivalent queries share one wire subscription; leases are
reference-counted; subscription ids stay stable across reconnect.

Collation
---------
TS orders filter keys with ``String.prototype.localeCompare`` (``subscription.ts``
lines 57 and 123), which is a UCA/DUCET collation — *not* a code-point sort.
Sorting by Python code point would emit a different ``filters`` key order (and
therefore a different canonical identity) whenever keys differ only by case or
carry non-ASCII letters. :func:`locale_compare` here is the shared
stdlib-only equivalent; ``arete.store`` imports it for its own ordering
(mirroring ``query-store.ts`` lines 64 and 387).

:func:`locale_compare` builds a three-level UCA-style sort key (primary = base
letter with case and diacritics removed, secondary = diacritics, tertiary =
case with lowercase first) and is verified against Node's default
``en-US``/ICU collator.

**Replicated exactly** (verified against Node v23 ``localeCompare``):

* All printable ASCII, in ICU order — whitespace and punctuation before digits
  before letters, with the exact ICU punctuation sequence
  (``_ - , ; : ! ? . ' " ( ) [ ] { } @ * / \\ & # % ` ^ + < = > | ~ $``).
  This covers base58 keys and dotted filter paths, the only inputs the SDK
  produces in practice.
* Case as a tertiary difference with lowercase first (``a`` < ``A``,
  ``test`` < ``Test``, ``aBc1`` < ``apple`` < ``Bqq`` < ``Zap1``).
* Canonically decomposable accented Latin as a secondary difference, in ICU's
  diacritic order (``a`` < ``á`` < ``à`` < ``ă`` < ``â`` < ``ǎ`` < ``å`` <
  ``ä`` < ``a̋`` < ``ã`` < ``ȧ`` … ), so ``etat`` < ``état`` and
  ``resume`` < ``résumé``.
* Level-by-level comparison over the whole string (a secondary difference
  anywhere loses to a primary difference anywhere).
* Canonical equivalence: NFC and NFD spellings compare equal.
* Completely ignorable characters (``Cc``/``Cf``: NUL, soft hyphen, ZWSP)
  contribute nothing, so ``"a\\u200bb" == "ab"``.
* Non-Latin *letters* fold by case, so Greek/Cyrillic order alphabetically
  (``α`` < ``Ω``) rather than by code point.
* A small Latin fold table: ``ß``/``æ``/``œ`` expand to ``ss``/``ae``/``oe``
  and sort just after them; ``ø đ ł ŧ`` sort as stroked ``o d l t``.
* Empty and equal strings (``"" < "a"``, ``"" == ""``).

**Approximated** (sign may differ from ICU; none of these occur in wire data):

* Non-ASCII punctuation, symbols and non-Latin scripts order by code point
  within their band, not by DUCET weight — so ``U+3000`` sorts after ASCII
  punctuation instead of with whitespace, and CJK/Greek/Cyrillic relative
  script order follows code point.
* Non-ASCII decimal digits sort after ASCII digits instead of interleaving by
  numeric value.
* Latin letters with no canonical decomposition and no fold-table entry
  (``ð þ ı ŋ``) fall back to the code-point tail of the letter band, so they
  sort after ``z`` instead of next to their base letter.
* Combining marks outside the modelled table order by code point, after all
  modelled marks.
* ICU contractions/expansions beyond the fold table above are not modelled.

Ties return ``0`` exactly as ``localeCompare`` does; both TS ``Array#sort``
and Python ``sorted`` are stable, so tied elements keep input order in both.
"""

from __future__ import annotations

import asyncio
import json
import math
import unicodedata
import uuid
from dataclasses import dataclass, field
from functools import lru_cache
from typing import TYPE_CHECKING, Any, Callable, Dict, List, Mapping, Optional, Tuple

from arete.errors import AreteConnectionError, AreteError, SubscriptionError
from arete.wire import (
    QUERY_FIELDS,
    RichUpdate,
    Update,
    is_valid_subscription_id,
    subscribe_envelope,
)

if TYPE_CHECKING:  # pragma: no cover - typing only
    from arete.store import QueryResult, Store


# -- collation (JS String.prototype.localeCompare equivalent) ---------------
#
# Weights are transcribed from Node's default en-US ICU collator; see the
# module docstring for exactly what is replicated and what is approximated.

# Printable ASCII punctuation/whitespace in ICU primary order.
_ASCII_PUNCTUATION_ORDER = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$"
_ASCII_PUNCTUATION_RANK = {
    character: index for index, character in enumerate(_ASCII_PUNCTUATION_ORDER)
}

# Combining marks in ICU secondary order (acute before grave before breve …).
_SECONDARY_MARK_ORDER = (
    "̓", "̔", "́", "̀", "̆", "̂", "̌",
    "̊", "̈", "̋", "̃", "̇", "̸", "̧",
    "̨", "̄", "̵", "̉", "̏", "̑", "̛",
    "̣", "̦", "̱",
)
_SECONDARY_MARK_RANK = {
    mark: index + 1 for index, mark in enumerate(_SECONDARY_MARK_ORDER)
}
# Sentinel appended by an expansion so that "ss" < "ß" / "ae" < "æ" the way
# ICU's tertiary expansion weights do. Sorts after every modelled mark.
_EXPANSION_MARK = "￿"
_SECONDARY_MARK_RANK[_EXPANSION_MARK] = len(_SECONDARY_MARK_ORDER) + 1
_UNMODELLED_MARK_BASE = 0x10000  # unmodelled marks trail the modelled ones

# Latin letters ICU treats as expansions of, or stroked forms of, ASCII bases.
_LATIN_FOLD = {
    "ß": "ss" + _EXPANSION_MARK,  # ß
    "ẞ": "SS" + _EXPANSION_MARK,  # ẞ
    "æ": "ae" + _EXPANSION_MARK,  # æ
    "Æ": "AE" + _EXPANSION_MARK,  # Æ
    "œ": "oe" + _EXPANSION_MARK,  # œ
    "Œ": "OE" + _EXPANSION_MARK,  # Œ
    "ø": "o̸",  # ø
    "Ø": "O̸",  # Ø
    "đ": "d̵",  # đ
    "Đ": "D̵",  # Đ
    "ł": "l̵",  # ł
    "Ł": "L̵",  # Ł
    "ŧ": "t̵",  # ŧ
    "Ŧ": "T̵",  # Ŧ
}

_BAND_PUNCTUATION = 0  # whitespace, punctuation, symbols
_BAND_DIGIT = 1
_BAND_LETTER = 2  # letters and everything not otherwise classified

_CollationKey = Tuple[Tuple[Tuple[int, int], ...], Tuple[int, ...], Tuple[int, ...]]


def _lowercased(character: str) -> str:
    """``str.lower`` restricted to single-character results (``İ`` expands)."""
    lowered = character.lower()
    return lowered if len(lowered) == 1 else character


def _primary_weight(character: str) -> Tuple[int, int]:
    lowered = _lowercased(character)
    ascii_rank = _ASCII_PUNCTUATION_RANK.get(lowered)
    if ascii_rank is not None:
        return (_BAND_PUNCTUATION, ascii_rank)
    category = unicodedata.category(lowered)
    if category == "Nd":
        return (_BAND_DIGIT, ord(lowered))
    if category[0] in ("P", "S", "Z"):
        return (_BAND_PUNCTUATION, ord(lowered))
    return (_BAND_LETTER, ord(lowered))


def _secondary_weight(mark: str) -> int:
    rank = _SECONDARY_MARK_RANK.get(mark)
    return rank if rank is not None else _UNMODELLED_MARK_BASE + ord(mark)


def _folded_nfd(text: str) -> str:
    decomposed = unicodedata.normalize("NFD", text)
    if not any(character in _LATIN_FOLD for character in decomposed):
        return decomposed
    return "".join(_LATIN_FOLD.get(character, character) for character in decomposed)


@lru_cache(maxsize=8192)
def collation_key(text: str) -> _CollationKey:
    """Three-level UCA-style sort key: (primary, secondary, tertiary).

    Sorting by this key is equivalent to sorting with :func:`locale_compare`.
    """
    primary: List[Tuple[int, int]] = []
    secondary: List[int] = []
    tertiary: List[int] = []
    for character in _folded_nfd(text):
        if character == _EXPANSION_MARK or unicodedata.combining(character):
            # Marks are primary-ignorable: they only add a secondary weight.
            secondary.append(_secondary_weight(character))
            tertiary.append(0)
            continue
        if unicodedata.category(character) in ("Cc", "Cf"):
            continue  # completely ignorable in DUCET
        primary.append(_primary_weight(character))
        secondary.append(0)
        tertiary.append(0 if character == _lowercased(character) else 1)
    return (tuple(primary), tuple(secondary), tuple(tertiary))


def locale_compare(left: str, right: str) -> int:
    """``left.localeCompare(right)`` for the cases the SDK actually produces.

    Returns ``-1`` / ``0`` / ``1``. See the module docstring for the precise
    fidelity envelope against Node's default ICU collator.
    """
    if left == right:
        return 0
    left_key = collation_key(left)
    right_key = collation_key(right)
    if left_key < right_key:
        return -1
    if left_key > right_key:
        return 1
    return 0


def validate_subscription_id(subscription_id: str) -> None:
    if not isinstance(subscription_id, str) or len(subscription_id) == 0 \
            or subscription_id.strip() != subscription_id:
        raise TypeError("subscriptionId must be non-empty with no surrounding whitespace")
    if not is_valid_subscription_id(subscription_id):
        if len(subscription_id.encode("utf-8")) > 128:
            raise TypeError("subscriptionId must not exceed 128 bytes")
        raise TypeError("subscriptionId must not contain control characters")


def create_subscription_id() -> str:
    return f"a4-{uuid.uuid4()}"


def _canonical_json_value(value: Any, path: str) -> Any:
    if value is None or isinstance(value, (str, bool)):
        return value
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise TypeError(f"{path} must contain JSON values")
        # JSON.stringify(2.0) is "2"; keep canonical output byte-identical.
        return int(value) if value.is_integer() else value
    if isinstance(value, (list, tuple)):
        return [
            _canonical_json_value(entry, f"{path}[{index}]")
            for index, entry in enumerate(value)
        ]
    if isinstance(value, Mapping):
        # TS subscription.ts:57 — localeCompare, not a code-point sort.
        return {
            key: _canonical_json_value(entry, f"{path}.{key}")
            for key, entry in sorted(value.items(), key=lambda item: collation_key(str(item[0])))
        }
    raise TypeError(f"{path} must contain JSON values")


def _assert_positive_integer(value: Any, field_name: str) -> None:
    if value is not None and (isinstance(value, bool) or not isinstance(value, int) or value <= 0):
        raise TypeError(f"{field_name} must be a positive integer")


def normalize_query(query: Mapping[str, Any]) -> Dict[str, Any]:
    """Validate a wire-shaped query and return it in canonical field order."""
    view = query.get("view")
    if not isinstance(view, str) or len(view) == 0:
        raise TypeError("query.view must be a non-empty string")
    for key in query:
        if key not in QUERY_FIELDS:
            raise TypeError("query contains an unknown protocol v2 field")
    key_value = query.get("key")
    if key_value is not None and not isinstance(key_value, str):
        raise TypeError("query.key must be a string")
    partition = query.get("partition")
    if partition is not None and not isinstance(partition, str):
        raise TypeError("query.partition must be a string")
    filters = query.get("filters")
    if filters is not None and not isinstance(filters, Mapping):
        raise TypeError("query.filters must be an object")
    after = query.get("after")
    if after is not None and not isinstance(after, str):
        raise TypeError("query.after must be a string")
    _assert_positive_integer(query.get("take"), "query.take")
    _assert_positive_integer(query.get("snapshotLimit"), "query.snapshotLimit")
    skip = query.get("skip")
    if skip is not None and (isinstance(skip, bool) or not isinstance(skip, int) or skip < 0):
        raise TypeError("query.skip must be a non-negative integer")

    normalized: Dict[str, Any] = {"view": view}
    if key_value is not None:
        normalized["key"] = key_value
    if partition is not None:
        normalized["partition"] = partition
    if filters is not None:
        # TS subscription.ts:123 — localeCompare, not a code-point sort.
        normalized["filters"] = {
            path: _canonical_json_value(value, f"query.filters.{path}")
            for path, value in sorted(filters.items(), key=lambda item: collation_key(str(item[0])))
        }
    if query.get("take") is not None:
        normalized["take"] = query["take"]
    if skip is not None:
        normalized["skip"] = skip
    if after is not None:
        normalized["after"] = after
    if query.get("snapshotLimit") is not None:
        normalized["snapshotLimit"] = query["snapshotLimit"]
    return normalized


def canonical_query_key(query: Mapping[str, Any], snapshot_enabled: bool = True) -> str:
    """Canonical subscription identity: compact JSON of {query, snapshot}."""
    if not isinstance(snapshot_enabled, bool):
        raise TypeError("snapshot.enabled must be a boolean")
    return json.dumps(
        {"query": normalize_query(query), "snapshot": {"enabled": snapshot_enabled}},
        separators=(",", ":"),
        ensure_ascii=False,
    )


@dataclass(frozen=True)
class Subscription:
    """A protocol v2 wire subscription with a stable client-selected id."""

    subscription_id: str
    query: Mapping[str, Any]  # normalized, wire (camelCase) field names
    snapshot_enabled: bool = True

    def to_wire(self) -> dict:
        return subscribe_envelope(self.subscription_id, self.query, self.snapshot_enabled)

    def canonical_identity(self) -> str:
        return canonical_query_key(self.query, self.snapshot_enabled)


@dataclass
class _Tracker:
    subscription: Subscription
    query_key: str
    ref_count: int = 1
    refresh_future: Optional["asyncio.Future[None]"] = None


class QueryLease:
    """One reference to a shared wire subscription.

    ``release()`` (or breaking the consuming stream) decrements the refcount;
    the wire subscription is cancelled when the final lease is released.
    """

    def __init__(self, registry: "SubscriptionRegistry", tracker: _Tracker) -> None:
        self._registry = registry
        self._tracker = tracker
        self._released = False

    @property
    def subscription(self) -> Subscription:
        return self._tracker.subscription

    @property
    def query_key(self) -> str:
        return self._tracker.query_key

    def get_result(self) -> "QueryResult":
        result = self._registry._store.get_result(self._tracker.subscription.subscription_id)
        if result is None:
            raise SubscriptionError("Query lease has been released", "SUBSCRIPTION_NOT_FOUND")
        return result

    def on_change(self, callback: Callable[[], None]) -> Callable[[], None]:
        return self._registry._store.on_change(
            self._tracker.subscription.subscription_id, callback
        )

    def on_update(self, callback: Callable[[Update], None]) -> Callable[[], None]:
        return self._registry._store.on_update(
            self._tracker.subscription.subscription_id, callback
        )

    def on_rich_update(self, callback: Callable[[RichUpdate], None]) -> Callable[[], None]:
        return self._registry._store.on_rich_update(
            self._tracker.subscription.subscription_id, callback
        )

    def refresh(self) -> "asyncio.Future[None]":
        return self._registry._refresh_tracker(self._tracker)

    def release(self) -> None:
        if self._released:
            return
        self._released = True
        self._registry._release_tracker(self._tracker)


def _completed_future() -> "asyncio.Future[None]":
    future: "asyncio.Future[None]" = asyncio.get_event_loop().create_future()
    future.set_result(None)
    return future


def _rejected_future(error: BaseException) -> "asyncio.Future[None]":
    future: "asyncio.Future[None]" = asyncio.get_event_loop().create_future()
    future.set_exception(error)
    future.exception()  # mark retrieved so abandonment never warns
    return future


def _wrap_refresh_error(value: BaseException) -> AreteError:
    if isinstance(value, AreteError):
        return value
    return SubscriptionError(str(value) or "Subscription refresh failed", "SUBSCRIPTION_ERROR", value)


class SubscriptionRegistry:
    """Refcounted registry of canonical queries → stable wire subscriptions."""

    def __init__(self, connection: Any, store: "Store") -> None:
        self._connection = connection
        self._store = store
        self._by_key: Dict[str, _Tracker] = {}
        self._by_id: Dict[str, _Tracker] = {}

    def subscribe(self, query: Mapping[str, Any], snapshot_enabled: bool = True) -> QueryLease:
        normalized = normalize_query(query)
        query_key = canonical_query_key(normalized, snapshot_enabled)
        tracker = self._by_key.get(query_key)

        if tracker is not None:
            tracker.ref_count += 1
        else:
            subscription = Subscription(
                subscription_id=create_subscription_id(),
                query=normalized,
                snapshot_enabled=snapshot_enabled,
            )
            tracker = _Tracker(subscription=subscription, query_key=query_key)
            self._store.register(subscription, query_key)
            try:
                self._connection.subscribe(subscription)
            except BaseException:
                self._store.unregister(subscription.subscription_id)
                raise
            self._by_key[query_key] = tracker
            self._by_id[subscription.subscription_id] = tracker

        return QueryLease(self, tracker)

    def refresh(
        self, query: Mapping[str, Any], snapshot_enabled: bool = True
    ) -> "asyncio.Future[None]":
        tracker = self._get_tracker(query, snapshot_enabled)
        if tracker is None:
            return _rejected_future(SubscriptionError(
                f"Cannot refresh inactive query '{query.get('view')}'",
                "SUBSCRIPTION_NOT_FOUND",
            ))
        return self._refresh_tracker(tracker)

    def refresh_view(self, view: str, key: Optional[str] = None) -> "asyncio.Future[None]":
        """Refresh every active subscription for a view (optionally one key).

        Resolves immediately when nothing matches — a no-op, not an error.
        """
        matches = [
            tracker for tracker in list(self._by_key.values())
            if tracker.subscription.query.get("view") == view
            and (key is None or tracker.subscription.query.get("key") == key)
        ]
        if not matches:
            return _completed_future()
        futures = [self._refresh_tracker(tracker) for tracker in matches]
        return asyncio.ensure_future(_gather_refreshes(futures))

    def get_ref_count(self, query: Mapping[str, Any], snapshot_enabled: bool = True) -> int:
        tracker = self._get_tracker(query, snapshot_enabled)
        return tracker.ref_count if tracker is not None else 0

    def get_active_subscriptions(self) -> List[Subscription]:
        return [tracker.subscription for tracker in self._by_key.values()]

    def get_query_result(
        self, query: Mapping[str, Any], snapshot_enabled: bool = True
    ) -> Optional["QueryResult"]:
        tracker = self._get_tracker(query, snapshot_enabled)
        if tracker is None:
            return None
        return self._store.get_result(tracker.subscription.subscription_id)

    def handle_connection_state(self, state: str) -> None:
        if state == "reconnecting":
            self._store.begin_reconnect()
        if state == "error":
            self._store.fail_refreshing(AreteConnectionError(
                "Connection failed while refreshing subscriptions", "CONNECTION_ERROR"
            ))

    def clear(self) -> None:
        for tracker in list(self._by_key.values()):
            try:
                self._connection.unsubscribe(tracker.subscription.subscription_id)
            except Exception:
                pass  # Local release must complete even when the socket cannot send.
            finally:
                self._store.unregister(tracker.subscription.subscription_id)
        self._by_key.clear()
        self._by_id.clear()

    # -- internal ----------------------------------------------------------

    def _get_tracker(
        self, query: Mapping[str, Any], snapshot_enabled: bool
    ) -> Optional[_Tracker]:
        return self._by_key.get(canonical_query_key(query, snapshot_enabled))

    def _refresh_tracker(self, tracker: _Tracker) -> "asyncio.Future[None]":
        subscription_id = tracker.subscription.subscription_id
        if self._by_id.get(subscription_id) is not tracker:
            return _rejected_future(SubscriptionError(
                "Cannot refresh a released query lease", "SUBSCRIPTION_NOT_FOUND"
            ))
        if tracker.refresh_future is not None and not tracker.refresh_future.done():
            return tracker.refresh_future

        wait_for_snapshot = tracker.subscription.snapshot_enabled
        completion = self._store.begin_refresh(subscription_id, wait_for_snapshot)
        try:
            self._connection.refresh(tracker.subscription)
        except BaseException as exc:
            error = _wrap_refresh_error(exc)
            self._store.fail_refresh(subscription_id, error)
            if wait_for_snapshot:
                return completion  # already rejected via fail_refresh
            return _rejected_future(error)

        tracker.refresh_future = completion
        return completion

    def _release_tracker(self, tracker: _Tracker) -> None:
        if self._by_id.get(tracker.subscription.subscription_id) is not tracker:
            return
        tracker.ref_count -= 1
        if tracker.ref_count > 0:
            return

        del self._by_key[tracker.query_key]
        del self._by_id[tracker.subscription.subscription_id]
        try:
            self._connection.unsubscribe(tracker.subscription.subscription_id)
        except Exception:
            pass  # Connection already removed local membership before sending.
        finally:
            self._store.unregister(tracker.subscription.subscription_id)


async def _gather_refreshes(futures: List["asyncio.Future[None]"]) -> None:
    await asyncio.gather(*futures)
