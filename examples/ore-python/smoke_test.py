"""Offline smoke test for the generated `ore_stack` Python package.

Validates the generated binding against the real `arete` runtime without any
network access: StackDef shape, typed views, entity converters, raw
instruction building (byte-exact), PDA derivation (known values), error
metadata, and program-read descriptors.

Run with the SDK venv from `python/arete-sdk`:

    cd python/arete-sdk && .venv/bin/python ../../examples/ore-python/smoke_test.py
"""

from __future__ import annotations

import pathlib
import sys

_HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))
sys.path.insert(0, str(_HERE.parent.parent / "python" / "arete-sdk"))

import arete  # noqa: E402
from arete.program_read_transport import validate_program_read_descriptor  # noqa: E402
from arete.views import ViewDef  # noqa: E402

import ore_stack  # noqa: E402
from ore_stack import ORE_STREAM_STACK, models, programs, views  # noqa: E402

AUTHORITY = "HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T"

# Known-good values (independently derived; the miner PDA matches the value
# printed by examples/ore-rust for the same authority).
EXPECTED_MINER_PDA = ("BgK3ft7LAXygKC1DmGy64EjYkuZmq6saMRasXUGF1bhu", 250)
EXPECTED_BOARD_PDA = ("BrcSxdp1nXFzou1YyDnQJcPNBNHgoypZmTsyKBSLLXzi", 255)
# discriminator [6] + u64 amount LE + u32 squares LE
EXPECTED_DEPLOY_DATA = bytes.fromhex("0640420f000000000003000000")


def check_stack_def() -> None:
    assert isinstance(ORE_STREAM_STACK, arete.StackDef)
    assert ORE_STREAM_STACK.name == "ore-stream"
    assert ORE_STREAM_STACK.endpoints.ws == "wss://ore.stack.arete.run"
    assert ORE_STREAM_STACK.endpoints.http == "https://ore.stack.arete.run"
    assert set(ORE_STREAM_STACK.views) == {
        "ore_round",
        "ore_board",
        "ore_treasury",
        "ore_miner",
    }
    assert set(ORE_STREAM_STACK.programs) == {"ore", "entropy"}
    assert set(ORE_STREAM_STACK.program_reads) == {"ore", "entropy"}
    for name, descriptor in ORE_STREAM_STACK.program_reads.items():
        validate_program_read_descriptor(name, descriptor)
    print("ok: StackDef shape")


def check_views() -> None:
    state = views.OreRoundViews.state
    assert isinstance(state, ViewDef)
    assert state.mode == "state"
    assert state.view == "OreRound/state"
    assert state.key_fields == ("round_id",)
    assert state.parser is models.ore_round_from_wire
    latest = ORE_STREAM_STACK.views["ore_round"]["latest"]
    assert latest.mode == "list" and latest.view == "OreRound/latest"
    assert ORE_STREAM_STACK.views["ore_miner"]["state"].key_fields == ("authority",)
    print("ok: typed views")


def check_models() -> None:
    round_ = models.ore_round_from_wire(
        {
            "id": {"round_id": "42", "round_address": AUTHORITY},
            "state": {"total_deployed": 1.5, "expires_at": "123456789"},
            "metrics": {"deploy_count": 7},
        }
    )
    assert round_.id.round_id == 42  # u64 decimal string -> int
    assert round_.id.round_address == AUTHORITY
    assert round_.state.expires_at == 123456789
    assert round_.state.total_deployed == 1.5
    assert round_.metrics.deploy_count == 7
    assert round_.results.winning_square is None  # absent stays None

    patch = models.ore_round_patch_from_wire({"state": {"expires_at": "5"}})
    assert patch == {"state": {"expires_at": 5}}

    board = models.board_from_wire(
        {
            "roundId": "9",
            "start_slot": 1,
            "end_slot": 2,
            "production_cost_ema": "3",
        }
    )
    assert board.round_id == 9  # camelCase account payloads normalize
    assert board.start_slot == 1

    # IDL struct converters are strict: a payload satisfying none of the
    # schema must raise rather than yield an all-None object (otherwise
    # arete.read's SCHEMA_VALIDATION guard is unreachable).
    try:
        models.board_from_wire({"nothing": "useful"})
    except ValueError:
        pass
    else:
        raise AssertionError("board_from_wire should reject a missing required field")
    print("ok: entity converters")


