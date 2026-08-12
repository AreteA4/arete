"""Data-driven instruction handler.

Python port of ``instructions/executor.ts`` (the pure building half).
Generated stack code produces :class:`InstructionHandler` values; ``build``
serializes args via the schema-driven serializer and resolves accounts from a
merged params object, so no imperative per-instruction code is required.
Building is pure — no network access.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, List, Mapping, Optional, Sequence, Union

from ._curve import decode_base58
from .accounts import AccountMeta, resolve_accounts
from .args import ArgSchema, serialize_args
from .errors import ErrorMetadata, InstructionError, lookup_program_error


@dataclass(frozen=True)
class BuiltAccountMeta:
    """Account meta in a built instruction, ready for transaction assembly."""

    pubkey: str
    is_signer: bool
    is_writable: bool


@dataclass
class BuiltInstruction:
    """A fully built instruction: program, ordered accounts, serialized data."""

    program_id: str
    accounts: List[BuiltAccountMeta]
    data: bytes


def _validated_address(address: str) -> str:
    try:
        decoded = decode_base58(address)
    except ValueError:
        raise InstructionError(f"Invalid pubkey: {address}") from None
    if len(decoded) != 32:
        raise InstructionError(f"Invalid pubkey: {address}")
    return address


@dataclass
class InstructionHandler:
    """Instruction definition consumed by the builder: program id,
    discriminator, ordered account metadata, argument schema, and IDL error
    definitions."""

    program_id: str
    discriminator: Union[bytes, Sequence[int]]
    accounts: List[AccountMeta]
    args: List[ArgSchema]
    errors: List[ErrorMetadata] = field(default_factory=list)

    @property
    def arg_names(self) -> List[str]:
        return [arg.name for arg in self.args]

    @property
    def account_names(self) -> List[str]:
        return [account.name for account in self.accounts]

    def error_for_code(self, code: int) -> Optional[ErrorMetadata]:
        """Looks up an IDL error definition by code."""
        return lookup_program_error(code, self.errors)

    def build(
        self,
        params: Optional[Mapping[str, Any]] = None,
        *,
        payer: Optional[str] = None,
        accounts: Optional[Mapping[str, str]] = None,
        remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,
    ) -> BuiltInstruction:
        """Builds the instruction from a merged params object.

        Params are IDL wire shape: keys matching a declared argument name are
        serialized args; keys matching a declared account name (with a string
        value) are account-address overrides — including signer slots, which
        win over the ``payer`` fallback. A ``resolve`` key carries helper-only
        PDA seed inputs. Anything else raises — a typo'd key silently dropped
        here would otherwise change the built instruction. The ``accounts``
        option remains an unvalidated escape hatch that wins over
        param-derived overrides; ``remaining_accounts`` are appended after the
        declared accounts (Anchor's ``remainingAccounts``).
        """
        if params is None:
            params = {}
        if not isinstance(params, Mapping):
            raise InstructionError(
                f"Invalid params: expected a mapping, got {type(params).__name__}"
            )

        args, overrides, resolve = self._split_params(params)
        if accounts:
            overrides.update(accounts)

        resolution = resolve_accounts(
            self.accounts,
            args,
            overrides=overrides,
            resolve=resolve,
            payer=payer,
            program_id=self.program_id or None,
        )
        if resolution.missing:
            raise InstructionError(
                "Missing required accounts: " + ", ".join(resolution.missing)
            )

        data = serialize_args(bytes(self.discriminator), args, self.args)
        built_accounts = [
            BuiltAccountMeta(
                pubkey=_validated_address(account.address),
                is_signer=account.is_signer,
                is_writable=account.is_writable,
            )
            for account in resolution.accounts
        ]
        if remaining_accounts:
            built_accounts.extend(remaining_accounts)

        return BuiltInstruction(
            program_id=_validated_address(self.program_id),
            accounts=built_accounts,
            data=data,
        )

    def _split_params(self, params: Mapping[str, Any]):
        """Splits a merged params object into serialized args, account-address
        overrides, and the helper-only ``resolve`` mapping."""
        arg_names = set(self.arg_names)
        account_names = set(self.account_names)

        args: dict = {}
        overrides: dict = {}
        resolve: Optional[Mapping[str, Any]] = None
        for key, value in params.items():
            if key in arg_names:
                args[key] = value
            elif key == "resolve" and "resolve" not in account_names:
                if value is None:
                    continue
                if not isinstance(value, Mapping):
                    raise InstructionError(
                        'Parameter "resolve" must be an object when provided'
                    )
                resolve = value
            elif key in account_names:
                if not isinstance(value, str):
                    # Non-string values are not valid account addresses.
                    raise InstructionError(
                        f'Parameter "{key}" is not a known argument and is not '
                        "a base58 account address"
                    )
                overrides[key] = value
            else:
                raise InstructionError(
                    f'Unknown parameter "{key}". '
                    f'Expected one of args [{", ".join(self.arg_names)}] '
                    f'or accounts [{", ".join(self.account_names)}]'
                )

        return args, overrides, resolve
