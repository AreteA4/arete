"""Generated program SDKs for the `OreStream` stack. Do not edit.

Instruction building is pure (no network access). Each program section
exposes `<PROG>_PROGRAM_ID`, `<Ix>Params` TypedDicts, `<prog>_<ix>(**params)`
builders returning `BuiltInstruction`, raw `<prog>_<ix>_handler()` escape
hatches, and a `<Prog>Pdas` namespace of PDA factories. Programs with a
recorded program spec additionally expose `<PROG>_PROGRAM_SPEC_HASH` /
`<PROG>_PROGRAM_RELEASE_HASH` plus `<prog>_read_descriptor()` for
release-addressed HTTP reads. `PROGRAMS` / `PROGRAM_READS` feed the stack
binding in `__init__.py`.
"""

from __future__ import annotations

from typing import Any, Dict, Mapping, Optional, Sequence, Tuple, TypedDict

from arete.instructions import (
    AccountMeta,
    AccountRefSeed,
    ArgRefSeed,
    ArgSchema,
    BuiltAccountMeta,
    BuiltInstruction,
    BytesSeed,
    ErrorMetadata,
    InstructionHandler,
    Known,
    LiteralSeed,
    Pda,
    PdaConfig,
    Signer,
    UserProvided,
)
from arete.program_read_transport import (
    LocalHttpTransportDef,
    ProgramReadDescriptor,
    ProgramReleaseReference,
)
from arete.read import ProgramAccountReadDef
from arete.stack import PdaFactory, ProgramDef

from . import models

__all__ = [
    "OreAutomateParams",
    "ore_automate",
    "ore_automate_handler",
    "OreCheckpointParams",
    "ore_checkpoint",
    "ore_checkpoint_handler",
    "OreClaimSolParams",
    "ore_claim_sol",
    "ore_claim_sol_handler",
    "OreClaimOreParams",
    "ore_claim_ore",
    "ore_claim_ore_handler",
    "OreCloseParams",
    "ore_close",
    "ore_close_handler",
    "OreDeployParams",
    "ore_deploy",
    "ore_deploy_handler",
    "OreLogParams",
    "ore_log",
    "ore_log_handler",
    "OreResetParams",
    "ore_reset",
    "ore_reset_handler",
    "OreBuybackParams",
    "ore_buyback",
    "ore_buyback_handler",
    "OreBuryParams",
    "ore_bury",
    "ore_bury_handler",
    "OreWrapParams",
    "ore_wrap",
    "ore_wrap_handler",
    "OreSetAdminParams",
    "ore_set_admin",
    "ore_set_admin_handler",
    "OreNewVarParams",
    "ore_new_var",
    "ore_new_var_handler",
    "OreReloadSolParams",
    "ore_reload_sol",
    "ore_reload_sol_handler",
    "ORE_PROGRAM_ID",
    "ORE_PROGRAM_SPEC_HASH",
    "ORE_PROGRAM_RELEASE_HASH",
    "ore_read_descriptor",
    "ORE_ERRORS",
    "OrePdas",
    "ORE_PROGRAM",
    "EntropyOpenParams",
    "entropy_open",
    "entropy_open_handler",
    "EntropyCloseParams",
    "entropy_close",
    "entropy_close_handler",
    "EntropyNextParams",
    "entropy_next",
    "entropy_next_handler",
    "EntropyRevealParams",
    "entropy_reveal",
    "entropy_reveal_handler",
    "EntropySampleParams",
    "entropy_sample",
    "entropy_sample_handler",
    "ENTROPY_PROGRAM_ID",
    "ENTROPY_PROGRAM_SPEC_HASH",
    "ENTROPY_PROGRAM_RELEASE_HASH",
    "entropy_read_descriptor",
    "ENTROPY_ERRORS",
    "ENTROPY_PROGRAM",
    "PROGRAMS",
    "PROGRAM_READS",
]


# ==========================================================================
# Program `ore` (program ID `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`)
# ==========================================================================

ORE_PROGRAM_ID = "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"

