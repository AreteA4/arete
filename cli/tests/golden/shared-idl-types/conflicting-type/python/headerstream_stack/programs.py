"""Generated program SDKs for the `HeaderStream` stack. Do not edit.

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
    "AlphaConfigureParams",
    "alpha_configure",
    "alpha_configure_handler",
    "ALPHA_PROGRAM_ID",
    "ALPHA_PROGRAM_SPEC_HASH",
    "ALPHA_PROGRAM_RELEASE_HASH",
    "alpha_read_descriptor",
    "ALPHA_ERRORS",
    "ALPHA_PROGRAM",
    "BetaConfigureParams",
    "beta_configure",
    "beta_configure_handler",
    "BETA_PROGRAM_ID",
    "BETA_PROGRAM_SPEC_HASH",
    "BETA_PROGRAM_RELEASE_HASH",
    "beta_read_descriptor",
    "BETA_ERRORS",
    "BETA_PROGRAM",
    "PROGRAMS",
    "PROGRAM_READS",
]


# ==========================================================================
# Program `alpha` (program ID `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`)
# ==========================================================================

ALPHA_PROGRAM_ID = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM"

#: Content hash of the exact program specification captured at generation time.
ALPHA_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:07ca57c38922e4521a86f57a079fc28eec26733af78a3bef1af19c69c65b88ab"

#: Release identity addressing hosted account reads for this program.
ALPHA_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:7ca2173db2139bb3009579d116fbd6a1bedf01b1885f0b08ecc52cacf8219b7c"


def alpha_read_descriptor() -> ProgramReadDescriptor:
    """Exact release-addressed read descriptor for program `alpha`."""
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash=ALPHA_PROGRAM_RELEASE_HASH,
            program_spec_hash=ALPHA_PROGRAM_SPEC_HASH,
        ),
        transport=LocalHttpTransportDef(),
    )

#: IDL error metadata for program `alpha`.
ALPHA_ERRORS: Tuple[ErrorMetadata, ...] = ()

# Typed params for `configure`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
AlphaConfigureParams = TypedDict(
    "AlphaConfigureParams",
    {
        # arg `header` (`Header`)
        "header": Any,
        # Optional address override for the `authority` signer (defaults to the payer).
        "authority": str,
        # Address of the `vault` account.
        "vault": str,
    },
    total=False,
)


def alpha_configure(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Builds the `configure` instruction.

    Pure (no network). Params use IDL wire names plus documented account aliases (see `AlphaConfigureParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return alpha_configure_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def alpha_configure_handler() -> InstructionHandler:
    """Raw instruction handler for `configure` (escape hatch)."""
    return InstructionHandler(
        program_id=ALPHA_PROGRAM_ID,
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
            ArgSchema(name="header", type={"struct": [{"name": "version", "type": "u8"}, {"name": "owner", "type": "pubkey"}]}),
        ],
        errors=[],
    )

#: Portable program SDK definition consumed by `arete.stack`.
ALPHA_PROGRAM = ProgramDef(
    name="alpha",
    program_id=ALPHA_PROGRAM_ID,
    raw_instructions={
        "configure": alpha_configure_handler(),
    },
    pdas={},
    accounts={},
    errors=ALPHA_ERRORS,
    program_spec_hash=ALPHA_PROGRAM_SPEC_HASH,
)


# ==========================================================================
# Program `beta` (program ID `Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS`)
# ==========================================================================

BETA_PROGRAM_ID = "Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS"

#: Content hash of the exact program specification captured at generation time.
BETA_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:5c21623087e4efea884fb1f00db5f302c5c4db81673a227ed95d09d42e02e809"

#: Release identity addressing hosted account reads for this program.
BETA_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:18231d9aaf7d096cbe46a1a171c98a935fe0984deb0301726520b86a4c656beb"


def beta_read_descriptor() -> ProgramReadDescriptor:
    """Exact release-addressed read descriptor for program `beta`."""
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash=BETA_PROGRAM_RELEASE_HASH,
            program_spec_hash=BETA_PROGRAM_SPEC_HASH,
        ),
        transport=LocalHttpTransportDef(),
    )

#: IDL error metadata for program `beta`.
BETA_ERRORS: Tuple[ErrorMetadata, ...] = ()

# Typed params for `configure`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
BetaConfigureParams = TypedDict(
    "BetaConfigureParams",
    {
        # arg `header` (`Header`)
        "header": Any,
        # Optional address override for the `authority` signer (defaults to the payer).
        "authority": str,
        # Address of the `vault` account.
        "vault": str,
    },
    total=False,
)


def beta_configure(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Builds the `configure` instruction.

    Pure (no network). Params use IDL wire names plus documented account aliases (see `BetaConfigureParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (signer fallback address),
    `accounts` (unvalidated overrides), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return beta_configure_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def beta_configure_handler() -> InstructionHandler:
    """Raw instruction handler for `configure` (escape hatch)."""
    return InstructionHandler(
        program_id=BETA_PROGRAM_ID,
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
            ArgSchema(name="header", type={"struct": [{"name": "version", "type": "u8"}, {"name": "owner", "type": "pubkey"}, {"name": "flags", "type": "u16"}]}),
        ],
        errors=[],
    )

#: Portable program SDK definition consumed by `arete.stack`.
BETA_PROGRAM = ProgramDef(
    name="beta",
    program_id=BETA_PROGRAM_ID,
    raw_instructions={
        "configure": beta_configure_handler(),
    },
    pdas={},
    accounts={},
    errors=BETA_ERRORS,
    program_spec_hash=BETA_PROGRAM_SPEC_HASH,
)


PROGRAMS: Dict[str, ProgramDef] = {
    "alpha": ALPHA_PROGRAM,
    "beta": BETA_PROGRAM,
}

PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {
    "alpha": alpha_read_descriptor(),
    "beta": beta_read_descriptor(),
}
