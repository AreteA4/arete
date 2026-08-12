"""PDA derivation and typed seed serialization.

Python port of ``instructions/pda.ts``, ``instructions/seed-serializer.ts``,
and ``instructions/pda-dsl.ts``: Solana's program-derived-address algorithm
(sha256 of seeds + bump + program id + ``"ProgramDerivedAddress"``, bump 255
down to 0, rejecting on-curve candidates) plus strict, width-exact seed
encoding when a declared type is present and legacy heuristics when it is not.
"""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Sequence, Tuple, Union

from ._curve import decode_base58, encode_base58, is_on_curve
from .errors import InstructionError

MAX_SEEDS = 16
MAX_SEED_LENGTH = 32
_PDA_MARKER = b"ProgramDerivedAddress"
_INT_TYPE_RE = re.compile(r"^[ui](8|16|32|64|128)$")


@dataclass(frozen=True)
class LiteralSeed:
    """Fixed string seed, encoded as UTF-8 bytes."""

    value: str


@dataclass(frozen=True)
class BytesSeed:
    """Raw byte seed."""

    value: bytes


@dataclass(frozen=True)
class ArgRefSeed:
    """Reference to an instruction argument (dot-path into args, falling back
    to the helper-only ``resolve`` map), serialized at ``arg_type``'s width."""

    arg: str
    arg_type: Optional[str] = None


@dataclass(frozen=True)
class AccountRefSeed:
    """The 32-byte address of a previously resolved account."""

    account: str


PdaSeed = Union[LiteralSeed, BytesSeed, ArgRefSeed, AccountRefSeed]


@dataclass(frozen=True)
class PdaConfig:
    """Configuration for PDA derivation."""

    seeds: Sequence[PdaSeed]
    # Program that owns this PDA (defaults to the instruction's program id).
    program_id: Optional[str] = None


def normalize_seed_type(arg_type: Optional[str]) -> Optional[str]:
    """Normalizes IDL/codegen type spellings (``"Pubkey"``, ``"publicKey"``,
    ``"solana_pubkey::Pubkey"``, ``"String"``, ...) to a canonical seed type:
    ``"pubkey"``, ``"string"``, or ``"u8"``..``"i128"``. Returns ``None`` for
    types that cannot be a seed."""
    if not arg_type:
        return None
    # Strip any path qualifier (e.g. solana_pubkey::Pubkey).
    tail = arg_type.rsplit("::", 1)[-1].strip()
    if _INT_TYPE_RE.match(tail):
        return tail
    if tail in ("pubkey", "Pubkey", "publicKey", "PublicKey"):
        return "pubkey"
    if tail in ("string", "String", "str"):
        return "string"
    return None


def _encode_seed_int(value: int, size: int, signed: bool) -> bytes:
    """Little-endian two's-complement at a fixed byte width, with an overflow
    check so out-of-range values fail instead of silently truncating."""
    out = bytearray()
    n = value
    for _ in range(size):
        out.append(n & 0xFF)
        n >>= 8
    fits = n in (0, -1) if signed else n == 0
    if not fits:
        raise InstructionError(f"Seed value {value} does not fit in {size * 8} bits")
    return bytes(out)


def _seed_int(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value.strip(), 10)
        except ValueError:
            return None
    return None