#: Content hash of the exact program specification captured at generation time.
ORE_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:3bfd8a90dcb0bdab01235344fb61e35912990942396295e5546f242c56e9ea97"

#: Release identity addressing hosted account reads for this program.
ORE_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:a6d6453e013dc0a8d725e3d351557a08a7203381b9fac78c6218a3926a1b467b"


def ore_read_descriptor() -> ProgramReadDescriptor:
    """Release-addressed read descriptor for program `ore` (HTTP reads
    over the client's HTTP base URL)."""
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash=ORE_PROGRAM_RELEASE_HASH,
            program_spec_hash=ORE_PROGRAM_SPEC_HASH,
        ),
        transport=LocalHttpTransportDef(),
    )

#: IDL error metadata for program `ore`.
ORE_ERRORS: Tuple[ErrorMetadata, ...] = (
    ErrorMetadata(code=0, name="AmountTooSmall", msg="Amount too small"),
    ErrorMetadata(code=1, name="NotAuthorized", msg="Not authorized"),
    ErrorMetadata(code=2, name="InvalidExecutor", msg="Invalid executor"),
)

_ORE_PDAS: Dict[str, PdaConfig] = {
    "automation": PdaConfig(seeds=(LiteralSeed("automation"), AccountRefSeed("authority"))),
    "board": PdaConfig(seeds=(LiteralSeed("board"),)),
    "config": PdaConfig(seeds=(LiteralSeed("config"),)),
    "miner": PdaConfig(seeds=(LiteralSeed("miner"), AccountRefSeed("authority"))),
    "treasury": PdaConfig(seeds=(LiteralSeed("treasury"),)),
}


class OrePdas:
    """PDA factories for program `ore`: `.derive(**seeds)` returns
    `(address, bump)`; unknown seed kwargs fail closed."""

    automation = PdaFactory("automation", _ORE_PDAS["automation"], ORE_PROGRAM_ID)
    board = PdaFactory("board", _ORE_PDAS["board"], ORE_PROGRAM_ID)
    config = PdaFactory("config", _ORE_PDAS["config"], ORE_PROGRAM_ID)
    miner = PdaFactory("miner", _ORE_PDAS["miner"], ORE_PROGRAM_ID)
    treasury = PdaFactory("treasury", _ORE_PDAS["treasury"], ORE_PROGRAM_ID)

# Typed params for `automate`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreAutomateParams = TypedDict(
    "OreAutomateParams",
    {
        # arg `amount` (`u64`)
        "amount": int,
        # arg `deposit` (`u64`)
        "deposit": int,
        # arg `fee` (`u64`)
        "fee": int,
        # arg `mask` (`u64`)
        "mask": int,
        # arg `strategy` (`u8`)
        "strategy": int,
        # arg `reload` (`u64`)
        "reload": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `automation` account.
        "automation": str,
        # Address of the `executor` account.
        "executor": str,
        # Address of the `miner` account.
        "miner": str,
    },
    total=False,
)


