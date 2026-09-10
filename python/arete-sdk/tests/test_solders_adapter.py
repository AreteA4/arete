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

import asyncio
import base64
import json
import logging
import subprocess
import sys
import warnings
from pathlib import Path

import httpx
import pytest
from solders.compute_budget import (
    request_heap_frame,
    set_compute_unit_limit,
    set_compute_unit_price,
    set_loaded_accounts_data_size_limit,
)
from solders.hash import Hash
from solders.keypair import Keypair
from solders.message import MessageV1
from solders.transaction import VersionedTransaction

from arete.adapters.solders import (
    MAX_COMPUTE_UNIT_LIMIT,
    MAX_LOADED_ACCOUNTS_DATA_SIZE,
    SoldersAdapterConfig,
    SoldersWalletAdapter,
    to_instruction,
)
from arete.client import Arete
from arete.instructions import BuiltAccountMeta, BuiltInstruction
from arete.operations import (
    TransactionExecutionError,
    create_prepared_instruction,
)
from arete.rpc import RpcTransactionTransport
from arete.stack import StackDef, StackEndpoints
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
        self.polled = []
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
        self.polled.append(signature)
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


def submitted_signature(transport):
    """The signature the submitted bytes actually carry.

    V1 sends resolve their two SIMD-0385 budgets before signing, so the
    payload is not the budget-less kit fixture and its signature is derived
    here rather than read from the corpus.
    """
    return str(sent_transaction(transport).signatures[0])


# ---------------------------------------------------------------------------
# Configuration validation
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "field,value",
    [
        ("poll_interval", -0.5),
        ("poll_interval", "0.5"),
        ("confirmation_timeout", -1),
        ("compute_unit_margin", -1),
    ],
)
def test_out_of_range_polling_configuration_is_rejected_at_construction(field, value):
    with pytest.raises(ValueError, match=field):
        SoldersAdapterConfig(keypair=PAYER, **{field: value})


@pytest.mark.asyncio
async def test_an_invalid_poll_interval_fails_before_anything_is_submitted():
    """Range checks belong at construction, not in ``_reconcile``.

    A poll interval the sleep cannot use only blows up after
    ``send_transaction`` already succeeded, and that exception carries no
    signature, so the executor classifies a landed transaction as
    not-submitted -- an invitation to send the same payment twice.
    """
    transport = FakeTransport(statuses=[status("processed")], allow_send=False)

    with pytest.raises(ValueError, match="poll_interval"):
        await send(
            transport,
            [memo([1, 2, 3])],
            SendOptions(transaction_version=1),
            poll_interval="0.5",
            confirmation_timeout=5,
        )

    assert transport.calls == []


@pytest.mark.asyncio
async def test_an_unset_confirmation_level_fails_before_anything_is_submitted():
    """The same shape one level up: ``SendOptions`` reads ``None`` as "unset",
    but this is what an unset per-send level falls back to, so it reaches the
    confirmation-rank lookup as a ``KeyError`` after the single submission."""
    transport = FakeTransport(allow_send=False)

    with pytest.raises(ValueError, match="confirmation_level"):
        await send(
            transport,
            [memo([1, 2, 3])],
            SendOptions(transaction_version=1),
            confirmation_level=None,
        )

    assert transport.calls == []


# ---------------------------------------------------------------------------
# Cross-codec conformance against the @solana/kit fixtures
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
@pytest.mark.parametrize("name,version", [("legacy", "legacy"), ("v0", 0)])
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


