"""Generated entity models for the `VaultStream` stack. Do not edit.

Wire payloads are snake_case and pass through untransformed; u64/u128 decimal
strings convert to `int`. `*_from_wire` builds full dataclasses (IDL struct
converters reject payloads missing a required key; entity converters leave
missing keys None); `*_patch_from_wire` converts only the keys present in a
patch. Fields fed by `#[capture]` mappings or event handlers arrive inside a
`CaptureWrapper` / `EventWrapper` envelope and are typed as such.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Generic, List, Mapping, Optional, TypeVar

_T = TypeVar("_T")

__all__ = [
    "EventWrapper",
    "event_wrapper_from_wire",
    "CaptureWrapper",
    "capture_wrapper_from_wire",
    "VaultId",
    "vault_id_from_wire",
    "vault_id_patch_from_wire",
    "VaultBalance",
    "vault_balance_from_wire",
    "vault_balance_patch_from_wire",
    "Vault",
    "vault_from_wire",
    "vault_patch_from_wire",
]

def _snake_key(key: str) -> str:
    out = []
    for index, ch in enumerate(key):
        if ch.isascii() and ch.isupper():
            if index != 0:
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def _mapping(value: Any, context: str) -> Dict[str, Any]:
    if not isinstance(value, Mapping):
        raise TypeError(
            f"{context} payload must be a mapping, got {type(value).__name__}"
        )
    return {_snake_key(key): item for key, item in value.items()}


def _to_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, (str, float)):
        return int(value)
    raise TypeError(f"Cannot convert {type(value).__name__} to int")


def _to_int_list(value: Any) -> Optional[List[Optional[int]]]:
    if value is None:
        return None
    return [_to_int(item) for item in value]


def _convert(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return converter(value)


def _convert_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [converter(item) for item in value]


def _convert_capture(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return capture_wrapper_from_wire(value, converter)


def _convert_capture_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [capture_wrapper_from_wire(item, converter) for item in value]


def _convert_event(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return event_wrapper_from_wire(value, converter)


def _convert_event_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [event_wrapper_from_wire(item, converter) for item in value]


def _require(data: Mapping[str, Any], key: str, context: str) -> Any:
    if key not in data:
        raise ValueError(f"{context} payload is missing required field '{key}'")
    return data[key]


@dataclass
class EventWrapper(Generic[_T]):
    """Wrapper for captured events (timestamp + data + provenance)."""

    timestamp: int = 0
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None


def event_wrapper_from_wire(value: Any, converter: Any = None) -> EventWrapper:
    """Converts a wire event wrapper into :class:`EventWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "EventWrapper")
    inner = data.get("data")
    return EventWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
    )


@dataclass
class CaptureWrapper(Generic[_T]):
    """Wrapper for captured accounts (timestamp + address + data + provenance)."""

    timestamp: int = 0
    account_address: Optional[str] = None
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None


def capture_wrapper_from_wire(value: Any, converter: Any = None) -> CaptureWrapper:
    """Converts a wire account-capture wrapper into :class:`CaptureWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "CaptureWrapper")
    inner = data.get("data")
    return CaptureWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        account_address=data.get("account_address"),
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
    )


@dataclass
class VaultId:
    """`id` section of `Vault`."""

    address: Optional[str] = None


def vault_id_from_wire(value: Any) -> VaultId:
    """Converts a wire payload into :class:`VaultId`."""
    data = _mapping(value, "VaultId")
    return VaultId(
        address=data.get("address"),
    )


def vault_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `VaultId` patch; only present keys appear."""
    data = _mapping(value, "VaultId patch")
    out: Dict[str, Any] = {}
    if "address" in data:
        out["address"] = data["address"]
    return out


@dataclass
class VaultBalance:
    """`balance` section of `Vault`."""

    amount: Optional[int] = None


def vault_balance_from_wire(value: Any) -> VaultBalance:
    """Converts a wire payload into :class:`VaultBalance`."""
    data = _mapping(value, "VaultBalance")
    return VaultBalance(
        amount=_to_int(data.get("amount")),
    )


def vault_balance_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `VaultBalance` patch; only present keys appear."""
    data = _mapping(value, "VaultBalance patch")
    out: Dict[str, Any] = {}
    if "amount" in data:
        out["amount"] = _to_int(data["amount"])
    return out


@dataclass
class Vault:
    """Entity `Vault`."""

    id: VaultId = field(default_factory=VaultId)
    balance: VaultBalance = field(default_factory=VaultBalance)


def vault_from_wire(value: Any) -> Vault:
    """Converts a merged wire entity into :class:`Vault`."""
    data = _mapping(value, "Vault")
    return Vault(
        id=vault_id_from_wire(data.get("id") or {}),
        balance=vault_balance_from_wire(data.get("balance") or {}),
    )


def vault_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Vault` patch; only present keys appear."""
    data = _mapping(value, "Vault patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = vault_id_patch_from_wire(data["id"] or {})
    if "balance" in data:
        out["balance"] = vault_balance_patch_from_wire(data["balance"] or {})
    return out
