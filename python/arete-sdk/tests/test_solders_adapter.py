"""Tests for the optional solders transaction adapter.

This module imports ``solders`` unconditionally: it is the adapter suite, and a
base environment must omit it (``--ignore``) rather than silently skip it, so a
missing extra in an adapter environment fails loudly.

The conformance tests replay ``tests/fixtures/transaction-v1/transactions.json``
-- real signed payloads produced by ``@solana/kit`` 8.2.0 from the same
deterministic keys and blockhash. Every byte the adapter puts on the wire is
compared against that independent codec, so nothing here asserts the adapter
against itself.
"""

from __future__ import annotations

import base64
import json
import subprocess
import sys
from pathlib import Path

import pytest
from solders.compute_budget import (
    request_heap_frame,
    set_compute_unit_limit,
    set_compute_unit_price,
    set_loaded_accounts_data_size_limit,
)
from solders.keypair import Keypair
from solders.message import MessageV1
from solders.transaction import VersionedTransaction

from arete.adapters.solders import SoldersAdapterConfig, SoldersWalletAdapter
from arete.instructions import BuiltAccountMeta, BuiltInstruction
from arete.transactions import (
    LatestBlockhashResult,
    TransactionFeeResult,
    TransactionSendResult,
    TransactionSignatureStatus,
    TransactionSimulationResult,
    TransactionTransportError,
)
from arete.wallet import SendOptions, WalletError, WalletExecutionContext

MEMO_PROGRAM = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
COMPUTE_BUDGET_PROGRAM = "ComputeBudget111111111111111111111111111111"
# The fixtures' lifetime blockhash: the all-ones system address, unreplayable.
FIXTURE_BLOCKHASH = "11111111111111111111111111111111"
U64_MAX = 0xFFFF_FFFF_FFFF_FFFF

FIXTURES = json.loads(
    (
        Path(__file__).resolve().parents[3]
        / "tests"
        / "fixtures"
        / "transaction-v1"
        / "transactions.json"
    ).read_text()
)

PAYER = Keypair.from_seed(bytes([1] * 32))
COSIGNER = Keypair.from_seed(bytes([2] * 32))


def fixture(name):
    return FIXTURES["fixtures"][name]


def memo(data, *, signers=(), readonly=()):
    """The fixture generator's memo instruction: writable signers, then extras."""
    accounts = [
        BuiltAccountMeta(pubkey=address, is_signer=True, is_writable=True)
        for address in signers
    ] + [
        BuiltAccountMeta(pubkey=address, is_signer=False, is_writable=False)
        for address in readonly
    ]
    return BuiltInstruction(
        program_id=MEMO_PROGRAM, accounts=accounts, data=bytes(data)
    )


def status(level="confirmed", *, slot=42, err=None):
    return TransactionSignatureStatus(
        signature="", slot=slot, confirmation_status=level, err=err
    )


def simulation(*, units=1200, size=4096, err=None, logs=("Program log: ok",)):
    return TransactionSimulationResult(
        context_slot=7,
        err=err,
        logs=None if logs is None else list(logs),
        units_consumed=units,
        loaded_accounts_data_size=size,
    )