@pytest.mark.parametrize("name", ["v1", "v1_oversize", "v1_two_signatures"])
def test_v1_wire_bytes_match_the_kit_fixtures(name):
    """V1 codec conformance, compiled directly rather than sent.

    Every kit V1 fixture carries an empty ``TransactionConfig``, and a V1
    message with no budgets requests the SIMD-0385 *minimum* -- so
    :meth:`sign_and_send` can never produce these exact bytes. What the
    corpus pins is the codec, which is checked here against the fixture's
    own inputs.
    """
    expected = fixture(name)
    signers = [PAYER] if expected["signatureCount"] == 1 else [PAYER, COSIGNER]
    accounts = [] if expected["signatureCount"] == 1 else [
        str(keypair.pubkey()) for keypair in signers
    ]
    data = {"v1": [1, 2, 3], "v1_oversize": [7] * 1400, "v1_two_signatures": [9]}[name]

    message = MessageV1.try_compile(
        PAYER.pubkey(),
        [to_instruction(memo(data, signers=accounts))],
        Hash.from_string(FIXTURE_BLOCKHASH),
        None,
    )
    transaction = VersionedTransaction(message, signers)
    wire = bytes(transaction)

    assert base64.b64encode(wire).decode("ascii") == expected["base64"]
    assert len(wire) == expected["bytes"]
    assert len(transaction.signatures) == expected["signatureCount"]
    assert str(transaction.signatures[0]) == expected["firstSignature"]


@pytest.mark.asyncio
async def test_v1_payload_over_the_legacy_ceiling_is_accepted():
    assert fixture("v1_oversize")["bytes"] > 1232
    transport = FakeTransport()

    await send(transport, [memo([7] * 1400)], SendOptions(transaction_version=1))

    decoded = sent_transaction(transport)
    assert isinstance(decoded.message, MessageV1)
    assert len(base64.b64decode(transport.sent[0])) > 1232


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
async def test_two_signature_v1_signs_in_message_order():
    transport = FakeTransport()

    result = await send(
        transport,
        [memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())])],
        SendOptions(transaction_version=1, signers=(COSIGNER,)),
    )

    decoded = sent_transaction(transport)
    assert len(decoded.signatures) == 2
    assert result.signature == str(decoded.signatures[0])
    assert decoded.verify_with_results() == [True, True]


@pytest.mark.asyncio
async def test_configured_signer_covers_a_required_cosigner():
    transport = FakeTransport()

    await send(
        transport,
        [memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())])],
        SendOptions(transaction_version=1),
        signers=(COSIGNER,),
    )

    assert sent_transaction(transport).verify_with_results() == [True, True]


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
        compute_unit_margin=1000,
    )

    config = sent_transaction(transport).message.config
    assert config.compute_unit_limit == 2200
    # 4096 measured bytes round up to one 32 KiB page, plus a page of
    # headroom for account growth between simulation and execution.
    assert config.loaded_accounts_data_size_limit == 65536
    assert transport.calls.count("simulate") == 1


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "measured,expected",
    [
        (0, 32768),
        (1, 65536),
        (32768, 65536),
        (32769, 98304),
        (4097, 65536),
        # At the ceiling the headroom cannot push the budget out of range.
        (MAX_LOADED_ACCOUNTS_DATA_SIZE, MAX_LOADED_ACCOUNTS_DATA_SIZE),
    ],
)
async def test_estimated_data_budgets_are_page_rounded_and_bounded(
    measured, expected
):
    transport = FakeTransport(result=simulation(units=1200, size=measured))

    await send(transport, [memo([1])], SendOptions(transaction_version=1))

    config = sent_transaction(transport).message.config
    assert config.loaded_accounts_data_size_limit == expected


@pytest.mark.asyncio
async def test_estimated_compute_budgets_are_bounded_by_the_protocol_maximum():
    transport = FakeTransport(
        result=simulation(units=MAX_COMPUTE_UNIT_LIMIT, size=4096)
    )

    await send(
        transport,
        [memo([1])],
        SendOptions(transaction_version=1),
        compute_unit_margin=1000,
    )

    config = sent_transaction(transport).message.config
    assert config.compute_unit_limit == MAX_COMPUTE_UNIT_LIMIT


