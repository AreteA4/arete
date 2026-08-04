"""Token amount helpers (port of ``typescript/core/src/amounts.ts``).

An :data:`AmountInput` is either raw base units (a bare ``int``, or
``{"raw": int | str}``) or UI units (``{"ui": str | int | float}``). UI
parsing is exact string math pinned to the mint's decimals — no float
precision loss. Decimals are fetched from the chain read endpoint only when
they are unknown and actually needed.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any, Dict, Mapping, Optional, Union

from arete.chain import ChainClient

AmountInput = Union[int, Mapping[str, Any]]

_UI_AMOUNT_RE = re.compile(r"^\d+(?:\.\d+)?$")


@dataclass(frozen=True)
class ResolvedAmount:
    raw: int
    decimals: int


def parse_ui_amount_to_raw(value: Union[str, int, float], decimals: int) -> int:
    """Convert a UI amount ("1.5") to raw base units using string math."""
    trimmed = str(value).strip()
    if not _UI_AMOUNT_RE.match(trimmed):
        raise ValueError(f"Invalid UI amount: {value}")

    whole_part, _, fraction_part = trimmed.partition(".")
    if len(fraction_part) > decimals:
        excess = fraction_part[decimals:]
        if any(ch != "0" for ch in excess):
            raise ValueError(
                f"UI amount {value} has more fractional digits than the mint's "
                f"{decimals} decimals"
            )
    fraction = fraction_part.ljust(decimals, "0")[:decimals]
    whole = int(whole_part or "0") * 10**decimals
    return whole + int(fraction or "0")


def format_raw_to_ui(raw: Union[int, str], decimals: int) -> str:
    """Format raw base units as a UI decimal string (inverse of
    :func:`parse_ui_amount_to_raw`)."""
    value = _to_int(raw, "raw amount")
    negative = value < 0
    magnitude = -value if negative else value
    scale = 10**decimals
    whole, fraction = divmod(magnitude, scale)
    sign = "-" if negative else ""
    if decimals == 0 or fraction == 0:
        return f"{sign}{whole}"
    fraction_text = str(fraction).rjust(decimals, "0").rstrip("0")
    return f"{sign}{whole}.{fraction_text}"


def _to_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"Invalid {name}: {value!r}")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        text = value.strip()
        if re.match(r"^-?\d+$", text):
            return int(text)
    raise ValueError(f"Invalid {name}: {value!r}")


def _is_raw_input(amount: AmountInput) -> bool:
    return isinstance(amount, int) or (
        isinstance(amount, Mapping) and "raw" in amount
    )


def to_raw_amount(amount: AmountInput, decimals: int) -> int:
    """Resolve an :data:`AmountInput` to raw base units with known decimals."""
    if isinstance(amount, bool):
        raise ValueError(f"Invalid amount input: {amount!r}")
    if isinstance(amount, int):
        return amount
    if isinstance(amount, Mapping):
        if "raw" in amount:
            return _to_int(amount["raw"], "raw amount")
        if "ui" in amount:
            return parse_ui_amount_to_raw(amount["ui"], decimals)
    raise ValueError(f"Invalid amount input: {amount!r}")


async def get_mint_decimals(chain: ChainClient, mint: str) -> int:
    """Fetch a mint's decimals via the chain read endpoint, raising when
    unavailable."""
    account = await chain.mint(mint)
    if account is None or account.decimals is None:
        raise ValueError(
            f"Mint {mint} is missing decimals on the configured read endpoint."
        )
    return account.decimals


async def resolve_amount(
    chain: ChainClient,
    *,
    mint: str,
    amount: AmountInput,
    decimals: Optional[int] = None,
) -> ResolvedAmount:
    """Resolve an :data:`AmountInput` to raw base units, fetching the mint's
    decimals only when they are unknown (a bare int or ``{"raw"}`` input with
    explicit ``decimals`` never touches the network)."""
    if _is_raw_input(amount):
        raw = amount if isinstance(amount, int) else _to_int(amount["raw"], "raw amount")
        resolved = decimals if decimals is not None else await get_mint_decimals(chain, mint)
        return ResolvedAmount(raw=raw, decimals=resolved)

    resolved = decimals if decimals is not None else await get_mint_decimals(chain, mint)
    return ResolvedAmount(raw=to_raw_amount(amount, resolved), decimals=resolved)


async def resolve_amount_to_raw(
    chain: ChainClient,
    *,
    mint: str,
    amount: AmountInput,
    decimals: Optional[int] = None,
) -> int:
    """Resolve an :data:`AmountInput` to raw base units without forcing a
    decimals fetch when the input is already expressed in raw units."""
    if isinstance(amount, int) and not isinstance(amount, bool):
        return amount
    if isinstance(amount, Mapping) and "raw" in amount:
        return _to_int(amount["raw"], "raw amount")

    resolved = decimals if decimals is not None else await get_mint_decimals(chain, mint)
    return to_raw_amount(amount, resolved)


async def resolve_amounts_to_raw(
    chain: ChainClient,
    inputs: Mapping[str, Mapping[str, Any]],
) -> Dict[str, int]:
    """Resolve a named set of amount inputs (each ``{"mint", "amount"[,
    "decimals"]}``) to raw base units, preserving keys."""
    resolved: Dict[str, int] = {}
    for name, entry in inputs.items():
        resolved[name] = await resolve_amount_to_raw(
            chain,
            mint=entry["mint"],
            amount=entry["amount"],
            decimals=entry.get("decimals"),
        )
    return resolved
