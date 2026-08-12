"""Wallet adapter boundary for the Arete SDK.

Python projection of ``typescript/core/src/wallet/types.ts`` with the Rust
SDK's classified-failure model (``rust/arete-a4-sdk/src/wallet.rs``).

The core SDK is intentionally RPC-free: it only constructs
:class:`arete.instructions.BuiltInstruction` values. Everything
network-related (recent blockhash, message compilation, signing, sending, and
confirmation) lives behind the :class:`WalletAdapter` boundary, implemented by
adapters that wrap the Solana library of your choice (a raw keypair signer for
scripts, a remote signer, ...).

Divergences from the TypeScript surface (idiom, not semantics):

- TS wallet failures are arbitrary thrown values that the executor duck-types
  (``normalizeTransactionError`` walks ``cause`` chains looking for outcome
  shapes and 4001 rejection codes). Python adapters instead classify their own
  failures: :class:`WalletError` carries an optional structured
  :class:`TransactionFailureOutcome` which the operation executor consumes
  directly. A ``WalletError`` without an outcome is classified as
  ``not-submitted`` in the ``send`` phase.
- The outcome model (four terminal statuses ``confirmed | not-submitted |
  submitted-unknown | chain-failed``, each with the phase that produced it)
  lives here because the wallet boundary is the classifier; ``arete.operations``
  re-exports it.
- Program errors inside outcomes reuse
  :class:`arete.instructions.ErrorMetadata` (``code``/``name``/``msg``) rather
  than a separate ``ProgramError`` shape.
"""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from typing import (
    Any,
    Mapping,
    Optional,
    Protocol,
    Sequence,
    Tuple,
    runtime_checkable,
)

from arete.errors import AreteError
from arete.instructions import (
    BuiltInstruction,
    ErrorMetadata,
    format_program_error,
)
from arete.transactions import TransactionTransport

CONFIRMATION_LEVELS: Tuple[str, ...] = ("processed", "confirmed", "finalized")

FAILURE_STATUSES: Tuple[str, ...] = (
    "not-submitted",
    "submitted-unknown",
    "chain-failed",
)

FAILURE_PHASES: Tuple[str, ...] = (
    "build",
    "wallet",
    "send",
    "confirmation",
    "chain",
)

_PHASES_BY_STATUS = {
    "not-submitted": ("build", "wallet", "send"),
    "submitted-unknown": ("send", "confirmation"),
    "chain-failed": ("confirmation", "chain"),
}

_SEND_OPTION_FIELDS = ("confirmation_level", "skip_preflight", "signers")


@dataclass(frozen=True)
class SendOptions:
    """Options forwarded to the wallet adapter when sending a transaction.

    The core SDK does not interpret these; it passes them straight through to
    the adapter, which owns all RPC semantics. ``extra`` is the Python
    rendering of the TS index signature: adapter-specific passthrough options
    (priority fees, lookup tables, ...). ``signers`` are optional extra local
    signers for this send; their concrete type depends on the adapter.
    """

    confirmation_level: Optional[str] = None
    skip_preflight: Optional[bool] = None
    signers: Optional[Tuple[Any, ...]] = None
    extra: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if (
            self.confirmation_level is not None
            and self.confirmation_level not in CONFIRMATION_LEVELS
        ):
            raise ValueError(
                f"confirmation_level must be one of {CONFIRMATION_LEVELS}, "
                f"got {self.confirmation_level!r}"
            )
        if self.signers is not None and not isinstance(self.signers, tuple):
            object.__setattr__(self, "signers", tuple(self.signers))

    @classmethod
    def coerce(cls, value: Any) -> "SendOptions":
        """``None`` → defaults; :class:`SendOptions` passthrough; a mapping's
        unknown keys land in ``extra`` (adapter passthrough)."""
        if value is None:
            return cls()
        if isinstance(value, cls):
            return value
        if isinstance(value, Mapping):
            known = {name: value[name] for name in _SEND_OPTION_FIELDS if name in value}
            extra = {
                key: item
                for key, item in value.items()
                if key not in _SEND_OPTION_FIELDS and key != "extra"
            }
            nested = value.get("extra")
            if isinstance(nested, Mapping):
                extra.update(nested)
            return cls(extra=extra, **known)
        raise TypeError(
            f"send options must be a SendOptions or mapping, got {type(value).__name__}"
        )

    def merged(self, overrides: Optional["SendOptions"]) -> "SendOptions":
        """Field-wise merge where ``overrides`` wins; ``extra`` maps merge."""
        if overrides is None:
            return self
        return SendOptions(
            confirmation_level=(
                overrides.confirmation_level
                if overrides.confirmation_level is not None
                else self.confirmation_level
            ),
            skip_preflight=(
                overrides.skip_preflight
                if overrides.skip_preflight is not None
                else self.skip_preflight
            ),
            signers=overrides.signers if overrides.signers is not None else self.signers,
            extra={**dict(self.extra), **dict(overrides.extra)},
        )

    def with_signers(self, signers: Optional[Sequence[Any]]) -> "SendOptions":
        if signers is None:
            return self
        return replace(self, signers=tuple(signers))


