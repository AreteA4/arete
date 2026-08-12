"""Tests for arete.views: the six canonical verbs on list and state views."""

from __future__ import annotations

import asyncio
import json

import pytest

from arete import UNSET
from arete.errors import SubscriptionError
from arete.store import Store
from arete.subscription import SubscriptionRegistry
from arete.views import (
    DEFAULT_INITIAL_DATA_TIMEOUT,
    InitialDataTimeoutError,
    ListViewHandle,
    StateViewHandle,
    ViewDef,
    ViewGroupHandle,
    ViewsNamespace,
    create_view_handle,
)
from arete.wire import parse_frame

TIMEOUT = 3.0


class FakeConnection:
    def __init__(self):
        self.subscribed = []
        self.unsubscribed = []
        self.refreshed = []

    def subscribe(self, subscription):
        self.subscribed.append(subscription)

    def unsubscribe(self, subscription_id):
        self.unsubscribed.append(subscription_id)

    def refresh(self, subscription):
        self.refreshed.append(subscription)


def make_env():
    connection = FakeConnection()
    store = Store()
    registry = SubscriptionRegistry(connection, store)
    return connection, store, registry


def feed(store: Store, frame: dict) -> None:
    store.handle_frame(parse_frame(json.dumps(frame)))


def snapshot(sid, entries, *, entity, mode="list", key=None, snapshot_id="snap",
             authoritative=True, complete=True):
    frame = {
        "protocolVersion": 2,
        "subscriptionId": sid,
        "snapshotId": snapshot_id,
        "authoritative": authoritative,
        "mode": mode,
        "entity": entity,
        "op": "snapshot",
        "data": [{"key": k, "data": v} for k, v in entries],
        "complete": complete,
    }
    if key is not None:
        frame["key"] = key
    return frame


def live(sid, op, key, data=None, *, entity, mode="list", seq=None, append=None):
    frame = {
        "protocolVersion": 2,
        "subscriptionId": sid,
        "mode": mode,
        "entity": entity,
        "op": op,
        "key": key,
        "data": data,
    }
    if seq is not None:
        frame["seq"] = seq
    if append is not None:
        frame["append"] = append
    return frame


async def collect(stream, count):
    """Consume `count` items then close the stream (releasing its lease)."""
    items = []
    try:
        async for item in stream:
            items.append(item)
            if len(items) >= count:
                break
    finally:
        await stream.aclose()
    return items