def check_capture_wrappers() -> None:
    """`#[capture]`-fed fields arrive wrapped; the converter must parse the
    envelope and not mistake it for the bare account struct."""
    treasury = models.ore_treasury_from_wire(
        {
            "id": {"address": AUTHORITY},
            "treasury_snapshot": {
                "timestamp": 1730000000,
                "account_address": AUTHORITY,
                "slot": 381471241,
                "signature": "4xNEYTVL8DB28W87",
                "data": {
                    "motherlode": "123456789",
                    "miner_rewards_factor": {"value": "1"},
                    "total_refined": "7",
                    "total_unclaimed": "0",
                },
            },
        }
    )
    snapshot = treasury.treasury_snapshot
    assert isinstance(snapshot, models.CaptureWrapper)
    assert snapshot.timestamp == 1730000000
    assert snapshot.account_address == AUTHORITY
    assert snapshot.slot == 381471241
    assert snapshot.signature == "4xNEYTVL8DB28W87"
    assert isinstance(snapshot.data, models.Treasury)
    # The regression: these were silently None when the wrapper was parsed as
    # the account struct.
    assert snapshot.data.motherlode == 123456789
    assert snapshot.data.total_refined == 7
    assert snapshot.data.total_unclaimed == 0

    miner = models.ore_miner_from_wire(
        {
            "id": {"authority": AUTHORITY},
            "miner_snapshot": {
                "timestamp": 1,
                "account_address": AUTHORITY,
                "data": {
                    "authority": AUTHORITY,
                    "auto_return": "0",
                    "checkpoint_id": "1",
                    "checkpoint_fee": "2",
                    "deployed": ["1000000000", "2000000000", "0"],
                    "mass": ["1"],
                    "cumulative": ["2"],
                    "round_id": "3",
                    "rewards_factor": {"value": "1"},
                    "rewards_sol": "4",
                    "refined_ore": "5",
                    "rewards_ore": "6",
                    "last_claim_ore_at": "7",
                    "last_claim_sol_at": "8",
                    "lifetime_rewards_ore": "9",
                    "lifetime_deployed": "10",
                    "lifetime_rewards_sol": "11",
                },
            },
        }
    )
    assert isinstance(miner.miner_snapshot, models.CaptureWrapper)
    assert miner.miner_snapshot.data.deployed == [1000000000, 2000000000, 0]
    assert sum(miner.miner_snapshot.data.deployed) == 3000000000

    # Patch converters carry the wrapper too.
    patch = models.ore_treasury_patch_from_wire(
        {
            "treasury_snapshot": {
                "timestamp": 2,
                "account_address": AUTHORITY,
                "data": {
                    "motherlode": "1",
                    "miner_rewards_factor": None,
                    "total_refined": "2",
                    "total_unclaimed": "3",
                },
            }
        }
    )
    assert isinstance(patch["treasury_snapshot"], models.CaptureWrapper)
    assert patch["treasury_snapshot"].data.motherlode == 1
    print("ok: capture wrappers")


def check_u64_arrays() -> None:
    """Entity-projected `Vec<u64>` fields convert to `List[int]`, matching the
    IDL path (`Miner.deployed`) and the TypeScript `bigint[]`."""
    state = models.ore_round_state_from_wire(
        {
            "deployed_per_square": ["1000000000", "2000000000", "0"],
            "count_per_square": ["1", "2"],
            "deployed_per_square_ui": [1.5, 2.5],
        }
    )
    assert state.deployed_per_square == [1000000000, 2000000000, 0]
    assert sum(state.deployed_per_square) == 3000000000  # was a TypeError
    assert state.count_per_square == [1, 2]
    assert state.deployed_per_square_ui == [1.5, 2.5]  # floats stay floats

    miner_state = models.ore_miner_state_from_wire(
        {"deployed_per_square": ["5", "6"]}
    )
    assert miner_state.deployed_per_square == [5, 6]

    patch = models.ore_round_state_patch_from_wire(
        {"count_per_square": ["3", "4"]}
    )
    assert patch == {"count_per_square": [3, 4]}
    print("ok: u64 arrays")