@pytest.mark.asyncio
async def test_the_provisional_message_declares_the_protocol_maxima():
    """The message that gets measured must be executable.

    Signature verification being off does not lift a resource limit, so a
    provisional V1 message carrying the SIMD-0385 minimum could never
    produce an ordinary successful estimate.
    """
    transport = FakeTransport()

    await send(transport, [memo([1])], SendOptions(transaction_version=1))

    provisional = VersionedTransaction.from_bytes(
        base64.b64decode(transport.simulated[0])
    ).message.config
    assert provisional.compute_unit_limit == MAX_COMPUTE_UNIT_LIMIT
    assert provisional.loaded_accounts_data_size_limit == (
        MAX_LOADED_ACCOUNTS_DATA_SIZE
    )


@pytest.mark.asyncio
async def test_an_explicit_budget_is_preserved_in_the_provisional_message():
    transport = FakeTransport(result=simulation(units=1200, size=4096))

    await send(
        transport,
        [memo([1])],
        SendOptions(
            transaction_version=1, resources={"computeUnitLimit": 5000}
        ),
    )

    provisional = VersionedTransaction.from_bytes(
        base64.b64decode(transport.simulated[0])
    ).message.config
    assert provisional.compute_unit_limit == 5000
    assert provisional.loaded_accounts_data_size_limit == (
        MAX_LOADED_ACCOUNTS_DATA_SIZE
    )
    final = sent_transaction(transport).message.config
    assert final.compute_unit_limit == 5000, "an explicit budget is never raised"
    assert final.loaded_accounts_data_size_limit == 65536


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
@pytest.mark.parametrize(
    "units,size,missing",
    [
        (None, None, "computeUnitLimit"),
        (1200, None, "loadedAccountsDataSizeLimit"),
        (None, 4096, "computeUnitLimit"),
    ],
)
async def test_a_missing_metric_refuses_to_sign_a_v1_transaction(
    units, size, missing
):
    """An unmeasurable V1 ceiling cannot be left unset.

    Unset requests the minimum (SIMD-0385), so signing anyway would submit a
    transaction that can only fail on chain.
    """
    transport = FakeTransport(result=simulation(units=units, size=size, logs=None))

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    outcome = caught.value.outcome
    assert outcome.status == "not-submitted"
    assert outcome.phase == "build"
    assert missing in caught.value.message
    assert "send" not in transport.calls


@pytest.mark.asyncio
async def test_missing_metrics_leave_a_v0_request_to_the_runtime_defaults():
    """legacy/v0 carry ceilings as ``ComputeBudget`` instructions, where an
    omitted one means the runtime's own default rather than zero."""
    transport = FakeTransport(result=simulation(units=None, size=None, logs=None))

    await send(
        transport,
        [memo([1, 2, 3])],
        SendOptions(transaction_version=0),
        estimate_resources=True,
    )

    assert transport.sent[0] == fixture("v0")["base64"]


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
    # The provisional V1 message declares the maxima the caller omitted, so
    # it is the budget-carrying payload -- wider than the budget-less kit
    # fixture, and exactly the bytes that were simulated.
    assert result.extra["wireBytes"] == len(base64.b64decode(transport.simulated[0]))
    assert result.extra["resources"] == {
        "computeUnitLimit": str(MAX_COMPUTE_UNIT_LIMIT),
        "loadedAccountsDataSizeLimit": str(MAX_LOADED_ACCOUNTS_DATA_SIZE),
    }


@pytest.mark.asyncio
async def test_inspection_reports_the_resources_it_would_apply():
    transport = FakeTransport(allow_send=False)

    result = await adapter(transport, estimate_resources=True).inspect_transaction(
        [memo([1])], SendOptions(transaction_version=1)
    )

    assert result.extra["resources"] == {
        "computeUnitLimit": "2200",
        "loadedAccountsDataSizeLimit": "65536",
    }