class FakeTransport:
    """Recording :class:`arete.transactions.TransactionTransport`.

    ``calls`` is the ordered relay surface the adapter touched, which is how
    "inspection never submits" and "submit exactly once" are asserted.
    """

    def __init__(
        self,
        *,
        blockhash=FIXTURE_BLOCKHASH,
        last_valid_block_height=100,
        block_height=10,
        statuses=None,
        result=None,
        fee=5000,
        send_error=None,
        relay_signature=None,
        allow_send=True,
    ):
        self.calls = []
        self.sent = []
        self.fee_messages = []
        self.simulated = []
        self.blockhash = blockhash
        self.last_valid_block_height = last_valid_block_height
        self.block_height = block_height
        self.statuses = list(statuses) if statuses is not None else [status()]
        self.result = result if result is not None else simulation()
        self.fee = fee
        self.send_error = send_error
        self.relay_signature = relay_signature
        self.allow_send = allow_send

    async def get_latest_blockhash(self, *, commitment=None, min_context_slot=None):
        self.calls.append("latest-blockhash")
        return LatestBlockhashResult(
            blockhash=self.blockhash,
            context_slot=1,
            last_valid_block_height=self.last_valid_block_height,
        )

    async def get_fee_for_message(
        self, message, *, commitment=None, min_context_slot=None
    ):
        self.calls.append("fee")
        self.fee_messages.append(message)
        return TransactionFeeResult(fee_lamports=self.fee, context_slot=3)

    async def simulate_transaction(
        self,
        transaction,
        *,
        commitment=None,
        min_context_slot=None,
        accounts=None,
        inner_instructions=None,
        replace_recent_blockhash=None,
    ):
        self.calls.append("simulate")
        self.simulated.append(transaction)
        return self.result

    async def send_transaction(
        self,
        transaction,
        *,
        skip_preflight=None,
        preflight_commitment=None,
        min_context_slot=None,
    ):
        assert self.allow_send, "the transaction must not be submitted"
        self.calls.append("send")
        self.sent.append(transaction)
        if self.send_error is not None:
            raise self.send_error
        signature = self.relay_signature
        if signature is None:
            signature = str(VersionedTransaction.from_bytes(
                base64.b64decode(transaction)
            ).signatures[0])
        return TransactionSendResult(signature=signature)

    async def get_signature_status(self, signature, *, search_transaction_history=None):
        self.calls.append("status")
        if not self.statuses:
            return None
        return self.statuses.pop(0) if len(self.statuses) > 1 else self.statuses[0]

    async def get_block_height(self, *, commitment=None, min_context_slot=None):
        self.calls.append("height")
        return self.block_height


def adapter(transport=None, **config):
    config.setdefault("keypair", PAYER)
    config.setdefault("poll_interval", 0)
    return SoldersWalletAdapter(SoldersAdapterConfig(transport=transport, **config))


async def send(transport, instructions, options=None, **config):
    return await adapter(transport, **config).sign_and_send(instructions, options)


def sent_transaction(transport):
    """The single submitted payload, decoded by solders."""
    assert len(transport.sent) == 1
    return VersionedTransaction.from_bytes(base64.b64decode(transport.sent[0]))


# ---------------------------------------------------------------------------
# Cross-codec conformance against the @solana/kit fixtures
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "name,version",
    [("legacy", "legacy"), ("v0", 0), ("v1", 1)],
)
async def test_wire_bytes_match_the_kit_fixtures(name, version):
    expected = fixture(name)
    transport = FakeTransport()
    result = await send(
        transport, [memo([1, 2, 3])], SendOptions(transaction_version=version)
    )

    assert transport.sent[0] == expected["base64"]
    decoded = sent_transaction(transport)
    assert len(base64.b64decode(transport.sent[0])) == expected["bytes"]
    assert len(decoded.signatures) == expected["signatureCount"]
    assert str(decoded.signatures[0]) == expected["firstSignature"]
    assert result.signature == expected["firstSignature"]
    assert result.slot == 42


@pytest.mark.asyncio
async def test_v1_payload_over_the_legacy_ceiling_is_accepted():
    expected = fixture("v1_oversize")
    assert expected["bytes"] > 1232
    transport = FakeTransport()

    await send(transport, [memo([7] * 1400)], SendOptions(transaction_version=1))

    assert transport.sent[0] == expected["base64"]
    assert isinstance(sent_transaction(transport).message, MessageV1)


@pytest.mark.asyncio
async def test_same_payload_is_rejected_for_v0():
    transport = FakeTransport()
    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([7] * 1400)], SendOptions(transaction_version=0))

    assert "1232-byte limit" in caught.value.message
    assert caught.value.outcome.status == "not-submitted"
    assert caught.value.outcome.phase == "build"
    assert "send" not in transport.calls


@pytest.mark.asyncio
async def test_two_signature_v1_matches_the_fixture():
    expected = fixture("v1_two_signatures")
    transport = FakeTransport()

    result = await send(
        transport,
        [memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())])],
        SendOptions(transaction_version=1, signers=(COSIGNER,)),
    )

    assert transport.sent[0] == expected["base64"]
    decoded = sent_transaction(transport)
    assert len(decoded.signatures) == 2 == expected["signatureCount"]
    assert result.signature == expected["firstSignature"]


@pytest.mark.asyncio
async def test_configured_signer_covers_a_required_cosigner():
    transport = FakeTransport()

    await send(
        transport,
        [memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())])],
        SendOptions(transaction_version=1),
        signers=(COSIGNER,),
    )

    assert transport.sent[0] == fixture("v1_two_signatures")["base64"]


