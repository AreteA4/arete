"""Tests for arete.operations: prepared values, composition, the signer
registry, and the executor (ported from operations.test.ts +
program-instructions.test.ts)."""

from __future__ import annotations

import httpx
import pytest

from arete.instructions import BuiltAccountMeta, BuiltInstruction, ErrorMetadata
from arete.operations import (
    OperationExecutionError,
    SignerRegistry,
    TransactionExecutionError,
    append_flow_transactions,
    append_transaction_instructions,
    create_prepared_flow,
    create_prepared_instruction,
    create_prepared_transaction,
    create_signer_registry,
    describe_prepared_operation,
    execute_prepared_operation,
    format_prepared_operation,
    infer_signer_address,
    inspect_prepared_operation,
    prepend_flow_transaction_instructions,
    prepend_transaction_instructions,
    to_json_value,
    unwrap_operation_execution_error,
)
from arete.http import HttpAuthClient
from arete.transactions import HttpTransactionTransport
from arete.wallet import (
    SendResult,
    TransactionFailureOutcome,
    TransactionInspectionResult,
    UnsupportedTransactionVersionError,
    WalletError,
    WalletExecutionContext,
)


def ix(program="program", signers=(), writable=()):
    return BuiltInstruction(
        program_id=program,
        accounts=[
            *(
                BuiltAccountMeta(pubkey=key, is_signer=True, is_writable=True)
                for key in signers
            ),
            *(
                BuiltAccountMeta(pubkey=key, is_signer=False, is_writable=True)
                for key in writable
            ),
        ],
        data=b"",
    )


class FakeHost:
    def __init__(self, results=None, wallet=None, public_key=None):
        self.wallet = wallet
        self.public_key = public_key
        self.calls = []
        self._results = list(results or [])

    async def transaction(self, instructions, **options):
        self.calls.append({"instructions": list(instructions), **options})
        if not self._results:
            return SendResult(signature="signature")
        result = self._results.pop(0)
        if isinstance(result, Exception):
            raise result
        return result


class TestPreparedComposition:
    def test_instruction_infers_signers_from_account_metas(self):
        prepared = create_prepared_instruction(
            name="deploy", instruction=ix(signers=["alice", "alice", "bob"])
        )
        assert prepared.kind == "instruction"
        assert prepared.transaction.required_signer_addresses == ("alice", "bob")
        assert prepared.plan.transactions == (prepared.transaction,)

    def test_transaction_requires_exactly_one_source(self):
        with pytest.raises(ValueError, match="exactly one"):
            create_prepared_transaction(name="both")
        with pytest.raises(ValueError, match="exactly one"):
            create_prepared_transaction(
                name="both",
                instructions=[ix()],
                operations=[create_prepared_instruction(name="a", instruction=ix())],
            )

    def test_transaction_composes_prepared_instructions_in_order(self):
        first = create_prepared_instruction(
            name="first",
            instruction=ix(program="p1", signers=["alice"]),
            errors=[ErrorMetadata(code=1, name="E1", msg="one")],
        )
        second = create_prepared_instruction(
            name="second",
            instruction=ix(program="p2", signers=["bob"]),
            errors=[ErrorMetadata(code=2, name="E2", msg="two")],
        )
        composed = create_prepared_transaction(
            name="combo", instructions=[first, second]
        )
        assert [i.program_id for i in composed.transaction.instructions] == ["p1", "p2"]
        assert composed.transaction.required_signer_addresses == ("alice", "bob")
        assert [e.code for e in composed.transaction.errors] == [1, 2]

    def test_transaction_honors_outer_metadata_overrides(self):
        prepared = create_prepared_instruction(
            name="inner",
            instruction=ix(signers=["alice"]),
            errors=[ErrorMetadata(code=1, name="E1", msg="one")],
        )
        composed = create_prepared_transaction(
            name="combo",
            instructions=[prepared, ix(signers=["bob"])],
            required_signer_addresses=["explicit"],
            errors=[],
        )
        assert composed.transaction.required_signer_addresses == ("explicit",)
        assert composed.transaction.errors == ()

    def test_transaction_composes_operations_atomically(self):
        instruction_op = create_prepared_instruction(
            name="one", instruction=ix(program="p1", signers=["alice"])
        )
        transaction_op = create_prepared_transaction(
            name="two", instructions=[ix(program="p2"), ix(program="p3")]
        )
        composed = create_prepared_transaction(
            name="atomic", operations=[instruction_op, transaction_op]
        )
        assert [i.program_id for i in composed.transaction.instructions] == [
            "p1",
            "p2",
            "p3",
        ]

    def test_empty_instructions_fail_closed(self):
        with pytest.raises(ValueError, match="at least one"):
            create_prepared_transaction(name="empty", instructions=[])
        with pytest.raises(ValueError, match="at least one"):
            create_prepared_flow(name="empty", transactions=[])

    def test_flow_prepend_append(self):
        flow = create_prepared_flow(
            name="flow",
            transactions=[{"name": "first", "instructions": [ix(signers=["alice"])]}],
        )
        flow = append_flow_transactions(
            flow, [{"name": "second", "instructions": [ix(signers=["bob"])]}]
        )
        assert [t.name for t in flow.plan.transactions] == ["first", "second"]

        flow = prepend_flow_transaction_instructions(
            flow, 1, [ix(signers=["carol"])]
        )
        assert flow.plan.transactions[1].required_signer_addresses == ("carol", "bob")
        with pytest.raises(ValueError, match="no transaction at index"):
            prepend_flow_transaction_instructions(flow, 5, [ix()])

    def test_transaction_body_prepend_append_signer_order(self):
        base = create_prepared_instruction(
            name="base", instruction=ix(signers=["alice"])
        ).transaction
        prepended = prepend_transaction_instructions(base, [ix(signers=["fee-payer"])])
        assert prepended.required_signer_addresses == ("fee-payer", "alice")
        appended = append_transaction_instructions(base, [ix(signers=["zed"])])
        assert appended.required_signer_addresses == ("alice", "zed")


