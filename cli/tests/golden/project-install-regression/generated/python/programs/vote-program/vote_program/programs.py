"""Generated program SDKs for the `vote` stack. Do not edit.

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
import json

from arete.instructions import (
    AccountMeta,
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
    program_read_descriptor_from_wire,
)
from arete.read import ProgramAccountReadDef
from arete.stack import ProgramDef
from arete.gateway import HostedSolanaGatewayBindings

from . import models

__all__ = [
    "VoteProgramPingParams",
    "vote_program_ping",
    "vote_program_ping_handler",
    "VOTE_PROGRAM_PROGRAM_ID",
    "VOTE_PROGRAM_PROGRAM_SPEC_HASH",
    "VOTE_PROGRAM_PROGRAM_RELEASE_HASH",
    "vote_program_read_descriptor",
    "VOTE_PROGRAM_PACKAGE_RELEASE_HASH",
    "VOTE_PROGRAM_ERRORS",
    "VOTE_PROGRAM_PROGRAM",
    "PROGRAMS",
    "PROGRAM_READS",
]


# ==========================================================================
# Program `vote_program` (program ID `Vote111111111111111111111111111111111111111`)
# ==========================================================================

VOTE_PROGRAM_PROGRAM_ID = "Vote111111111111111111111111111111111111111"

#: Content hash of the exact program specification captured at generation time.
VOTE_PROGRAM_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86"

#: Release identity addressing hosted account reads for this program.
VOTE_PROGRAM_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"


def vote_program_read_descriptor() -> ProgramReadDescriptor:
    """Exact release-addressed read descriptor for program `vote_program`."""
    return program_read_descriptor_from_wire(json.loads("{\"release\":{\"programReleaseHash\":\"arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"programSpecHash\":\"arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86\"},\"transport\":{\"binding\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"mode\":\"signed_session\",\"required\":true,\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"prb_00000000000000000000000000000077\",\"targetKind\":\"program-read-binding\"},\"endpoint\":\"https://reads.example.test/private/\",\"programReadBindingId\":\"prb_00000000000000000000000000000077\"},\"kind\":\"hosted-binding\"}}"))

#: Program package release this program SDK was generated from.
VOTE_PROGRAM_PACKAGE_RELEASE_HASH = "arete:registry-package-release:v2:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"

#: IDL error metadata for program `vote_program`.
VOTE_PROGRAM_ERRORS: Tuple[ErrorMetadata, ...] = ()

# Typed params for `ping`: instruction args plus overridable accounts
# (wire-name keys; required/optional noted per key).
VoteProgramPingParams = TypedDict(
    "VoteProgramPingParams",
    {
        # Address of the `payer` signer.
        "payer": str,
    },
    total=False,
)


def vote_program_ping(
    *,
    wallet: Optional[str] = None,
    accounts: Optional[Mapping[str, str]] = None,
    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    **params: Any,
) -> BuiltInstruction:
    """Builds the `ping` instruction.

    Pure (no network). Params use IDL wire names plus documented account aliases (see `VoteProgramPingParams`);
    unknown params fail closed.

    Reserved keyword-only options: `wallet` (the address of `signer_kind="wallet"`
    signers; the generated signers are caller-provided, as in TypeScript), `accounts`
    (addresses that override the params), `remaining_accounts`. Account names
    (including `payer`) stay available as params.
    """
    return vote_program_ping_handler().build(
        dict(params),
        payer=wallet,
        accounts=accounts,
        remaining_accounts=remaining_accounts,
    )


def vote_program_ping_handler() -> InstructionHandler:
    """Raw instruction handler for `ping` (escape hatch)."""
    return InstructionHandler(
        program_id=VOTE_PROGRAM_PROGRAM_ID,
        discriminator=bytes([1, 2, 3, 4, 5, 6, 7, 8]),
        accounts=[
            AccountMeta(
                name="payer",
                is_signer=True,
                is_writable=True,
                resolution=Signer(),
                is_optional=False,
                signer_kind="provided",
            ),
        ],
        args=[],
        errors=[],
    )

#: Portable program SDK definition consumed by `arete.stack`.
VOTE_PROGRAM_PROGRAM = ProgramDef(
    name="vote_program",
    program_id=VOTE_PROGRAM_PROGRAM_ID,
    raw_instructions={
        "ping": vote_program_ping_handler(),
    },
    pdas={},
    accounts={},
    errors=VOTE_PROGRAM_ERRORS,
    program_spec_hash=VOTE_PROGRAM_PROGRAM_SPEC_HASH,
    package_release_hash=VOTE_PROGRAM_PACKAGE_RELEASE_HASH,
    gateway=HostedSolanaGatewayBindings.from_dict(json.loads("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"anonymous\",\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}")),
)


PROGRAMS: Dict[str, ProgramDef] = {
    "vote_program": VOTE_PROGRAM_PROGRAM,
}

PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {
    "vote_program": vote_program_read_descriptor(),
}
