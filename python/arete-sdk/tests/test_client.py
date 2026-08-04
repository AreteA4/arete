"""Tests for arete.client: connect lifecycle (fake connection factory),
http-only mode, chain/transactions wiring, and the execute path."""

from __future__ import annotations

import asyncio
import dataclasses
import json

import pytest

from arete.chain import HttpChainClient
from arete.client import Arete, validate_program_reads
from arete.errors import AreteError
from arete.gateway import (
    HostedSolanaGatewayBindings,
    HostedSolanaGatewayCapabilityBinding,
    SolanaGatewayAuthMetadata,
)
from arete.instructions import (
    AccountMeta,
    ArgSchema,
    ErrorMetadata,
    InstructionHandler,
    Signer,
    UserProvided,
    encode_base58,
)
from arete.operations import (
    TransactionExecutionError,
    create_prepared_instruction,
)
from arete.stack import ProgramDef, StackDef, StackEndpoints
from arete.transactions import HttpTransactionTransport
from arete.views import ViewDef
from arete.wallet import SendResult, TransactionFailureOutcome, WalletError

TIMEOUT = 3.0
PROGRAM_ID = encode_base58(bytes([7] * 32))
ALICE = encode_base58(bytes([1] * 32))
BOB = encode_base58(bytes([2] * 32))

DEPLOY_HANDLER = InstructionHandler(
    program_id=PROGRAM_ID,
    discriminator=bytes([1]),
    accounts=[
        AccountMeta("signer", True, True, Signer()),
        AccountMeta("miner", False, True, UserProvided()),
    ],
    args=[ArgSchema("amount", "u64")],
    errors=[ErrorMetadata(code=6000, name="AmountTooSmall", msg="Amount too small")],
)


def make_stack(**overrides):
    values = dict(
        name="ore-stream",
        endpoints=StackEndpoints(ws="wss://example.test/ws"),
        views={
            "ore_round": {
                "latest": ViewDef(mode="list", view="OreRound/latest"),
            }
        },
        programs={
            "ore": ProgramDef(
                name="ore",
                program_id=PROGRAM_ID,
                raw_instructions={"deploy": DEPLOY_HANDLER},
            )
        },
    )
    values.update(overrides)
    return StackDef(**values)


class FakeWebSocket:
    def __init__(self):
        self.sent = []
        self._queue: "asyncio.Queue" = asyncio.Queue()
        self.close_code = None
        self.close_reason = ""

    async def send(self, payload):
        self.sent.append(payload)

    async def close(self, code=1000, reason=""):
        self.close_code = code
        self._queue.put_nowait(None)

    def push(self, message):
        self._queue.put_nowait(
            message if isinstance(message, str) else json.dumps(message)
        )

    def __aiter__(self):
        return self

    async def __anext__(self):
        item = await self._queue.get()
        if item is None:
            raise StopAsyncIteration
        return item


class FakeConnectFactory:
    def __init__(self):
        self.urls = []
        self.sockets = []

    async def __call__(self, url, headers):
        self.urls.append(url)
        ws = FakeWebSocket()
        self.sockets.append(ws)
        return ws


class FakeWallet:
    def __init__(self, results=None, public_key=ALICE):
        self.public_key = public_key
        self.calls = []
        self._results = list(results or [])

    async def sign_and_send(self, instructions, options=None, context=None):
        self.calls.append(
            {"instructions": list(instructions), "options": options, "context": context}
        )
        if self._results:
            result = self._results.pop(0)
            if isinstance(result, Exception):
                raise result
            return result
        return SendResult(signature="sig")


GATEWAY_ID = "sgb_00000000000000000000000000000001"


def gateway_binding(scopes):
    return HostedSolanaGatewayCapabilityBinding(
        endpoint="https://solana.example.test/gateway/",
        auth_policy="signed_session",
        solana_gateway_binding_id=GATEWAY_ID,
        cluster="mainnet-beta",
        region="us-west-1",
        auth=SolanaGatewayAuthMetadata(
            required=True,
            mode="signed_session",
            session_endpoint="https://api.example.test/ws/sessions",
            jwks_url="https://api.example.test/.well-known/jwks.json",
            token_transport="bearer",
            audience="arete:solana-gateway",
            target_kind="solana-gateway-binding",
            target_id=GATEWAY_ID,
            scopes=tuple(scopes),
            accepted_key_classes=("publishable",),
            transaction_entitlement_required=False,
        ),
    )


