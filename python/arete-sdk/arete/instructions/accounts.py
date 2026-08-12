"""Instruction account resolution.

Python port of ``instructions/account-resolver.ts``. Resolution order:

1. Non-PDA accounts (signer, known, user-provided) resolve first.
2. PDA accounts resolve in dependency order (accounts they reference via
   ``AccountRefSeed`` come first).

Output preserves the original account order. Omitted optional accounts that
precede a resolved account get the program id as a placeholder (Anchor's
convention); trailing omitted optionals are dropped.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Mapping, Optional, Union

from ._curve import decode_base58
from .errors import InstructionError
from .pda import (
    AccountRefSeed,
    ArgRefSeed,
    BytesSeed,
    LiteralSeed,
    PdaConfig,
    find_program_address,
    get_value_by_path,
    serialize_seed_value,
)


@dataclass(frozen=True)
class Signer:
    """Must sign; resolved from an override or the fallback payer."""


@dataclass(frozen=True)
class Known:
    """Fixed, well-known address (e.g. the System Program)."""

    address: str


@dataclass(frozen=True)
class Pda:
    """Derived from seeds via :class:`PdaConfig`."""

    config: PdaConfig


@dataclass(frozen=True)
class UserProvided:
    """Must be supplied by the caller."""


AccountResolution = Union[Signer, Known, Pda, UserProvided]


@dataclass(frozen=True)
class AccountMeta:
    """Metadata for a single account in an instruction."""

    name: str
    is_signer: bool
    is_writable: bool
    resolution: AccountResolution
    is_optional: bool = False


@dataclass(frozen=True)
class ResolvedAccount:
    """A single account with its resolved base58 address."""

    name: str
    address: str
    is_signer: bool
    is_writable: bool


@dataclass
class AccountResolutionResult:
    """Result of resolving an instruction's accounts."""

    accounts: List[ResolvedAccount] = field(default_factory=list)
    missing: List[str] = field(default_factory=list)


def _sort_by_dependency(metas: List[AccountMeta]) -> List[AccountMeta]:
    """Topologically sorts accounts so ``AccountRefSeed`` dependencies resolve
    first: non-PDA accounts, then PDAs in dependency order."""
    non_pda = [meta for meta in metas if not isinstance(meta.resolution, Pda)]
    pdas = [meta for meta in metas if isinstance(meta.resolution, Pda)]
    pda_by_name = {meta.name: meta for meta in pdas}

    sorted_pdas: List[AccountMeta] = []
    visited: set = set()
    visiting: set = set()

    def visit(name: str) -> None:
        if name in visited:
            return
        if name in visiting:
            raise InstructionError("Circular dependency in PDA accounts: " + name)
        meta = pda_by_name.get(name)
        if meta is None:
            return  # Not a PDA; resolved in the non-PDA pass.
        visiting.add(name)
        assert isinstance(meta.resolution, Pda)
        for seed in meta.resolution.config.seeds:
            if isinstance(seed, AccountRefSeed) and seed.account in pda_by_name:
                visit(seed.account)
        visiting.discard(name)
        visited.add(name)
        sorted_pdas.append(meta)

    for meta in pdas:
        visit(meta.name)

    return non_pda + sorted_pdas


def resolve_accounts(
    metas: List[AccountMeta],
    args: Mapping[str, Any],
    *,
    overrides: Optional[Mapping[str, str]] = None,
    resolve: Optional[Mapping[str, Any]] = None,
    payer: Optional[str] = None,
    program_id: Optional[str] = None,
) -> AccountResolutionResult:
    """Resolves instruction accounts against args, overrides, and a payer.

    ``overrides`` are explicit account-address overrides (including signer
    slots, which win over the ``payer`` fallback); ``resolve`` carries
    helper-only PDA seed inputs that are not serialized on-chain;
    ``program_id`` is the fallback program for PDA derivation and the
    placeholder for omitted non-trailing optional accounts.
    """
    overrides = overrides or {}
    resolved: Dict[str, ResolvedAccount] = {}
    missing: List[str] = []

    for meta in _sort_by_dependency(metas):
        account = _resolve_single(
            meta, args, overrides, resolve, payer, program_id, resolved
        )
        if account is not None:
            resolved[meta.name] = account
        elif not meta.is_optional:
            missing.append(meta.name)

    # Return accounts in original order. Omitted optional accounts that
    # precede a resolved account cannot simply be dropped — that would shift
    # every later account into the wrong slot — so they get the program id as
    # a placeholder; trailing omitted optionals are dropped as usual.
    last_resolved = -1
    for index, meta in enumerate(metas):
        if meta.name in resolved:
            last_resolved = index

    accounts: List[ResolvedAccount] = []
    for index, meta in enumerate(metas):
        account = resolved.get(meta.name)
        if account is not None:
            accounts.append(account)
        elif meta.is_optional and index < last_resolved:
            if not program_id:
                raise InstructionError(
                    f'Omitted optional account "{meta.name}" precedes other '
                    "accounts and needs the program ID as a placeholder, but no "
                    "program ID was provided."
                )
            accounts.append(
                ResolvedAccount(
                    name=meta.name,
                    address=program_id,
                    is_signer=False,
                    is_writable=False,
                )
            )

    return AccountResolutionResult(accounts=accounts, missing=missing)