@pytest.mark.asyncio
async def test_inspection_simulates_the_exact_bytes_it_would_submit():
    inspecting = FakeTransport(allow_send=False)
    sending = FakeTransport()

    # Estimating on both sides: the budgets inspection resolves are the ones
    # the send resolves, so the payload it simulates is the payload that
    # would go out.
    await adapter(inspecting, estimate_resources=True).inspect_transaction(
        [memo([1, 2, 3])], SendOptions(transaction_version=1)
    )
    await send(
        sending,
        [memo([1, 2, 3])],
        SendOptions(transaction_version=1),
        estimate_resources=True,
    )

    inspected = base64.b64decode(inspecting.simulated[-1])
    submitted = base64.b64decode(sending.sent[0])
    # Same size and same message: the simulated payload differs from the
    # submitted one only in the signature slot it never filled in.
    assert len(inspected) == len(submitted)
    assert VersionedTransaction.from_bytes(inspected).message == (
        VersionedTransaction.from_bytes(submitted).message
    )


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "method,error",
    [
        (
            "get_fee_for_message",
            TransactionTransportError(
                503, code="UPSTREAM_UNAVAILABLE", message="relay down"
            ),
        ),
        # A malformed relay response surfaces as an ordinary parse error.
        ("simulate_transaction", KeyError("value")),
    ],
)
async def test_inspection_reports_a_relay_outage_as_a_wallet_error(method, error):
    """A relay outage is an adapter failure, not a raw transport exception:
    the wallet contract only speaks :class:`WalletError`."""

    async def fail(*args, **kwargs):
        raise error

    transport = FakeTransport(allow_send=False)
    setattr(transport, method, fail)

    with pytest.raises(WalletError) as caught:
        await adapter(transport).inspect_transaction(
            [memo([1, 2, 3])], SendOptions(transaction_version=1)
        )

    outcome = caught.value.outcome
    assert outcome.status == "not-submitted"
    assert outcome.phase == "build"
    assert outcome.cause is error


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
            confirmation_timeout=0.02,
        )

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "confirmation"
    assert outcome.signature == submitted_signature(transport)
    assert transport.calls.count("send") == 1
    assert len(transport.sent) == 1


@pytest.mark.asyncio
async def test_a_stalled_submission_ends_at_the_deadline_with_the_signature():
    """The relay takes the transaction and never answers.

    Nothing here imposes a request timeout of its own, so without a bound on
    the dispatch the caller waits forever on a transaction that may already
    have landed.
    """

    class Stalling(FakeTransport):
        async def send_transaction(self, transaction, **kwargs):
            self.calls.append("send")
            self.sent.append(transaction)
            await asyncio.Future()
            raise AssertionError("unreachable")

    transport = Stalling()
    with pytest.raises(WalletError) as caught:
        await asyncio.wait_for(
            send(
                transport,
                [memo([1, 2, 3])],
                SendOptions(transaction_version=1),
                confirmation_timeout=0.03,
                poll_interval=0.001,
            ),
            timeout=1.0,
        )

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "send"
    assert outcome.signature == submitted_signature(transport)
    assert transport.calls.count("send") == 1
    assert "status" not in transport.calls, "the deadline is spent before polling"