@pytest.mark.asyncio
async def test_missing_signer_fails_before_submission():
    transport = FakeTransport()
    with pytest.raises(WalletError) as caught:
        await send(
            transport,
            [memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())])],
            SendOptions(transaction_version=1),
        )

    assert str(COSIGNER.pubkey()) in caught.value.message
    assert caught.value.outcome.status == "not-submitted"
    assert caught.value.outcome.phase == "wallet"
    assert "send" not in transport.calls


# ---------------------------------------------------------------------------
# Resources: estimation, overrides, exact u64 fees, bounds
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_estimation_fills_unset_ceilings_from_simulation():
    transport = FakeTransport(result=simulation(units=1200, size=4096))

    await send(
        transport,
        [memo([1])],
        SendOptions(transaction_version=1),
        estimate_resources=True,
        compute_unit_margin=1000,
    )

    config = sent_transaction(transport).message.config
    assert config.compute_unit_limit == 2200
    assert config.loaded_accounts_data_size_limit == 4096
    assert transport.calls.count("simulate") == 1


@pytest.mark.asyncio
async def test_explicit_overrides_win_and_skip_simulation():
    transport = FakeTransport(result=simulation(units=1200, size=4096))

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=1,
            resources={
                "computeUnitLimit": "5000",
                "loadedAccountsDataSizeLimit": "1000",
            },
        ),
        estimate_resources=True,
    )

    config = sent_transaction(transport).message.config
    assert config.compute_unit_limit == 5000
    assert config.loaded_accounts_data_size_limit == 1000
    assert "simulate" not in transport.calls


@pytest.mark.asyncio
async def test_missing_simulation_metrics_leave_the_request_untouched():
    transport = FakeTransport(result=simulation(units=None, size=None, logs=None))

    await send(
        transport,
        [memo([1, 2, 3])],
        SendOptions(transaction_version=1),
        estimate_resources=True,
    )

    # Nothing to estimate from: the transaction is the unbudgeted fixture.
    assert transport.sent[0] == fixture("v1")["base64"]
    assert sent_transaction(transport).message.config.compute_unit_limit is None


@pytest.mark.asyncio
async def test_unsigned_simulation_failure_is_never_a_submission():
    transport = FakeTransport(
        result=simulation(err={"InstructionError": [0, {"Custom": 6000}]})
    )
    with pytest.raises(WalletError) as caught:
        await send(
            transport,
            [memo([1])],
            SendOptions(transaction_version=1),
            estimate_resources=True,
        )

    assert caught.value.outcome.status == "not-submitted"
    assert caught.value.outcome.phase == "build"
    assert caught.value.outcome.signature is None
    assert "send" not in transport.calls


@pytest.mark.asyncio
async def test_v1_priority_fee_survives_the_full_u64_range():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=1,
            resources={"priorityFeeLamports": str(U64_MAX)},
        ),
    )

    assert sent_transaction(transport).message.config.priority_fee == U64_MAX


@pytest.mark.asyncio
async def test_legacy_compute_unit_price_survives_the_full_u64_range():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=0,
            resources={"computeUnitPriceMicroLamports": str(U64_MAX)},
        ),
    )

    message = sent_transaction(transport).message
    assert message.instructions[0].data == set_compute_unit_price(U64_MAX).data


@pytest.mark.asyncio
async def test_v0_resources_become_compute_budget_instructions_in_order():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=0,
            resources={
                "computeUnitLimit": 300_000,
                "computeUnitPriceMicroLamports": 7,
                "heapSize": 65_536,
                "loadedAccountsDataSizeLimit": 12_345,
            },
        ),
    )

    message = sent_transaction(transport).message
    assert [bytes(ix.data) for ix in message.instructions[:4]] == [
        bytes(set_compute_unit_limit(300_000).data),
        bytes(set_compute_unit_price(7).data),
        bytes(request_heap_frame(65_536).data),
        bytes(set_loaded_accounts_data_size_limit(12_345).data),
    ]
    assert bytes(message.instructions[4].data) == bytes([1])


@pytest.mark.asyncio
async def test_v1_carries_the_budget_inline_without_extra_instructions():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=1,
            resources={"computeUnitLimit": 300_000, "heapSize": 65_536},
        ),
    )

    message = sent_transaction(transport).message
    assert len(message.instructions) == 1
    assert message.config.compute_unit_limit == 300_000
    assert message.config.heap_size == 65_536