@pytest.mark.asyncio
class TestConnectLifecycle:
    async def test_connect_and_disconnect(self):
        factory = FakeConnectFactory()
        a4 = await Arete.connect(make_stack(), connect_factory=factory)
        try:
            assert a4.is_connected()
            assert a4.connection_state == "connected"
            assert factory.urls == ["wss://example.test/ws"]
            assert a4.stack_name == "ore-stream"
        finally:
            await a4.disconnect()
        assert not a4.is_connected()
        assert a4.connection_state == "disconnected"

    async def test_auto_connect_false_defers_the_socket(self):
        factory = FakeConnectFactory()
        a4 = await Arete.connect(
            make_stack(), auto_connect=False, connect_factory=factory
        )
        assert not a4.is_connected()
        assert factory.urls == []
        await a4.disconnect()

    async def test_url_option_overrides_stack_endpoint(self):
        factory = FakeConnectFactory()
        a4 = await Arete.connect(
            make_stack(), url="wss://override.test/ws", connect_factory=factory
        )
        assert factory.urls == ["wss://override.test/ws"]
        await a4.disconnect()

    async def test_missing_websocket_url_fails_closed(self):
        stack = make_stack(endpoints=StackEndpoints(ws=""))
        with pytest.raises(AreteError, match="WebSocket URL is required"):
            await Arete.connect(stack)

    async def test_invalid_transport_rejected(self):
        with pytest.raises(AreteError, match="transport"):
            await Arete.connect(make_stack(), transport="carrier-pigeon")

    async def test_async_context_manager(self):
        factory = FakeConnectFactory()
        async with await Arete.connect(make_stack(), connect_factory=factory) as a4:
            assert a4.is_connected()
        assert not a4.is_connected()

    async def test_connection_state_hook(self):
        factory = FakeConnectFactory()
        states = []
        a4 = await Arete.connect(
            make_stack(), auto_connect=False, connect_factory=factory
        )
        unsubscribe = a4.on_connection_state_change(
            lambda state, error=None: states.append(state)
        )
        await a4.connect_socket()
        assert "connected" in states
        unsubscribe()
        await a4.disconnect()

    async def test_processed_slot_cursor(self):
        factory = FakeConnectFactory()
        a4 = await Arete.connect(make_stack(), connect_factory=factory)
        try:
            assert a4.processed_slot is None
            waiter = asyncio.create_task(a4.wait_for_processed_slot(42, timeout=TIMEOUT))
            await asyncio.sleep(0)
            factory.sockets[0].push(
                {
                    "protocolVersion": 2,
                    "subscriptionId": "sub-1",
                    "mode": "list",
                    "entity": "OreRound/latest",
                    "op": "upsert",
                    "key": "1",
                    "data": {"id": 1},
                    "seq": "42:00000001",
                }
            )
            assert await asyncio.wait_for(waiter, TIMEOUT) == 42
            assert a4.processed_slot == 42
        finally:
            await a4.disconnect()


@pytest.mark.asyncio
class TestHttpOnlyMode:
    async def test_views_fail_fast_with_websocket_disabled(self):
        a4 = await Arete.connect(
            make_stack(endpoints=StackEndpoints(ws="", http="https://api.example.test")),
            transport="http",
        )
        assert not a4.is_connected()
        with pytest.raises(AreteError) as excinfo:
            await a4.views.ore_round.latest.get()
        assert excinfo.value.code == "WEBSOCKET_DISABLED"

    async def test_http_mode_requires_an_http_endpoint(self):
        # Not independently readable (no programs at all) and no HTTP base.
        stack = make_stack(endpoints=StackEndpoints(ws=""), programs={})
        with pytest.raises(AreteError, match="HTTP endpoint is required"):
            await Arete.connect(stack, transport="http")

    async def test_http_mode_allows_zero_account_program_only_stacks(self):
        stack = StackDef(
            name="program-only",
            endpoints=StackEndpoints(ws=""),
            programs={
                "ore": ProgramDef(
                    name="ore",
                    program_id=PROGRAM_ID,
                    raw_instructions={"deploy": DEPLOY_HANDLER},
                )
            },
        )
        a4 = await Arete.connect(stack, transport="http")
        built = a4.programs.ore.raw.deploy.build(amount=1, signer=ALICE, miner=BOB)
        assert built.program_id == PROGRAM_ID
        with pytest.raises(AreteError, match="no HTTP endpoint"):
            a4.chain

    async def test_implicit_http_for_viewless_program_only_stack(self):
        # No transport given, no ws endpoint, no views, programs independently
        # readable (zero-account) -> implicit http-only connect succeeds.
        stack = StackDef(
            name="program-only",
            programs={
                "ore": ProgramDef(name="ore", program_id=PROGRAM_ID)
            },
        )
        a4 = await Arete.connect(stack)
        assert not a4.is_connected()