def _resolve_single(
    meta: AccountMeta,
    args: Mapping[str, Any],
    overrides: Mapping[str, str],
    resolve: Optional[Mapping[str, Any]],
    payer: Optional[str],
    program_id: Optional[str],
    resolved: Mapping[str, ResolvedAccount],
) -> Optional[ResolvedAccount]:
    resolution = meta.resolution
    if isinstance(resolution, Signer):
        address = overrides.get(meta.name) or payer
        if not address:
            return None
        return ResolvedAccount(
            name=meta.name,
            address=address,
            is_signer=True,
            is_writable=meta.is_writable,
        )
    if isinstance(resolution, Known):
        return ResolvedAccount(
            name=meta.name,
            address=resolution.address,
            is_signer=meta.is_signer,
            is_writable=meta.is_writable,
        )
    if isinstance(resolution, UserProvided):
        address = overrides.get(meta.name)
        if not address:
            return None
        return ResolvedAccount(
            name=meta.name,
            address=address,
            is_signer=meta.is_signer,
            is_writable=meta.is_writable,
        )
    if isinstance(resolution, Pda):
        return _resolve_pda(meta, resolution.config, args, resolve, resolved, program_id)
    raise InstructionError(f"Unknown account resolution: {resolution!r}")


def _resolve_pda(
    meta: AccountMeta,
    config: PdaConfig,
    args: Mapping[str, Any],
    resolve: Optional[Mapping[str, Any]],
    resolved: Mapping[str, ResolvedAccount],
    fallback_program_id: Optional[str],
) -> ResolvedAccount:
    pda_program_id = config.program_id or fallback_program_id
    if not pda_program_id:
        raise InstructionError(
            f'Cannot derive PDA for "{meta.name}": no programId specified. '
            "Either set PdaConfig.program_id or pass program_id."
        )

    seeds = []
    for seed in config.seeds:
        if isinstance(seed, LiteralSeed):
            seeds.append(seed.value.encode("utf-8"))
        elif isinstance(seed, BytesSeed):
            seeds.append(bytes(seed.value))
        elif isinstance(seed, ArgRefSeed):
            value = get_value_by_path(args, seed.arg)
            if value is None:
                value = get_value_by_path(resolve, seed.arg)
            if value is None:
                raise InstructionError(
                    f"PDA seed references missing argument: {seed.arg} "
                    f'(for account "{meta.name}")'
                )
            seeds.append(serialize_seed_value(value, seed.arg_type))
        elif isinstance(seed, AccountRefSeed):
            referenced = resolved.get(seed.account)
            if referenced is None:
                raise InstructionError(
                    f"PDA seed references unresolved account: {seed.account} "
                    f'(for account "{meta.name}")'
                )
            # Account addresses are 32 bytes.
            seeds.append(decode_base58(referenced.address))
        else:
            raise InstructionError(f"Unknown seed type: {seed!r}")

    address, _bump = find_program_address(seeds, pda_program_id)
    return ResolvedAccount(
        name=meta.name,
        address=address,
        is_signer=meta.is_signer,
        is_writable=meta.is_writable,
    )


def validate_account_resolution(result: AccountResolutionResult) -> None:
    """Raises if any required accounts are missing."""
    if result.missing:
        raise InstructionError(
            "Missing required accounts: " + ", ".join(result.missing)
        )
