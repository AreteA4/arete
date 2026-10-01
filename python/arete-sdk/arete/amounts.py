"""Token amount helpers (port of ``typescript/core/src/amounts.ts``).

An :data:`AmountInput` is either raw base units (a bare ``int``, or
``{"raw": int | str}``) or UI units (``{"ui": str | int | float}``). UI
parsing is exact string math pinned to the mint's decimals — no float
precision loss. Decimals are fetched from the chain read endpoint only when
they are unknown and actually needed. :func:`decode_amount_input` checks and
normalizes an extension's amount input the way TypeScript reads one.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Dict, Mapping, Optional, Union

from arete.chain import ChainClient

#: A token amount: raw base units as a bare ``int`` or ``{"raw": …}``, or UI
#: units as ``{"ui": …}`` (TypeScript ``AmountInput``).
AmountInput = Union[int, Mapping[str, Any]]

# TypeScript's `/^\d+(?:\.\d+)?$/`: ASCII digits only.
_UI_AMOUNT_RE = re.compile(r"^\d+(?:\.\d+)?$", re.ASCII)


@dataclass(frozen=True)
class ResolvedAmount:
    raw: int
    decimals: int


def parse_ui_amount_to_raw(value: Union[str, int, float], decimals: int) -> int:
    """Convert a UI amount ("1.5") to raw base units using string math.

    A float is read as the text JavaScript's ``String(number)`` gives it
    (``1e-7``, ``10000000000000000``), as TypeScript does."""
    text = _javascript_number_text(value) if isinstance(value, float) else str(value)
    trimmed = text.strip()
    if not _UI_AMOUNT_RE.match(trimmed):
        raise ValueError(f"Invalid UI amount: {text}")

    whole_part, _, fraction_part = trimmed.partition(".")
    if len(fraction_part) > decimals:
        excess = fraction_part[decimals:]
        if any(ch != "0" for ch in excess):
            raise ValueError(
                f"UI amount {text} has more fractional digits than the mint's "
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


def _javascript_number_text(value: float) -> str:
    """JavaScript's ``String(number)``: the shortest round-trip digits, in the
    exponent form JavaScript uses below 1e-6 and from 1e21."""
    if math.isnan(value):
        return "NaN"
    if math.isinf(value):
        return "Infinity" if value > 0 else "-Infinity"
    if value == 0:
        return "0"
    sign = "-" if value < 0 else ""
    # repr() gives the shortest round-trip digits; lay them out as
    # ECMAScript's Number::toString does (value = 0.digits * 10**point).
    decimal = Decimal(repr(abs(value))).normalize().as_tuple()
    digits = "".join(str(digit) for digit in decimal.digits)
    count = len(digits)
    point = int(decimal.exponent) + count
    if count <= point <= 21:
        return sign + digits + "0" * (point - count)
    if 0 < point <= 21:
        return sign + digits[:point] + "." + digits[point:]
    if -6 < point <= 0:
        return sign + "0." + "0" * -point + digits
    exponent = point - 1
    mantissa = digits if count == 1 else digits[0] + "." + digits[1:]
    return f"{sign}{mantissa}e{'+' if exponent >= 0 else '-'}{abs(exponent)}"


def _javascript_text(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float):
        return _javascript_number_text(value)
    return str(value)


# JavaScript's StrWhiteSpaceChar (WhiteSpace and LineTerminator).
_JAVASCRIPT_WHITESPACE = (
    "\t\n\v\f\r \u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006"
    "\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
)
_BIGINT_DECIMAL_RE = re.compile(r"[+-]?[0-9]+")
_BIGINT_RADIX_RE = {
    16: re.compile(r"0[xX]([0-9a-fA-F]+)"),
    8: re.compile(r"0[oO]([0-7]+)"),
    2: re.compile(r"0[bB]([01]+)"),
}


def _bigint_from_text(text: str) -> int:
    """JavaScript's ``BigInt(string)``: surrounding whitespace ignored, the
    empty string is ``0``, decimal with an optional sign, or ``0x``/``0o``/
    ``0b`` digits."""
    trimmed = text.strip(_JAVASCRIPT_WHITESPACE)
    if not trimmed:
        return 0
    if _BIGINT_DECIMAL_RE.fullmatch(trimmed):
        return int(trimmed, 10)
    for radix, pattern in _BIGINT_RADIX_RE.items():
        match = pattern.fullmatch(trimmed)
        if match:
            return int(match.group(1), radix)
    raise ValueError(f"Cannot convert {text} to a BigInt")


def _decode_raw_amount(value: Any) -> int:
    if isinstance(value, bool) or value is None or not isinstance(value, (int, float, str)):
        raise TypeError(f"Cannot convert {_javascript_text(value)} to a BigInt")
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        if not math.isfinite(value) or not value.is_integer():
            raise ValueError(
                f"The number {_javascript_number_text(value)} cannot be converted "
                "to a BigInt because it is not an integer"
            )
        return int(value)
    return _bigint_from_text(value)


def _decode_ui_amount(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, float):
        return _javascript_number_text(value)
    if isinstance(value, int) and not isinstance(value, bool):
        return str(value)
    raise TypeError(f"Invalid UI amount: {_javascript_text(value)}")


def decode_amount_input(value: Any) -> Dict[str, Union[int, str]]:
    """Decodes an extension's amount input the way TypeScript reads an
    ``AmountInput`` (and Rust's ``AmountInput`` deserializes one), returning
    ``{"raw": int}`` or ``{"ui": str}``, which every amount helper accepts:

    - a bare ``int``: raw base units (TypeScript ``bigint``);
    - ``{"raw": …}``: an ``int``, an integral ``float``, or a string that
      JavaScript's ``BigInt(…)`` reads (decimal with an optional sign, or
      ``0x``/``0o``/``0b``; surrounding whitespace ignored; empty is ``0``);
    - ``{"ui": …}``: a string, kept as is, or a number, as the text
      JavaScript's ``String(number)`` gives (``1e-7``, ``1e+21``), so
      :func:`parse_ui_amount_to_raw` raises the TypeScript message.

    A mapping carries exactly one of ``raw`` or ``ui``. Raises ``TypeError``
    for any other shape or value type (a bare ``str``, ``float`` or ``bool``
    is not an amount), and ``ValueError`` with JavaScript's ``BigInt``
    message for a raw value it cannot convert. A negative raw amount decodes,
    as a TypeScript bigint does; Rust's unsigned ``AmountInput`` rejects it.
    """
    if isinstance(value, int) and not isinstance(value, bool):
        return {"raw": value}
    if isinstance(value, Mapping):
        keys = list(value.keys())
        if len(keys) != 1 or keys[0] not in ("raw", "ui"):
            raise TypeError(
                "An amount has exactly one of 'raw' or 'ui', got "
                f"{sorted(map(str, keys))!r}"
            )
        if keys[0] == "raw":
            return {"raw": _decode_raw_amount(value["raw"])}
        return {"ui": _decode_ui_amount(value["ui"])}
    raise TypeError(
        f"Invalid amount input: {value!r}; expected an int, "
        "{'raw': …} or {'ui': …}"
    )


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
