"""Generated program SDKs for the `vault_stream` stack. Do not edit.

Instruction building is pure (no network access). Each program section
exposes `<PROG>_PROGRAM_ID`, `<Ix>Params` TypedDicts, `<prog>_<ix>(**params)`
builders returning `BuiltInstruction`, raw `<prog>_<ix>_handler()` escape
hatches, and a `<Prog>Pdas` namespace of PDA factories. Programs with a
recorded program spec additionally expose `<PROG>_PROGRAM_SPEC_HASH` /
`<PROG>_PROGRAM_RELEASE_HASH` plus `<prog>_read_descriptor()` for
release-addressed HTTP reads. `PROGRAMS` / `PROGRAM_READS` compose with stack
bindings and standalone session members.
"""

from __future__ import annotations

from typing import Any, Dict, Mapping, Optional, Sequence, Tuple, TypedDict

from arete.instructions import (
    AccountMeta,
    ArgSchema,
    BuiltAccountMeta,
    BuiltInstruction,
    ErrorMetadata,
    InstructionHandler,
    Known,
    Signer,
    UserProvided,
)
from arete.program_read_transport import (
    LocalHttpTransportDef,
    ProgramReadDescriptor,
    ProgramReleaseReference,
)
from arete.read import ProgramAccountReadDef
from arete.stack import ProgramDef

from . import models

__all__ = [
    "VaultDepositParams",
    "vault_deposit",
    "vault_deposit_handler",
    "VAULT_PROGRAM_ID",
    "VAULT_PROGRAM_SPEC_HASH",
    "VAULT_PROGRAM_RELEASE_HASH",
    "vault_read_descriptor",
    "VAULT_ERRORS",
    "VAULT_PROGRAM",
    "PROGRAMS",
    "PROGRAM_READS",
]


# ==========================================================================
# Program `vault` (program ID `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`)
# ==========================================================================

VAULT_PROGRAM_ID = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM"

#: Content hash of the exact program specification captured at generation time.
VAULT_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba"

#: Release identity addressing hosted account reads for this program.
VAULT_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:472ece19e168367b6bd0a61a8e5908345db325bd1ac3f7d31febdd6a3fe09777"


def vault_read_descriptor() -> ProgramReadDescriptor:
    """Exact release-addressed read descriptor for program `vault`."""
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash=VAULT_PROGRAM_RELEASE_HASH,
            program_spec_hash=VAULT_PROGRAM_SPEC_HASH,
        ),
        transport=LocalHttpTransportDef(),
    )

#: IDL error metadata for program `vault`.
VAULT_ERRORS: Tuple[ErrorMetadata, ...] = (
    ErrorMetadata(code=0, name="AmountTooSmall", msg="Amount too small"),
)

# Typed params for `deposit`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
VaultDepositParams = TypedDict(
    "VaultDepositParams",
    {
        # arg `amount` (`u64`)
        "amount": int,
        # Optional address override for the `authority` signer (defaults to the payer).
        "authority": str,
        # Address of the `vault` account.
        "vault": str,
    },
    total=False,
)


def vault_deposit(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Builds the `deposit` instruction.

    Pure (no network). Params use IDL wire names plus documented account aliases (see `VaultDepositParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return vault_deposit_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def vault_deposit_handler() -> InstructionHandler:
    """Raw instruction handler for `deposit` (escape hatch)."""
    return InstructionHandler(
        program_id=VAULT_PROGRAM_ID,
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
                name="vault",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
        ],
        errors=list(VAULT_ERRORS),
    )

#: Portable program SDK definition consumed by `arete.stack`.
VAULT_PROGRAM = ProgramDef(
    name="vault",
    program_id=VAULT_PROGRAM_ID,
    raw_instructions={
        "deposit": vault_deposit_handler(),
    },
    pdas={},
    accounts={},
    errors=VAULT_ERRORS,
    program_spec_hash=VAULT_PROGRAM_SPEC_HASH,
)


PROGRAMS: Dict[str, ProgramDef] = {
    "vault": VAULT_PROGRAM,
}

PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {
    "vault": vault_read_descriptor(),
}