@dataclass(frozen=True)
class SendResult:
    """Result returned by a wallet adapter after broadcasting a transaction."""

    signature: str
    slot: Optional[int] = None


@dataclass(frozen=True)
class WalletExecutionContext:
    """Execution context passed through to wallet adapters.

    The executing client passes its :class:`arete.transactions.TransactionTransport`
    on every ``sign_and_send`` so adapters can fetch blockhashes, simulate,
    send, and poll signature status through the stack relay instead of a
    direct RPC connection.
    """

    transaction_transport: Optional[TransactionTransport] = None


@dataclass(frozen=True)
class TransactionInspectionResult:
    """Unsigned transaction inspection returned by a capable wallet adapter.

    Inspection must not sign or submit the transaction.
    """

    fee_lamports: Optional[int] = None
    logs: Optional[Tuple[str, ...]] = None
    compute_units_consumed: Optional[int] = None
    context_slot: Optional[int] = None
    error: Any = None
    extra: Mapping[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Transaction outcome model (canonical §7: outcomes are data)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ConfirmedTransactionOutcome:
    """The confirmed terminal status (normally reported through receipts)."""

    signature: str
    slot: Optional[int] = None

    status: str = "confirmed"
    phase: str = "confirmation"


@dataclass(frozen=True)
class TransactionFailureOutcome:
    """One of the three failure terminal statuses with the phase that
    produced it. Outcomes are data — the executor raises
    :class:`arete.operations.OperationExecutionError` *holding* one of these.

    Prefer the :meth:`not_submitted` / :meth:`submitted_unknown` /
    :meth:`chain_failed` factories, which validate status/phase combinations
    and derive a default message.
    """

    status: str
    phase: str
    message: str = ""
    signature: Optional[str] = None
    slot: Optional[int] = None
    program_error: Optional[ErrorMetadata] = None
    cause: Any = None

    def __post_init__(self) -> None:
        if self.status not in FAILURE_STATUSES:
            raise ValueError(
                f"status must be one of {FAILURE_STATUSES}, got {self.status!r}"
            )
        allowed = _PHASES_BY_STATUS[self.status]
        if self.phase not in allowed:
            raise ValueError(
                f"phase for status '{self.status}' must be one of {allowed}, "
                f"got {self.phase!r}"
            )
        if self.status == "submitted-unknown" and not self.signature:
            raise ValueError("submitted-unknown outcomes require a signature")
        if not self.message:
            object.__setattr__(self, "message", _derive_outcome_message(self))

    @classmethod
    def not_submitted(
        cls,
        phase: str = "send",
        *,
        message: str = "",
        cause: Any = None,
    ) -> "TransactionFailureOutcome":
        return cls(status="not-submitted", phase=phase, message=message, cause=cause)

    @classmethod
    def submitted_unknown(
        cls,
        signature: str,
        *,
        phase: str = "confirmation",
        slot: Optional[int] = None,
        message: str = "",
        cause: Any = None,
    ) -> "TransactionFailureOutcome":
        return cls(
            status="submitted-unknown",
            phase=phase,
            signature=signature,
            slot=slot,
            message=message,
            cause=cause,
        )

    @classmethod
    def chain_failed(
        cls,
        *,
        phase: str = "chain",
        signature: Optional[str] = None,
        slot: Optional[int] = None,
        program_error: Optional[ErrorMetadata] = None,
        message: str = "",
        cause: Any = None,
    ) -> "TransactionFailureOutcome":
        return cls(
            status="chain-failed",
            phase=phase,
            signature=signature,
            slot=slot,
            program_error=program_error,
            message=message,
            cause=cause,
        )


def _default_outcome_message(outcome: "TransactionFailureOutcome") -> str:
    if outcome.status == "not-submitted":
        return f"Transaction was not submitted during {outcome.phase}"
    if outcome.status == "submitted-unknown":
        return (
            f"Transaction {outcome.signature} was submitted but its status is unknown"
        )
    if outcome.program_error is not None:
        return format_program_error(outcome.program_error)
    if outcome.signature:
        return f"Transaction {outcome.signature} failed on chain"
    return "Transaction failed on chain"


def _derive_outcome_message(outcome: "TransactionFailureOutcome") -> str:
    cause = outcome.cause
    if isinstance(cause, BaseException) and str(cause):
        return str(cause)
    return _default_outcome_message(outcome)


class WalletError(AreteError):
    """Failure reported by a wallet adapter.

    Adapters classify their own failures: when the adapter knows how far the
    transaction got (wallet rejection, submitted-but-unconfirmed, failed on
    chain with a program error code), it attaches a structured
    :class:`TransactionFailureOutcome`; the operation executor consumes it
    directly. Without an outcome the executor classifies the failure as
    ``not-submitted`` in the ``send`` phase.
    """

    def __init__(
        self,
        message: str = "",
        *,
        outcome: Optional[TransactionFailureOutcome] = None,
        cause: Any = None,
    ) -> None:
        if not message and outcome is not None:
            message = outcome.message
        super().__init__(message or "Wallet operation failed", "WALLET_ERROR")
        self.outcome = outcome
        self.cause = cause if cause is not None else (
            outcome.cause if outcome is not None else None
        )

    @classmethod
    def from_outcome(cls, outcome: TransactionFailureOutcome) -> "WalletError":
        return cls(outcome.message, outcome=outcome)

    def into_outcome(self, fallback_phase: str = "send") -> TransactionFailureOutcome:
        """The attached structured outcome, or a not-submitted fallback at
        ``fallback_phase`` carrying this error's message."""
        if self.outcome is not None:
            return self.outcome
        return TransactionFailureOutcome.not_submitted(
            fallback_phase, message=self.message, cause=self.cause or self
        )


@runtime_checkable
class WalletAdapter(Protocol):
    """Wallet adapter interface for signing and sending transactions.

    Implementations own blockhash fetching, message compilation (legacy or
    v0), signing, sending, and confirmation. The core SDK only needs
    ``public_key`` for signer-account resolution and ``sign_and_send`` to
    broadcast built instructions.

    Optional capabilities (checked structurally with ``getattr``):

    - ``signer_addresses`` — addresses the adapter can satisfy without
      per-send signers (defaults to ``[public_key]``).
    - ``async def inspect_transaction(instructions, options=None, context=None)
      -> TransactionInspectionResult`` — unsigned inspection; must not sign,
      submit, or prompt a wallet.

    Failures raise :class:`WalletError` (classified when possible).
    """

    public_key: str

    async def sign_and_send(
        self,
        instructions: Sequence[BuiltInstruction],
        options: Optional[SendOptions] = None,
        context: Optional[WalletExecutionContext] = None,
    ) -> SendResult:
        """Compile, sign, and broadcast built instructions as one transaction.

        Accepting a sequence (rather than a single instruction) makes batching
        and composition fall out for free.
        """
        ...


def wallet_signer_addresses(wallet: Any) -> Tuple[str, ...]:
    """Signer addresses an adapter can satisfy, including its public key."""
    if wallet is None:
        return ()
    addresses = []
    declared = getattr(wallet, "signer_addresses", None)
    if declared is not None and not callable(declared):
        addresses.extend(str(address) for address in declared)
    public_key = getattr(wallet, "public_key", None)
    if isinstance(public_key, str) and public_key:
        addresses.append(public_key)
    seen = set()
    unique = []
    for address in addresses:
        if address not in seen:
            seen.add(address)
            unique.append(address)
    return tuple(unique)
