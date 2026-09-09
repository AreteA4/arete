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
    TransactionResourceOptions,
    UnsupportedTransactionVersionError,
    WalletAdapter,
    WalletError,
    WalletExecutionContext,
    ensure_transaction_version_supported,
    wallet_signer_addresses,
    wallet_supported_transaction_versions,
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


class TestTransactionResourceOptions:
    def test_v1_contract_wire_keys_are_camel_case_decimal_strings(self):
        options = TransactionResourceOptions(
            compute_unit_limit=200_000,
            loaded_accounts_data_size_limit=65_536,
            heap_size=32_768,
            priority_fee_lamports=10_000_000_000_000_000_001,
        )
        assert options.to_wire() == {
            "computeUnitLimit": "200000",
            "loadedAccountsDataSizeLimit": "65536",
            "heapSize": "32768",
            # Exact to the lamport: a u64 that round-tripped through a double
            # would come back as ...000.
            "priorityFeeLamports": "10000000000000000001",
        }

    def test_v1_contract_coerce_accepts_both_casings_and_decimal_strings(self):
        assert TransactionResourceOptions.coerce(
            {"computeUnitLimit": "200000", "heap_size": 1024}
        ) == TransactionResourceOptions(compute_unit_limit=200_000, heap_size=1024)
        assert TransactionResourceOptions.coerce(None) is None

    def test_v1_contract_rejects_unknown_resource_options(self):
        with pytest.raises(ValueError, match="Unsupported resource option"):
            TransactionResourceOptions.coerce({"priorityFee": 1})

    def test_v1_contract_rejects_lossy_and_out_of_range_quantities(self):
        with pytest.raises(ValueError, match="precision"):
            TransactionResourceOptions(priority_fee_lamports=1e19)
        with pytest.raises(ValueError, match="precision"):
            TransactionResourceOptions(compute_unit_limit=200_000.0)
        with pytest.raises(ValueError, match="bool"):
            TransactionResourceOptions(heap_size=True)
        with pytest.raises(ValueError, match="between 0 and 4294967295"):
            TransactionResourceOptions(compute_unit_limit=4_294_967_296)
        with pytest.raises(ValueError, match="decimal integer string"):
            TransactionResourceOptions(priority_fee_lamports="1_000")

    def test_v1_contract_fee_fields_are_mutually_exclusive(self):
        with pytest.raises(ValueError, match="mutually exclusive"):
            TransactionResourceOptions(
                priority_fee_lamports=5_000, compute_unit_price_micro_lamports=1
            )

    def test_merged_override_replaces_the_whole_fee_slot(self):
        base = TransactionResourceOptions(
            compute_unit_limit=1_000, compute_unit_price_micro_lamports=7
        )
        merged = base.merged(TransactionResourceOptions(priority_fee_lamports=5_000))
        assert merged.compute_unit_limit == 1_000
        assert merged.priority_fee_lamports == 5_000
        assert merged.compute_unit_price_micro_lamports is None


class TestTransactionVersionOptions:
    def test_v1_contract_version_is_legacy_or_a_bare_number(self):
        assert SendOptions(transaction_version="legacy").transaction_version == "legacy"
        assert SendOptions(transaction_version=1).transaction_version == 1
        for bad in ("0", "1", 2, True, "v0"):
            with pytest.raises(ValueError, match="transaction_version"):
                SendOptions(transaction_version=bad)

    def test_v1_contract_fee_fields_are_version_bound(self):
        # Construction no longer validates: a per-call override is coerced on
        # its own, before execution defaults merge into it, so a fee that is
        # valid against the configured version must survive being built alone.
        partial = SendOptions(resources={"priorityFeeLamports": "5000"})
        assert partial.resources.priority_fee_lamports == 5000

        # The EFFECTIVE options are what must be consistent, so validate() is
        # the enforcement point, and every dispatch path calls it.
        with pytest.raises(ValueError, match="priority_fee_lamports requires"):
            partial.validate()
        with pytest.raises(ValueError, match="priority_fee_lamports requires"):
            SendOptions(
                transaction_version="legacy", resources={"priorityFeeLamports": "5000"}
            ).validate()
        with pytest.raises(ValueError, match="not valid for transaction_version 1"):
            SendOptions(
                transaction_version=1,
                resources={"computeUnitPriceMicroLamports": "7"},
            ).validate()
        assert (
            SendOptions(transaction_version=1, resources={"priorityFeeLamports": "5000"})
            .validate()
            .resources.priority_fee_lamports
            == 5000
        )

    def test_coerce_maps_camel_case_version_instead_of_burying_it_in_extra(self):
        options = SendOptions.coerce({"transactionVersion": 1, "lookupTables": ["t"]})
        assert options.transaction_version == 1
        assert options.extra == {"lookupTables": ["t"]}

    def test_merged_carries_version_and_resources(self):
        base = SendOptions(
            confirmation_level="confirmed",
            resources={"computeUnitLimit": 1_000, "heapSize": 256},
        )
        merged = base.merged(
            SendOptions(
                transaction_version=1, resources={"priorityFeeLamports": "5000"}
            )
        )
        assert merged.transaction_version == 1
        assert merged.confirmation_level == "confirmed"
        assert merged.resources == TransactionResourceOptions(
            compute_unit_limit=1_000, heap_size=256, priority_fee_lamports=5_000
        )

    def test_merge_rejects_a_combination_it_would_otherwise_produce(self):
        base = SendOptions(resources={"computeUnitPriceMicroLamports": "7"})
        with pytest.raises(ValueError, match="not valid for transaction_version 1"):
            base.merged(SendOptions(transaction_version=1))


class TestTransactionVersionCapability:
    class Adapter:
        public_key = "wallet"

        def __init__(self, supported=None):
            if supported is not None:
                self.supported_transaction_versions = supported

        async def sign_and_send(self, instructions, options=None, context=None):
            return SendResult(signature="sig")

    def test_missing_capability_means_unknown_not_unsupported(self):
        adapter = self.Adapter()
        assert wallet_supported_transaction_versions(adapter) is None
        # Old callers and old adapters keep working.
        ensure_transaction_version_supported(adapter, None)
        ensure_transaction_version_supported(adapter, 0)
        ensure_transaction_version_supported(adapter, "legacy")

    def test_v1_contract_explicit_v1_against_undeclared_adapter_fails(self):
        with pytest.raises(UnsupportedTransactionVersionError) as info:
            ensure_transaction_version_supported(self.Adapter(), 1)
        error = info.value
        assert error.version == 1
        assert error.supported is None
        # Classified as a build-phase rejection, never a downgrade.
        assert error.outcome.status == "not-submitted"
        assert error.outcome.phase == "build"
        assert isinstance(error, WalletError)

    def test_v1_contract_declared_capability_is_authoritative(self):
        v0_only = self.Adapter(supported=(0,))
        assert wallet_supported_transaction_versions(v0_only) == (0,)
        ensure_transaction_version_supported(v0_only, 0)
        with pytest.raises(UnsupportedTransactionVersionError, match="supports"):
            ensure_transaction_version_supported(v0_only, "legacy")
        with pytest.raises(UnsupportedTransactionVersionError):
            ensure_transaction_version_supported(v0_only, 1)
        ensure_transaction_version_supported(self.Adapter(supported=[0, 1]), 1)

    def test_declared_capability_must_hold_known_versions(self):
        with pytest.raises(ValueError, match="supported_transaction_versions"):
            wallet_supported_transaction_versions(self.Adapter(supported=(2,)))