@pytest.mark.asyncio
async def test_a_stalled_status_request_ends_at_the_deadline():
    class Stalling(FakeTransport):
        async def get_signature_status(self, signature, **kwargs):
            self.calls.append("status")
            await asyncio.Future()
            raise AssertionError("unreachable")

    transport = Stalling()
    with pytest.raises(WalletError) as caught:
        await asyncio.wait_for(
            send(
                transport,
                [memo([1, 2, 3])],
                SendOptions(transaction_version=1),
                confirmation_timeout=0.03,
                poll_interval=0.001,
            ),
            timeout=1.0,
        )

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "confirmation"
    assert outcome.signature == submitted_signature(transport)
    assert transport.calls.count("send") == 1


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
    assert outcome.signature == submitted_signature(transport)
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
            confirmation_timeout=0.02,
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
async def test_ambiguous_relay_failure_keeps_the_local_signature(caplog):
    # The relay names a different signature on its way out; the adapter signed
    # the bytes it sent, so its own signature is the one to reconcile.
    impostor = fixture("legacy")["firstSignature"]
    transport = FakeTransport(
        send_error=TransactionTransportError(
            504,
            code="TIMEOUT",
            message="upstream timeout",
            submission_state="unknown",
            signature=impostor,
        )
    )

    with caplog.at_level(logging.WARNING, logger="arete.adapters.solders"):
        with pytest.raises(WalletError) as caught:
            await send(
                transport, [memo([1, 2, 3])], SendOptions(transaction_version=1)
            )
    assert impostor in caplog.text

    outcome = caught.value.outcome
    assert outcome.status == "submitted-unknown"
    assert outcome.phase == "send"
    assert outcome.signature == submitted_signature(transport)
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_a_relay_echoed_signature_never_replaces_the_derived_one(caplog):
    """Reconciliation polls only what this adapter signed.

    Believing the echo would poll a different transaction entirely: it can
    report another payment's status, or never confirm the one submitted.
    """
    impostor = fixture("legacy")["firstSignature"]
    transport = FakeTransport(relay_signature=impostor)

    with caplog.at_level(logging.WARNING, logger="arete.adapters.solders"):
        result = await send(
            transport, [memo([1, 2, 3])], SendOptions(transaction_version=1)
        )

    local = submitted_signature(transport)
    assert impostor != local
    assert impostor in caplog.text
    assert result.signature == local
    assert transport.polled == [local]
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_reporting_a_signature_mismatch_cannot_break_a_submitted_send():
    """Reporting runs after the transaction may already be on the wire.

    `warnings.warn` raises under `-W error`, which would lose the signature and
    let the executor call a submitted transaction never-sent — the one
    misclassification that invites paying twice. A log cannot do that.
    """

    transport = FakeTransport(relay_signature=fixture("legacy")["firstSignature"])

    with warnings.catch_warnings():
        warnings.simplefilter("error")
        result = await send(
            transport, [memo([1, 2, 3])], SendOptions(transaction_version=1)
        )

    assert result.signature == submitted_signature(transport)
    assert transport.calls.count("send") == 1


@pytest.mark.asyncio
async def test_transport_failure_without_classification_is_submitted_unknown():
    transport = FakeTransport(send_error=RuntimeError("socket closed"))

    with pytest.raises(WalletError) as caught:
        await send(transport, [memo([1, 2, 3])], SendOptions(transaction_version=1))

    assert caught.value.outcome.status == "submitted-unknown"
    assert caught.value.outcome.signature == submitted_signature(transport)
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
    assert caught.value.outcome.signature == submitted_signature(transport)
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


# ---------------------------------------------------------------------------
# The public client resolves adapter defaults before it validates
# ---------------------------------------------------------------------------


def make_client_stack():
    """Smallest stack a client needs: no views, no programs, HTTP only."""
    return StackDef(
        name="adapter-tests",
        endpoints=StackEndpoints(ws="", http="https://api.example.test"),
        views={},
        programs={},
    )


async def make_client(wallet, transactions=None):
    return await Arete.connect(
        make_client_stack(),
        transport="http",
        wallet=wallet,
        transactions=transactions,
    )


def v1_default_adapter(transport=None, **config):
    config.setdefault("transaction_version", 1)
    return adapter(transport, **config)


@pytest.mark.asyncio
async def test_the_public_client_honours_the_adapter_default_version():
    """Client-side validation runs before the adapter is reached.

    Reading the caller's options alone made a V1-defaulted adapter's valid
    priority fee a v0 fee mismatch, before any blockhash call.
    """
    transport = FakeTransport()
    a4 = await make_client(v1_default_adapter(transport), transactions=transport)

    await a4.transaction(
        [memo([1])], send={"resources": {"priorityFeeLamports": 5000}}
    )

    assert sent_transaction(transport).message.config.priority_fee == 5000


@pytest.mark.asyncio
async def test_the_public_client_still_rejects_a_fee_bound_to_another_version():
    transport = FakeTransport(allow_send=False)
    a4 = await make_client(v1_default_adapter(transport), transactions=transport)

    with pytest.raises(ValueError, match="compute_unit_price_micro_lamports"):
        await a4.transaction(
            [memo([1])],
            send={"resources": {"computeUnitPriceMicroLamports": 1000}},
        )

    assert transport.calls == []


