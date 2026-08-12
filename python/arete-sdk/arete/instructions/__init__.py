"""Instruction building: Borsh argument serialization, PDA derivation, and
account resolution.

Python port of the TypeScript instruction layer
(``typescript/core/src/instructions/``) and its Rust sibling
(``rust/arete-a4-sdk/src/instruction/``), with byte-for-byte identical wire
output: little-endian integers, u32 length prefixes, single-byte option and
enum prefixes, UTF-8-sorted map keys, and Anchor's placeholder convention for
omitted optional accounts.

Generated stack code produces :class:`InstructionHandler` values; callers
invoke :meth:`InstructionHandler.build` with a merged params object (args plus
account-address overrides) and receive a :class:`BuiltInstruction` ready for
transaction assembly. Building is pure — no network access.
"""

from __future__ import annotations

from ._curve import decode_base58, encode_base58, is_on_curve
from .accounts import (
    AccountMeta,
    AccountResolution,
    AccountResolutionResult,
    Known,
    Pda,
    ResolvedAccount,
    Signer,
    UserProvided,
    resolve_accounts,
    validate_account_resolution,
)
from .args import ArgSchema, ArgType, serialize_args
from .errors import (
    ErrorMetadata,
    InstructionError,
    format_program_error,
    lookup_program_error,
    parse_program_error,
)
from .handler import BuiltAccountMeta, BuiltInstruction, InstructionHandler
from .pda import (
    AccountRefSeed,
    ArgRefSeed,
    BytesSeed,
    LiteralSeed,
    PdaConfig,
    PdaSeed,
    derive_pda,
    find_program_address,
    normalize_seed_type,
    serialize_seed_value,
)

__all__ = [
    "AccountMeta",
    "AccountRefSeed",
    "AccountResolution",
    "AccountResolutionResult",
    "ArgRefSeed",
    "ArgSchema",
    "ArgType",
    "BuiltAccountMeta",
    "BuiltInstruction",
    "BytesSeed",
    "ErrorMetadata",
    "InstructionError",
    "InstructionHandler",
    "Known",
    "LiteralSeed",
    "Pda",
    "PdaConfig",
    "PdaSeed",
    "ResolvedAccount",
    "Signer",
    "UserProvided",
    "decode_base58",
    "derive_pda",
    "encode_base58",
    "find_program_address",
    "format_program_error",
    "is_on_curve",
    "lookup_program_error",
    "normalize_seed_type",
    "parse_program_error",
    "resolve_accounts",
    "serialize_args",
    "serialize_seed_value",
    "validate_account_resolution",
]