def serialize_seed_value(value: Any, arg_type: Optional[str] = None) -> bytes:
    """Serializes one PDA seed value.

    With a recognized ``arg_type``, encoding is strict and width-exact; an
    incompatible value raises rather than deriving a wrong address. Without
    one, the legacy heuristics apply: raw bytes pass through, 43/44-character
    strings are tried as base58, other strings are UTF-8, integers are 8-byte
    little-endian.
    """
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    if isinstance(value, (list, tuple)):
        try:
            return bytes(value)
        except (ValueError, TypeError):
            raise InstructionError(
                "Byte-array seeds must contain integers in 0..=255"
            ) from None

    canonical = normalize_seed_type(arg_type)

    if canonical == "pubkey":
        if not isinstance(value, str):
            raise InstructionError(
                f"Pubkey seed requires a base58 string, got {type(value).__name__}"
            )
        try:
            decoded = decode_base58(value)
        except ValueError:
            raise InstructionError(
                f"Pubkey seed '{value}' is not valid base58"
            ) from None
        if len(decoded) != 32:
            raise InstructionError(
                f"Pubkey seed '{value}' decoded to {len(decoded)} bytes, expected 32"
            )
        return decoded

    if canonical == "string":
        if not isinstance(value, str):
            raise InstructionError(
                f"String seed requires a string value, got {type(value).__name__}"
            )
        return value.encode("utf-8")

    if canonical is not None:
        number = _seed_int(value)
        if number is None:
            raise InstructionError(
                f"Numeric seed of type {canonical} requires an integer, "
                f"got {type(value).__name__}"
            )
        bits = int(canonical[1:])
        return _encode_seed_int(number, bits // 8, canonical.startswith("i"))

    # Untyped: legacy heuristics.
    if isinstance(value, str):
        if len(value) in (43, 44):
            try:
                return decode_base58(value)
            except ValueError:
                pass
        return value.encode("utf-8")

    number = _seed_int(value)
    if number is not None and not isinstance(value, str):
        return _encode_seed_int(number, 8, True)

    raise InstructionError(
        f"Cannot serialize value for PDA seed: {type(value).__name__}"
    )


def find_program_address(seeds: Sequence[bytes], program_id: str) -> Tuple[str, int]:
    """Derives a program-derived address from serialized seeds.

    Returns ``(address, bump)`` where ``address`` is base58 and ``bump`` is the
    canonical bump seed (the highest value in 255..0 whose sha256 candidate is
    off the ed25519 curve).
    """
    if len(seeds) > MAX_SEEDS:
        raise InstructionError(f"Maximum of {MAX_SEEDS} seeds allowed")
    for index, seed in enumerate(seeds):
        if len(seed) > MAX_SEED_LENGTH:
            raise InstructionError(
                f"Seed {index} exceeds maximum length of {MAX_SEED_LENGTH} bytes"
            )

    try:
        program_id_bytes = decode_base58(program_id)
    except ValueError:
        raise InstructionError(f"Invalid pubkey: {program_id}") from None
    if len(program_id_bytes) != 32:
        raise InstructionError("Program ID must be 32 bytes")

    prefix = b"".join(bytes(seed) for seed in seeds)
    for bump in range(255, -1, -1):
        candidate = hashlib.sha256(
            prefix + bytes([bump]) + program_id_bytes + _PDA_MARKER
        ).digest()
        if not is_on_curve(candidate):
            return encode_base58(candidate), bump

    raise InstructionError("Unable to find a valid PDA")


def get_value_by_path(source: Optional[Mapping[str, Any]], path: str) -> Any:
    """Dot-path lookup: a direct key wins, otherwise ``"a.b.c"`` walks nested
    mappings. ``None`` values count as missing."""
    if source is None:
        return None
    if path in source:
        return source[path]
    current: Any = source
    for segment in path.split("."):
        if not isinstance(current, Mapping):
            return None
        current = current.get(segment)
    return current


def derive_pda(
    config: PdaConfig,
    *,
    args: Optional[Mapping[str, Any]] = None,
    accounts: Optional[Mapping[str, str]] = None,
    resolve: Optional[Mapping[str, Any]] = None,
    program_id: Optional[str] = None,
) -> Tuple[str, int]:
    """Derives a PDA from a :class:`PdaConfig` and derivation context.

    ``config.program_id`` wins over the ``program_id`` fallback. ``args`` and
    ``resolve`` feed ``ArgRefSeed`` lookups (args first); ``accounts`` feeds
    ``AccountRefSeed`` lookups. Returns ``(address, bump)``.
    """
    pda_program_id = config.program_id or program_id
    if not pda_program_id:
        raise InstructionError(
            "Cannot derive PDA: no program ID specified. Either set "
            "PdaConfig.program_id or pass program_id."
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
                raise InstructionError(f"Missing arg for PDA seed: {seed.arg}")
            seeds.append(serialize_seed_value(value, seed.arg_type))
        elif isinstance(seed, AccountRefSeed):
            address = (accounts or {}).get(seed.account)
            if not address:
                raise InstructionError(f"Missing account for PDA seed: {seed.account}")
            seeds.append(decode_base58(address))
        else:
            raise InstructionError(f"Unknown seed type: {seed!r}")

    return find_program_address(seeds, pda_program_id)
