"""Tests for the internal protocol v2 store.

Runs the shared conformance fixtures from ``tests/fixtures/websocket-v2`` plus
unit cases for snapshot authority, patch/append merge, remove-vs-delete, and
server-declared sort.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from arete.errors import AreteError
from arete.store import QueryResult, Store, deep_merge_with_append
from arete.subscription import Subscription, canonical_query_key, normalize_query
from arete.wire import Update, parse_frame

FIXTURES = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "websocket-v2"


def fixture(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text())


class Harness:
    def __init__(self):
        self.store = Store()
        self.updates = []
        self.rich_updates = []

    def register(self, subscription_id: str, query: dict, snapshot_enabled: bool = True):
        normalized = normalize_query(query)
        subscription = Subscription(
            subscription_id=subscription_id,
            query=normalized,
            snapshot_enabled=snapshot_enabled,
        )
        self.store.register(subscription, canonical_query_key(normalized, snapshot_enabled))
        self.store.on_update(subscription_id, lambda u: self.updates.append((subscription_id, u)))
        self.store.on_rich_update(
            subscription_id, lambda u: self.rich_updates.append((subscription_id, u))
        )
        return subscription

    def process(self, frame: dict) -> None:
        self.store.handle_frame(parse_frame(json.dumps(frame)))

    def result(self, subscription_id: str) -> QueryResult:
        result = self.store.get_result(subscription_id)
        assert result is not None
        return result


class TestConformanceFixtures:
    def test_manifest_lists_the_fixture_set(self):
        manifest = fixture("manifest.json")
        assert manifest["protocolVersion"] == 2
        assert manifest["fixtures"] == [
            "keyed-state.json",
            "list-windows.json",
            "filters.json",
            "multi-batch-authoritative.json",
            "empty-snapshot.json",
            "remove.json",
            "delete.json",
            "incremental-snapshot.json",
            "reconnect-replacement.json",
            "errors.json",
        ]

    def test_keyed_state_snapshot_and_patch_apply_to_their_query(self):
        h = Harness()
        keyed_state = fixture("keyed-state.json")
        request = keyed_state["client"][0]
        h.register(request["subscriptionId"], request["query"])
        for frame in keyed_state["server"]:
            h.process(frame)

        result = h.result(request["subscriptionId"])
        assert result.keys == ("wallet-a",)
        assert result.data == ({"authority": "wallet-a", "score": 2},)
        assert result.is_loading is False

    def test_list_windows_and_filters_have_distinct_identities(self):
        list_windows = fixture("list-windows.json")
        identities = {
            canonical_query_key(request["query"])
            for request in list_windows["client"]
        }
        assert len(identities) == 2
        assert list_windows["expectedKeys"]["rounds:page-1"] == ["6", "5"]
        assert list_windows["expectedKeys"]["rounds:page-2"] == ["4", "3"]

        filters = fixture("filters.json")
        assert filters["client"][0]["query"]["filters"] == {
            "state.status": "open",
            "market.symbol": "SOL",
        }
        assert len(filters["notMatching"]) == 2

    def test_authoritative_batches_stage_and_commit_atomically(self):
        h = Harness()
        h.register("things:all", {"view": "Thing/list"})
        multi_batch = fixture("multi-batch-authoritative.json")

        h.process(multi_batch["server"][0])
        result = h.result("things:all")
        assert result.keys == ()
        assert result.is_loading is True

        h.process(multi_batch["server"][1])
        result = h.result("things:all")
        assert result.keys == ("three", "two", "one")
        assert result.is_loading is False

        empty = dict(fixture("empty-snapshot.json")["server"][0])
        empty.update(subscriptionId="things:all", entity="Thing/list", mode="list")
        del empty["key"]
        h.process(empty)
        result = h.result("things:all")
        assert result.keys == ()
        assert result.data == ()

    def test_empty_authoritative_state_snapshot_resolves_loading(self):
        h = Harness()
        h.register("state:missing", {"view": "Thing/state", "key": "missing"})
        h.process(fixture("empty-snapshot.json")["server"][0])
        result = h.result("state:missing")
        assert result.keys == ()
        assert result.is_loading is False

    def test_incremental_snapshot_merges_without_replacing(self):
        h = Harness()
        incremental = fixture("incremental-snapshot.json")
        request = incremental["client"][0]
        h.register(request["subscriptionId"], request["query"])

        initial = dict(incremental["server"][0])
        initial.update(
            authoritative=True,
            snapshotId="initial",
            data=[{"key": "order-10", "data": {"_seq": "40:000000000010"}}],
        )
        h.process(initial)
        h.process(incremental["server"][0])

        assert h.result(request["subscriptionId"]).keys == (
            "order-10",
            "order-11",
            "order-12",
        )

    def test_remove_is_query_local_and_delete_is_source_wide(self):
        h = Harness()
        h.register("orders:open", {"view": "Order/list", "filters": {"state.status": "open"}})
        h.register("orders:all", {"view": "Order/list"})
        for subscription_id in ("orders:open", "orders:all"):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "snapshotId": f"snapshot:{subscription_id}",
                "authoritative": True,
                "mode": "list",
                "entity": "Order/list",
                "op": "snapshot",
                "data": [{"key": "order-7", "data": {"id": 7}}],
                "complete": True,
            })

        h.process(fixture("remove.json")["server"][0])
        assert h.result("orders:open").keys == ()
        assert h.result("orders:all").keys == ("order-7",)
        # remove evicts the query only; the entity stays in the source view.
        assert h.store.get_entity("Order/list", "order-7") == {"id": 7}

        h.process(fixture("delete.json")["server"][0])
        assert h.result("orders:all").keys == ()
        assert h.store.get_entity("Order/list", "order-7") is None

        remove_ops = [u.op for sid, u in h.updates if sid == "orders:open"]
        assert remove_ops[-1] == "remove"
        delete_ops = [u.op for sid, u in h.updates if sid == "orders:all"]
        assert delete_ops[-1] == "delete"

    def test_sequenced_patch_normalizes_once_across_queries(self):
        h = Harness()
        h.register("events:all", {"view": "Event/list"})
        h.register("events:open", {"view": "Event/list", "filters": {"state.status": "open"}})
        for subscription_id in ("events:all", "events:open"):
            query = h.store.get_subscription(subscription_id).query
            h.process({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "op": "subscribed",
                "query": dict(query),
                "mode": "list",
            })
            h.process({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "mode": "list",
                "entity": "Event/list",
                "op": "upsert",
                "key": "event-1",
                "data": {"values": ["a"]},
                "seq": "50:000000000001",
            })
        for subscription_id in ("events:all", "events:open"):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": subscription_id,
                "mode": "list",
                "entity": "Event/list",
                "op": "patch",
                "key": "event-1",
                "data": {"values": ["b"]},
                "append": ["values"],
                "seq": "51:000000000001",
            })

        assert h.store.get_entity("Event/list", "event-1") == {"values": ["a", "b"]}
        assert h.result("events:all").keys == ("event-1",)
        assert h.result("events:open").keys == ("event-1",)

    def test_reconnect_retains_data_then_replaces_on_complete_snapshot(self):
        h = Harness()
        reconnect = fixture("reconnect-replacement.json")
        before, after = reconnect["sessions"]
        h.register(before["subscriptionId"], {"view": "Round/list"})

        def snapshot(session):
            return {
                "protocolVersion": 2,
                "subscriptionId": session["subscriptionId"],
                "snapshotId": session["snapshotId"],
                "authoritative": True,
                "mode": "list",
                "entity": "Round/list",
                "op": "snapshot",
                "data": [
                    {"key": key, "data": {"id": key}}
                    for key in session["authoritativeKeys"]
                ],
                "complete": True,
            }

        h.process(snapshot(before))
        h.store.begin_reconnect()
        result = h.result(before["subscriptionId"])
        assert result.keys == ("10", "9")
        assert result.is_refreshing is True
        assert result.is_loading is False

        h.process(snapshot(after))
        result = h.result(before["subscriptionId"])
        assert result.keys == ("11", "10")
        assert result.is_refreshing is False

    def test_protocol_errors_route_to_the_identified_query(self):
        h = Harness()
        for case in fixture("errors.json")["cases"]:
            response = case["response"]
            if response["subscriptionId"] == "duplicate":
                h.register("duplicate", {"view": "Thing/list"})
                h.process(response)
                error = h.result("duplicate").error
                assert error is not None
                assert error.code == "duplicate-subscription-id"
            else:
                frame = parse_frame(json.dumps(response))
                assert frame.subscription_id == response["subscriptionId"]


class TestSnapshotSemantics:
    def test_batches_disagreeing_on_authority_fail_the_query(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        base = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "snapshotId": "snap",
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
        }
        h.process({**base, "authoritative": True, "complete": False,
                   "data": [{"key": "a", "data": {}}]})
        h.process({**base, "authoritative": False, "complete": True,
                   "data": [{"key": "b", "data": {}}]})
        error = h.result("s").error
        assert error is not None and "authoritative" in str(error)

    def test_new_snapshot_id_restarts_staging(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        base = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "authoritative": True,
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
        }
        h.process({**base, "snapshotId": "one", "complete": False,
                   "data": [{"key": "stale", "data": {}}]})
        h.process({**base, "snapshotId": "two", "complete": True,
                   "data": [{"key": "fresh", "data": {}}]})
        assert h.result("s").keys == ("fresh",)

    def test_completed_snapshot_emits_upsert_and_created_updates(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "snapshotId": "snap",
            "authoritative": True,
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
            "data": [{"key": "a", "data": {"id": 1}}],
            "complete": True,
        })
        assert ("s", Update(op="upsert", key="a", data={"id": 1})) in h.updates
        rich = [u for sid, u in h.rich_updates if sid == "s"]
        assert rich[-1].type == "created"
        assert rich[-1].data == {"id": 1}

    def test_snapshot_disabled_subscription_resolves_on_ack(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"}, snapshot_enabled=False)
        result = h.result("s")
        assert result.is_loading is False
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Thing/list"},
            "mode": "list",
        })
        assert h.result("s").is_loading is False

    def test_ack_for_a_different_view_fails_the_query(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Other/list"},
            "mode": "list",
        })
        error = h.result("s").error
        assert error is not None and error.code == "INVALID_FRAME"


class TestPatchMerge:
    def test_deep_merge_with_append_paths(self):
        target = {
            "id": 1,
            "state": {"status": "open", "tags": ["a"], "meta": {"x": 1}},
            "log": ["one"],
        }
        source = {
            "state": {"tags": ["b"], "meta": {"y": 2}},
            "log": ["two"],
        }
        merged = deep_merge_with_append(target, source, ["state.tags"])
        assert merged == {
            "id": 1,
            "state": {"status": "open", "tags": ["a", "b"], "meta": {"x": 1, "y": 2}},
            "log": ["two"],  # not an append path: replaced
        }
        # inputs are not mutated
        assert target["state"]["tags"] == ["a"]

    def test_non_object_source_replaces(self):
        assert deep_merge_with_append({"a": 1}, 5, []) == 5
        assert deep_merge_with_append(3, {"a": 1}, []) == {"a": 1}

    def test_patch_before_any_entity_uses_patch_as_value(self):
        h = Harness()
        h.register("s", {"view": "Thing/state", "key": "k"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "state",
            "entity": "Thing/state",
            "op": "patch",
            "key": "k",
            "data": {"score": 2},
        })
        assert h.store.get_entity("Thing/state", "k") == {"score": 2}

    def test_stale_sequence_patch_does_not_remerge(self):
        h = Harness()
        h.register("s", {"view": "Thing/state", "key": "k"})
        upsert = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "state",
            "entity": "Thing/state",
            "op": "upsert",
            "key": "k",
            "data": {"values": ["a"]},
            "seq": "50:000000000001",
        }
        h.process(upsert)
        h.process({
            **upsert,
            "op": "patch",
            "data": {"values": ["b"]},
            "append": ["values"],
            "seq": "49:000000000001",  # older than stored
        })
        assert h.store.get_entity("Thing/state", "k") == {"values": ["a"]}

    def test_stale_sequence_upsert_keeps_newer_entity(self):
        h = Harness()
        h.register("s", {"view": "Thing/state", "key": "k"})
        base = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "state",
            "entity": "Thing/state",
            "op": "upsert",
            "key": "k",
        }
        h.process({**base, "data": {"v": "new"}, "seq": "50:000000000002"})
        h.process({**base, "data": {"v": "old"}, "seq": "50:000000000001"})
        assert h.store.get_entity("Thing/state", "k") == {"v": "new"}


class TestServerSort:
    def test_live_keys_follow_server_declared_sort(self):
        h = Harness()
        h.register("s", {"view": "Round/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list"},
            "mode": "list",
            "sort": {"field": ["id", "round_id"], "order": "desc"},
        })
        for round_id in (5, 9, 7):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": str(round_id),
                "data": {"id": {"round_id": round_id}},
            })
        assert h.result("s").keys == ("9", "7", "5")

    def test_unsorted_list_falls_back_to_seq_descending(self):
        h = Harness()
        h.register("s", {"view": "Round/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list"},
            "mode": "list",
        })
        for key, seq in (("a", "10:0001"), ("c", "12:0001"), ("b", "11:0001")):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": key,
                "data": {"k": key},
                "seq": seq,
            })
        assert h.result("s").keys == ("c", "b", "a")

    def test_cursor_query_orders_seq_ascending(self):
        h = Harness()
        h.register("s", {"view": "Round/list", "after": "9:0001"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list", "after": "9:0001"},
            "mode": "list",
        })
        for key, seq in (("b", "11:0001"), ("a", "10:0001")):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": key,
                "data": {"k": key},
                "seq": seq,
            })
        assert h.result("s").keys == ("a", "b")

    def test_key_tie_break_uses_localecompare_not_code_point(self):
        # Finding 7: TS breaks sort ties with leftKey.localeCompare(rightKey)
        # (query-store.ts:387). Node:
        #   ['Zap1','aBc1','Bqq','apple'].sort((a, b) => a.localeCompare(b))
        #     -> ['aBc1','apple','Bqq','Zap1']
        # A Python code-point sort yields ['Bqq','Zap1','aBc1','apple'].
        # Entities with no `_seq` all tie (both sequences None => 0), so the
        # tie-break decides the whole order for this very common case.
        h = Harness()
        h.register("s", {"view": "Round/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list"},
            "mode": "list",
        })
        for key in ("Zap1", "aBc1", "Bqq", "apple"):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": key,
                "data": {"k": key},
            })
        assert h.result("s").keys == ("aBc1", "apple", "Bqq", "Zap1")

    def test_string_sort_field_uses_localecompare_not_code_point(self):
        # query-store.ts:64 — compareValues falls through to
        # String(left).localeCompare(String(right)) for string fields.
        h = Harness()
        h.register("s", {"view": "Round/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list"},
            "mode": "list",
            "sort": {"field": ["name"], "order": "asc"},
        })
        for index, name in enumerate(("Zap1", "aBc1", "Bqq", "apple")):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": f"k{index}",
                "data": {"name": name},
            })
        result = h.result("s")
        assert [entity["name"] for entity in result.data] == [
            "aBc1", "apple", "Bqq", "Zap1"
        ]

    def test_accented_sort_field_orders_as_a_secondary_difference(self):
        # Node: ['zone','état'].sort(localeCompare) -> ['état','zone'];
        # a code-point sort puts 'zone' (U+007A) before 'état' (U+00E9).
        h = Harness()
        h.register("s", {"view": "Round/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "op": "subscribed",
            "query": {"view": "Round/list"},
            "mode": "list",
            "sort": {"field": ["name"], "order": "asc"},
        })
        for index, name in enumerate(("zone", "état")):
            h.process({
                "protocolVersion": 2,
                "subscriptionId": "s",
                "mode": "list",
                "entity": "Round/list",
                "op": "upsert",
                "key": f"k{index}",
                "data": {"name": name},
            })
        assert [entity["name"] for entity in h.result("s").data] == ["état", "zone"]


class TestRichUpdates:
    def test_updated_carries_before_after_and_patch(self):
        h = Harness()
        h.register("s", {"view": "Thing/state", "key": "k"})
        base = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "state",
            "entity": "Thing/state",
            "key": "k",
        }
        h.process({**base, "op": "upsert", "data": {"score": 1}, "seq": "1:0001"})
        h.process({**base, "op": "patch", "data": {"score": 2}, "seq": "2:0001"})

        rich = [u for _sid, u in h.rich_updates]
        assert rich[0].type == "created"
        assert rich[1].type == "updated"
        assert rich[1].before == {"score": 1}
        assert rich[1].after == {"score": 2}
        assert rich[1].patch == {"score": 2}

    def test_remove_and_delete_carry_last_known(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        base = {
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "list",
            "entity": "Thing/list",
            "key": "k",
        }
        h.process({**base, "op": "upsert", "data": {"id": 1}})
        h.process({**base, "op": "remove", "data": None})
        removed = h.rich_updates[-1][1]
        assert removed.type == "removed"
        assert removed.last_known == {"id": 1}

        h.process({**base, "op": "upsert", "data": {"id": 1}})
        h.process({**base, "op": "delete", "data": None})
        deleted = h.rich_updates[-1][1]
        assert deleted.type == "deleted"
        assert deleted.last_known == {"id": 1}


class TestLifecycle:
    def test_unregister_is_idempotent_and_result_becomes_none(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        h.store.unregister("s")
        assert h.store.get_result("s") is None
        h.store.unregister("s")

    def test_clear_wipes_entities_and_records(self):
        h = Harness()
        h.register("s", {"view": "Thing/list"})
        h.process({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "mode": "list",
            "entity": "Thing/list",
            "op": "upsert",
            "key": "a",
            "data": {"id": 1},
        })
        h.store.clear()
        assert h.store.get_result("s") is None
        assert h.store.get_entity("Thing/list", "a") is None

    def test_unknown_subscription_listener_registration_raises(self):
        h = Harness()
        with pytest.raises(AreteError, match="Unknown local subscription"):
            h.store.on_update("ghost", lambda u: None)
