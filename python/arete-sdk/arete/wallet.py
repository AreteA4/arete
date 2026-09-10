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

_SEND_OPTION_FIELDS = (
    "confirmation_level",
    "skip_preflight",
    "signers",
    "transaction_version",
    "resources",
)

#: Transaction versions an adapter may build (SIMD-0385 adds ``1``). Numeric
#: versions are JSON numbers, never stringified numbers.
TRANSACTION_VERSIONS: Tuple[Any, ...] = ("legacy", 0, 1)

#: Final encoded transaction size ceilings, in bytes.
MAX_TRANSACTION_BYTES = 1232
V1_MAX_TRANSACTION_BYTES = 4096

#: Transaction V1 per-transaction caps.
V1_MAX_SIGNATURES = 12
V1_MAX_ACCOUNTS = 64
V1_MAX_INSTRUCTIONS = 64

_U32_MAX = 0xFFFF_FFFF
_U64_MAX = 0xFFFF_FFFF_FFFF_FFFF

# snake_case accessor -> canonical camelCase wire key, with its integer width.
_RESOURCE_OPTION_KEYS: Tuple[Tuple[str, str, int], ...] = (
    ("compute_unit_limit", "computeUnitLimit", _U32_MAX),
    ("loaded_accounts_data_size_limit", "loadedAccountsDataSizeLimit", _U32_MAX),
    ("heap_size", "heapSize", _U32_MAX),
    ("priority_fee_lamports", "priorityFeeLamports", _U64_MAX),
    (
        "compute_unit_price_micro_lamports",
        "computeUnitPriceMicroLamports",
        _U64_MAX,
    ),
)


def _resource_int(value: Any, name: str, maximum: int) -> int:
    """Accept an exact ``int`` or a decimal string.

    Floats are rejected rather than coerced: a value that arrived as a double
    may already have lost precision, and coercing it would launder that loss
    into a fee or a budget ceiling. ``bool`` is an ``int`` subclass and is
    never a valid quantity here.
    """
    if isinstance(value, bool):
        raise ValueError(f"{name} must be an unsigned integer, got a bool")
    if isinstance(value, float):
        raise ValueError(
            f"{name} must be an int or a decimal string, not a float ({value!r}): "
            "a float may already have lost precision and will not be coerced"
        )
    if isinstance(value, str):
        if not value or not value.isascii() or not value.isdigit():
            raise ValueError(f"{name} must be a decimal integer string, got {value!r}")
        value = int(value)
    if not isinstance(value, int):
        raise ValueError(
            f"{name} must be an unsigned integer, got {type(value).__name__}"
        )
    if value < 0 or value > maximum:
        raise ValueError(f"{name} must be between 0 and {maximum}, got {value}")
    return value


