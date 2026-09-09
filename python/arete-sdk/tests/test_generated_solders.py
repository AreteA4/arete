"""One generated-client integration through the real optional Solana adapter.

Run explicitly in the solana-extra environment. Missing adapters/codecs are
collection failures, never skips or successful mock-adapter substitutes.
"""

import base64
import sys
from pathlib import Path

import pytest
from solders.keypair import Keypair
from solders.message import MessageV1
from solders.transaction import VersionedTransaction

from arete.adapters.solders import SoldersAdapterConfig, SoldersWalletAdapter
from arete.transactions import LatestBlockhashResult, TransactionSendResult, TransactionSignatureStatus, TransactionSimulationResult

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "examples/ore-python"))
from ore_stack.programs import ore_log  # noqa: E402


class RecordingRelay:
    def __init__(self):
        self.sent = []

    async def get_latest_blockhash(self, **_):
        return LatestBlockhashResult(
            blockhash="11111111111111111111111111111111",
            context_slot=1, last_valid_block_height=100,
        )

    async def get_fee_for_message(self, *_args, **_kwargs):
        raise AssertionError("Unexpected fee request")

    async def simulate_transaction(self, *_args, **_kwargs):
        return TransactionSimulationResult(context_slot=1, err=None, logs=[], units_consumed=1_000, loaded_accounts_data_size=1_024)

    async def send_transaction(self, transaction, **_):
        self.sent.append(transaction)
        decoded = VersionedTransaction.from_bytes(base64.b64decode(transaction))
        return TransactionSendResult(signature=str(decoded.signatures[0]))

    async def get_signature_status(self, signature, **_):
        return TransactionSignatureStatus(
            signature=signature, slot=2, confirmation_status="confirmed", err=None,
        )

    async def get_block_height(self, **_):
        return 1


@pytest.mark.asyncio
async def test_generated_ore_instruction_through_v1_adapter():
    payer = Keypair.from_seed(bytes([1] * 32))
    relay = RecordingRelay()
    wallet = SoldersWalletAdapter(SoldersAdapterConfig(keypair=payer, transport=relay))
    instruction = ore_log(signer=str(payer.pubkey()))
    result = await wallet.sign_and_send([instruction], {
        "transactionVersion": 1,
        "resources": {
            "computeUnitLimit": 200_000, "loadedAccountsDataSizeLimit": 1_048_576,
            "heapSize": 32_768, "priorityFeeLamports": 7,
        },
    })
    assert len(relay.sent) == 1
    transaction = VersionedTransaction.from_bytes(base64.b64decode(relay.sent[0]))
    message = transaction.message
    assert isinstance(message, MessageV1)
    assert message.config.compute_unit_limit == 200_000
    assert message.config.loaded_accounts_data_size_limit == 1_048_576
    assert message.config.heap_size == 32_768
    assert message.config.priority_fee == 7
    assert len(message.instructions) == 1
    ix = message.instructions[0]
    assert str(message.account_keys[ix.program_id_index]) == "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"
    assert bytes(ix.data) == bytes([8])
    assert [str(message.account_keys[index]) for index in ix.accounts] == [str(payer.pubkey())]
    assert result.signature == str(transaction.signatures[0])