class TestSignerRegistry:
    def test_round_trip(self):
        registry = create_signer_registry([("alice", {"kp": 1})])
        registry.register("bob", {"kp": 2})
        assert registry.has("alice") and registry.has("bob")
        assert registry.addresses() == ("alice", "bob")
        assert registry.get("alice") == {"kp": 1}
        assert len(registry.entries()) == 2
        assert registry.unregister("alice") is True
        assert registry.unregister("alice") is False
        registry.clear()
        assert len(registry) == 0

    def test_rejects_empty_addresses(self):
        registry = SignerRegistry()
        with pytest.raises(ValueError, match="must not be empty"):
            registry.register("", object())
        with pytest.raises(ValueError, match="must not be empty"):
            SignerRegistry([("", object())])

    def test_infer_signer_address(self):
        assert infer_signer_address("addr") == "addr"
        assert infer_signer_address({"address": "a"}) == "a"
        assert infer_signer_address({"public_key": "p"}) == "p"

        class Signer:
            pubkey = "pk"

        assert infer_signer_address(Signer()) == "pk"
        assert infer_signer_address({"opaque": True}) is None
        assert infer_signer_address("") is None


@pytest.mark.asyncio
class TestExecutePreparedOperation:
    async def test_rejects_required_signers_with_no_inferable_address(self):
        host = FakeHost()
        operation = create_prepared_flow(
            name="test-flow",
            transactions=[
                {
                    "name": "test-transaction",
                    "instructions": [ix()],
                    "required_signer_addresses": ["required"],
                }
            ],
        )
        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(
                host, operation, signers=[{"opaque_signer": True}]
            )
        error = excinfo.value
        assert error.outcome.status == "not-submitted"
        assert error.outcome.phase == "build"
        assert "Missing signer(s) for test-transaction: required" in str(error)
        assert host.calls == []

    async def test_opaque_signer_does_not_hide_missing_required_signer(self):
        host = FakeHost(public_key="wallet")
        operation = create_prepared_flow(
            name="test-flow",
            transactions=[
                {
                    "name": "test-transaction",
                    "instructions": [ix()],
                    "required_signer_addresses": ["wallet", "missing"],
                }
            ],
        )
        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(
                host, operation, signers=[{"opaque_signer": True}]
            )
        assert "Missing signer(s) for test-transaction: missing" in str(excinfo.value)
        assert host.calls == []

    async def test_per_call_wallet_override_does_not_inherit_the_host_key(self):
        """TS: `wallet?.publicKey ?? host.publicKey` — the host key is only a
        fallback for a wallet without one, so an override wallet that cannot
        sign fails closed before dispatch."""

        class OtherWallet:
            public_key = "BBB"

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        host = FakeHost(public_key="AAA")
        operation = create_prepared_flow(
            name="flow",
            transactions=[
                {
                    "name": "tx",
                    "instructions": [ix()],
                    "required_signer_addresses": ["AAA"],
                }
            ],
        )
        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation, wallet=OtherWallet())
        assert excinfo.value.outcome.status == "not-submitted"
        assert excinfo.value.outcome.phase == "build"
        assert "Missing signer(s) for tx: AAA" in str(excinfo.value)
        assert host.calls == []

    async def test_host_key_still_backs_a_wallet_without_a_public_key(self):
        class KeylessWallet:
            public_key = None

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        host = FakeHost(public_key="AAA")
        operation = create_prepared_flow(
            name="flow",
            transactions=[
                {
                    "name": "tx",
                    "instructions": [ix()],
                    "required_signer_addresses": ["AAA"],
                }
            ],
        )
        receipt = await execute_prepared_operation(host, operation, wallet=KeylessWallet())
        assert receipt.signatures == ("signature",)

    async def test_wallet_signer_addresses_satisfy_validation(self):
        class MultiWallet:
            public_key = "wallet"
            signer_addresses = ("delegate",)

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        host = FakeHost(wallet=MultiWallet())
        operation = create_prepared_flow(
            name="flow",
            transactions=[
                {
                    "name": "tx",
                    "instructions": [ix()],
                    "required_signer_addresses": ["wallet", "delegate"],
                }
            ],
        )
        receipt = await execute_prepared_operation(host, operation)
        assert receipt.signatures == ("signature",)

    async def test_registry_addresses_count_and_values_are_forwarded(self):
        registry = SignerRegistry([("registered", {"kp": 1})])
        host = FakeHost()
        operation = create_prepared_flow(
            name="flow",
            transactions=[
                {
                    "name": "tx",
                    "instructions": [ix()],
                    "required_signer_addresses": ["registered"],
                }
            ],
        )
        receipt = await execute_prepared_operation(
            host, operation, signer_registry=registry
        )
        assert receipt.signatures == ("signature",)
        assert host.calls[0]["signers"] == [{"kp": 1}]

    async def test_classifies_outcomeless_wallet_error_as_not_submitted_send(self):
        rejected = WalletError("User rejected the wallet request")
        host = FakeHost(results=[rejected], public_key="wallet")
        operation = create_prepared_instruction(name="wallet-rejection", instruction=ix())

        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        error = excinfo.value
        assert error.outcome.status == "not-submitted"
        assert error.outcome.phase == "send"
        assert error.completed_receipts == ()
        assert len(host.calls) == 1

    async def test_classified_wallet_rejection_is_not_submitted_wallet_phase(self):
        rejected = WalletError.from_outcome(
            TransactionFailureOutcome.not_submitted(
                "wallet", message="User rejected the request"
            )
        )
        host = FakeHost(results=[rejected], public_key="wallet")
        operation = create_prepared_instruction(name="wallet-rejection", instruction=ix())

        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        assert excinfo.value.outcome.status == "not-submitted"
        assert excinfo.value.outcome.phase == "wallet"

    async def test_preserves_known_signature_when_confirmation_unknown(self):
        timeout = WalletError.from_outcome(
            TransactionFailureOutcome.submitted_unknown(
                "known-signature",
                message="confirmation timed out",
            )
        )
        host = FakeHost(results=[timeout], public_key="wallet")
        operation = create_prepared_instruction(
            name="confirmation-timeout", instruction=ix()
        )

        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        error = excinfo.value
        assert error.outcome.status == "submitted-unknown"
        assert error.outcome.phase == "confirmation"
        assert error.signature == "known-signature"
        assert unwrap_operation_execution_error(error) is error.outcome

    async def test_resolves_program_error_against_transaction_metadata(self):
        failure = WalletError.from_outcome(
            TransactionFailureOutcome.chain_failed(
                signature="ore-signature",
                slot=123,
                program_error=ErrorMetadata(
                    code=6000, name="CustomError6000", msg="Unknown error with code 6000"
                ),
            )
        )
        host = FakeHost(results=[failure], public_key="wallet")
        operation = create_prepared_instruction(
            name="ore-deploy",
            instruction=ix(),
            errors=[ErrorMetadata(code=6000, name="OreProgramError", msg="ORE deploy failed")],
        )

        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        outcome = excinfo.value.outcome
        assert outcome.status == "chain-failed"
        assert outcome.signature == "ore-signature"
        assert outcome.slot == 123
        assert outcome.program_error.name == "OreProgramError"
        assert outcome.message == "OreProgramError (6000): ORE deploy failed"

    async def test_unstructured_adapter_failure_is_classified_as_chain_failed(self):
        """An adapter error carrying a signature and a raw InstructionError
        already executed on chain: the executor must report chain-failed with
        the resolved program error, never not-submitted."""

        class AdapterError(Exception):
            def __init__(self):
                super().__init__("Transaction simulation failed")
                self.signature = "SIG123"
                self.error = {"InstructionError": [0, {"Custom": 6001}]}

        cause = AdapterError()
        host = FakeHost(results=[cause], public_key="wallet")
        operation = create_prepared_instruction(
            name="ore-deploy",
            instruction=ix(),
            errors=[ErrorMetadata(code=6001, name="RoundClosed", msg="Round is closed")],
        )

        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        outcome = excinfo.value.outcome
        assert outcome.status == "chain-failed"
        assert outcome.phase == "chain"
        assert outcome.signature == "SIG123"
        assert outcome.program_error.name == "RoundClosed"
        assert excinfo.value.signature == "SIG123"
        assert unwrap_operation_execution_error(excinfo.value) is outcome

    async def test_keeps_confirmed_receipt_when_success_callback_throws(self):
        host = FakeHost(
            results=[SendResult(signature="confirmed-signature", slot=99)],
            public_key="wallet",
        )
        operation = create_prepared_instruction(
            name="confirmed-operation", instruction=ix(), artifacts={"confirmed": True}
        )
        observed = []
        callback_cause = RuntimeError("reconciliation failed")

        def on_success(event):
            raise callback_cause

        receipt = await execute_prepared_operation(
            host,
            operation,
            on_transaction_success=on_success,
            on_callback_error=observed.append,
        )
        assert receipt.transaction.signature == "confirmed-signature"
        assert receipt.transaction.slot == 99
        assert receipt.transaction.transaction_name == "confirmed-operation"
        assert len(receipt.callback_errors) == 1
        error = receipt.callback_errors[0]
        assert error.phase == "transaction-success"
        assert error.cause is callback_cause
        assert error.receipt.signature == "confirmed-signature"
        assert observed == [error]

    async def test_callback_error_observer_failures_do_not_alter_execution(self):
        host = FakeHost(public_key="wallet")
        operation = create_prepared_instruction(name="op", instruction=ix())

        def bad_start(event):
            raise RuntimeError("start observer failed")

        def bad_observer(error):
            raise RuntimeError("observer failed too")

        receipt = await execute_prepared_operation(
            host,
            operation,
            on_transaction_start=bad_start,
            on_callback_error=bad_observer,
        )
        assert receipt.signatures == ("signature",)
        assert receipt.callback_errors[0].phase == "transaction-start"

    async def test_preserves_completed_flow_receipts_on_later_failure(self):
        host = FakeHost(
            results=[
                SendResult(signature="first-signature", slot=10),
                WalletError.from_outcome(
                    TransactionFailureOutcome.chain_failed(
                        signature="second-signature",
                        slot=11,
                        message="second transaction failed",
                    )
                ),
            ],
            public_key="wallet",
        )
        operation = create_prepared_flow(
            name="partial-flow",
            transactions=[
                {"name": "first", "instructions": [ix()]},
                {"name": "second", "instructions": [ix()]},
            ],
        )
        with pytest.raises(OperationExecutionError) as excinfo:
            await execute_prepared_operation(host, operation)
        error = excinfo.value
        assert error.completed_receipts[0].signature == "first-signature"
        assert error.completed_receipts[0].slot == 10
        assert error.failed_transaction_index == 1
        assert error.outcome.status == "chain-failed"
        assert error.outcome.signature == "second-signature"
        assert error.outcome.slot == 11
        assert len(host.calls) == 2

    async def test_flow_signatures_in_execution_order(self):
        host = FakeHost(
            results=[SendResult(signature="one"), SendResult(signature="two")],
            public_key="wallet",
        )
        operation = create_prepared_flow(
            name="flow",
            transactions=[
                {"name": "first", "instructions": [ix()]},
                {"name": "second", "instructions": [ix()]},
            ],
        )
        receipt = await execute_prepared_operation(host, operation)
        assert receipt.kind == "flow"
        assert receipt.signatures == ("one", "two")
        assert [r.transaction_name for r in receipt.transactions] == ["first", "second"]

    async def test_transaction_errors_forwarded_to_host(self):
        host = FakeHost(public_key="wallet")
        metadata = ErrorMetadata(code=1, name="E", msg="m")
        operation = create_prepared_instruction(
            name="op", instruction=ix(), errors=[metadata]
        )
        await execute_prepared_operation(host, operation)
        assert host.calls[0]["errors"] == [metadata]