@pytest.mark.asyncio
async def test_client_execution_honours_the_adapter_default_version():
    transport = FakeTransport()
    a4 = await make_client(v1_default_adapter(transport), transactions=transport)
    prepared = create_prepared_instruction(name="memo", instruction=memo([1]))

    await a4.execute(prepared, send={"resources": {"priorityFeeLamports": 5000}})

    assert sent_transaction(transport).message.config.priority_fee == 5000


@pytest.mark.asyncio
async def test_client_inspection_honours_the_adapter_default_version():
    transport = FakeTransport(allow_send=False)
    a4 = await make_client(v1_default_adapter(transport), transactions=transport)
    prepared = create_prepared_instruction(name="memo", instruction=memo([1]))

    inspection = await a4.inspect_operation(
        prepared, inspect={"resources": {"priorityFeeLamports": 5000}}
    )

    assert inspection.transaction.extra["transactionVersion"] == 1
    assert "send" not in transport.calls


@pytest.mark.asyncio
async def test_execute_accepts_a_per_call_solders_cosigner():
    """``pubkey()`` is a method returning a ``Pubkey``; signer validation has
    to recognise it or fail closed on a cosigner the adapter can sign for."""
    transport = FakeTransport()
    a4 = await make_client(v1_default_adapter(transport), transactions=transport)
    prepared = create_prepared_instruction(
        name="memo",
        instruction=memo([9], signers=[str(PAYER.pubkey()), str(COSIGNER.pubkey())]),
    )

    await a4.execute(prepared, signers=[COSIGNER])

    decoded = sent_transaction(transport)
    assert len(decoded.signatures) == 2
    assert decoded.verify_with_results() == [True, True]


# ---------------------------------------------------------------------------
# Arete by default, direct RPC as the explicit escape hatch
# ---------------------------------------------------------------------------


def rpc_node(*, send_error=None, results=None):
    """Mock Solana node: enough JSON-RPC for one whole send.

    ``sendTransaction`` answers with the signature the submitted bytes carry,
    the way a real node does, so confirmation reconciles against it.
    """
    calls = []

    def handler(request):
        body = json.loads(request.content)
        method = body["method"]
        calls.append(method)
        if results and method in results:
            payload = results[method]
        elif method == "getLatestBlockhash":
            payload = {
                "context": {"slot": 1},
                "value": {
                    "blockhash": FIXTURE_BLOCKHASH,
                    "lastValidBlockHeight": 100,
                },
            }
        elif method == "getFeeForMessage":
            payload = {"context": {"slot": 3}, "value": 5000}
        elif method == "simulateTransaction":
            payload = {
                "context": {"slot": 7},
                "value": {
                    "err": None,
                    "logs": ["Program log: ok"],
                    "unitsConsumed": 1200,
                    "loadedAccountsDataSize": 4096,
                },
            }
        elif method == "sendTransaction":
            if send_error is not None:
                return httpx.Response(
                    200, json={"jsonrpc": "2.0", "id": body["id"], "error": send_error}
                )
            payload = str(
                VersionedTransaction.from_bytes(
                    base64.b64decode(body["params"][0])
                ).signatures[0]
            )
        elif method == "getSignatureStatuses":
            payload = {
                "context": {"slot": 9},
                "value": [
                    {
                        "slot": 42,
                        "confirmations": None,
                        "err": None,
                        "confirmationStatus": "confirmed",
                    }
                ],
            }
        elif method == "getBlockHeight":
            payload = 10
        else:
            raise AssertionError(f"unexpected RPC method {method!r}")
        return httpx.Response(
            200, json={"jsonrpc": "2.0", "id": body["id"], "result": payload}
        )

    transport = RpcTransactionTransport(
        "https://node.example/rpc",
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)),
    )
    return transport, calls


