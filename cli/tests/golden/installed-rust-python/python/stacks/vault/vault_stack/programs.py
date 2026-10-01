"""Generated program SDKs for the `VaultStream` stack. Do not edit.

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
    program_read_descriptor_from_wire,
)
from arete.read import ProgramAccountReadDef
from arete.stack import ProgramDef
from arete.gateway import HostedSolanaGatewayBindings

from . import models

__all__ = [
    "VaultDepositParams",
    "vault_deposit",
    "vault_deposit_handler",
    "VAULT_PROGRAM_ID",
    "VAULT_PROGRAM_SPEC_HASH",
    "VAULT_PROGRAM_RELEASE_HASH",
    "vault_read_descriptor",
    "VAULT_PACKAGE_RELEASE_HASH",
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
VAULT_PROGRAM_SPEC_HASH = "arete:h1:program-spec:sha256:5469c29441aaeac7da96e8f1ac69a42de33bf7805a76c83c75da9d762213b952"

#: Release identity addressing hosted account reads for this program.
VAULT_PROGRAM_RELEASE_HASH = "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"


def vault_read_descriptor() -> ProgramReadDescriptor:
    """Exact release-addressed read descriptor for program `vault`."""
    return program_read_descriptor_from_wire(json.loads("{\"release\":{\"programReleaseHash\":\"arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"programSpecHash\":\"arete:h1:program-spec:sha256:5469c29441aaeac7da96e8f1ac69a42de33bf7805a76c83c75da9d762213b952\"},\"transport\":{\"binding\":{\"auth\":{\"mode\":\"signed_session\",\"required\":true,\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"prb_00000000000000000000000000000001\",\"targetKind\":\"program-read-binding\"},\"endpoint\":\"https://reads.example.test/vault/\",\"programReadBindingId\":\"prb_00000000000000000000000000000001\"},\"kind\":\"hosted-binding\"}}"))

#: Program package release this program SDK was generated from.
VAULT_PACKAGE_RELEASE_HASH = "arete:registry-package-release:v2:sha256:7777777777777777777777777777777777777777777777777777777777777777"

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
        # Address of the `authority` signer.
        "authority": str,
        # Address of the `vault` account.
        "vault": str,
        # Address of the `mint` account.
        "mint": str,
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

    Reserved keyword-only options: `wallet` (the address of `signer_kind="wallet"`
    signers; the generated signers are caller-provided, as in TypeScript), `accounts`
    (addresses that override the params), `remaining_accounts`. Account names
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
                signer_kind="provided",
            ),
            AccountMeta(
                name="vault",
                is_signer=False,
                is_writable=True,
                resolution=UserProvided(),
                is_optional=False,
            ),
            AccountMeta(
                name="mint",
                is_signer=False,
                is_writable=False,
                resolution=UserProvided(),
                is_optional=False,
            ),
        ],
        args=[
            ArgSchema(name="amount", type="u64"),
        ],
        errors=list(VAULT_ERRORS),
    )

_VAULT_ACCOUNTS: Dict[str, ProgramAccountReadDef] = {
    "vault": ProgramAccountReadDef(account="Vault", parser=models.vault_vault_from_wire),
}

#: Portable program SDK definition consumed by `arete.stack`.
VAULT_PROGRAM = ProgramDef(
    name="vault",
    program_id=VAULT_PROGRAM_ID,
    raw_instructions={
        "deposit": vault_deposit_handler(),
    },
    pdas={},
    accounts=dict(_VAULT_ACCOUNTS),
    errors=VAULT_ERRORS,
    program_spec_hash=VAULT_PROGRAM_SPEC_HASH,
    gateway=HostedSolanaGatewayBindings.from_dict(json.loads("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}")),
)


PROGRAMS: Dict[str, ProgramDef] = {
    "vault": VAULT_PROGRAM,
}

PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {
    "vault": vault_read_descriptor(),
}
