"""Tests for the failure-classification ladder in arete.operations.

Port of ``typescript/core/src/instructions/error-parser.ts``
(``normalizeTransactionError`` + ``extractTransactionContext``) expressed in
the Python outcome-as-data model.
"""

from __future__ import annotations

import pytest

from arete.instructions import ErrorMetadata
from arete.operations import (
    TransactionExecutionError,
    classify_execution_failure,
)
from arete.wallet import TransactionFailureOutcome, WalletError

ROUND_CLOSED = ErrorMetadata(code=6001, name="RoundClosed", msg="Round is closed")


class AdapterError(Exception):
    """An adapter/host exception that carries raw RPC context, the way real
    Solana adapters (and the TS wallet adapters) raise them."""

    def __init__(self, message, *, signature=None, slot=None, error=None, code=None):
        super().__init__(message)
        if signature is not None:
            self.signature = signature
        if slot is not None:
            self.slot = slot
        if error is not None:
            self.error = error
        if code is not None:
            self.code = code


class TestChainFailureLadder:
    def test_adapter_error_with_signature_and_instruction_error_is_chain_failed(self):
        """Reviewer reproduction: an adapter error with `.signature` and a raw
        `InstructionError` payload already executed and reverted on chain — it
        must never be reported as `not-submitted` ("safe to retry")."""
        cause = AdapterError(
            "Transaction simulation failed",
            signature="SIG123",
            error={"InstructionError": [0, {"Custom": 6001}]},
        )

        outcome = classify_execution_failure(cause, [ROUND_CLOSED])

        assert outcome.status == "chain-failed"
        assert outcome.phase == "chain"
        assert outcome.signature == "SIG123"
        assert outcome.program_error == ROUND_CLOSED
        assert outcome.message == "RoundClosed (6001): Round is closed"
        assert outcome.cause is cause

    def test_instruction_error_without_metadata_gets_the_synthetic_program_error(self):
        cause = AdapterError(
            "custom program error: 0x1771",
            error={"InstructionError": [1, {"Custom": 6001}]},
        )
        outcome = classify_execution_failure(cause, [])
        assert outcome.status == "chain-failed"
        assert outcome.program_error == ErrorMetadata(
            code=6001, name="CustomError6001", msg="Unknown error with code 6001"
        )

    def test_slot_and_signature_are_recovered_through_the_cause_chain(self):
        inner = AdapterError("confirmation failed", signature="SIG-NESTED", slot=77)
        outer = RuntimeError("send failed")
        outer.__cause__ = inner
        outcome = classify_execution_failure(outer, [ROUND_CLOSED])
        assert outcome.status == "submitted-unknown"
        assert outcome.phase == "confirmation"
        assert outcome.signature == "SIG-NESTED"
        assert outcome.slot == 77

    def test_signature_without_program_error_is_submitted_unknown(self):
        cause = AdapterError("node dropped the connection", signature="SIG-ONLY")
        outcome = classify_execution_failure(cause)
        assert outcome.status == "submitted-unknown"
        assert outcome.phase == "confirmation"
        assert outcome.signature == "SIG-ONLY"
        assert outcome.cause is cause

    def test_non_deterministic_direct_code_still_classifies_as_chain_failed(self):
        cause = AdapterError("program failed", code=6001)
        outcome = classify_execution_failure(cause, [])
        assert outcome.status == "chain-failed"
        assert outcome.program_error.code == 6001

    def test_deterministic_direct_code_resolves_against_metadata(self):
        cause = AdapterError("program failed", code=6001)
        outcome = classify_execution_failure(cause, [ROUND_CLOSED])
        assert outcome.status == "chain-failed"
        assert outcome.program_error == ROUND_CLOSED

    def test_deterministic_match_upgrades_a_structured_send_failure(self):
        """A structured outcome no longer masks a chain failure hiding in the
        cause (TS runs the deterministic match before the existing outcome)."""
        cause = TransactionExecutionError(
            TransactionFailureOutcome.not_submitted(
                "send",
                message="send failed",
                cause=AdapterError(
                    "rpc rejected",
                    signature="SIG-UPGRADE",
                    error={"InstructionError": [0, {"Custom": 6001}]},
                ),
            )
        )
        outcome = classify_execution_failure(cause, [ROUND_CLOSED])
        assert outcome.status == "chain-failed"
        assert outcome.signature == "SIG-UPGRADE"
        assert outcome.program_error == ROUND_CLOSED


class TestWalletRejection:
    @pytest.mark.parametrize(
        "cause",
        [
            AdapterError("Something went wrong", code=4001),
            AdapterError("Something went wrong", code="4001"),
            AdapterError("Something went wrong", code="ACTION_REJECTED"),
            AdapterError("User rejected the request"),
            AdapterError("Transaction was declined by the user."),
            type("UserRejectedRequestError", (Exception,), {})("nope"),
        ],
    )
    def test_recognized_rejections_are_not_submitted_in_the_wallet_phase(self, cause):
        outcome = classify_execution_failure(cause)
        assert outcome.status == "not-submitted"
        assert outcome.phase == "wallet"

    def test_unrelated_failures_fall_back_to_the_send_phase(self):
        cause = AdapterError("blockhash not found")
        outcome = classify_execution_failure(cause)
        assert outcome.status == "not-submitted"
        assert outcome.phase == "send"
        assert outcome.cause is cause

    def test_fallback_phase_is_configurable(self):
        outcome = classify_execution_failure(RuntimeError("compile failed"), (), "wallet")
        assert outcome.status == "not-submitted"
        assert outcome.phase == "wallet"

    def test_outcome_less_wallet_errors_keep_adapter_owned_classification(self):
        """Python divergence: adapters classify their own rejections through
        an attached outcome, so the message heuristic is skipped for
        WalletError (it stays not-submitted/send)."""
        outcome = classify_execution_failure(WalletError("User rejected the request"))
        assert outcome.status == "not-submitted"
        assert outcome.phase == "send"
        assert outcome.message == "User rejected the request"


class TestExistingOutcomes:
    def test_structured_outcome_without_a_program_error_is_returned_unchanged(self):
        existing = TransactionFailureOutcome.submitted_unknown(
            "known-signature", message="confirmation timed out"
        )
        assert classify_execution_failure(WalletError.from_outcome(existing)) is existing

    def test_resolved_program_errors_survive_missing_metadata(self):
        existing = TransactionFailureOutcome.chain_failed(
            signature="sig",
            program_error=ErrorMetadata(6001, "RoundClosed", "Round is closed"),
        )
        outcome = classify_execution_failure(WalletError.from_outcome(existing), [])
        assert outcome is existing
        assert outcome.program_error.name == "RoundClosed"

    def test_chain_failed_outcomes_are_re_resolved_against_metadata(self):
        existing = TransactionFailureOutcome.chain_failed(
            signature="sig",
            slot=9,
            program_error=ErrorMetadata(
                6001, "CustomError6001", "Unknown error with code 6001"
            ),
        )
        outcome = classify_execution_failure(
            WalletError.from_outcome(existing), [ROUND_CLOSED]
        )
        assert outcome.program_error == ROUND_CLOSED
        assert outcome.signature == "sig"
        assert outcome.slot == 9
        assert outcome.message == "RoundClosed (6001): Round is closed"