class TestOperationExecutionError:
    def make(self, **kwargs):
        operation = create_prepared_instruction(name="error-context", instruction=ix())
        return OperationExecutionError(
            operation=operation,
            failed_transaction=operation.transaction,
            failed_transaction_index=0,
            **kwargs,
        )

    def test_fallback_outcome_when_none_is_explicit(self):
        cause = RuntimeError("host failed without structured context")
        error = self.make(cause=cause)
        assert error.outcome.status == "not-submitted"
        assert error.outcome.phase == "send"
        assert error.outcome.cause is cause
        assert str(error) == (
            "[OPERATION_FAILED] Operation 'error-context' failed at transaction 1 "
            "(error-context): host failed without structured context"
        )
        assert unwrap_operation_execution_error(error) is error.outcome

    def test_structured_cause_context_wins_over_explicit_outcome(self):
        transaction_error = TransactionExecutionError(
            TransactionFailureOutcome.submitted_unknown(
                "submitted-signature", message="confirmation timed out"
            )
        )
        error = self.make(
            outcome=TransactionFailureOutcome.not_submitted(
                "build", message="stale explicit outcome"
            ),
            cause=transaction_error,
        )
        assert error.outcome is transaction_error.outcome
        assert error.signature == "submitted-signature"

    def test_recursively_exposes_nested_outcome(self):
        chain_outcome = TransactionFailureOutcome.chain_failed(
            signature="sig", program_error=ErrorMetadata(6000, "Ore", "failed")
        )
        inner = self.make(cause=WalletError.from_outcome(chain_outcome))
        outer = self.make(cause=inner)
        assert unwrap_operation_execution_error(outer) is chain_outcome
        assert outer.outcome is chain_outcome


