"""Instruction error types and IDL error metadata lookup."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Sequence

from arete.errors import AreteError


class InstructionError(AreteError):
    """Raised when instruction building, PDA derivation, or account
    resolution fails."""


@dataclass(frozen=True)
class ErrorMetadata:
    """Program error definition from the IDL."""

    code: int
    name: str
    msg: str


def lookup_program_error(
    code: int, errors: Sequence[ErrorMetadata]
) -> Optional[ErrorMetadata]:
    """Finds the IDL error definition for ``code``, or ``None``."""
    for error in errors:
        if error.code == code:
            return error
    return None


def parse_program_error(code: int, errors: Sequence[ErrorMetadata]) -> ErrorMetadata:
    """Resolves ``code`` against IDL error definitions.

    Unknown codes fall back to a synthetic entry, mirroring the TypeScript
    error parser (``CustomError<code>`` / ``Unknown error with code <code>``).
    """
    found = lookup_program_error(code, errors)
    if found is not None:
        return found
    return ErrorMetadata(
        code=code,
        name=f"CustomError{code}",
        msg=f"Unknown error with code {code}",
    )


def format_program_error(error: ErrorMetadata) -> str:
    """Human-readable rendering: ``Name (code): message``."""
    return f"{error.name} ({error.code}): {error.msg}"