@pytest.mark.asyncio
class TestTransportWiring:
    async def test_default_chain_and_transactions_over_stack_http_endpoint(self):
        a4 = await Arete.connect(
            make_stack(
                endpoints=StackEndpoints(
                    ws="wss://example.test/ws", http="https://api.example.test"
                )
            ),
            auto_connect=False,
        )
        assert isinstance(a4.chain, HttpChainClient)
        assert a4.chain._base_url == "https://api.example.test"
        assert isinstance(a4.transactions, HttpTransactionTransport)

    async def test_http_endpoint_derives_from_websocket_url(self):
        a4 = await Arete.connect(make_stack(), auto_connect=False)
        assert a4.chain._base_url == "https://example.test/ws"

    async def test_http_url_option_wins(self):
        a4 = await Arete.connect(
            make_stack(), http_url="https://explicit.example.test", auto_connect=False
        )
        assert a4.chain._base_url == "https://explicit.example.test"

    async def test_injected_transports_win(self):
        chain = object()
        transactions = object()
        a4 = await Arete.connect(
            make_stack(), chain=chain, transactions=transactions, auto_connect=False
        )
        assert a4.chain is chain
        assert a4.transactions is transactions

    async def test_gateway_bindings_wire_chain_and_transactions(self):
        stack = make_stack(
            gateway=HostedSolanaGatewayBindings(
                chain=gateway_binding(["read"]),
                transactions=gateway_binding(
                    ["transaction:inspect", "transaction:send"]
                ),
            )
        )
        a4 = await Arete.connect(stack, auto_connect=False)
        assert isinstance(a4.chain, HttpChainClient)
        assert a4.chain._base_url == "https://solana.example.test/gateway"
        assert isinstance(a4.transactions, HttpTransactionTransport)

    async def test_program_reads_key_mismatch_fails_closed(self):
        stack = make_stack()
        descriptor_stack = dataclasses.replace(stack)
        descriptor_stack.program_reads = {"nope": object()}
        with pytest.raises(AreteError, match="must exactly match"):
            validate_program_reads(descriptor_stack)


