"""Tests for arete.wire: protocol v2 frames, gzip, seq, u64 helpers."""

from __future__ import annotations

import gzip
import json

import pytest

from arete.errors import AreteError
from arete.wire import (
    EntityFrame,
    ErrorFrame,
    Seq,
    SnapshotFrame,
    SubscribedFrame,
    UnsubscribedFrame,
    compare_seq,
    format_u64,
    frame_slot,
    is_gzip_data,
    parse_frame,
    parse_seq,
    parse_u64,
    ping_envelope,
    refresh_auth_envelope,
    seq_slot,
    subscribe_envelope,
    unsubscribe_envelope,
)


def encode(frame: dict) -> str:
    return json.dumps(frame)


class TestParseFrame:
    def test_parses_subscribed_frame(self):
        frame = parse_frame(encode({
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1",
            "op": "subscribed",
            "query": {"view": "OreRound/latest", "take": 10, "skip": 0},
            "mode": "list",
            "sort": {"field": ["id", "roundId"], "order": "desc"},
        }))
        assert isinstance(frame, SubscribedFrame)
        assert frame.subscription_id == "rounds:page-1"
        assert frame.mode == "list"
        assert frame.sort is not None
        assert frame.sort.field == ("id", "roundId")
        assert frame.sort.order == "desc"

    def test_parses_snapshot_frame(self):
        frame = parse_frame(encode({
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1",
            "snapshotId": "snap-1",
            "authoritative": True,
            "mode": "list",
            "entity": "OreRound/latest",
            "op": "snapshot",
            "data": [{"key": "100", "data": {"id": {"round_id": "100"}}}],
            "complete": True,
        }))
        assert isinstance(frame, SnapshotFrame)
        assert frame.snapshot_id == "snap-1"
        assert frame.authoritative is True
        assert frame.complete is True
        assert frame.data[0].key == "100"
        # snake_case payloads pass through untransformed
        assert frame.data[0].data == {"id": {"round_id": "100"}}

    def test_parses_live_frames(self):
        frame = parse_frame(encode({
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1",
            "mode": "list",
            "entity": "OreRound/latest",
            "op": "patch",
            "key": "101",
            "data": {"values": ["b"]},
            "append": ["values"],
            "seq": "1235:000000000001",
        }))
        assert isinstance(frame, EntityFrame)
        assert frame.op == "patch"
        assert frame.append == ("values",)
        assert frame.seq == "1235:000000000001"

    def test_parses_unsubscribed_and_error_frames(self):
        unsub = parse_frame(encode({
            "protocolVersion": 2,
            "subscriptionId": "things:all",
            "op": "unsubscribed",
        }))
        assert isinstance(unsub, UnsubscribedFrame)

        error = parse_frame(encode({
            "type": "error",
            "protocolVersion": 2,
            "subscriptionId": "rounds:page-1",
            "error": "duplicate-subscription-id",
            "message": "subscriptionId is already active on this connection",
            "code": "duplicate-subscription-id",
            "retryable": False,
            "fatal": False,
        }))
        assert isinstance(error, ErrorFrame)
        assert error.code == "duplicate-subscription-id"
        assert error.retryable is False

        anonymous = parse_frame(encode({
            "type": "error",
            "protocolVersion": 2,
            "subscriptionId": None,
            "code": "malformed-message",
            "fatal": False,
        }))
        assert isinstance(anonymous, ErrorFrame)
        assert anonymous.subscription_id is None

    def test_error_frame_carries_structured_body_fields(self):
        frame = parse_frame(encode({
            "type": "error",
            "protocolVersion": 2,
            "subscriptionId": "s",
            "code": "rate-limit-exceeded",
            "fatal": False,
            "retryable": True,
            "retry_after": 2.5,
            "suggested_action": "slow down",
            "docs_url": "https://docs.arete.run/limits",
        }))
        assert isinstance(frame, ErrorFrame)
        assert frame.retry_after == 2.5
        assert frame.suggested_action == "slow down"
        assert frame.docs_url == "https://docs.arete.run/limits"


class TestGzip:
    def test_detects_gzip_magic(self):
        assert is_gzip_data(b"\x1f\x8b\x08\x00")
        assert not is_gzip_data(b"{}")
        assert not is_gzip_data(b"\x1f")

    def test_parses_gzip_binary_frame(self):
        payload = {
            "protocolVersion": 2,
            "subscriptionId": "things:all",
            "op": "unsubscribed",
        }
        frame = parse_frame(gzip.compress(json.dumps(payload).encode("utf-8")))
        assert isinstance(frame, UnsubscribedFrame)

    def test_parses_plain_binary_frame(self):
        payload = {
            "protocolVersion": 2,
            "subscriptionId": "things:all",
            "op": "unsubscribed",
        }
        frame = parse_frame(json.dumps(payload).encode("utf-8"))
        assert isinstance(frame, UnsubscribedFrame)


