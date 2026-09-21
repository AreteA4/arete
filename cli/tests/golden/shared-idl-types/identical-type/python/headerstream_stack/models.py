"""Generated entity models for the `HeaderStream` stack. Do not edit.

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
    "Header",
    "header_from_wire",
    "header_patch_from_wire",
    "AlphaVaultId",
    "alpha_vault_id_from_wire",
    "alpha_vault_id_patch_from_wire",
    "AlphaVaultState",
    "alpha_vault_state_from_wire",
    "alpha_vault_state_patch_from_wire",
    "AlphaVault",
    "alpha_vault_from_wire",
    "alpha_vault_patch_from_wire",
    "BetaVaultId",
    "beta_vault_id_from_wire",
    "beta_vault_id_patch_from_wire",
    "BetaVaultState",
    "beta_vault_state_from_wire",
    "beta_vault_state_patch_from_wire",
    "BetaVault",
    "beta_vault_from_wire",
    "beta_vault_patch_from_wire",
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
    event_index: Optional[int] = None
    ix_path: Optional[str] = None


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
        event_index=_to_int(data.get("event_index")),
        ix_path=data.get("ix_path"),
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
class Header:
    """Resolved type `Header`."""

    version: Optional[int] = None
    owner: Optional[str] = None


def header_from_wire(value: Any) -> Header:
    """Converts a wire payload into :class:`Header`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Header")
    return Header(
        version=_to_int(_require(data, "version", "Header")),
        owner=_require(data, "owner", "Header"),
    )


def header_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Header` patch; only present keys appear."""
    data = _mapping(value, "Header patch")
    out: Dict[str, Any] = {}
    if "version" in data:
        out["version"] = _to_int(data["version"])
    if "owner" in data:
        out["owner"] = data["owner"]
    return out


@dataclass
class AlphaVaultId:
    """`id` section of `AlphaVault`."""

    address: Optional[str] = None


def alpha_vault_id_from_wire(value: Any) -> AlphaVaultId:
    """Converts a wire payload into :class:`AlphaVaultId`."""
    data = _mapping(value, "AlphaVaultId")
    return AlphaVaultId(
        address=data.get("address"),
    )


def alpha_vault_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `AlphaVaultId` patch; only present keys appear."""
    data = _mapping(value, "AlphaVaultId patch")
    out: Dict[str, Any] = {}
    if "address" in data:
        out["address"] = data["address"]
    return out


@dataclass
class AlphaVaultState:
    """`state` section of `AlphaVault`."""

    header: Optional[Header] = None


def alpha_vault_state_from_wire(value: Any) -> AlphaVaultState:
    """Converts a wire payload into :class:`AlphaVaultState`."""
    data = _mapping(value, "AlphaVaultState")
    return AlphaVaultState(
        header=_convert(data.get("header"), header_from_wire),
    )


def alpha_vault_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `AlphaVaultState` patch; only present keys appear."""
    data = _mapping(value, "AlphaVaultState patch")
    out: Dict[str, Any] = {}
    if "header" in data:
        out["header"] = _convert(data["header"], header_from_wire)
    return out


@dataclass
class AlphaVault:
    """Entity `AlphaVault`."""

    id: AlphaVaultId = field(default_factory=AlphaVaultId)
    state: AlphaVaultState = field(default_factory=AlphaVaultState)


def alpha_vault_from_wire(value: Any) -> AlphaVault:
    """Converts a merged wire entity into :class:`AlphaVault`."""
    data = _mapping(value, "AlphaVault")
    return AlphaVault(
        id=alpha_vault_id_from_wire(data.get("id") or {}),
        state=alpha_vault_state_from_wire(data.get("state") or {}),
    )


def alpha_vault_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `AlphaVault` patch; only present keys appear."""
    data = _mapping(value, "AlphaVault patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = alpha_vault_id_patch_from_wire(data["id"] or {})
    if "state" in data:
        out["state"] = alpha_vault_state_patch_from_wire(data["state"] or {})
    return out


@dataclass
class BetaVaultId:
    """`id` section of `BetaVault`."""

    address: Optional[str] = None


def beta_vault_id_from_wire(value: Any) -> BetaVaultId:
    """Converts a wire payload into :class:`BetaVaultId`."""
    data = _mapping(value, "BetaVaultId")
    return BetaVaultId(
        address=data.get("address"),
    )


def beta_vault_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `BetaVaultId` patch; only present keys appear."""
    data = _mapping(value, "BetaVaultId patch")
    out: Dict[str, Any] = {}
    if "address" in data:
        out["address"] = data["address"]
    return out


@dataclass
class BetaVaultState:
    """`state` section of `BetaVault`."""

    header: Optional[Header] = None


def beta_vault_state_from_wire(value: Any) -> BetaVaultState:
    """Converts a wire payload into :class:`BetaVaultState`."""
    data = _mapping(value, "BetaVaultState")
    return BetaVaultState(
        header=_convert(data.get("header"), header_from_wire),
    )


def beta_vault_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `BetaVaultState` patch; only present keys appear."""
    data = _mapping(value, "BetaVaultState patch")
    out: Dict[str, Any] = {}
    if "header" in data:
        out["header"] = _convert(data["header"], header_from_wire)
    return out


@dataclass
class BetaVault:
    """Entity `BetaVault`."""

    id: BetaVaultId = field(default_factory=BetaVaultId)
    state: BetaVaultState = field(default_factory=BetaVaultState)


def beta_vault_from_wire(value: Any) -> BetaVault:
    """Converts a merged wire entity into :class:`BetaVault`."""
    data = _mapping(value, "BetaVault")
    return BetaVault(
        id=beta_vault_id_from_wire(data.get("id") or {}),
        state=beta_vault_state_from_wire(data.get("state") or {}),
    )


def beta_vault_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `BetaVault` patch; only present keys appear."""
    data = _mapping(value, "BetaVault patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = beta_vault_id_patch_from_wire(data["id"] or {})
    if "state" in data:
        out["state"] = beta_vault_state_patch_from_wire(data["state"] or {})
    return out
