"""Tests for arete.subscription: canonical identity + refcounted registry.

Ports the meaningful cases from ``typescript/core/src/subscription.test.ts``.
"""

from __future__ import annotations

import asyncio

import pytest

from arete.errors import AreteConnectionError, SubscriptionError
from arete.store import Store
from arete.subscription import (
    SubscriptionRegistry,
    canonical_query_key,
    create_subscription_id,
    normalize_query,
    validate_subscription_id,
)
from arete.wire import SnapshotEntity, SnapshotFrame


class FakeConnection:
    def __init__(self):
        self.subscribed = []
        self.unsubscribed = []
        self.refreshed = []
        self.subscribe_error = None
        self.unsubscribe_error = None
        self.refresh_error = None

    def subscribe(self, subscription):
        if self.subscribe_error is not None:
            error, self.subscribe_error = self.subscribe_error, None
            raise error
        self.subscribed.append(subscription)

    def unsubscribe(self, subscription_id):
        if self.unsubscribe_error is not None:
            error, self.unsubscribe_error = self.unsubscribe_error, None
            raise error
        self.unsubscribed.append(subscription_id)

    def refresh(self, subscription):
        if self.refresh_error is not None:
            error, self.refresh_error = self.refresh_error, None
            raise error
        self.refreshed.append(subscription)


def create_registry():
    connection = FakeConnection()
    store = Store()
    registry = SubscriptionRegistry(connection, store)
    return connection, store, registry


def snapshot_frame(subscription_id, snapshot_id, entries, *, entity="Round/list",
                   mode="list", authoritative=True, complete=True, key=None):
    return SnapshotFrame(
        subscription_id=subscription_id,
        snapshot_id=snapshot_id,
        authoritative=authoritative,
        mode=mode,
        entity=entity,
        key=key,
        data=tuple(SnapshotEntity(key=k, data=v) for k, v in entries),
        complete=complete,
    )


class TestCanonicalIdentity:
    BASE = {
        "view": "Thing/list",
        "key": "key",
        "partition": "partition",
        "filters": {"state.value": {"nested": True}},
        "take": 1,
        "skip": 0,
        "after": "1:0001",
        "snapshotLimit": 10,
    }

    def test_canonical_json_is_byte_identical_to_ts(self):
        # Exact JSON.stringify output of the TS canonicalization for this query.
        expected = (
            '{"query":{"view":"Thing/list","key":"key","partition":"partition",'
            '"filters":{"state.value":{"nested":true}},"take":1,"skip":0,'
            '"after":"1:0001","snapshotLimit":10},"snapshot":{"enabled":true}}'
        )
        assert canonical_query_key(self.BASE) == expected

    def test_field_order_is_canonical_not_insertion(self):
        shuffled = {
            "snapshotLimit": 10,
            "after": "1:0001",
            "skip": 0,
            "take": 1,
            "filters": {"state.value": {"nested": True}},
            "partition": "partition",
            "key": "key",
            "view": "Thing/list",
        }
        assert canonical_query_key(shuffled) == canonical_query_key(self.BASE)

    def test_filter_key_order_is_sorted(self):
        left = canonical_query_key({
            "view": "Position/list",
            "filters": {"owner": "wallet", "state.status": "open"},
        })
        right = canonical_query_key({
            "view": "Position/list",
            "filters": {"state.status": "open", "owner": "wallet"},
        })
        assert left == right
        assert '"filters":{"owner":"wallet","state.status":"open"}' in left

    def test_nested_filter_object_keys_are_sorted(self):
        left = canonical_query_key({
            "view": "V", "filters": {"f": {"b": 1, "a": 2}},
        })
        assert '"filters":{"f":{"a":2,"b":1}}' in left

    def test_filter_keys_sort_by_localecompare_not_code_point(self):
        # Finding 6: TS sorts filter keys with localeCompare
        # (subscription.ts:123). Node:
        #   ['état', 'zone'].sort((a, b) => a.localeCompare(b))
        #     -> ['état', 'zone']
        # A Python code-point sort yields ['zone', 'état'] instead, which
        # changes both the wire `filters` key order and the dedup identity.
        identity = canonical_query_key({
            "view": "Position/list",
            "filters": {"état": "x", "zone": "y"},
        })
        assert '"filters":{"état":"x","zone":"y"}' in identity
        # Insertion order must not matter.
        assert canonical_query_key({
            "view": "Position/list",
            "filters": {"zone": "y", "état": "x"},
        }) == identity

    def test_filter_keys_sort_case_insensitively_at_the_primary_level(self):
        # Node: ['Owner', 'address'].sort(localeCompare) -> ['address', 'Owner'];
        # a code-point sort would put 'Owner' (U+004F) first.
        identity = canonical_query_key({
            "view": "Position/list",
            "filters": {"Owner": "wallet", "address": "acct"},
        })
        assert '"filters":{"address":"acct","Owner":"wallet"}' in identity

    def test_nested_filter_object_keys_also_use_localecompare(self):
        # subscription.ts:57 — the nested canonicalJsonValue sort.
        identity = canonical_query_key({
            "view": "V", "filters": {"f": {"zone": 1, "état": 2, "Etat": 3}},
        })
        assert '"filters":{"f":{"Etat":3,"état":2,"zone":1}}' in identity

    def test_identity_includes_snapshot_and_every_field(self):
        identity = canonical_query_key(self.BASE, True)
        assert canonical_query_key(dict(self.BASE), True) == identity
        assert canonical_query_key(self.BASE, False) != identity
        assert canonical_query_key({**self.BASE, "skip": 1}) != identity

    def test_normalize_rejects_unknown_fields_and_bad_types(self):
        with pytest.raises(TypeError, match="unknown protocol v2 field"):
            normalize_query({"view": "V", "bogus": 1})
        with pytest.raises(TypeError, match="view must be a non-empty string"):
            normalize_query({"view": ""})
        with pytest.raises(TypeError, match="take must be a positive integer"):
            normalize_query({"view": "V", "take": 0})
        with pytest.raises(TypeError, match="skip must be a non-negative integer"):
            normalize_query({"view": "V", "skip": -1})
        with pytest.raises(TypeError, match="filters must be an object"):
            normalize_query({"view": "V", "filters": [1]})
        with pytest.raises(TypeError, match="must contain JSON values"):
            canonical_query_key({"view": "V", "filters": {"a": object()}})

    def test_subscription_id_validation(self):
        validate_subscription_id("rounds:page-1")
        with pytest.raises(TypeError):
            validate_subscription_id("")
        with pytest.raises(TypeError):
            validate_subscription_id(" padded")
        with pytest.raises(TypeError):
            validate_subscription_id("a\nb")
        with pytest.raises(TypeError):
            validate_subscription_id("é" * 65)
        assert create_subscription_id().startswith("a4-")