def check_raw_build() -> None:
    instruction = programs.ore_deploy(
        amount=1_000_000,
        squares=3,
        signer=AUTHORITY,
        authority=AUTHORITY,
        round="11111111111111111111111111111111",
        entropyVar="11111111111111111111111111111111",
        entropyProgram=programs.ENTROPY_PROGRAM_ID,
    )
    assert isinstance(instruction, arete.BuiltInstruction)
    assert instruction.program_id == programs.ORE_PROGRAM_ID
    assert instruction.data == EXPECTED_DEPLOY_DATA
    assert len(instruction.accounts) == 12
    assert instruction.accounts[0].pubkey == AUTHORITY  # signer slot
    assert instruction.accounts[0].is_signer

    # Fail-closed: unknown params raise.
    try:
        programs.ore_deploy(amount=1, squares=1, bogus="nope")
    except arete.InstructionError:
        pass
    else:
        raise AssertionError("unknown param should fail closed")

    # The signer fallback option is `wallet`; `payer` stays a real account
    # override (it is an IDL account name on several ore/entropy instructions).
    entropy_payer = "11111111111111111111111111111111"
    opened = programs.entropy_open(
        wallet=AUTHORITY,
        payer=entropy_payer,
        id=1,
        commit=[0] * 32,
        isAuto=0,
        samples=1,
        endAt=0,
        provider=programs.ENTROPY_PROGRAM_ID,
        var=programs.ENTROPY_PROGRAM_ID,
    )
    open_accounts = [meta.name for meta in programs.entropy_open_handler().accounts]
    payer_index = open_accounts.index("payer")
    authority_index = open_accounts.index("authority")
    assert opened.accounts[payer_index].pubkey == entropy_payer
    assert opened.accounts[authority_index].pubkey == AUTHORITY

    # The raw handler escape hatch builds identical bytes.
    handler = programs.ore_deploy_handler()
    assert isinstance(handler, arete.InstructionHandler)
    built = handler.build(
        {
            "amount": 1_000_000,
            "squares": 3,
            "signer": AUTHORITY,
            "authority": AUTHORITY,
            "round": "11111111111111111111111111111111",
            "entropyVar": "11111111111111111111111111111111",
            "entropyProgram": programs.ENTROPY_PROGRAM_ID,
        }
    )
    assert built.data == EXPECTED_DEPLOY_DATA
    print("ok: raw instruction build")


def check_pdas() -> None:
    assert programs.OrePdas.miner.derive(authority=AUTHORITY) == EXPECTED_MINER_PDA
    assert programs.OrePdas.board.derive() == EXPECTED_BOARD_PDA
    # Unknown seed kwargs fail closed.
    try:
        programs.OrePdas.board.derive(bogus="1")
    except TypeError:
        pass
    else:
        raise AssertionError("unknown pda seed kwarg should fail closed")
    print("ok: PDA derivation")


def check_errors_and_reads() -> None:
    metadata = ORE_STREAM_STACK.programs["ore"].errors
    assert (metadata[1].code, metadata[1].name) == (1, "NotAuthorized")
    parsed = arete.parse_program_error(2, metadata)
    assert parsed.name == "InvalidExecutor"
    descriptor = programs.ore_read_descriptor()
    assert descriptor.transport_kind == "local-http"
    assert descriptor.release.program_spec_hash == programs.ORE_PROGRAM_SPEC_HASH
    assert (
        ORE_STREAM_STACK.programs["ore"].program_spec_hash
        == programs.ORE_PROGRAM_SPEC_HASH
    )
    assert set(ORE_STREAM_STACK.programs["ore"].accounts) == {
        "automation",
        "board",
        "miner",
        "treasury",
    }
    print("ok: errors + read descriptors")


def main() -> None:
    assert ore_stack.__all__[0] == "ORE_STREAM_STACK"
    check_stack_def()
    check_views()
    check_models()
    check_capture_wrappers()
    check_u64_arrays()
    check_raw_build()
    check_pdas()
    check_errors_and_reads()
    print("\nsmoke test passed")


if __name__ == "__main__":
    main()