@pytest.mark.asyncio
class TestInspectPreparedOperation:
    async def test_uses_unsigned_inspection_and_never_signs(self):
        calls = {"sign": 0, "inspect": []}

        class InspectingWallet:
            public_key = "wallet"

            async def sign_and_send(self, instructions, options=None, context=None):
                calls["sign"] += 1
                return SendResult(signature="sig")

            async def inspect_transaction(self, instructions, options=None, context=None):
                calls["inspect"].append(list(instructions))
                return {
                    "fee_lamports": 5000,
                    "context_slot": 50,
                    "error": {"InstructionError": [0, {"Custom": 6001}]},
                }

        operation = create_prepared_instruction(
            name="inspect-me",
            instruction=ix(),
            artifacts={"amount": 1},
            errors=[ErrorMetadata(code=6001, name="InspectionFailure", msg="would fail")],
        )
        result = await inspect_prepared_operation(InspectingWallet(), operation)
        assert result.description["kind"] == "instruction"
        assert result.description["name"] == "inspect-me"
        assert result.description["artifacts"] == {"amount": 1}
        assert result.program_error == ErrorMetadata(
            code=6001, name="InspectionFailure", msg="would fail"
        )
        assert calls["sign"] == 0
        assert len(calls["inspect"]) == 1

    async def test_rejects_flows_before_invoking_inspection(self):
        called = []

        class InspectingWallet:
            public_key = "wallet"

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

            async def inspect_transaction(self, instructions, options=None, context=None):
                called.append(instructions)
                return {}

        flow = create_prepared_flow(
            name="unsupported-flow",
            transactions=[{"name": "only-stage", "instructions": [ix()]}],
        )
        with pytest.raises(ValueError, match="flow inspection is not supported"):
            await inspect_prepared_operation(InspectingWallet(), flow)
        assert called == []

    async def test_rejects_wallets_without_inspection(self):
        class PlainWallet:
            public_key = "wallet"

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        operation = create_prepared_instruction(name="op", instruction=ix())
        with pytest.raises(WalletError, match="does not support"):
            await inspect_prepared_operation(PlainWallet(), operation)

    async def test_v1_contract_inspection_carries_the_simulated_data_size(self):
        """The relay's loadedAccountsDataSize has to survive all the way to
        the inspection result, or a V1 budget cannot be sized (contract §1,
        §4). The same test pins that inspection touches only the simulate
        route: no send, no signing."""
        routes = []
        signed = []

        def handler(request: httpx.Request) -> httpx.Response:
            routes.append(request.url.path)
            return httpx.Response(
                200,
                json={
                    "contextSlot": "50",
                    "unitsConsumed": "1200",
                    "loadedAccountsDataSize": "65536",
                },
            )

        transport = HttpTransactionTransport(
            "https://stack.example/",
            HttpAuthClient(
                http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler))
            ),
        )

        class SimulatingWallet:
            public_key = "wallet"

            async def sign_and_send(self, instructions, options=None, context=None):
                signed.append(instructions)
                return SendResult(signature="sig")

            async def inspect_transaction(self, instructions, options=None, context=None):
                simulation = await context.transaction_transport.simulate_transaction(
                    "unsigned-base64"
                )
                return TransactionInspectionResult(
                    compute_units_consumed=simulation.units_consumed,
                    context_slot=simulation.context_slot,
                    loaded_accounts_data_size=simulation.loaded_accounts_data_size,
                )

        operation = create_prepared_instruction(name="op", instruction=ix())
        result = await inspect_prepared_operation(
            SimulatingWallet(),
            operation,
            None,
            WalletExecutionContext(transaction_transport=transport),
        )
        assert result.transaction.loaded_accounts_data_size == 65536
        assert result.transaction.compute_units_consumed == 1200
        assert routes == ["/transactions/v1/simulate"]
        assert signed == []

    async def test_v1_contract_rejects_unsupported_version_before_inspecting(self):
        calls = {"sign": 0, "inspect": 0}

        class InspectingWallet:
            public_key = "wallet"

            async def sign_and_send(self, instructions, options=None, context=None):
                calls["sign"] += 1
                return SendResult(signature="sig")

            async def inspect_transaction(self, instructions, options=None, context=None):
                calls["inspect"] += 1
                return TransactionInspectionResult()

        operation = create_prepared_instruction(name="op", instruction=ix())
        with pytest.raises(UnsupportedTransactionVersionError):
            await inspect_prepared_operation(
                InspectingWallet(), operation, {"transaction_version": 1}
            )
        assert calls == {"sign": 0, "inspect": 0}

        # A v0 inspection against the same adapter still works: a missing
        # capability is unknown, not unsupported.
        assert (
            await inspect_prepared_operation(
                InspectingWallet(), operation, {"transaction_version": 0}
            )
        ).transaction == TransactionInspectionResult()


