"""Tests for arete.connection over an in-process websockets server (loopback,
hermetic — no external network)."""

from __future__ import annotations

import asyncio
import contextlib
import gzip
import json
import socket
from urllib.parse import parse_qs, urlparse

import pytest
from websockets.asyncio.server import serve

from arete.connection import ConnectionManager, SocketIssue
from arete.errors import AreteError, ProcessedSlotTimeoutError, SubscriptionError
from arete.store import Store
from arete.subscription import Subscription, SubscriptionRegistry
from arete.wire import ErrorFrame

TIMEOUT = 3.0


@contextlib.asynccontextmanager
async def serve_ws(handler):
    server = await serve(handler, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    try:
        yield f"ws://127.0.0.1:{port}"
    finally:
        server.close()
        await server.wait_closed()


@contextlib.asynccontextmanager
async def managed(manager: ConnectionManager):
    try:
        yield manager
    finally:
        await manager.disconnect()


async def wait_until(predicate, timeout: float = TIMEOUT):
    deadline = asyncio.get_running_loop().time() + timeout
    while not predicate():
        if asyncio.get_running_loop().time() > deadline:
            raise AssertionError("condition was not met in time")
        await asyncio.sleep(0.01)


def make_manager(url, **kwargs):
    kwargs.setdefault("reconnect_intervals", (0.05,))
    manager = ConnectionManager(url, **kwargs)
    store = Store()
    registry = SubscriptionRegistry(manager, store)
    manager.on_frame(store.handle_frame)
    manager.on_connection_state_change(
        lambda state, error=None: registry.handle_connection_state(state)
    )
    return manager, store, registry


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class FakeAuthState:
    """Duck-typed stand-in for arete.auth.AuthState."""

    config = None

    def __init__(self):
        self.tokens = ["tok-1", "tok-2"]
        self.resolved = 0
        self.cleared = False

    async def resolve_token(self, force_refresh: bool = False):
        token = self.tokens[min(self.resolved, len(self.tokens) - 1)]
        self.resolved += 1
        return token

    def get_refresh_delay(self):
        return 0.05 if self.resolved < 2 else None

    def clear_token(self):
        self.cleared = True


@pytest.mark.asyncio
async def test_connect_subscribe_and_gzip_snapshot_roundtrip():
    received = []

    async def handler(conn):
        async for raw in conn:
            message = json.loads(raw)
            received.append(message)
            if message.get("type") == "subscribe":
                sid = message["subscriptionId"]
                await conn.send(json.dumps({
                    "protocolVersion": 2,
                    "subscriptionId": sid,
                    "op": "subscribed",
                    "query": message["query"],
                    "mode": "list",
                }))
                snapshot = {
                    "protocolVersion": 2,
                    "subscriptionId": sid,
                    "snapshotId": "snap-1",
                    "authoritative": True,
                    "mode": "list",
                    "entity": "Round/list",
                    "op": "snapshot",
                    "data": [{"key": "10", "data": {"id": 10}}],
                    "complete": True,
                }
                await conn.send(gzip.compress(json.dumps(snapshot).encode("utf-8")))

    async with serve_ws(handler) as url:
        manager, store, registry = make_manager(url)
        async with managed(manager):
            await manager.connect()
            assert manager.connection_state == "connected"
            assert manager.is_connected()

            lease = registry.subscribe({"view": "Round/list", "take": 2})
            sid = lease.subscription.subscription_id
            await wait_until(lambda: any(
                m.get("type") == "subscribe" for m in received
            ))
            envelope = next(m for m in received if m.get("type") == "subscribe")
            assert envelope == {
                "type": "subscribe",
                "protocolVersion": 2,
                "subscriptionId": sid,
                "query": {"view": "Round/list", "take": 2},
                "snapshot": {"enabled": True},
            }

            await wait_until(lambda: store.get_result(sid) is not None
                             and store.get_result(sid).keys == ("10",))
            result = store.get_result(sid)
            assert result.data == ({"id": 10},)
            assert result.is_loading is False

            lease.release()
            await wait_until(lambda: any(
                m.get("type") == "unsubscribe" for m in received
            ))
            unsub = next(m for m in received if m.get("type") == "unsubscribe")
            assert unsub == {
                "type": "unsubscribe",
                "protocolVersion": 2,
                "subscriptionId": sid,
            }
    assert manager.connection_state == "disconnected"


@pytest.mark.asyncio
async def test_reconnect_resubscribes_active_leases_with_same_id():
    subscribes = []
    connection_count = 0

    async def handler(conn):
        nonlocal connection_count
        connection_count += 1
        index = connection_count
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "subscribe":
                subscribes.append((index, message["subscriptionId"]))
                if index == 1:
                    await conn.close(code=1001)
                    return

    async with serve_ws(handler) as url:
        manager, _store, registry = make_manager(url)
        async with managed(manager):
            await manager.connect()
            lease = registry.subscribe({"view": "Round/list"})
            sid = lease.subscription.subscription_id

            await wait_until(lambda: len(subscribes) >= 2)
            assert subscribes[0] == (1, sid)
            assert subscribes[1] == (2, sid)  # stable id across reconnect
            await wait_until(lambda: manager.connection_state == "connected")


@pytest.mark.asyncio
async def test_subscription_queued_before_connect_is_flushed_on_open():
    received = []

    async def handler(conn):
        async for raw in conn:
            received.append(json.loads(raw))

    async with serve_ws(handler) as url:
        manager, _store, registry = make_manager(url)
        async with managed(manager):
            lease = registry.subscribe({"view": "Round/list"})
            assert received == []
            await manager.connect()
            await wait_until(lambda: any(
                m.get("type") == "subscribe" for m in received
            ))
            assert received[0]["subscriptionId"] == lease.subscription.subscription_id


@pytest.mark.asyncio
async def test_ping_keepalive_carries_protocol_version():
    pings = []

    async def handler(conn):
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "ping":
                pings.append(message)

    async with serve_ws(handler) as url:
        manager, _store, _registry = make_manager(url, ping_interval_seconds=0.05)
        async with managed(manager):
            await manager.connect()
            await wait_until(lambda: len(pings) >= 1)
            assert pings[0] == {"type": "ping", "protocolVersion": 2}


@pytest.mark.asyncio
async def test_structured_socket_issue_dispatch():
    async def handler(conn):
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "subscribe":
                await conn.send(json.dumps({
                    "type": "error",
                    "protocolVersion": 2,
                    "subscriptionId": None,
                    "error": "rate-limit-exceeded",
                    "message": "too many subscriptions",
                    "code": "rate-limit-exceeded",
                    "retryable": True,
                    "retry_after": 5,
                    "fatal": False,
                }))

    async with serve_ws(handler) as url:
        manager, _store, registry = make_manager(url)
        issues = []
        frames = []
        manager.on_socket_issue(issues.append)
        manager.on_frame(frames.append)
        async with managed(manager):
            await manager.connect()
            registry.subscribe({"view": "Round/list"})
            await wait_until(lambda: len(issues) >= 1)

            issue = issues[0]
            assert isinstance(issue, SocketIssue)
            assert issue.code == "rate-limit-exceeded"
            assert issue.retryable is True
            assert issue.retry_after == 5
            assert issue.fatal is False
            assert issue.subscription_id is None
            # The v2 error envelope is also dispatched as a frame.
            assert any(isinstance(f, ErrorFrame) for f in frames)


@pytest.mark.asyncio
async def test_processed_slot_tracking_and_waiting():
    async def handler(conn):
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "subscribe":
                await conn.send(json.dumps({
                    "protocolVersion": 2,
                    "subscriptionId": message["subscriptionId"],
                    "mode": "list",
                    "entity": "Round/list",
                    "op": "upsert",
                    "key": "1",
                    "data": {"id": 1},
                    "seq": "123:000000000001",
                }))

    async with serve_ws(handler) as url:
        manager, _store, registry = make_manager(url)
        async with managed(manager):
            await manager.connect()
            assert manager.processed_slot is None
            waiter = asyncio.create_task(manager.wait_for_processed_slot(123))
            registry.subscribe({"view": "Round/list"})
            assert await asyncio.wait_for(waiter, TIMEOUT) == 123
            assert manager.processed_slot == 123
            # Already-processed slots resolve immediately.
            assert await manager.wait_for_processed_slot(100) == 123

            with pytest.raises(ProcessedSlotTimeoutError) as excinfo:
                await manager.wait_for_processed_slot(999, timeout=0.05)
            assert excinfo.value.target_slot == 999
            assert excinfo.value.processed_slot == 123

            with pytest.raises(ValueError):
                await manager.wait_for_processed_slot(-1)


@pytest.mark.asyncio
async def test_refresh_auth_frame_is_scheduled_and_sent():
    refresh_messages = []
    connect_tokens = []

    async def handler(conn):
        query = parse_qs(urlparse(conn.request.path).query)
        connect_tokens.append(query.get("hs_token", [None])[0])
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "refresh_auth":
                refresh_messages.append(message)
                await conn.send(json.dumps({"success": True, "expires_at": 9999999999}))

    auth_state = FakeAuthState()
    async with serve_ws(handler) as url:
        manager, _store, _registry = make_manager(url, auth_state=auth_state)
        async with managed(manager):
            await manager.connect()
            assert connect_tokens == ["tok-1"]
            await wait_until(lambda: len(refresh_messages) >= 1)
            assert refresh_messages[0] == {
                "type": "refresh_auth",
                "protocolVersion": 2,
                "token": "tok-2",
            }
            # Connection stayed up: in-band refresh, no rotation.
            assert manager.connection_state == "connected"
            assert len(connect_tokens) == 1


@pytest.mark.asyncio
async def test_failed_refresh_auth_clears_token_and_rotates_connection():
    connection_count = 0

    async def handler(conn):
        nonlocal connection_count
        connection_count += 1
        async for raw in conn:
            message = json.loads(raw)
            if message.get("type") == "refresh_auth":
                await conn.send(json.dumps({
                    "success": False, "error": "token-expired",
                }))

    auth_state = FakeAuthState()
    async with serve_ws(handler) as url:
        manager, _store, _registry = make_manager(url, auth_state=auth_state)
        async with managed(manager):
            await manager.connect()
            await wait_until(lambda: connection_count >= 2)
            assert auth_state.cleared is True
            await wait_until(lambda: manager.connection_state == "connected")


@pytest.mark.asyncio
async def test_duplicate_subscription_id_with_different_query_is_rejected():
    manager, _store, _registry = make_manager("ws://127.0.0.1:1")
    first = Subscription("dup", {"view": "Round/list"})
    manager.subscribe(first)  # queued (not connected)
    manager.subscribe(first)  # same payload: idempotent
    with pytest.raises(SubscriptionError, match="already registered locally"):
        manager.subscribe(Subscription("dup", {"view": "Other/list"}))


@pytest.mark.asyncio
async def test_refresh_requires_an_active_subscription():
    manager, _store, _registry = make_manager("ws://127.0.0.1:1")
    with pytest.raises(SubscriptionError, match="Cannot refresh inactive subscription"):
        manager.refresh(Subscription("ghost", {"view": "Round/list"}))
    manager.subscribe(Subscription("s", {"view": "Round/list"}))
    with pytest.raises(SubscriptionError, match="Cannot change a subscription"):
        manager.refresh(Subscription("s", {"view": "Round/list", "take": 1}))


@pytest.mark.asyncio
async def test_http_only_mode_fails_subscriptions_fast():
    manager = ConnectionManager(None)
    with pytest.raises(AreteError, match="WebSocket transport is disabled"):
        manager.subscribe(Subscription("s", {"view": "Round/list"}))
    with pytest.raises(AreteError, match="WebSocket transport is disabled"):
        await manager.connect()


@pytest.mark.asyncio
async def test_initial_connect_failure_raises_and_reports_error_state():
    url = f"ws://127.0.0.1:{free_port()}"  # nothing is listening here
    manager, _store, _registry = make_manager(url)
    states = []
    manager.on_connection_state_change(lambda state, error=None: states.append(state))
    with pytest.raises(AreteError):
        await manager.connect()
    assert manager.connection_state == "error"
    assert states[0] == "connecting"


@pytest.mark.asyncio
async def test_reconnect_gives_up_after_max_attempts():
    async def handler(conn):
        async for _raw in conn:
            pass

    server = await serve(handler, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    manager, _store, _registry = make_manager(
        f"ws://127.0.0.1:{port}", reconnect_intervals=(0.02,), max_reconnect_attempts=2
    )
    async with managed(manager):
        await manager.connect()
        assert manager.connection_state == "connected"
        # Kill the server: the drop triggers reconnects that all fail, and the
        # manager gives up after max_reconnect_attempts.
        server.close()
        await server.wait_closed()
        await wait_until(lambda: manager.connection_state == "error")