@dataclass(frozen=True)
class TransactionResourceOptions:
    """Compute-budget and fee ceilings shared by send and unsigned inspection.

    snake_case accessors map to the canonical camelCase wire keys
    (:meth:`to_wire`). Arithmetic is integer-only; the u64 fee values become
    decimal strings at the JSON boundary, never floats.

    The two fee fields are mutually exclusive and version-bound:
    ``priority_fee_lamports`` is transaction-V1 only,
    ``compute_unit_price_micro_lamports`` is legacy/v0 only. A mismatch is a
    rejection, never a conversion.
    """

    compute_unit_limit: Optional[int] = None
    loaded_accounts_data_size_limit: Optional[int] = None
    heap_size: Optional[int] = None
    priority_fee_lamports: Optional[int] = None
    compute_unit_price_micro_lamports: Optional[int] = None

    def __post_init__(self) -> None:
        for name, _wire, maximum in _RESOURCE_OPTION_KEYS:
            value = getattr(self, name)
            if value is not None:
                object.__setattr__(self, name, _resource_int(value, name, maximum))
        if (
            self.priority_fee_lamports is not None
            and self.compute_unit_price_micro_lamports is not None
        ):
            raise ValueError(
                "priority_fee_lamports (V1) and compute_unit_price_micro_lamports "
                "(legacy/v0) are mutually exclusive"
            )

    @classmethod
    def coerce(cls, value: Any) -> Optional["TransactionResourceOptions"]:
        """``None`` → ``None``; passthrough; a mapping keyed by snake_case
        accessors or canonical camelCase wire keys. Unrecognized keys are
        rejected, never silently ignored."""
        if value is None or isinstance(value, cls):
            return value
        if not isinstance(value, Mapping):
            raise TypeError(
                "resource options must be a TransactionResourceOptions or mapping, "
                f"got {type(value).__name__}"
            )
        by_key = {}
        for name, wire, _maximum in _RESOURCE_OPTION_KEYS:
            by_key[name] = name
            by_key[wire] = name
        fields: dict = {}
        for key, item in value.items():
            name = by_key.get(key)
            if name is None:
                raise ValueError(f"Unsupported resource option {key!r}")
            if name in fields and fields[name] != item:
                raise ValueError(
                    f"Conflicting values for resource option {name!r}"
                )
            fields[name] = item
        return cls(**fields)

    def merged(
        self, overrides: Optional["TransactionResourceOptions"]
    ) -> "TransactionResourceOptions":
        """Field-wise merge where ``overrides`` wins. The two fee fields are
        one slot: an override that names either fee replaces both, so a
        connect-time ``compute_unit_price_micro_lamports`` default does not
        make a per-call V1 ``priority_fee_lamports`` unreachable."""
        if overrides is None:
            return self
        merged = {
            name: (
                getattr(overrides, name)
                if getattr(overrides, name) is not None
                else getattr(self, name)
            )
            for name, _wire, _maximum in _RESOURCE_OPTION_KEYS
        }
        if (
            overrides.priority_fee_lamports is not None
            or overrides.compute_unit_price_micro_lamports is not None
        ):
            merged["priority_fee_lamports"] = overrides.priority_fee_lamports
            merged["compute_unit_price_micro_lamports"] = (
                overrides.compute_unit_price_micro_lamports
            )
        return TransactionResourceOptions(**merged)

    def to_wire(self) -> dict:
        """Canonical camelCase keys with decimal-string values; unset fields
        are omitted."""
        return {
            wire: str(getattr(self, name))
            for name, wire, _maximum in _RESOURCE_OPTION_KEYS
            if getattr(self, name) is not None
        }


def _validate_transaction_version(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, bool) or value not in TRANSACTION_VERSIONS:
        raise ValueError(
            f"transaction_version must be one of {TRANSACTION_VERSIONS}, got {value!r}"
        )
    return value


def _validate_version_fees(
    version: Any, resources: Optional[TransactionResourceOptions]
) -> None:
    """Reject a fee field that does not belong to the requested version. An
    omitted version is the existing v0 default."""
    if resources is None:
        return
    effective = 0 if version is None else version
    if resources.priority_fee_lamports is not None and effective != 1:
        raise ValueError(
            "priority_fee_lamports requires transaction_version 1, got "
            f"{effective!r}"
        )
    if resources.compute_unit_price_micro_lamports is not None and effective == 1:
        raise ValueError(
            "compute_unit_price_micro_lamports is not valid for transaction_version 1; "
            "use priority_fee_lamports"
        )