class TestRegistry:
    def test_refcounts_equivalent_canonical_queries_with_one_opaque_id(self):
        connection, _store, registry = create_registry()
        first = registry.subscribe({
            "view": "Position/list",
            "filters": {"owner": "wallet", "state.status": "open"},
            "take": 10,
            "skip": 0,
        })
        equivalent = registry.subscribe({
            "view": "Position/list",
            "filters": {"state.status": "open", "owner": "wallet"},
            "skip": 0,
            "take": 10,
        })

        assert len(connection.subscribed) == 1
        assert first.subscription.subscription_id.startswith("a4-")
        assert first.subscription.query == {
            "view": "Position/list",
            "filters": {"owner": "wallet", "state.status": "open"},
            "take": 10,
            "skip": 0,
        }
        assert first.subscription.snapshot_enabled is True
        assert (
            equivalent.subscription.subscription_id
            == first.subscription.subscription_id
        )
        assert registry.get_ref_count(first.subscription.query) == 2

        first.release()
        assert connection.unsubscribed == []
        equivalent.release()
        assert connection.unsubscribed == [first.subscription.subscription_id]

    def test_wire_envelope_shape(self):
        _connection, _store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/list", "take": 2})
        wire = lease.subscription.to_wire()
        assert wire["type"] == "subscribe"
        assert wire["protocolVersion"] == 2
        assert wire["query"] == {"view": "Round/list", "take": 2}
        assert wire["snapshot"] == {"enabled": True}

    def test_different_queries_on_same_view_coexist(self):
        connection, _store, registry = create_registry()
        first = registry.subscribe({"view": "Round/list", "take": 2, "skip": 0})
        second = registry.subscribe({"view": "Round/list", "take": 2, "skip": 2})
        third = registry.subscribe({
            "view": "Round/list", "filters": {"state.status": "open"}, "take": 2,
        })

        assert len(connection.subscribed) == 3
        assert len({
            first.subscription.subscription_id,
            second.subscription.subscription_id,
            third.subscription.subscription_id,
        }) == 3

    def test_does_not_retain_query_when_connection_rejects_registration(self):
        connection, store, registry = create_registry()
        connection.subscribe_error = RuntimeError("connection rejected subscription")

        with pytest.raises(RuntimeError, match="connection rejected"):
            registry.subscribe({"view": "Miner/state", "key": "wallet"})
        assert registry.get_ref_count({"view": "Miner/state", "key": "wallet"}) == 0
        assert registry.get_active_subscriptions() == []

    @pytest.mark.asyncio
    async def test_refresh_uses_stable_id_and_retains_membership(self):
        connection, store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/list", "take": 2})
        sid = lease.subscription.subscription_id
        store.handle_frame(snapshot_frame(sid, "initial", [("10", {"id": 10})]))

        refresh = lease.refresh()
        await asyncio.sleep(0)
        assert connection.refreshed[-1] is lease.subscription
        assert not refresh.done()

        result = lease.get_result()
        assert result.keys == ("10",)
        assert result.is_loading is False
        assert result.is_refreshing is True

        store.handle_frame(snapshot_frame(sid, "refreshed", [("10", {"id": 10})]))
        await refresh
        assert lease.get_result().is_refreshing is False

    @pytest.mark.asyncio
    async def test_release_rejects_pending_refresh(self):
        _connection, _store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/state", "key": "10"})
        refresh = lease.refresh()

        lease.release()

        with pytest.raises(SubscriptionError, match="released while refreshing"):
            await refresh

    @pytest.mark.asyncio
    async def test_reconnect_keeps_pending_refreshes_alive(self):
        _connection, store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/state", "key": "10"})
        refresh = lease.refresh()

        registry.handle_connection_state("reconnecting")
        assert lease.get_result().is_refreshing is False
        assert lease.get_result().error is None

        store.handle_frame(snapshot_frame(
            lease.subscription.subscription_id, "reconnected", [],
            entity="Round/state", mode="state",
        ))
        assert await refresh is None

    @pytest.mark.asyncio
    async def test_terminal_connection_error_rejects_pending_refreshes(self):
        _connection, _store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/state", "key": "10"})
        refresh = lease.refresh()

        registry.handle_connection_state("reconnecting")
        registry.handle_connection_state("error")

        with pytest.raises(
            AreteConnectionError, match="Connection failed while refreshing"
        ):
            await refresh
        result = lease.get_result()
        assert result.is_refreshing is False
        assert result.error is not None and result.error.code == "CONNECTION_ERROR"

    @pytest.mark.asyncio
    async def test_release_survives_unsubscribe_failure_and_rejects_refresh(self):
        connection, _store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/state", "key": "10"})
        refresh = lease.refresh()
        connection.unsubscribe_error = RuntimeError("unsubscribe failed")

        lease.release()  # must not raise
        with pytest.raises(SubscriptionError, match="released while refreshing"):
            await refresh

    @pytest.mark.asyncio
    async def test_refresh_send_failure_marks_query_and_rejects(self):
        connection, _store, registry = create_registry()
        lease = registry.subscribe({"view": "Round/list", "take": 2})
        connection.refresh_error = RuntimeError("refresh send failed")

        with pytest.raises(SubscriptionError, match="refresh send failed"):
            await lease.refresh()
        result = lease.get_result()
        assert result.is_loading is False
        assert result.is_refreshing is False
        assert result.error is not None and "refresh send failed" in str(result.error)

    @pytest.mark.asyncio
    async def test_refresh_of_inactive_query_rejects(self):
        _connection, _store, registry = create_registry()
        with pytest.raises(SubscriptionError, match="Cannot refresh inactive query"):
            await registry.refresh({"view": "Ghost/list"})

    @pytest.mark.asyncio
    async def test_refresh_view_refreshes_every_active_subscription(self):
        connection, _store, registry = create_registry()
        first = registry.subscribe({"view": "Round/state", "key": "1"}, False)
        second = registry.subscribe({"view": "Round/state", "key": "2"}, False)
        registry.subscribe({"view": "Miner/state", "key": "wallet"}, False)

        await registry.refresh_view("Round/state")

        assert len(connection.refreshed) == 2
        assert first.subscription in connection.refreshed
        assert second.subscription in connection.refreshed

    @pytest.mark.asyncio
    async def test_refresh_view_narrows_to_a_single_key(self):
        connection, _store, registry = create_registry()
        registry.subscribe({"view": "Round/state", "key": "1"}, False)
        second = registry.subscribe({"view": "Round/state", "key": "2"}, False)

        await registry.refresh_view("Round/state", "2")

        assert connection.refreshed == [second.subscription]

    @pytest.mark.asyncio
    async def test_refresh_view_is_noop_without_matches(self):
        connection, _store, registry = create_registry()
        registry.subscribe({"view": "Round/state", "key": "1"})

        assert await registry.refresh_view("Round/state", "unknown") is None
        assert await registry.refresh_view("Treasury/state") is None
        assert connection.refreshed == []

    def test_clear_releases_everything_even_when_unsubscribe_fails(self):
        connection, _store, registry = create_registry()
        registry.subscribe({"view": "Round/state", "key": "1"})
        registry.subscribe({"view": "Round/state", "key": "2"})
        connection.unsubscribe_error = RuntimeError("socket gone")

        registry.clear()
        assert registry.get_active_subscriptions() == []
        assert registry.get_ref_count({"view": "Round/state", "key": "1"}) == 0

    def test_get_query_result_absent_vs_registered(self):
        _connection, _store, registry = create_registry()
        assert registry.get_query_result({"view": "Round/list"}) is None
        lease = registry.subscribe({"view": "Round/list"})
        result = registry.get_query_result({"view": "Round/list"})
        assert result is not None
        assert result.subscription_id == lease.subscription.subscription_id