@pytest.mark.asyncio
class TestExecution:
    async def make_client(self, wallet=None, **connect_kwargs):
        return await Arete.connect(
            make_stack(
                endpoints=StackEndpoints(ws="", http="https://api.example.test")
            ),
            transport="http",
            wallet=wallet,
            **connect_kwargs,
        )

    async def test_transaction_requires_a_wallet(self):
        a4 = await self.make_client()
        with pytest.raises(TransactionExecutionError) as excinfo:
            await a4.transaction([])
        assert excinfo.value.outcome.status == "not-submitted"
        assert excinfo.value.outcome.phase == "wallet"

    async def test_transaction_passes_transport_context_and_signers(self):
        wallet = FakeWallet()
        a4 = await self.make_client(wallet=wallet)
        built = a4.programs.ore.raw.deploy.build(amount=1, miner=BOB)
        result = await a4.transaction([built], signers=["extra"], send={"skip_preflight": True})
        assert result == SendResult(signature="sig")
        call = wallet.calls[0]
        assert call["context"].transaction_transport is a4.transactions
        assert call["options"].signers == ("extra",)
        assert call["options"].skip_preflight is True

    async def test_transaction_resolves_chain_failure_against_stack_errors(self):
        failure = WalletError.from_outcome(
            TransactionFailureOutcome.chain_failed(
                signature="sig",
                program_error=ErrorMetadata(
                    code=6000, name="CustomError6000", msg="Unknown error with code 6000"
                ),
            )
        )
        wallet = FakeWallet(results=[failure])
        a4 = await self.make_client(wallet=wallet)
        with pytest.raises(TransactionExecutionError) as excinfo:
            await a4.transaction([])
        assert excinfo.value.outcome.program_error.name == "AmountTooSmall"

    async def test_transaction_parses_unstructured_chain_failures_against_errors(self):
        """The documented contract: every adapter failure runs through the
        classification ladder, so a raw InstructionError payload resolves
        against the stack's error metadata."""

        class AdapterError(Exception):
            def __init__(self):
                super().__init__("Transaction simulation failed")
                self.signature = "SIG123"
                self.error = {"InstructionError": [0, {"Custom": 6000}]}

        wallet = FakeWallet(results=[AdapterError()])
        a4 = await self.make_client(wallet=wallet)
        with pytest.raises(TransactionExecutionError) as excinfo:
            await a4.transaction([])
        outcome = excinfo.value.outcome
        assert outcome.status == "chain-failed"
        assert outcome.signature == "SIG123"
        assert outcome.program_error.name == "AmountTooSmall"

    async def test_execute_with_wallet_override_fails_closed_on_default_signer(self):
        """The client's default wallet address must not satisfy signers for a
        per-call wallet override that cannot sign."""
        default_wallet = FakeWallet(public_key=ALICE)
        other = FakeWallet(public_key=BOB)
        a4 = await self.make_client(wallet=default_wallet)
        built = a4.programs.ore.raw.deploy.build(amount=1, signer=ALICE, miner=BOB)
        prepared = create_prepared_instruction(name="ore.deploy", instruction=built)
        from arete.operations import OperationExecutionError

        with pytest.raises(OperationExecutionError) as excinfo:
            await a4.execute(prepared, wallet=other)
        assert excinfo.value.outcome.status == "not-submitted"
        assert excinfo.value.outcome.phase == "build"
        assert f"Missing signer(s) for ore.deploy: {ALICE}" in str(excinfo.value)
        assert other.calls == []
        assert default_wallet.calls == []

    async def test_execute_prepared_instruction_with_default_wallet(self):
        wallet = FakeWallet(results=[SendResult(signature="deploy-sig", slot=5)])
        a4 = await self.make_client(wallet=wallet)
        built = a4.programs.ore.raw.deploy.build(amount=1, miner=BOB)
        prepared = create_prepared_instruction(name="ore.deploy", instruction=built)
        receipt = await a4.execute(prepared)
        assert receipt.kind == "instruction"
        assert receipt.signatures == ("deploy-sig",)
        assert receipt.transaction.slot == 5
        # The wallet saw the client's transaction transport.
        assert wallet.calls[0]["context"].transaction_transport is a4.transactions

    async def test_execute_fails_closed_on_missing_signer_without_wallet(self):
        a4 = await self.make_client()
        built = a4.programs.ore.raw.deploy.build(amount=1, signer=ALICE, miner=BOB)
        prepared = create_prepared_instruction(name="ore.deploy", instruction=built)
        from arete.operations import OperationExecutionError

        with pytest.raises(OperationExecutionError) as excinfo:
            await a4.execute(prepared)
        assert excinfo.value.outcome.phase == "build"

    async def test_execution_defaults_merge_under_call_options(self):
        wallet = FakeWallet()
        events = {"default": 0, "call": 0}
        a4 = await self.make_client(
            wallet=wallet,
            execution={
                "send": {"confirmation_level": "finalized"},
                "on_transaction_success": lambda event: events.__setitem__(
                    "default", events["default"] + 1
                ),
            },
        )
        built = a4.programs.ore.raw.deploy.build(amount=1, miner=BOB)
        prepared = create_prepared_instruction(name="ore.deploy", instruction=built)
        await a4.execute(
            prepared,
            send={"skip_preflight": True},
            on_transaction_success=lambda event: events.__setitem__(
                "call", events["call"] + 1
            ),
        )
        options = wallet.calls[0]["options"]
        assert options.confirmation_level == "finalized"
        assert options.skip_preflight is True
        assert events == {"default": 1, "call": 1}

    async def test_unknown_execution_default_rejected(self):
        with pytest.raises(AreteError, match="Unknown execution default"):
            await self.make_client(execution={"on_typo": lambda event: None})

    async def test_set_wallet(self):
        a4 = await self.make_client()
        assert a4.wallet is None
        assert a4.public_key is None
        wallet = FakeWallet()
        a4.set_wallet(wallet)
        assert a4.wallet is wallet
        assert a4.public_key == ALICE
        a4.set_wallet(None)
        assert a4.wallet is None

    async def test_inspect_operation_uses_wallet_inspection(self):
        class InspectingWallet(FakeWallet):
            async def inspect_transaction(self, instructions, options=None, context=None):
                self.calls.append({"inspect": list(instructions), "context": context})
                return {"error": {"InstructionError": [0, {"Custom": 6000}]}}

        wallet = InspectingWallet()
        a4 = await self.make_client(wallet=wallet)
        built = a4.programs.ore.raw.deploy.build(amount=1, miner=BOB)
        prepared = create_prepared_instruction(
            name="ore.deploy",
            instruction=built,
            errors=[ErrorMetadata(code=6000, name="AmountTooSmall", msg="Amount too small")],
        )
        inspection = await a4.inspect_operation(prepared)
        assert inspection.program_error.name == "AmountTooSmall"
        assert wallet.calls[0]["context"].transaction_transport is a4.transactions