@dataclass(frozen=True)
class SendOptions:
    """Options forwarded to the wallet adapter when sending a transaction.

    The core SDK does not interpret these; it passes them straight through to
    the adapter, which owns all RPC semantics. ``extra`` is the Python
    rendering of the TS index signature: adapter-specific passthrough options
    (lookup tables, ...). ``signers`` are optional extra local signers for
    this send; their concrete type depends on the adapter.

    ``transaction_version`` (``"legacy" | 0 | 1``) and ``resources`` are the
    typed replacements for stuffing budgets and fees into ``extra``. An
    omitted ``transaction_version`` means the adapter's existing default (v0
    for first-party builders); an explicit version an adapter does not
    advertise raises :class:`UnsupportedTransactionVersionError` rather than
    silently downgrading (see :func:`ensure_transaction_version_supported`).
    """

    confirmation_level: Optional[str] = None
    skip_preflight: Optional[bool] = None
    signers: Optional[Tuple[Any, ...]] = None
    extra: Mapping[str, Any] = field(default_factory=dict)
    transaction_version: Optional[Any] = None
    resources: Optional[TransactionResourceOptions] = None

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
        object.__setattr__(
            self,
            "transaction_version",
            _validate_transaction_version(self.transaction_version),
        )
        object.__setattr__(
            self, "resources", TransactionResourceOptions.coerce(self.resources)
        )

    def validate(self) -> "SendOptions":
        """Reject an incompatible version/fee pair in the EFFECTIVE options.

        Deliberately not called from ``__post_init__``: a per-call override is
        coerced on its own, before execution defaults merge into it, so
        validating there refuses ``{"resources": {"priorityFeeLamports": ...}}``
        against the *default* version 0 rather than the version the caller
        configured. Callers validate once, after merging and before dispatch.
        """
        _validate_version_fees(self.transaction_version, self.resources)
        return self

    @classmethod
    def coerce(cls, value: Any) -> "SendOptions":
        """``None`` → defaults; :class:`SendOptions` passthrough; a mapping's
        unknown keys land in ``extra`` (adapter passthrough).

        ``transactionVersion`` is accepted as an alias for
        ``transaction_version`` so a camelCase mapping cannot smuggle a
        version request into ``extra``, where it would be ignored.
        """
        if value is None:
            return cls()
        if isinstance(value, cls):
            return value
        if isinstance(value, Mapping):
            known = {name: value[name] for name in _SEND_OPTION_FIELDS if name in value}
            if "transactionVersion" in value:
                if "transaction_version" in value:
                    raise ValueError(
                        "send options carry both transaction_version and "
                        "transactionVersion"
                    )
                known["transaction_version"] = value["transactionVersion"]
            extra = {
                key: item
                for key, item in value.items()
                if key not in _SEND_OPTION_FIELDS
                and key not in ("extra", "transactionVersion")
            }
            nested = value.get("extra")
            if isinstance(nested, Mapping):
                extra.update(nested)
            return cls(extra=extra, **known)
        raise TypeError(
            f"send options must be a SendOptions or mapping, got {type(value).__name__}"
        )

    def merged(self, overrides: Optional["SendOptions"]) -> "SendOptions":
        """Field-wise merge where ``overrides`` wins; ``extra`` maps merge and
        ``resources`` merges field-wise.

        The merged result is the effective request, so it is validated here —
        a version/fee combination the merge produces is rejected, while a
        partial override that only makes sense once the configured version is
        inherited is not.
        """
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
            transaction_version=(
                overrides.transaction_version
                if overrides.transaction_version is not None
                else self.transaction_version
            ),
            resources=(
                self.resources.merged(overrides.resources)
                if self.resources is not None
                else overrides.resources
            ),
        ).validate()

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

    ``loaded_accounts_data_size`` carries the simulated loaded account data
    size (bytes) straight through from the relay's ``loadedAccountsDataSize``
    so callers can size a transaction-V1 budget. ``None`` means the relay did
    not report it; ``0`` means it reported zero.
    """

    fee_lamports: Optional[int] = None
    logs: Optional[Tuple[str, ...]] = None
    compute_units_consumed: Optional[int] = None
    context_slot: Optional[int] = None
    error: Any = None
    extra: Mapping[str, Any] = field(default_factory=dict)
    loaded_accounts_data_size: Optional[int] = None


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


class UnsupportedTransactionVersionError(WalletError):
    """An explicit ``transaction_version`` the adapter does not advertise.

    Raised before anything is built or signed, so it carries a
    ``not-submitted`` outcome in the ``build`` phase. Never a downgrade: the
    request fails instead of quietly running as another version.
    """

    def __init__(self, version: Any, supported: Optional[Tuple[Any, ...]]) -> None:
        if supported is None:
            detail = "the adapter declares no supported_transaction_versions"
        else:
            detail = f"the adapter supports {supported}"
        super().__init__(
            f"Wallet adapter does not support transaction_version {version!r}: "
            f"{detail}",
            outcome=TransactionFailureOutcome.not_submitted(
                "build",
                message=(
                    f"Wallet adapter does not support transaction_version "
                    f"{version!r}: {detail}"
                ),
            ),
        )
        self.version = version
        self.supported = supported


def wallet_supported_transaction_versions(wallet: Any) -> Optional[Tuple[Any, ...]]:
    """Versions the adapter advertises, or ``None`` when it advertises nothing.

    ``None`` means *unknown*, never *none supported*: adapters written before
    this capability existed keep working for the versions they always built.
    """
    declared = getattr(wallet, "supported_transaction_versions", None)
    if declared is None or callable(declared) or isinstance(declared, (str, bytes)):
        return None
    versions = tuple(declared)
    for version in versions:
        if isinstance(version, bool) or version not in TRANSACTION_VERSIONS:
            raise ValueError(
                "supported_transaction_versions must contain only "
                f"{TRANSACTION_VERSIONS}, got {version!r}"
            )
    return versions


def ensure_transaction_version_supported(wallet: Any, version: Any) -> None:
    """Fail closed on an explicit version the adapter cannot build.

    - ``version is None`` (no explicit request) always passes: existing
      callers are unaffected.
    - No declared capability: legacy/v0 pass (unknown, not unsupported), an
      explicit ``1`` fails.
    - Declared capability: the version must be in it.
    """
    if version is None:
        return
    version = _validate_transaction_version(version)
    supported = wallet_supported_transaction_versions(wallet)
    if supported is None:
        if version == 1:
            raise UnsupportedTransactionVersionError(version, None)
        return
    if version not in supported:
        raise UnsupportedTransactionVersionError(version, supported)


def adapter_send_defaults(wallet: Any) -> "SendOptions":
    """The adapter's own default send options, or empty ones.

    Layering these under the execution defaults and the per-call override
    keeps every intermediate merge valid: ``SendOptions.merged`` validates
    the version/fee pair it produces, so a merge that starts from nothing
    rejects a V1 fee before the adapter's configured version is in scope.
    """
    resolve = getattr(wallet, "resolve_send_options", None)
    return resolve(None) if callable(resolve) else SendOptions()


def resolve_transaction_options(wallet: Any, options: Any) -> "SendOptions":
    """Effective send options for pre-dispatch validation.

    An adapter may expose ``resolve_send_options(options) -> SendOptions`` to
    merge its own defaults under the per-call override first. Without it the
    caller's options are read alone, so an adapter configured with
    ``transaction_version=1`` sees a valid ``priorityFeeLamports`` rejected
    here as a v0 fee mismatch, before it can supply the version it will
    actually compile.

    Adapters without the hook keep the previous behaviour exactly: coerce,
    validate, then check the explicit version against the declared
    capability.
    """
    resolve = getattr(wallet, "resolve_send_options", None)
    effective = resolve(options) if callable(resolve) else SendOptions.coerce(options)
    effective = effective.validate()
    ensure_transaction_version_supported(wallet, effective.transaction_version)
    return effective


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
    - ``supported_transaction_versions`` — the versions this adapter can
      build, a subset of :data:`TRANSACTION_VERSIONS`. Absent means
      *unknown*, not *none*: existing adapters keep serving callers who ask
      for no particular version, while an explicit ``transaction_version=1``
      raises :class:`UnsupportedTransactionVersionError`.

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