class TestUse:
    @pytest.mark.asyncio
    async def test_yields_snapshot_then_merged_entities_filtering_removals(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        stream = handle.use(take=2)
        task = asyncio.create_task(collect(stream, 3))
        await asyncio.sleep(0)  # let the stream subscribe
        sid = connection.subscribed[0].subscription_id
        assert connection.subscribed[0].query == {"view": "Round/list", "take": 2}

        feed(store, snapshot(sid, [("10", {"id": 10})], entity="Round/list"))
        feed(store, live(sid, "upsert", "11", {"id": 11}, entity="Round/list"))
        feed(store, live(sid, "remove", "10", entity="Round/list"))  # never yielded
        feed(store, live(sid, "patch", "11", {"score": 1}, entity="Round/list"))

        items = await asyncio.wait_for(task, TIMEOUT)
        assert items == [{"id": 10}, {"id": 11}, {"id": 11, "score": 1}]
        # Closing the stream released the refcounted lease.
        assert connection.unsubscribed == [sid]
        assert registry.get_ref_count({"view": "Round/list", "take": 2}) == 0

    @pytest.mark.asyncio
    async def test_state_use_takes_typed_key_kwargs_and_filters_by_key(self):
        connection, store, registry = make_env()
        handle = StateViewHandle("OreRound/state", registry, key_fields=("round_id",))

        stream = handle.use(round_id=42)
        task = asyncio.create_task(collect(stream, 1))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        assert connection.subscribed[0].query == {"view": "OreRound/state", "key": "42"}

        feed(store, live(sid, "upsert", "43", {"round_id": 43},
                         entity="OreRound/state", mode="state"))
        feed(store, live(sid, "upsert", "42", {"round_id": 42},
                         entity="OreRound/state", mode="state"))
        items = await asyncio.wait_for(task, TIMEOUT)
        assert items == [{"round_id": 42}]

    @pytest.mark.asyncio
    async def test_parser_override_applies_to_yields(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        stream = handle.use(parser=lambda entity: ("parsed", entity["id"]))
        task = asyncio.create_task(collect(stream, 1))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        feed(store, snapshot(sid, [("1", {"id": 1})], entity="Round/list"))
        assert await asyncio.wait_for(task, TIMEOUT) == [("parsed", 1)]


class TestWatch:
    @pytest.mark.asyncio
    async def test_yields_full_update_taxonomy(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        stream = handle.watch()
        task = asyncio.create_task(collect(stream, 5))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id

        feed(store, snapshot(sid, [("1", {"id": 1})], entity="Round/list"))
        feed(store, live(sid, "upsert", "2", {"id": 2}, entity="Round/list"))
        feed(store, live(sid, "patch", "2", {"score": 5}, entity="Round/list"))
        feed(store, live(sid, "remove", "1", entity="Round/list"))
        feed(store, live(sid, "delete", "2", entity="Round/list"))

        updates = await asyncio.wait_for(task, TIMEOUT)
        assert [u.op for u in updates] == ["upsert", "upsert", "patch", "remove", "delete"]
        assert updates[0].data == {"id": 1}
        assert updates[2].data == {"score": 5}  # patch carries the partial
        assert updates[3].data is None and updates[4].data is None

    @pytest.mark.asyncio
    async def test_equivalent_watches_share_one_wire_subscription(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        first = handle.watch(take=1)
        second = handle.watch(take=1)
        task_one = asyncio.create_task(collect(first, 1))
        task_two = asyncio.create_task(collect(second, 1))
        await asyncio.sleep(0)
        assert len(connection.subscribed) == 1
        sid = connection.subscribed[0].subscription_id

        feed(store, live(sid, "upsert", "1", {"id": 1}, entity="Round/list"))
        assert (await asyncio.wait_for(task_one, TIMEOUT))[0].op == "upsert"
        assert (await asyncio.wait_for(task_two, TIMEOUT))[0].op == "upsert"
        # Both streams closed: exactly one wire unsubscribe.
        assert connection.unsubscribed == [sid]


class TestWatchRich:
    @pytest.mark.asyncio
    async def test_yields_before_after_diffs(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        stream = handle.watch_rich()
        task = asyncio.create_task(collect(stream, 5))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id

        feed(store, live(sid, "upsert", "1", {"id": 1}, entity="Round/list"))
        feed(store, live(sid, "patch", "1", {"score": 2}, entity="Round/list"))
        feed(store, live(sid, "remove", "1", entity="Round/list"))
        feed(store, live(sid, "upsert", "2", {"id": 2}, entity="Round/list"))
        feed(store, live(sid, "delete", "2", entity="Round/list"))

        updates = await asyncio.wait_for(task, TIMEOUT)
        assert [u.type for u in updates] == [
            "created", "updated", "removed", "created", "deleted",
        ]
        assert updates[0].data == {"id": 1}
        assert updates[1].before == {"id": 1}
        assert updates[1].after == {"id": 1, "score": 2}
        assert updates[1].patch == {"score": 2}
        # remove is query-local: the last known entity is still reported
        assert updates[2].last_known == {"id": 1, "score": 2}
        assert updates[4].last_known == {"id": 2}


class TestGet:
    @pytest.mark.asyncio
    async def test_list_get_awaits_snapshot_completion(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        task = asyncio.create_task(handle.get(take=2))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        assert not task.done()

        feed(store, snapshot(sid, [("10", {"id": 10})], entity="Round/list",
                             complete=False))
        await asyncio.sleep(0)
        assert not task.done()  # incomplete batches keep the read pending

        feed(store, snapshot(sid, [("9", {"id": 9})], entity="Round/list"))
        assert await asyncio.wait_for(task, TIMEOUT) == [{"id": 10}, {"id": 9}]
        # One-shot read released its lease.
        assert connection.unsubscribed == [sid]

    @pytest.mark.asyncio
    async def test_state_get_returns_entity_or_none(self):
        connection, store, registry = make_env()
        handle = StateViewHandle("OreRound/state", registry, key_fields=("round_id",))

        task = asyncio.create_task(handle.get(round_id=42))
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        feed(store, snapshot(sid, [("42", {"round_id": 42})],
                             entity="OreRound/state", mode="state", key="42"))
        assert await asyncio.wait_for(task, TIMEOUT) == {"round_id": 42}

        missing = asyncio.create_task(handle.get(round_id=404))
        await asyncio.sleep(0)
        sid = connection.subscribed[-1].subscription_id
        feed(store, snapshot(sid, [], entity="OreRound/state", mode="state",
                             key="404"))
        assert await asyncio.wait_for(missing, TIMEOUT) is None

    @pytest.mark.asyncio
    async def test_get_propagates_query_errors(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        task = asyncio.create_task(handle.get())
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        feed(store, {
            "type": "error",
            "protocolVersion": 2,
            "subscriptionId": sid,
            "code": "subscription-rejected",
            "message": "view not found",
            "fatal": False,
        })
        with pytest.raises(SubscriptionError, match="view not found"):
            await asyncio.wait_for(task, TIMEOUT)
        assert connection.unsubscribed == [sid]

    @pytest.mark.asyncio
    async def test_get_one_returns_first_element_or_none(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        task = asyncio.create_task(handle.get_one())
        await asyncio.sleep(0)
        sid = connection.subscribed[0].subscription_id
        feed(store, snapshot(sid, [("10", {"id": 10}), ("9", {"id": 9})],
                             entity="Round/list"))
        assert await asyncio.wait_for(task, TIMEOUT) == {"id": 10}

        empty = asyncio.create_task(handle.get_one(take=1))
        await asyncio.sleep(0)
        sid = connection.subscribed[-1].subscription_id
        feed(store, snapshot(sid, [], entity="Round/list"))
        assert await asyncio.wait_for(empty, TIMEOUT) is None


class TestGetTimeout:
    @pytest.mark.asyncio
    async def test_list_get_times_out_and_releases_the_lease(self):
        """A socket that never delivers (auto_connect=False, reconnect storm)
        must not hang the caller forever."""
        connection, _store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        with pytest.raises(InitialDataTimeoutError) as excinfo:
            await asyncio.wait_for(handle.get(timeout=0.01), TIMEOUT)
        assert excinfo.value.view == "Round/list"
        assert excinfo.value.timeout == 0.01
        assert excinfo.value.code == "INITIAL_DATA_TIMEOUT"
        assert "Round/list" in str(excinfo.value)
        # The one-shot read released its refcounted lease on the timeout path.
        sid = connection.subscribed[0].subscription_id
        assert connection.unsubscribed == [sid]
        assert registry.get_ref_count({"view": "Round/list"}) == 0

    @pytest.mark.asyncio
    async def test_state_get_and_get_one_are_bounded_too(self):
        _connection, _store, registry = make_env()
        state = StateViewHandle("OreRound/state", registry, key_fields=("round_id",))
        with pytest.raises(InitialDataTimeoutError):
            await asyncio.wait_for(state.get(round_id=42, timeout=0.01), TIMEOUT)
        with pytest.raises(InitialDataTimeoutError):
            await asyncio.wait_for(state.get_one(round_id=42, timeout=0.01), TIMEOUT)

        listed = ListViewHandle("Round/list", registry)
        with pytest.raises(InitialDataTimeoutError):
            await asyncio.wait_for(listed.get_one(timeout=0.01), TIMEOUT)

    @pytest.mark.asyncio
    async def test_handle_default_bounds_get_without_an_explicit_timeout(self):
        _connection, _store, registry = make_env()
        handle = ListViewHandle("Round/list", registry, initial_data_timeout=0.01)
        with pytest.raises(InitialDataTimeoutError):
            await asyncio.wait_for(handle.get(), TIMEOUT)
        assert DEFAULT_INITIAL_DATA_TIMEOUT == 5.0

    @pytest.mark.asyncio
    async def test_timeout_none_waits_and_cancellation_still_releases(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry, initial_data_timeout=0.01)

        # An explicit None opts out of the bound entirely.
        task = asyncio.create_task(handle.get(timeout=None))
        await asyncio.sleep(0.05)
        assert not task.done()
        sid = connection.subscribed[0].subscription_id
        feed(store, snapshot(sid, [("1", {"id": 1})], entity="Round/list"))
        assert await asyncio.wait_for(task, TIMEOUT) == [{"id": 1}]

        cancelled = asyncio.create_task(handle.get(timeout=None))
        await asyncio.sleep(0)
        cancelled.cancel()
        with pytest.raises(asyncio.CancelledError):
            await cancelled
        assert registry.get_ref_count({"view": "Round/list"}) == 0

    @pytest.mark.asyncio
    async def test_timeout_does_not_change_subscription_identity(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        task = asyncio.create_task(handle.get(take=2, timeout=1.0))
        await asyncio.sleep(0)
        assert connection.subscribed[0].query == {"view": "Round/list", "take": 2}
        feed(store, snapshot(connection.subscribed[0].subscription_id, [],
                             entity="Round/list"))
        assert await asyncio.wait_for(task, TIMEOUT) == []

    def test_timeout_is_rejected_by_the_streaming_verbs(self):
        _connection, _store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        for verb in ("use", "watch", "watch_rich"):
            with pytest.raises(TypeError, match="unexpected keyword argument 'timeout'"):
                getattr(handle, verb)(timeout=1)
        with pytest.raises(TypeError, match="unexpected keyword argument 'timeout'"):
            handle.get_sync(timeout=1)

    @pytest.mark.asyncio
    async def test_invalid_timeout_values_are_rejected(self):
        _connection, _store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        with pytest.raises(TypeError, match="timeout must be a number"):
            await handle.get(timeout="soon")
        with pytest.raises(ValueError, match="timeout must be greater than 0"):
            await handle.get(timeout=0)

    @pytest.mark.asyncio
    async def test_client_wires_its_initial_data_timeout_into_view_handles(self):
        _connection, _store, registry = make_env()
        views = ViewsNamespace(
            registry,
            {"ore_round": {"latest": ViewDef("list", "OreRound/latest")}},
            0.01,
        )
        with pytest.raises(InitialDataTimeoutError):
            await asyncio.wait_for(views.ore_round.latest.get(), TIMEOUT)


class TestGetSync:
    @pytest.mark.asyncio
    async def test_list_absent_vs_empty_vs_data(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)

        # No equivalent active subscription: UNSET sentinel, not [].
        assert handle.get_sync() is UNSET

        lease = registry.subscribe({"view": "Round/list"})
        sid = lease.subscription.subscription_id
        feed(store, snapshot(sid, [], entity="Round/list"))
        assert handle.get_sync() == []  # subscribed-but-empty

        feed(store, live(sid, "upsert", "1", {"id": 1}, entity="Round/list"))
        assert handle.get_sync() == [{"id": 1}]

        # Non-equivalent options are a different identity.
        assert handle.get_sync(take=5) is UNSET

        lease.release()
        assert handle.get_sync() is UNSET

    @pytest.mark.asyncio
    async def test_state_absent_vs_empty_vs_data(self):
        connection, store, registry = make_env()
        handle = StateViewHandle("OreRound/state", registry, key_fields=("round_id",))

        assert handle.get_sync(round_id=42) is UNSET

        lease = registry.subscribe({"view": "OreRound/state", "key": "42"})
        sid = lease.subscription.subscription_id
        feed(store, snapshot(sid, [], entity="OreRound/state", mode="state", key="42"))
        assert handle.get_sync(round_id=42) is None  # subscribed, entity absent

        feed(store, live(sid, "upsert", "42", {"round_id": 42},
                         entity="OreRound/state", mode="state"))
        assert handle.get_sync(round_id=42) == {"round_id": 42}
        lease.release()

    @pytest.mark.asyncio
    async def test_parser_applies_to_get_sync(self):
        connection, store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        lease = registry.subscribe({"view": "Round/list"})
        feed(store, snapshot(lease.subscription.subscription_id,
                             [("1", {"id": 1})], entity="Round/list"))
        assert handle.get_sync(parser=lambda e: e["id"]) == [1]


class TestKeyAndOptionValidation:
    def make_state(self, key_fields=("round_id",)):
        _connection, _store, registry = make_env()
        return StateViewHandle("OreRound/state", registry, key_fields=key_fields)

    def test_unknown_option_kwarg_is_a_type_error(self):
        _connection, _store, registry = make_env()
        handle = ListViewHandle("Round/list", registry)
        with pytest.raises(TypeError, match="unexpected keyword argument 'bogus'"):
            handle.get_sync(bogus=1)

    def test_missing_key_field(self):
        with pytest.raises(TypeError, match="missing field 'round_id'"):
            self.make_state().get_sync()

    def test_positional_key_rejected_when_key_fields_exist(self):
        with pytest.raises(TypeError, match="keyword arguments"):
            self.make_state().get_sync("42")

    def test_composite_keys_are_unsupported(self):
        with pytest.raises(TypeError, match="unsupported composite key"):
            self.make_state(key_fields=("owner", "position")).get_sync(
                owner="w", position=1
            )

    def test_key_value_types(self):
        state = self.make_state()
        assert state.get_sync(round_id=42) is UNSET  # int keys serialize
        assert state.get_sync(round_id="42") is UNSET  # str keys pass through
        with pytest.raises(TypeError, match="string or integer"):
            state.get_sync(round_id=True)
        with pytest.raises(TypeError, match="string or integer"):
            state.get_sync(round_id=4.2)

    def test_scalar_key_fallback_without_key_fields(self):
        legacy = self.make_state(key_fields=())
        assert legacy.get_sync("legacy-key") is UNSET
        with pytest.raises(TypeError, match="requires a key"):
            legacy.get_sync()

    @pytest.mark.asyncio
    async def test_int_key_serializes_to_wire_string(self):
        connection, store, registry = make_env()
        state = StateViewHandle("OreRound/state", registry, key_fields=("round_id",))
        stream = state.watch(round_id=42, take=1)
        task = asyncio.create_task(collect(stream, 1))
        await asyncio.sleep(0)
        assert connection.subscribed[0].query == {
            "view": "OreRound/state", "key": "42", "take": 1,
        }
        feed(store, live(connection.subscribed[0].subscription_id, "upsert", "42",
                         {"round_id": 42}, entity="OreRound/state", mode="state"))
        await asyncio.wait_for(task, TIMEOUT)
        # An equivalent lease makes get_sync see the same identity.
        lease = registry.subscribe({"view": "OreRound/state", "key": "42", "take": 1})
        assert state.get_sync(round_id=42, take=1) is not UNSET
        lease.release()


class TestNamespace:
    def test_attribute_access_builds_and_caches_handles(self):
        _connection, _store, registry = make_env()
        views = ViewsNamespace(registry, {
            "ore_round": {
                "state": ViewDef("state", "OreRound/state", ("round_id",)),
                "latest": ViewDef("list", "OreRound/latest"),
            },
        })
        group = views.ore_round
        assert isinstance(group, ViewGroupHandle)
        assert views.ore_round is group
        assert isinstance(group.latest, ListViewHandle)
        assert isinstance(group.state, StateViewHandle)
        assert group.latest is group.latest
        assert group.latest.view == "OreRound/latest"

    def test_unknown_groups_and_views_raise_attribute_error(self):
        _connection, _store, registry = make_env()
        views = ViewsNamespace(registry, {"ore_round": {"latest": ViewDef("list", "V")}})
        with pytest.raises(AttributeError, match="no view group 'other'"):
            views.other
        with pytest.raises(AttributeError, match="no view 'missing'"):
            views.ore_round.missing

    def test_create_view_handle_rejects_unknown_mode(self):
        _connection, _store, registry = make_env()
        with pytest.raises(TypeError, match="Unknown view mode"):
            create_view_handle(ViewDef("append", "V"), registry)