class TestFrameValidation:
    def rejects(self, frame: dict) -> None:
        with pytest.raises(AreteError, match="Invalid WebSocket protocol v2 frame"):
            parse_frame(encode(frame))

    def test_rejects_wrong_protocol_version(self):
        self.rejects({"protocolVersion": 1, "subscriptionId": "x", "op": "unsubscribed"})

    def test_rejects_whitespace_subscription_id(self):
        self.rejects({
            "protocolVersion": 2, "subscriptionId": " things:all", "op": "unsubscribed",
        })

    def test_rejects_control_characters_in_subscription_id(self):
        self.rejects({
            "protocolVersion": 2, "subscriptionId": "a\x01b", "op": "unsubscribed",
        })

    def test_rejects_subscription_id_over_128_bytes(self):
        self.rejects({
            "protocolVersion": 2, "subscriptionId": "é" * 65, "op": "unsubscribed",
        })

    def test_rejects_unknown_query_field_in_subscribed(self):
        self.rejects({
            "protocolVersion": 2,
            "subscriptionId": "x",
            "op": "subscribed",
            "query": {"view": "Thing/list", "bogus": 1},
            "mode": "list",
        })

    def test_rejects_invalid_take_and_skip(self):
        self.rejects({
            "protocolVersion": 2,
            "subscriptionId": "x",
            "op": "subscribed",
            "query": {"view": "Thing/list", "take": 0},
            "mode": "list",
        })
        self.rejects({
            "protocolVersion": 2,
            "subscriptionId": "x",
            "op": "subscribed",
            "query": {"view": "Thing/list", "skip": -1},
            "mode": "list",
        })

    def test_rejects_snapshot_without_identity(self):
        self.rejects({
            "protocolVersion": 2,
            "subscriptionId": "x",
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
            "data": [],
            "complete": True,
        })

    def test_rejects_live_frame_without_data_member(self):
        self.rejects({
            "protocolVersion": 2,
            "subscriptionId": "x",
            "mode": "list",
            "entity": "Thing/list",
            "op": "upsert",
            "key": "1",
        })

    def test_rejects_undecodable_payload(self):
        with pytest.raises(AreteError):
            parse_frame("not json")


class TestSeq:
    def test_parse_seq(self):
        assert parse_seq("1234:000000000010") == Seq(
            slot=1234, index="000000000010", raw="1234:000000000010"
        )
        assert parse_seq("abc:zzz").slot is None
        assert parse_seq("42").index == ""
        assert seq_slot("42:x") == 42
        assert seq_slot("x:42") is None
        assert seq_slot(None) is None

    def test_slot_compares_numerically(self):
        assert compare_seq("10:a", "9:z") == 1
        assert compare_seq("9:z", "10:a") == -1
        assert compare_seq("100:0", "20:0") == 1  # not lexicographic on the slot

    def test_index_compares_lexicographically(self):
        assert compare_seq("5:000000000002", "5:000000000001") == 1
        assert compare_seq("5:000000000001", "5:000000000002") == -1
        assert compare_seq("5:0001", "5:0001") == 0

    def test_non_numeric_slots_fall_back_to_index(self):
        assert compare_seq("x:b", "y:a") == 1
        assert compare_seq("x:a", "y:a") == 0

    def test_extra_colons_use_second_segment_as_index(self):
        assert compare_seq("1:2:3", "1:2:9") == 0


class TestU64:
    def test_roundtrip(self):
        assert format_u64(0) == "0"
        assert format_u64(2**64 - 1) == "18446744073709551615"
        assert parse_u64("18446744073709551615") == 2**64 - 1
        assert parse_u64("0") == 0

    def test_bounds(self):
        with pytest.raises(ValueError):
            format_u64(-1)
        with pytest.raises(ValueError):
            format_u64(2**64)
        with pytest.raises(ValueError):
            parse_u64("18446744073709551616")

    def test_rejects_non_decimal_input(self):
        with pytest.raises(ValueError):
            parse_u64("12a")
        with pytest.raises(ValueError):
            parse_u64("-1")
        with pytest.raises(ValueError):
            parse_u64("")
        with pytest.raises(TypeError):
            format_u64(True)
        with pytest.raises(TypeError):
            format_u64("12")  # type: ignore[arg-type]


class TestClientEnvelopes:
    def test_every_envelope_declares_protocol_version_2(self):
        assert subscribe_envelope("s", {"view": "Thing/list"}) == {
            "type": "subscribe",
            "protocolVersion": 2,
            "subscriptionId": "s",
            "query": {"view": "Thing/list"},
            "snapshot": {"enabled": True},
        }
        assert subscribe_envelope("s", {"view": "V"}, False)["snapshot"] == {
            "enabled": False
        }
        assert unsubscribe_envelope("s") == {
            "type": "unsubscribe",
            "protocolVersion": 2,
            "subscriptionId": "s",
        }
        assert ping_envelope() == {"type": "ping", "protocolVersion": 2}
        assert refresh_auth_envelope("tok") == {
            "type": "refresh_auth",
            "protocolVersion": 2,
            "token": "tok",
        }


class TestFrameSlot:
    def entity(self, **overrides) -> EntityFrame:
        base = dict(
            subscription_id="s", mode="list", entity="Thing/list",
            op="upsert", key="1", data={"id": 1},
        )
        base.update(overrides)
        return EntityFrame(**base)

    def test_entity_frame_uses_seq(self):
        assert frame_slot(self.entity(seq="123:0001")) == 123

    def test_entity_frame_falls_back_to_data_seq(self):
        assert frame_slot(self.entity(data={"_seq": "77:0001"})) == 77

    def test_snapshot_frame_takes_max_entity_slot(self):
        frame = parse_frame(encode({
            "protocolVersion": 2,
            "subscriptionId": "s",
            "snapshotId": "snap",
            "authoritative": True,
            "mode": "list",
            "entity": "Thing/list",
            "op": "snapshot",
            "data": [
                {"key": "a", "data": {"_seq": "10:0001"}},
                {"key": "b", "data": {"_seq": "42:0001"}},
                {"key": "c", "data": {}},
            ],
            "complete": True,
        }))
        assert frame_slot(frame) == 42

    def test_non_entity_frames_have_no_slot(self):
        assert frame_slot(UnsubscribedFrame(subscription_id="s")) is None
        assert frame_slot(self.entity(seq="x:0001", data={})) is None