@pytest.mark.asyncio
@pytest.mark.parametrize("heap_size", [1000, 16 * 1024, 512 * 1024, 65_537])
async def test_out_of_range_heap_size_is_rejected(heap_size):
    transport = FakeTransport()
    with pytest.raises(WalletError, match="heapSize"):
        await send(
            transport,
            [memo([1])],
            SendOptions(transaction_version=1, resources={"heapSize": heap_size}),
        )

    assert "send" not in transport.calls


@pytest.mark.asyncio
async def test_in_range_heap_size_is_accepted():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        SendOptions(transaction_version=1, resources={"heapSize": 32 * 1024}),
    )

    assert sent_transaction(transport).message.config.heap_size == 32 * 1024


# ---------------------------------------------------------------------------
# V1 caps and option rejections
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_v1_rejects_more_than_twelve_signatures():
    keypairs = [Keypair.from_seed(bytes([n + 10] * 32)) for n in range(13)]
    transport = FakeTransport()
    instruction = memo([1], signers=[str(kp.pubkey()) for kp in keypairs])

    with pytest.raises(WalletError, match="12 required signatures"):
        await send(
            transport,
            [instruction],
            SendOptions(transaction_version=1, signers=tuple(keypairs)),
        )


@pytest.mark.asyncio
async def test_v1_rejects_more_than_sixty_four_accounts():
    readonly = [
        str(Keypair.from_seed(bytes([n + 40] * 32)).pubkey()) for n in range(64)
    ]
    transport = FakeTransport()

    with pytest.raises(WalletError, match="64 accounts"):
        await send(
            transport,
            [memo([1], readonly=readonly)],
            SendOptions(transaction_version=1),
        )


@pytest.mark.asyncio
async def test_v1_rejects_more_than_sixty_four_instructions():
    transport = FakeTransport()

    with pytest.raises(WalletError, match="64 top-level instructions"):
        await send(
            transport,
            [memo([n]) for n in range(65)],
            SendOptions(transaction_version=1),
        )


@pytest.mark.asyncio
async def test_caller_supplied_compute_budget_instruction_is_rejected():
    transport = FakeTransport()
    instruction = BuiltInstruction(
        program_id=COMPUTE_BUDGET_PROGRAM, accounts=[], data=bytes([2, 1, 0, 0, 0])
    )

    with pytest.raises(WalletError, match="ComputeBudget"):
        await send(transport, [instruction], SendOptions(transaction_version=1))

    assert transport.calls == []


@pytest.mark.asyncio
async def test_v1_rejects_address_lookup_table_inputs():
    transport = FakeTransport()

    with pytest.raises(WalletError, match="does not support address lookup tables"):
        await send(
            transport,
            [memo([1])],
            SendOptions(
                transaction_version=1,
                extra={
                    "addressLookupTables": [
                        "Sysvar1nstructions1111111111111111111111111"
                    ]
                },
            ),
        )

    assert transport.calls == []


@pytest.mark.asyncio
async def test_legacy_also_rejects_address_lookup_table_inputs():
    transport = FakeTransport()

    with pytest.raises(WalletError, match="does not compile address lookup tables"):
        await send(
            transport,
            [memo([1])],
            SendOptions(transaction_version=0, extra={"lookup_tables": []}),
        )


@pytest.mark.asyncio
async def test_empty_instruction_list_is_rejected():
    transport = FakeTransport()
    with pytest.raises(WalletError, match="at least one instruction"):
        await send(transport, [])


@pytest.mark.asyncio
async def test_missing_transport_fails_in_the_build_phase():
    with pytest.raises(WalletError, match="No transaction transport"):
        await adapter().sign_and_send([memo([1])])


def test_adapter_advertises_every_version_it_builds():
    assert adapter().supported_transaction_versions == ("legacy", 0, 1)
    assert adapter().public_key == str(PAYER.pubkey())
    assert adapter(signers=(COSIGNER,)).signer_addresses == (
        str(PAYER.pubkey()),
        str(COSIGNER.pubkey()),
    )


@pytest.mark.parametrize(
    "options",
    [
        {"transaction_version": 0, "resources": {"priorityFeeLamports": 5}},
        {"transaction_version": 1, "resources": {"computeUnitPriceMicroLamports": 5}},
        {"transaction_version": 2},
        {"resources": {"someOtherLimit": 5}},
        {
            "transaction_version": 1,
            "resources": {"priorityFeeLamports": 1, "computeUnitPriceMicroLamports": 1},
        },
    ],
)
def test_incompatible_options_are_rejected_by_the_shared_contract(options):
    with pytest.raises(ValueError):
        SendOptions.coerce(options).validate()


