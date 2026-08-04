"""Ore stack demo for the Python SDK.

Mirrors `examples/ore-rust/src/main.rs`: offline instruction building via the
generated raw builders, PDA derivation, and (connectivity-guarded) chain clock
reads, release-addressed account reads, and views streaming.

Run with any Python (>=3.9) that can import the `arete` SDK; the sys.path
setup below makes the in-repo SDK and the generated `ore_stack` package
importable without installation:

    python examples/ore-python/main.py
"""

from __future__ import annotations

import asyncio
import pathlib
import sys

_HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))
sys.path.insert(0, str(_HERE.parent.parent / "python" / "arete-sdk"))

import arete  # noqa: E402
from ore_stack import ORE_STREAM_STACK, models, programs  # noqa: E402

# Use your own API key in production (can be secret or publishable)
API_KEY = "hspk_alt8MN3BmJebxARE3IlOnnaAEibCrqqXfdG5VoGW"

# Demo wallet for instruction building (no transaction is sent).
AUTHORITY = "HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T"


def demo_deploy_params() -> dict:
    """Wire-shape params for the ore `deploy` instruction (see
    `programs.OreDeployParams`)."""
    return dict(
        amount=1_000_000,
        squares=3,
        signer=AUTHORITY,
        authority=AUTHORITY,
        round="11111111111111111111111111111111",
        entropyVar="11111111111111111111111111111111",
        entropyProgram=programs.ENTROPY_PROGRAM_ID,
    )


def print_instruction(label: str, instruction: arete.BuiltInstruction) -> None:
    print(f"Built `deploy` instruction ({label}):")
    print(f"  Program: {instruction.program_id}")
    print(f"  Accounts: {len(instruction.accounts)}")
    print(f"  Data length: {len(instruction.data)} bytes")


def print_round(round_: models.OreRound) -> None:
    print(f"\n=== Round #{round_.id.round_id or 0} ===")
    print(f"Address: {round_.id.round_address}")
    print(f"Motherlode: {round_.state.motherlode}")
    print(f"Total Deployed: {round_.state.total_deployed}")
    print(f"Expires At: {round_.state.expires_at}")
    print(f"Deploy Count: {round_.metrics.deploy_count}")
    print()


def print_treasury(treasury: models.OreTreasury) -> None:
    print("\n=== Treasury ===")
    print(f"Address: {treasury.id.address}")
    print(f"Motherlode: {treasury.state.motherlode}")
    print(f"Total Refined: {treasury.state.total_refined}")
    print(f"Total Unclaimed: {treasury.state.total_unclaimed}")
    print()


async def stream_first_round(a4: arete.Arete) -> None:
    async for round_ in a4.views.ore_round.latest.use(take=1):
        if round_.id.round_id is not None:
            print_round(round_)
        break


async def stream_first_treasury(a4: arete.Arete) -> None:
    async for treasury in a4.views.ore_treasury.list.use(take=1):
        if treasury.id.address is not None:
            print_treasury(treasury)
        break


async def main() -> None:
    # --- Program SDK demo: instruction building is pure (no network) ---
    print("--- Building an ore `deploy` instruction offline ---\n")
    miner_pda, bump = programs.OrePdas.miner.derive(authority=AUTHORITY)
    print(f"Derived miner PDA: {miner_pda} (bump {bump})")
    board_pda, _ = programs.OrePdas.board.derive()
    print(f"Derived board PDA: {board_pda}")
    instruction = programs.ore_deploy(**demo_deploy_params())
    print_instruction("standalone module", instruction)
    print()

    try:
        a4 = await arete.Arete.connect(
            ORE_STREAM_STACK,
            auth=arete.AuthConfig.from_api_key(API_KEY),
        )
    except Exception as error:  # noqa: BLE001 - connectivity-guarded demo
        print(f"warning: could not connect ({error}); offline demo only")
        return

    try:
        # The same raw builders are exposed on the connected client.
        instruction = a4.programs.ore.raw.deploy.build(**demo_deploy_params())
        print_instruction("via a4.programs.ore.raw", instruction)

        # --- Chain reads over the stack HTTP endpoint (best-effort) ---
        print("\n--- Reading the cluster clock ---\n")
        try:
            clock = await a4.chain.clock()
            print(f"Cluster clock: slot {clock.slot}")
        except Exception as error:  # noqa: BLE001
            print(f"warning: chain clock read failed: {error}")

        # --- Release-addressed program account reads (best-effort) ---
        print("\n--- Fetching the ore Board account ---\n")
        try:
            board = await a4.programs.ore.accounts.board.fetch(board_pda)
            if board is None:
                print(f"Board account not found at {board_pda}")
            else:
                print(f"Board account at {board_pda}:")
                print(f"  Round ID: {board.round_id}")
                print(f"  Start Slot: {board.start_slot}")
                print(f"  End Slot: {board.end_slot}")
        except Exception as error:  # noqa: BLE001
            print(f"warning: board account read failed: {error}")

        # --- Views streaming: one OreRound and one OreTreasury ---
        print("\n--- Streaming OreRound and OreTreasury updates ---\n")
        try:
            await asyncio.wait_for(
                asyncio.gather(stream_first_round(a4), stream_first_treasury(a4)),
                timeout=20,
            )
        except asyncio.TimeoutError:
            print("warning: no view updates arrived within 20s")
    finally:
        await a4.disconnect()


if __name__ == "__main__":
    asyncio.run(main())