class TestDescription:
    def test_describe_is_json_safe(self):
        operation = create_prepared_instruction(
            name="describe-me",
            instruction=BuiltInstruction(
                program_id="program",
                accounts=[
                    BuiltAccountMeta(pubkey="alice", is_signer=True, is_writable=True)
                ],
                data=bytes([1, 2, 3]),
            ),
            artifacts={"amount": 10, "raw": b"\x01"},
            errors=[ErrorMetadata(code=1, name="E", msg="m")],
        )
        description = describe_prepared_operation(operation)
        assert description["artifacts"] == {"amount": 10, "raw": [1]}
        transaction = description["transactions"][0]
        assert transaction["required_signer_addresses"] == ["alice"]
        assert transaction["errors"] == [{"code": 1, "name": "E", "msg": "m"}]
        assert transaction["instructions"][0]["data"] == [1, 2, 3]

    def test_to_json_value_rejects_circular_values(self):
        circular = {}
        circular["self"] = circular
        with pytest.raises(ValueError, match="circular"):
            to_json_value(circular)

    def test_format_prepared_operation(self):
        operation = create_prepared_instruction(
            name="pretty", instruction=ix(signers=["alice"])
        )
        rendered = format_prepared_operation(operation)
        assert "instruction: pretty" in rendered
        assert "Transactions: 1" in rendered
        assert "Signers: alice" in rendered