@pytest.mark.asyncio
async def test_incompatible_version_fee_pair_is_rejected_at_send():
    transport = FakeTransport()
    with pytest.raises(ValueError, match="priority_fee_lamports requires"):
        await send(
            transport,
            [memo([1])],
            {"transactionVersion": 0, "resources": {"priorityFeeLamports": 5}},
        )

    assert transport.calls == []


# ---------------------------------------------------------------------------
# Options coercion and merge
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_per_send_mapping_merges_over_configured_defaults():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        {"resources": {"computeUnitLimit": "10"}},
        transaction_version=1,
        resources={"priorityFeeLamports": 7},
    )

    config = sent_transaction(transport).message.config
    assert config.priority_fee == 7
    assert config.compute_unit_limit == 10


@pytest.mark.asyncio
async def test_per_send_fee_override_clears_the_opposite_fee_slot():
    transport = FakeTransport()

    await send(
        transport,
        [memo([1])],
        {
            "transactionVersion": 0,
            "resources": {"computeUnitPriceMicroLamports": 9},
        },
        transaction_version=1,
        resources={"priorityFeeLamports": 7},
    )

    message = sent_transaction(transport).message
    assert not isinstance(message, MessageV1)
    assert message.instructions[0].data == set_compute_unit_price(9).data


@pytest.mark.asyncio
async def test_omitted_version_builds_v0_by_default():
    transport = FakeTransport()

    await send(transport, [memo([1, 2, 3])])

    assert transport.sent[0] == fixture("v0")["base64"]


@pytest.mark.asyncio
async def test_context_transport_wins_over_the_configured_one():
    configured = FakeTransport(allow_send=False)
    from_context = FakeTransport()

    await adapter(configured).sign_and_send(
        [memo([1, 2, 3])],
        None,
        WalletExecutionContext(transaction_transport=from_context),
    )

    assert from_context.sent[0] == fixture("v0")["base64"]
    assert configured.calls == []


# ---------------------------------------------------------------------------
# Inspection
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_inspection_never_signs_prompts_or_sends(monkeypatch):
    transport = FakeTransport(allow_send=False)

    def fail(self, plan):
        raise AssertionError("inspection must not sign")

    monkeypatch.setattr(SoldersWalletAdapter, "_sign", fail)

    result = await adapter(transport).inspect_transaction(
        [memo([1, 2, 3])], SendOptions(transaction_version=1)
    )

    assert transport.calls == ["latest-blockhash", "fee", "simulate"]
    assert result.fee_lamports == 5000
    assert result.logs == ("Program log: ok",)
    assert result.compute_units_consumed == 1200
    assert result.loaded_accounts_data_size == 4096
    assert result.context_slot == 7
    assert result.error is None
    assert result.extra["transactionVersion"] == 1
    assert result.extra["feeContextSlot"] == 3
    assert result.extra["wireBytes"] == fixture("v1")["bytes"]


@pytest.mark.asyncio
async def test_inspection_reports_the_resources_it_would_apply():
    transport = FakeTransport(allow_send=False)

    result = await adapter(transport, estimate_resources=True).inspect_transaction(
        [memo([1])], SendOptions(transaction_version=1)
    )

    assert result.extra["resources"] == {
        "computeUnitLimit": "2200",
        "loadedAccountsDataSizeLimit": "4096",
    }


