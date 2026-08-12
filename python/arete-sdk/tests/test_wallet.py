"""Tests for arete.wallet: adapter protocol, send options, and the
classified transaction outcome model."""

from __future__ import annotations

import pytest

from arete.instructions import ErrorMetadata
from arete.wallet import (
    ConfirmedTransactionOutcome,
    SendOptions,
    SendResult,
    TransactionFailureOutcome,
    WalletAdapter,
    WalletError,
    WalletExecutionContext,
    wallet_signer_addresses,
)


class TestSendOptions:
    def test_defaults(self):
        options = SendOptions()
        assert options.confirmation_level is None
        assert options.skip_preflight is None
        assert options.signers is None
        assert options.extra == {}

    def test_rejects_unknown_confirmation_level(self):
        with pytest.raises(ValueError, match="confirmation_level"):
            SendOptions(confirmation_level="instant")

    def test_coerce_mapping_routes_unknown_keys_to_extra(self):
        options = SendOptions.coerce(
            {"skip_preflight": True, "priority_fee": 5000, "extra": {"nested": 1}}
        )
        assert options.skip_preflight is True
        assert options.extra == {"priority_fee": 5000, "nested": 1}

    def test_coerce_passthrough_and_none(self):
        options = SendOptions(confirmation_level="finalized")
        assert SendOptions.coerce(options) is options
        assert SendOptions.coerce(None) == SendOptions()

    def test_coerce_rejects_non_mapping(self):
        with pytest.raises(TypeError):
            SendOptions.coerce("finalized")

    def test_merged_overrides_win_and_extra_merges(self):
        base = SendOptions(
            confirmation_level="confirmed", skip_preflight=False, extra={"a": 1, "b": 1}
        )
        merged = base.merged(SendOptions(skip_preflight=True, extra={"b": 2}))
        assert merged.confirmation_level == "confirmed"
        assert merged.skip_preflight is True
        assert merged.extra == {"a": 1, "b": 2}

    def test_signers_normalized_to_tuple(self):
        options = SendOptions(signers=["s1", "s2"])
        assert options.signers == ("s1", "s2")
        assert SendOptions().with_signers(["s3"]).signers == ("s3",)


class TestOutcomeModel:
    def test_confirmed_outcome_shape(self):
        outcome = ConfirmedTransactionOutcome(signature="sig", slot=7)
        assert outcome.status == "confirmed"
        assert outcome.phase == "confirmation"

    def test_not_submitted_default_message(self):
        outcome = TransactionFailureOutcome.not_submitted("wallet")
        assert outcome.status == "not-submitted"
        assert outcome.message == "Transaction was not submitted during wallet"

    def test_message_prefers_cause_exception_text(self):
        cause = RuntimeError("connection reset")
        outcome = TransactionFailureOutcome.not_submitted("send", cause=cause)
        assert outcome.message == "connection reset"

    def test_submitted_unknown_requires_signature(self):
        with pytest.raises(ValueError, match="signature"):
            TransactionFailureOutcome(status="submitted-unknown", phase="confirmation")
        outcome = TransactionFailureOutcome.submitted_unknown("sig", slot=42)
        assert outcome.message == (
            "Transaction sig was submitted but its status is unknown"
        )

    def test_status_phase_combinations_are_validated(self):
        with pytest.raises(ValueError, match="status"):
            TransactionFailureOutcome(status="exploded", phase="send")
        with pytest.raises(ValueError, match="phase"):
            TransactionFailureOutcome.not_submitted("chain")
        with pytest.raises(ValueError, match="phase"):
            TransactionFailureOutcome.chain_failed(phase="send")

    def test_chain_failed_message_uses_program_error(self):
        error = ErrorMetadata(code=6000, name="AmountTooSmall", msg="Amount too small")
        outcome = TransactionFailureOutcome.chain_failed(
            signature="sig", program_error=error
        )
        assert outcome.message == "AmountTooSmall (6000): Amount too small"


class TestWalletError:
    def test_without_outcome_falls_back_to_phase(self):
        error = WalletError("connection reset")
        assert error.outcome is None
        outcome = error.into_outcome("send")
        assert outcome.status == "not-submitted"
        assert outcome.phase == "send"
        assert outcome.message == "connection reset"
        assert outcome.cause is error

    def test_prefers_attached_outcome(self):
        attached = TransactionFailureOutcome.submitted_unknown(
            "sig", slot=42, message="confirmation timed out"
        )
        error = WalletError.from_outcome(attached)
        assert str(error) == "[WALLET_ERROR] confirmation timed out"
        assert error.into_outcome("wallet") is attached


class TestWalletAdapterProtocol:
    def test_structural_conformance(self):
        class FakeWallet:
            public_key = "wallet-address"

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        assert isinstance(FakeWallet(), WalletAdapter)

    def test_signer_addresses_default_to_public_key(self):
        class FakeWallet:
            public_key = "wallet-address"

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        assert wallet_signer_addresses(FakeWallet()) == ("wallet-address",)

    def test_signer_addresses_include_declared_extras(self):
        class MultiWallet:
            public_key = "primary"
            signer_addresses = ("delegate", "primary")

            async def sign_and_send(self, instructions, options=None, context=None):
                return SendResult(signature="sig")

        assert wallet_signer_addresses(MultiWallet()) == ("delegate", "primary")
        assert wallet_signer_addresses(None) == ()

    def test_execution_context_carries_transport(self):
        transport = object()
        context = WalletExecutionContext(transaction_transport=transport)
        assert context.transaction_transport is transport
        assert WalletExecutionContext().transaction_transport is None
