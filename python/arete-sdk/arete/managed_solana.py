"""Managed Solana v1 contracts. Indexed discovery does not assert chain commitment."""
from __future__ import annotations
from dataclasses import dataclass
from datetime import datetime
import re
from typing import Any, Dict, Generic, Optional, TypeVar

MANAGED_SOLANA_CONTRACT_VERSION = "managed-solana/v1"
MAX_MANAGED_BATCH_ADDRESSES = 100
MAX_MANAGED_PAGE_SIZE = 100
T = TypeVar("T")

def decimal_u64(value: Any, field: str) -> int:
    if not isinstance(value, str) or not value or not value.isascii() or not value.isdigit():
        raise ValueError(f"{field} must be a decimal u64 string")
    parsed = int(value)
    if parsed > 18446744073709551615:
        raise ValueError(f"{field} exceeds u64")
    return parsed

def validate_address(value: str) -> None:
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    if not isinstance(value, str) or not value or len(value) > 44:
        raise ValueError("address must be a base58-encoded 32-byte value")
    integer = 0
    for char in value:
        if char not in alphabet:
            raise ValueError("address must be a base58-encoded 32-byte value")
        integer = integer * 58 + alphabet.index(char)
    size = (integer.bit_length() + 7) // 8 + len(value) - len(value.lstrip("1"))
    if size != 32:
        raise ValueError("address must be a base58-encoded 32-byte value")

def validate_page(limit: int = 100, cursor: Optional[str] = None) -> None:
    if isinstance(limit, bool) or not isinstance(limit, int) or not 1 <= limit <= 100:
        raise ValueError("limit must be between 1 and 100")
    if cursor is not None and (not isinstance(cursor, str) or not 1 <= len(cursor.encode()) <= 2048):
        raise ValueError("cursor must contain 1..2048 bytes")

@dataclass(frozen=True)
class ReadOptions:
    commitment: Optional[str] = None
    min_context_slot: Optional[int] = None

    def to_wire(self) -> Dict[str, Any]:
        value: Dict[str, Any] = {}
        if self.commitment is not None:
            if self.commitment not in ("processed", "confirmed", "finalized"):
                raise ValueError("Invalid commitment")
            value["commitment"] = self.commitment
        if self.min_context_slot is not None:
            if isinstance(self.min_context_slot, bool) or not isinstance(self.min_context_slot, int) or not 0 <= self.min_context_slot <= 18446744073709551615:
                raise ValueError("minContextSlot must fit in u64")
            value["minContextSlot"] = str(self.min_context_slot)
        return value

@dataclass(frozen=True)
class ReadContext:
    slot: int

@dataclass(frozen=True)
class Contextual(Generic[T]):
    context: Optional[ReadContext]
    value: T

def parse_context(value: Any, options: ReadOptions, required: bool = True) -> Optional[ReadContext]:
    if value is None and not required:
        return None
    if not isinstance(value, dict):
        raise ValueError("Missing actual read context")
    slot = decimal_u64(value.get("slot"), "context.slot")
    if options.min_context_slot is not None and slot < options.min_context_slot:
        raise ValueError("Read context is below minContextSlot")
    return ReadContext(slot)

@dataclass(frozen=True)
class DiscoveryProvenance:
    source: str
    observed_at: str
    watermark: Optional[int] = None

def parse_discovery(value: Any) -> DiscoveryProvenance:
    if not isinstance(value, dict) or not isinstance(value.get("source"), str) or not value["source"] or not isinstance(value.get("observedAt"), str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", value["observedAt"], re.IGNORECASE):
        raise ValueError("Invalid discovery provenance")
    datetime.fromisoformat(value["observedAt"].upper().replace("Z", "+00:00"))
    return DiscoveryProvenance(value["source"], value["observedAt"], None if value.get("watermark") is None else decimal_u64(value["watermark"], "watermark"))

@dataclass(frozen=True)
class OwnerTokenAccountsRequest:
    owner: str
    mint: Optional[str] = None
    token_program: Optional[str] = None
    limit: int = 100
    cursor: Optional[str] = None

    def to_wire(self) -> Dict[str, Any]:
        validate_address(self.owner)
        for address in (self.mint, self.token_program):
            if address is not None:
                validate_address(address)
        validate_page(self.limit, self.cursor)
        return {key: value for key, value in {"owner": self.owner, "mint": self.mint, "tokenProgram": self.token_program, "limit": self.limit, "cursor": self.cursor}.items() if value is not None}

@dataclass(frozen=True)
class OwnerTokenAccount:
    address: str
    mint: str
    token_program: str
    owner: str
    amount: int
    decimals: int
    state: str
    delegate: Optional[str] = None
    delegated_amount: Optional[int] = None
    close_authority: Optional[str] = None

@dataclass(frozen=True)
class OwnerTokenAccountsPage:
    items: tuple[OwnerTokenAccount, ...]
    next_cursor: Optional[str]
    discovery: DiscoveryProvenance

@dataclass(frozen=True)
class NativePositionQuery:
    owner: Optional[str] = None
    pool: Optional[str] = None
    limit: int = 100
    cursor: Optional[str] = None

    def to_wire(self) -> Dict[str, Any]:
        if self.owner is None and self.pool is None:
            raise ValueError("owner or pool is required")
        for address in (self.owner, self.pool):
            if address is not None:
                validate_address(address)
        validate_page(self.limit, self.cursor)
        return {key: value for key, value in {"owner": self.owner, "pool": self.pool, "limit": self.limit, "cursor": self.cursor}.items() if value is not None}

@dataclass(frozen=True)
class NativePositionPage:
    addresses: tuple[str, ...]
    next_cursor: Optional[str]
    discovery: DiscoveryProvenance

@dataclass(frozen=True)
class AccountTombstone:
    """Authoritative chain deletion; ingestion owns recognition and entity mapping."""
    address: str
    slot: int
    write_version: int

    @classmethod
    def from_json(cls, value: Dict[str, Any]) -> "AccountTombstone":
        validate_address(value["address"])
        if set(value) != {"address", "slot", "writeVersion"}:
            raise ValueError("Invalid tombstone fields")
        return cls(value["address"], decimal_u64(value["slot"], "slot"), decimal_u64(value["writeVersion"], "writeVersion"))