@pytest.mark.asyncio
async def test_inspection_simulates_the_exact_bytes_it_would_submit():
    inspecting = FakeTransport(allow_send=False)
    sending = FakeTransport()

    await adapter(inspecting).inspect_transaction(
        [memo([1, 2, 3])], SendOptions(transaction_version=1)
    )
    await send(sending, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    inspected = base64.b64decode(inspecting.simulated[0])
    submitted = base64.b64decode(sending.sent[0])
    # Same size and same message: the simulated payload differs from the
    # submitted one only in the signature slot it never filled in.
    assert len(inspected) == len(submitted)
    assert VersionedTransaction.from_bytes(inspected).message == (
        VersionedTransaction.from_bytes(submitted).message
    )


# ---------------------------------------------------------------------------
# Submit once, then reconcile
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_timeout_is_submitted_unknown_after_exactly_one_send():
    transport = FakeTransport(statuses=[], block_height=1)

    with pytest.raises(WalletError) as caught:
        await send(
            transport,
            [memo([1, 2, 3])],
            SendOptions(transaction_version=1),
            confirmation_timeout=0,
        )

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "confirmation"
    assert outcome.signature == fixture("v1")["firstSignature"]
    assert transport.calls.count("send") == 1
    assert len(transport.sent) == 1


@pytest.mark.asyncio
async def test_expired_blockhash_is_submitted_unknown():
    transport = FakeTransport(
        statuses=[], block_height=101, last_valid_block_height=100
    )

    with pytest.raises(WalletError, match="blockhash expired") as caught:
        await send(transport, [memo([1])], SendOptions(transaction_version=1))

    assert caught.value.outcome.status == "submitted-unknown"
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_chain_error_is_chain_failed_with_the_signature():
    err = {"InstructionError": [0, {"Custom": 6000}]}
    transport = FakeTransport(statuses=[status(err=err, slot=99)])

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    outcome = caught.value.outcome
    assert outcome.status == "chain-failed"
    assert outcome.phase == "chain"
    assert outcome.signature == fixture("v1")["firstSignature"]
    assert outcome.slot == 99
    assert outcome.cause == err


@pytest.mark.asyncio
async def test_processed_status_does_not_satisfy_a_finalized_request():
    transport = FakeTransport(statuses=[status("processed")], block_height=1)

    with pytest.raises(WalletError) as caught:
        await send(
            transport,
            [memo([1])],
            SendOptions(transaction_version=1, confirmation_level="finalized"),
            confirmation_timeout=0,
        )

    assert caught.value.outcome.status == "submitted-unknown"


@pytest.mark.asyncio
async def test_status_climbs_to_the_requested_commitment():
    transport = FakeTransport(
        statuses=[status("processed"), status("finalized", slot=77)], block_height=1
    )

    result = await send(
        transport,
        [memo([1])],
        SendOptions(transaction_version=1, confirmation_level="finalized"),
    )

    assert result.slot == 77
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_relay_rejection_is_not_submitted():
    transport = FakeTransport(
        send_error=TransactionTransportError(
            400,
            code="INVALID_TRANSACTION",
            message="blockhash not found",
            submission_state="not_submitted",
        )
    )

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1])], SendOptions(transaction_version=1))

    assert caught.value.outcome.status == "not-submitted"
    assert caught.value.outcome.phase == "send"
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_ambiguous_relay_failure_keeps_the_local_signature():
    transport = FakeTransport(
        send_error=TransactionTransportError(
            504, code="TIMEOUT", message="upstream timeout", submission_state="unknown"
        )
    )

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "send"
    assert outcome.signature == fixture("v1")["firstSignature"]
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_transport_failure_without_classification_is_submitted_unknown():
    transport = FakeTransport(send_error=RuntimeError("socket closed"))

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    assert caught.value.outcome.status == "submitted-unknown"
    assert caught.value.outcome.signature == fixture("v1")["firstSignature"]
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_status_polling_failure_never_resends():
    class Broken(FakeTransport):
        async def get_signature_status(
            self, signature, *, search_transaction_history=None
        ):
            self.calls.append("status")
            raise RuntimeError("status endpoint down")

    transport = Broken()
    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    assert caught.value.outcome.status == "submitted-unknown"
    assert caught.value.outcome.phase == "confirmation"
    assert caught.value.outcome.signature == fixture("v1")["firstSignature"]
    assert transport.calls.count("send") == 1


# ---------------------------------------------------------------------------
# The optional package boundary
# ---------------------------------------------------------------------------


_WITHOUT_SOLDERS = """
import sys


class Block:
    def find_spec(self, name, path=None, target=None):
        if name == "solders" or name.startswith("solders."):
            raise ModuleNotFoundError("No module named " + repr(name))
        return None


sys.meta_path.insert(0, Block())

import arete
import arete.adapters

assert "solders" not in sys.modules, "the base SDK must not import solders"
assert arete.Arete is not None

try:
    import arete.adapters.solders
except ImportError as error:
    print(str(error))
else:
    raise SystemExit("the adapter imported without solders")
"""


def test_base_sdk_imports_without_solders_and_names_the_extra():
    result = subprocess.run(
        [sys.executable, "-c", _WITHOUT_SOLDERS],
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert "arete-sdk[solana]" in result.stdout
    assert "solders >= 0.29" in result.stdout