@pytest.mark.asyncio
async def test_a_v1_send_runs_end_to_end_over_direct_rpc():
    rpc, calls = rpc_node()
    arete_relay = FakeTransport(allow_send=False)
    a4 = await make_client(
        v1_default_adapter(rpc, transport_selection="direct"),
        transactions=arete_relay,
    )

    # Both V1 budgets omitted: the estimator has to run over RPC too.
    result = await a4.transaction(
        [memo([1])], send={"resources": {"priorityFeeLamports": 5000}}
    )

    assert result.slot == 42
    assert calls.count("sendTransaction") == 1, calls
    assert calls.count("simulateTransaction") == 1, calls
    assert arete_relay.calls == [], (
        "an explicit RPC selection wins even with an Arete client attached"
    )


@pytest.mark.asyncio
async def test_arete_remains_the_default_backend():
    rpc, calls = rpc_node()
    arete_relay = FakeTransport()
    a4 = await make_client(v1_default_adapter(rpc), transactions=arete_relay)

    await a4.transaction([memo([1])], send={"resources": {"priorityFeeLamports": 5000}})

    assert arete_relay.calls.count("send") == 1
    assert calls == [], "auto keeps the client's own transport"


@pytest.mark.asyncio
async def test_unsigned_inspection_over_rpc_never_signs_or_sends(monkeypatch):
    rpc, calls = rpc_node()

    def fail(self, plan):
        raise AssertionError("inspection must not sign")

    monkeypatch.setattr(SoldersWalletAdapter, "_sign", fail)
    wallet = v1_default_adapter(rpc, transport_selection="direct")

    result = await wallet.inspect_transaction([memo([1])])

    assert result.fee_lamports == 5000
    assert result.compute_units_consumed == 1200
    assert result.loaded_accounts_data_size == 4096
    assert "sendTransaction" not in calls


@pytest.mark.asyncio
async def test_a_timed_out_rpc_send_makes_exactly_one_attempt():
    rpc, calls = rpc_node(
        results={"getSignatureStatuses": {"context": {"slot": 9}, "value": [None]}}
    )
    arete_relay = FakeTransport(allow_send=False)
    a4 = await make_client(
        v1_default_adapter(rpc, transport_selection="direct", confirmation_timeout=0.05),
        transactions=arete_relay,
    )

    with pytest.raises(TransactionExecutionError) as caught:
        await a4.transaction(
            [memo([1])], send={"resources": {"priorityFeeLamports": 5000}}
        )

    assert caught.value.outcome.status == "submitted-unknown"
    assert caught.value.outcome.signature
    assert calls.count("sendTransaction") == 1, calls
    assert arete_relay.calls == [], "no cross-transport fallback after uncertainty"


@pytest.mark.asyncio
async def test_a_failure_on_the_selected_transport_never_falls_back():
    rpc, calls = rpc_node(
        send_error={"code": -32002, "message": "Transaction simulation failed"}
    )
    arete_relay = FakeTransport(allow_send=False)
    a4 = await make_client(
        v1_default_adapter(rpc, transport_selection="direct"),
        transactions=arete_relay,
    )

    with pytest.raises(TransactionExecutionError) as caught:
        await a4.transaction(
            [memo([1])], send={"resources": {"priorityFeeLamports": 5000}}
        )

    assert caught.value.outcome.status == "not-submitted", (
        "the node proved it never dispatched"
    )
    assert calls.count("sendTransaction") == 1
    assert arete_relay.calls == []


@pytest.mark.asyncio
async def test_direct_selection_requires_a_configured_transport():
    wallet = adapter(None, transaction_version=1, transport_selection="direct")

    with pytest.raises(WalletError) as caught:
        await wallet.sign_and_send(
            [memo([1])],
            None,
            # Arete context present, and deliberately not used.
            WalletExecutionContext(transaction_transport=FakeTransport()),
        )

    assert "direct" in caught.value.message
    assert caught.value.outcome.phase == "build"


def test_an_unknown_transport_selection_is_rejected_at_construction():
    with pytest.raises(ValueError, match="transport_selection"):
        SoldersAdapterConfig(keypair=PAYER, transport_selection="rpc")