def ore_automate(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Configures or closes a miner automation account.
    Automation PDA seeds: ["automation", signer].
    Miner PDA seeds: ["miner", signer].

    Pure (no network). Params are IDL wire shape (see `OreAutomateParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.

    Codegen notes:
    - account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
    - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    """
    return ore_automate_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_automate_handler() -> InstructionHandler:
    """Raw instruction handler for `automate` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([0]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            # [arete codegen] account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="automation",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="executor",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
            # [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
            ArgSchema(name="deposit", type="u64"),
            ArgSchema(name="fee", type="u64"),
            ArgSchema(name="mask", type="u64"),
            ArgSchema(name="strategy", type="u8"),
            ArgSchema(name="reload", type="u64"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `checkpoint`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreCheckpointParams = TypedDict(
    "OreCheckpointParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `authority` account.
        "authority": str,
        # Address of the `round` account.
        "round": str,
    },
    total=False,
)


def ore_checkpoint(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Settles miner rewards for a completed round.
    Treasury PDA seeds: ["treasury"].

    Pure (no network). Params are IDL wire shape (see `OreCheckpointParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_checkpoint_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_checkpoint_handler() -> InstructionHandler:
    """Raw instruction handler for `checkpoint` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([2]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="authority",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="automation",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("automation"), AccountRefSeed("authority")))),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("miner"), AccountRefSeed("authority")))),
                is_optional=False,
            ),
            AccountMeta(
                name="round",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `claimSol`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreClaimSolParams = TypedDict(
    "OreClaimSolParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `miner` account.
        "miner": str,
    },
    total=False,
)


def ore_claim_sol(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Claims SOL rewards from the miner account.

    Pure (no network). Params are IDL wire shape (see `OreClaimSolParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.

    Codegen notes:
    - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    """
    return ore_claim_sol_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_claim_sol_handler() -> InstructionHandler:
    """Raw instruction handler for `claimSol` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([3]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            # [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `claimOre`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreClaimOreParams = TypedDict(
    "OreClaimOreParams",
    {
        # arg `bps` (`u64`)
        "bps": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `miner` account.
        "miner": str,
        # Address of the `recipient` account.
        "recipient": str,
        # Address of the `treasuryTokens` account.
        "treasuryTokens": str,
    },
    total=False,
)


def ore_claim_ore(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Claims a percentage of ORE token rewards from the treasury vault.
    The current instruction encodes bps as u64. Legacy empty payloads are accepted by the program as 10000 bps.

    Pure (no network). Params are IDL wire shape (see `OreClaimOreParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.

    Codegen notes:
    - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    """
    return ore_claim_ore_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_claim_ore_handler() -> InstructionHandler:
    """Raw instruction handler for `claimOre` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([4]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            # [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="mint",
                is_signer=False,
                is_writable=True,
                resolution=Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp"),
                is_optional=False,
            ),
            AccountMeta(
                name="recipient",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasuryTokens",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="tokenProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
                is_optional=False,
            ),
            AccountMeta(
                name="associatedTokenProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="bps", type="u64"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `close`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreCloseParams = TypedDict(
    "OreCloseParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `rentPayer` account.
        "rentPayer": str,
        # Address of the `round` account.
        "round": str,
    },
    total=False,
)


def ore_close(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Closes an expired round account and returns rent to the payer.
    Round PDA seeds: ["round", round_id].
    Treasury PDA seeds: ["treasury"].

    Pure (no network). Params are IDL wire shape (see `OreCloseParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_close_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_close_handler() -> InstructionHandler:
    """Raw instruction handler for `close` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([5]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="rentPayer",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="round",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `deploy`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreDeployParams = TypedDict(
    "OreDeployParams",
    {
        # arg `amount` (`u64`)
        "amount": int,
        # arg `squares` (`u32`)
        "squares": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `authority` account.
        "authority": str,
        # Address of the `round` account.
        "round": str,
        # Address of the `entropyVar` account.
        "entropyVar": str,
        # Address of the `entropyProgram` account.
        "entropyProgram": str,
    },
    total=False,
)


def ore_deploy(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Deploys SOL to selected squares for the current round.
    Automation PDA seeds: ["automation", authority].
    Config PDA seeds: ["config"].
    Miner PDA seeds: ["miner", authority].
    Round PDA seeds: ["round", board.round_id].

    Pure (no network). Params are IDL wire shape (see `OreDeployParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_deploy_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_deploy_handler() -> InstructionHandler:
    """Raw instruction handler for `deploy` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([6]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="authority",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="automation",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("automation"), AccountRefSeed("authority")))),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("miner"), AccountRefSeed("authority")))),
                is_optional=False,
            ),
            AccountMeta(
                name="round",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
            AccountMeta(
                name="entropyVar",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="entropyProgram",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
            ArgSchema(name="squares", type="u32"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `log`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreLogParams = TypedDict(
    "OreLogParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
    },
    total=False,
)


def ore_log(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Emits an arbitrary log message from the board PDA.
    Bytes following the discriminator are logged verbatim.

    Pure (no network). Params are IDL wire shape (see `OreLogParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_log_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_log_handler() -> InstructionHandler:
    """Raw instruction handler for `log` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([8]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `reset`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreResetParams = TypedDict(
    "OreResetParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `feeCollector` account.
        "feeCollector": str,
        # Address of the `round` account.
        "round": str,
        # Address of the `roundNext` account.
        "roundNext": str,
        # Address of the `topMiner` account.
        "topMiner": str,
        # Address of the `treasuryTokens` account.
        "treasuryTokens": str,
        # Address of the `entropyVar` account.
        "entropyVar": str,
        # Address of the `mintAuthority` account.
        "mintAuthority": str,
    },
    total=False,
)


def ore_reset(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Finalizes the current round, mints rewards, and opens the next round.
    Board PDA seeds: ["board"].
    Treasury PDA seeds: ["treasury"].
    Round PDA seeds: ["round", board.round_id] and ["round", board.round_id + 1].

    Pure (no network). Params are IDL wire shape (see `OreResetParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_reset_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_reset_handler() -> InstructionHandler:
    """Raw instruction handler for `reset` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([9]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="feeCollector",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="mint",
                is_signer=False,
                is_writable=True,
                resolution=Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp"),
                is_optional=False,
            ),
            AccountMeta(
                name="round",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="roundNext",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="topMiner",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasuryTokens",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="tokenProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
            AccountMeta(
                name="slotHashesSysvar",
                is_signer=False,
                is_writable=False,
                resolution=Known("SysvarS1otHashes111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="entropyVar",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="entropyProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X"),
                is_optional=False,
            ),
            AccountMeta(
                name="mintAuthority",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="mintProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("mintzxW6Kckmeyh1h6Zfdj9QcYgCzhPSGiC8ChZ6fCx"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `buyback`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreBuybackParams = TypedDict(
    "OreBuybackParams",
    {
        # Address of the `managerSol` account.
        "managerSol": str,
        # Address of the `treasuryOre` account.
        "treasuryOre": str,
        # Address of the `treasurySol` account.
        "treasurySol": str,
        # Address of the `stakeTreasury` account.
        "stakeTreasury": str,
        # Address of the `stakeTreasuryOre` account.
        "stakeTreasuryOre": str,
        # Address of the `stakeVesting` account.
        "stakeVesting": str,
        # Address of the `oreStakeProgram` account.
        "oreStakeProgram": str,
    },
    total=False,
)


def ore_buyback(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Swaps vaulted SOL to ORE through Jupiter, distributes staking yield, and burns the remainder.
    The 15 declared accounts are followed by Jupiter route accounts, and raw Jupiter instruction data follows the discriminator.

    Pure (no network). Params are IDL wire shape (see `OreBuybackParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_buyback_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_buyback_handler() -> InstructionHandler:
    """Raw instruction handler for `buyback` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([13]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Known("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T"),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=False,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="manager",
                is_signer=False,
                is_writable=True,
                resolution=Known("DJqfQWB8tZE6fzqWa8okncDh7ciTuD8QQKp1ssNETWee"),
                is_optional=False,
            ),
            AccountMeta(
                name="managerSol",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="mint",
                is_signer=False,
                is_writable=True,
                resolution=Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp"),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasuryOre",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="treasurySol",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeTreasury",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeTreasuryOre",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeVesting",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="tokenProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreStakeProgram",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

# Typed params for `bury`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreBuryParams = TypedDict(
    "OreBuryParams",
    {
        # arg `amount` (`u64`)
        "amount": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `sender` account.
        "sender": str,
        # Address of the `treasuryOre` account.
        "treasuryOre": str,
        # Address of the `stakeTreasury` account.
        "stakeTreasury": str,
        # Address of the `stakeTreasuryTokens` account.
        "stakeTreasuryTokens": str,
        # Address of the `stakeVesting` account.
        "stakeVesting": str,
        # Address of the `oreStakeProgram` account.
        "oreStakeProgram": str,
    },
    total=False,
)


def ore_bury(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Burns ORE and distributes yield to stakers.
    Treasury PDA seeds: ["treasury"].

    Pure (no network). Params are IDL wire shape (see `OreBuryParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_bury_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_bury_handler() -> InstructionHandler:
    """Raw instruction handler for `bury` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([24]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="sender",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="mint",
                is_signer=False,
                is_writable=True,
                resolution=Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp"),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasuryOre",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeTreasury",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeTreasuryTokens",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="stakeVesting",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="tokenProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"),
                is_optional=False,
            ),
            AccountMeta(
                name="oreStakeProgram",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `wrap`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreWrapParams = TypedDict(
    "OreWrapParams",
    {
        # arg `amount` (`u64`)
        "amount": int,
        # Address of the `treasurySol` account.
        "treasurySol": str,
    },
    total=False,
)


def ore_wrap(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Wraps SOL held by the treasury into WSOL for swapping.
    Treasury PDA seeds: ["treasury"].

    Pure (no network). Params are IDL wire shape (see `OreWrapParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_wrap_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_wrap_handler() -> InstructionHandler:
    """Raw instruction handler for `wrap` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([14]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Known("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T"),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=False,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasury",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("treasury"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="treasurySol",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `setAdmin`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreSetAdminParams = TypedDict(
    "OreSetAdminParams",
    {
        # arg `admin` (`solana_pubkey::Pubkey`)
        "admin": str,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
    },
    total=False,
)


def ore_set_admin(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Updates the program admin address.

    Pure (no network). Params are IDL wire shape (see `OreSetAdminParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_set_admin_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_set_admin_handler() -> InstructionHandler:
    """Raw instruction handler for `setAdmin` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([15]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="admin", type="pubkey"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `newVar`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreNewVarParams = TypedDict(
    "OreNewVarParams",
    {
        # arg `id` (`u64`)
        "id": int,
        # arg `commit` (`[u8; 32]`)
        "commit": Sequence[int],
        # arg `samples` (`u64`)
        "samples": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `provider` account.
        "provider": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def ore_new_var(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Creates a new entropy var account through the entropy program.

    Pure (no network). Params are IDL wire shape (see `OreNewVarParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return ore_new_var_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_new_var_handler() -> InstructionHandler:
    """Raw instruction handler for `newVar` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([19]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="board",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("board"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="config",
                is_signer=False,
                is_writable=True,
                resolution=Pda(PdaConfig(seeds=(LiteralSeed("config"),))),
                is_optional=False,
            ),
            AccountMeta(
                name="provider",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
            AccountMeta(
                name="entropyProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="id", type="u64"),
            ArgSchema(name="commit", type={"array": ("u8", 32)}),
            ArgSchema(name="samples", type="u64"),
        ],
        errors=list(ORE_ERRORS),
    )

# Typed params for `reloadSol`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
OreReloadSolParams = TypedDict(
    "OreReloadSolParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `automation` account.
        "automation": str,
        # Address of the `miner` account.
        "miner": str,
    },
    total=False,
)


def ore_reload_sol(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Deprecated since 3.8.15; this behavior is now included in checkpoint.

    Pure (no network). Params are IDL wire shape (see `OreReloadSolParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.

    Codegen notes:
    - account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
    - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    """
    return ore_reload_sol_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def ore_reload_sol_handler() -> InstructionHandler:
    """Raw instruction handler for `reloadSol` (escape hatch)."""
    return InstructionHandler(
        program_id=ORE_PROGRAM_ID,
        discriminator=bytes([21]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            # [arete codegen] account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="automation",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            # [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
            AccountMeta(
                name="miner",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ORE_ERRORS),
    )

_ORE_ACCOUNTS: Dict[str, ProgramAccountReadDef] = {
    "automation": ProgramAccountReadDef(account="Automation", parser=models.automation_from_wire),
    "board": ProgramAccountReadDef(account="Board", parser=models.board_from_wire),
    "miner": ProgramAccountReadDef(account="Miner", parser=models.miner_from_wire),
    "treasury": ProgramAccountReadDef(account="Treasury", parser=models.treasury_from_wire),
}

#: Portable program SDK definition consumed by `arete.stack`.
ORE_PROGRAM = ProgramDef(
    name="ore",
    program_id=ORE_PROGRAM_ID,
    raw_instructions={
        "automate": ore_automate_handler(),
        "checkpoint": ore_checkpoint_handler(),
        "claim_sol": ore_claim_sol_handler(),
        "claim_ore": ore_claim_ore_handler(),
        "close": ore_close_handler(),
        "deploy": ore_deploy_handler(),
        "log": ore_log_handler(),
        "reset": ore_reset_handler(),
        "buyback": ore_buyback_handler(),
        "bury": ore_bury_handler(),
        "wrap": ore_wrap_handler(),
        "set_admin": ore_set_admin_handler(),
        "new_var": ore_new_var_handler(),
        "reload_sol": ore_reload_sol_handler(),
    },
    pdas=dict(_ORE_PDAS),
    accounts=dict(_ORE_ACCOUNTS),
    errors=ORE_ERRORS,
    program_spec_hash=ORE_PROGRAM_SPEC_HASH,
)


# ==========================================================================
# Program `entropy` (program ID `3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X`)
# ==========================================================================

ENTROPY_PROGRAM_ID = "3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X"

#: Content hash of the exact program specification captured at generation time.
ENTROPY_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b"

#: Release identity addressing hosted account reads for this program.
ENTROPY_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:9e7d6811735b35f9fd144c1eaa21ac1a48720b706d81bd0d0cd9ad6ec7f32b6c"


def entropy_read_descriptor() -> ProgramReadDescriptor:
    """Release-addressed read descriptor for program `entropy` (HTTP reads
    over the client's HTTP base URL)."""
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash=ENTROPY_PROGRAM_RELEASE_HASH,
            program_spec_hash=ENTROPY_PROGRAM_SPEC_HASH,
        ),
        transport=LocalHttpTransportDef(),
    )

#: IDL error metadata for program `entropy`.
ENTROPY_ERRORS: Tuple[ErrorMetadata, ...] = (
    ErrorMetadata(code=0, name="IncompleteDigest", msg="Incomplete digest"),
    ErrorMetadata(code=1, name="InvalidSeed", msg="Invalid seed"),
)

# Typed params for `open`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
EntropyOpenParams = TypedDict(
    "EntropyOpenParams",
    {
        # arg `id` (`u64`)
        "id": int,
        # arg `commit` (`[u8; 32]`)
        "commit": Sequence[int],
        # arg `isAuto` (`u64`)
        "isAuto": int,
        # arg `samples` (`u64`)
        "samples": int,
        # arg `endAt` (`u64`)
        "endAt": int,
        # Optional address override for the `authority` signer (defaults to the payer).
        "authority": str,
        # Optional address override for the `payer` signer (defaults to the payer).
        "payer": str,
        # Address of the `provider` account.
        "provider": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def entropy_open(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Creates a new entropy var account.
    Var PDA seeds: ["var", authority, id].

    Pure (no network). Params are IDL wire shape (see `EntropyOpenParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return entropy_open_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def entropy_open_handler() -> InstructionHandler:
    """Raw instruction handler for `open` (escape hatch)."""
    return InstructionHandler(
        program_id=ENTROPY_PROGRAM_ID,
        discriminator=bytes([0]),
        accounts=[
            AccountMeta(
                name="authority",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="payer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="provider",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="id", type="u64"),
            ArgSchema(name="commit", type={"array": ("u8", 32)}),
            ArgSchema(name="isAuto", type="u64"),
            ArgSchema(name="samples", type="u64"),
            ArgSchema(name="endAt", type="u64"),
        ],
        errors=list(ENTROPY_ERRORS),
    )

# Typed params for `close`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
EntropyCloseParams = TypedDict(
    "EntropyCloseParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def entropy_close(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Closes an entropy var account and returns rent to the authority.

    Pure (no network). Params are IDL wire shape (see `EntropyCloseParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return entropy_close_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def entropy_close_handler() -> InstructionHandler:
    """Raw instruction handler for `close` (escape hatch)."""
    return InstructionHandler(
        program_id=ENTROPY_PROGRAM_ID,
        discriminator=bytes([1]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="systemProgram",
                is_signer=False,
                is_writable=False,
                resolution=Known("11111111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ENTROPY_ERRORS),
    )

# Typed params for `next`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
EntropyNextParams = TypedDict(
    "EntropyNextParams",
    {
        # arg `endAt` (`u64`)
        "endAt": int,
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def entropy_next(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Updates the var for the next random value sample.
    Resets the commit to the previous seed and clears slot_hash, seed, and value.

    Pure (no network). Params are IDL wire shape (see `EntropyNextParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return entropy_next_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def entropy_next_handler() -> InstructionHandler:
    """Raw instruction handler for `next` (escape hatch)."""
    return InstructionHandler(
        program_id=ENTROPY_PROGRAM_ID,
        discriminator=bytes([2]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="endAt", type="u64"),
        ],
        errors=list(ENTROPY_ERRORS),
    )

# Typed params for `reveal`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
EntropyRevealParams = TypedDict(
    "EntropyRevealParams",
    {
        # arg `seed` (`[u8; 32]`)
        "seed": Sequence[int],
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def entropy_reveal(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Reveals the seed and finalizes the random value.
    The seed must hash to the commit stored in the var account.

    Pure (no network). Params are IDL wire shape (see `EntropyRevealParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return entropy_reveal_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def entropy_reveal_handler() -> InstructionHandler:
    """Raw instruction handler for `reveal` (escape hatch)."""
    return InstructionHandler(
        program_id=ENTROPY_PROGRAM_ID,
        discriminator=bytes([4]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="seed", type={"array": ("u8", 32)}),
        ],
        errors=list(ENTROPY_ERRORS),
    )

# Typed params for `sample`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
EntropySampleParams = TypedDict(
    "EntropySampleParams",
    {
        # Optional address override for the `signer` signer (defaults to the payer).
        "signer": str,
        # Address of the `var` account.
        "var": str,
    },
    total=False,
)


def entropy_sample(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Samples the slot hash at the end_at slot.
    Must be called after the end_at slot has passed.

    Pure (no network). Params are IDL wire shape (see `EntropySampleParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return entropy_sample_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def entropy_sample_handler() -> InstructionHandler:
    """Raw instruction handler for `sample` (escape hatch)."""
    return InstructionHandler(
        program_id=ENTROPY_PROGRAM_ID,
        discriminator=bytes([5]),
        accounts=[
            AccountMeta(
                name="signer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
            ),
            AccountMeta(
                name="var",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="slotHashesSysvar",
                is_signer=False,
                is_writable=False,
                resolution=Known("SysvarS1otHashes111111111111111111111111111"),
                is_optional=False,
            ),
        ],
        args=[],
        errors=list(ENTROPY_ERRORS),
    )

#: Portable program SDK definition consumed by `arete.stack`.
ENTROPY_PROGRAM = ProgramDef(
    name="entropy",
    program_id=ENTROPY_PROGRAM_ID,
    raw_instructions={
        "open": entropy_open_handler(),
        "close": entropy_close_handler(),
        "next": entropy_next_handler(),
        "reveal": entropy_reveal_handler(),
        "sample": entropy_sample_handler(),
    },
    pdas={},
    accounts={},
    errors=ENTROPY_ERRORS,
    program_spec_hash=ENTROPY_PROGRAM_SPEC_HASH,
)


PROGRAMS: Dict[str, ProgramDef] = {
    "ore": ORE_PROGRAM,
    "entropy": ENTROPY_PROGRAM,
}

PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {
    "ore": ore_read_descriptor(),
    "entropy": entropy_read_descriptor(),
}
