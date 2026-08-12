"""Prepared operations, composition, and execution.

Python port of ``typescript/core/src/operations.ts`` +
``signer-registry.ts`` (Rust sibling: ``arete_sdk::operations``).

Prepared values are portable data: name, artifacts, per-transaction
instruction lists, required signer addresses, and error metadata. They
compose (prepend/append, ``create_prepared_transaction(operations=...)``)
and execute through :func:`execute_prepared_operation` with fail-closed
signer validation, per-transaction callbacks, and receipts.

The four-state outcome model is data, not exceptions: successful execution
returns receipts (the ``confirmed`` status); failures raise
:class:`OperationExecutionError` holding the
:class:`arete.wallet.TransactionFailureOutcome` (``not-submitted |
submitted-unknown | chain-failed`` with the phase that produced it).
"""

from __future__ import annotations

import dataclasses
import inspect as _inspect
import math as _math
import re as _re
from dataclasses import dataclass
from typing import (
    Any,
    Awaitable,
    Callable,
    Dict,
    Iterable,
    List,
    Mapping,
    Optional,
    Protocol,
    Sequence,
    Set,
    Tuple,
    Union,
)

from arete.errors import AreteError
from arete.instructions import (
    BuiltInstruction,
    ErrorMetadata,
    format_program_error,
    lookup_program_error,
    parse_program_error,
)
from arete.transactions import TransactionTransport
from arete.wallet import (
    ConfirmedTransactionOutcome,
    SendOptions,
    SendResult,
    TransactionFailureOutcome,
    TransactionInspectionResult,
    WalletAdapter,
    WalletError,
    WalletExecutionContext,
)

__all__ = [
    "OPERATION_KINDS",
    "PreparedTransactionBody",
    "OperationPlan",
    "PreparedInstruction",
    "PreparedTransaction",
    "PreparedFlow",
    "PreparedOperation",
    "create_prepared_transaction_body",
    "create_prepared_instruction",
    "create_prepared_transaction",
    "create_prepared_flow",
    "prepend_transaction_instructions",
    "append_transaction_instructions",
    "append_flow_transactions",
    "prepend_flow_transaction_instructions",
    "OperationTransactionReceipt",
    "OperationReceipt",
    "OperationExecutionEvent",
    "OperationCallbackError",
    "TransactionExecutionError",
    "OperationExecutionError",
    "OperationExecutionHost",
    "SignerRegistry",
    "infer_signer_address",
    "get_transaction_failure_outcome",
    "classify_execution_failure",
    "execute_prepared_operation",
    "unwrap_operation_execution_error",
    "OperationInspection",
    "inspect_prepared_operation",
    "extract_program_error_code",
    "to_json_value",
    "describe_prepared_operation",
    "format_prepared_operation",
    "ConfirmedTransactionOutcome",
    "TransactionFailureOutcome",
]

OPERATION_KINDS: Tuple[str, ...] = ("instruction", "transaction", "flow")


# ---------------------------------------------------------------------------
# Prepared value model
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class PreparedTransactionBody:
    """One transaction of a prepared operation."""

    name: str
    instructions: Tuple[BuiltInstruction, ...]
    required_signer_addresses: Tuple[str, ...] = ()
    errors: Tuple[ErrorMetadata, ...] = ()


@dataclass(frozen=True)
class OperationPlan:
    """The transaction-by-transaction execution plan of an operation."""

    name: str
    artifacts: Any
    transactions: Tuple[PreparedTransactionBody, ...]


@dataclass(frozen=True)
class PreparedInstruction:
    """A prepared single-instruction operation."""

    name: str
    instruction: BuiltInstruction
    transaction: PreparedTransactionBody
    plan: OperationPlan
    artifacts: Any = None

    kind: str = "instruction"


@dataclass(frozen=True)
class PreparedTransaction:
    """A prepared single-transaction (multi-instruction) operation."""

    name: str
    transaction: PreparedTransactionBody
    plan: OperationPlan
    artifacts: Any = None

    kind: str = "transaction"


@dataclass(frozen=True)
class PreparedFlow:
    """A prepared multi-transaction operation."""

    name: str
    plan: OperationPlan
    artifacts: Any = None

    kind: str = "flow"


PreparedOperation = Union[PreparedInstruction, PreparedTransaction, PreparedFlow]


def _dedupe(values: Iterable[str]) -> Tuple[str, ...]:
    seen: Set[str] = set()
    out: List[str] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            out.append(value)
    return tuple(out)


def _infer_signer_addresses(
    instructions: Sequence[BuiltInstruction],
) -> Tuple[str, ...]:
    return _dedupe(
        account.pubkey
        for instruction in instructions
        for account in instruction.accounts
        if account.is_signer
    )


def create_prepared_transaction_body(
    *,
    name: str,
    instructions: Sequence[BuiltInstruction],
    required_signer_addresses: Optional[Sequence[str]] = None,
    errors: Optional[Sequence[ErrorMetadata]] = None,
) -> PreparedTransactionBody:
    """Build a transaction body; signer addresses default to those inferred
    from the instructions' signer account metas. Fails closed on empty
    instruction lists."""
    instruction_tuple = tuple(instructions)
    if not instruction_tuple:
        raise ValueError(f"Transaction '{name}' must contain at least one item")
    return PreparedTransactionBody(
        name=name,
        instructions=instruction_tuple,
        required_signer_addresses=_dedupe(
            required_signer_addresses
            if required_signer_addresses is not None
            else _infer_signer_addresses(instruction_tuple)
        ),
        errors=tuple(errors or ()),
    )


def _create_plan(
    name: str, artifacts: Any, transactions: Sequence[PreparedTransactionBody]
) -> OperationPlan:
    transaction_tuple = tuple(transactions)
    if not transaction_tuple:
        raise ValueError(f"Flow '{name}' must contain at least one item")
    return OperationPlan(name=name, artifacts=artifacts, transactions=transaction_tuple)


def create_prepared_instruction(
    *,
    name: str,
    instruction: BuiltInstruction,
    artifacts: Any = None,
    required_signer_addresses: Optional[Sequence[str]] = None,
    errors: Optional[Sequence[ErrorMetadata]] = None,
) -> PreparedInstruction:
    transaction = create_prepared_transaction_body(
        name=name,
        instructions=[instruction],
        required_signer_addresses=required_signer_addresses,
        errors=errors,
    )
    return PreparedInstruction(
        name=name,
        instruction=instruction,
        transaction=transaction,
        plan=_create_plan(name, artifacts, [transaction]),
        artifacts=artifacts,
    )


def create_prepared_transaction(
    *,
    name: str,
    instructions: Optional[
        Sequence[Union[BuiltInstruction, PreparedInstruction]]
    ] = None,
    operations: Optional[
        Sequence[Union[PreparedInstruction, PreparedTransaction]]
    ] = None,
    artifacts: Any = None,
    required_signer_addresses: Optional[Sequence[str]] = None,
    errors: Optional[Sequence[ErrorMetadata]] = None,
) -> PreparedTransaction:
    """Compose one atomic transaction from built/prepared instructions or
    prepared operations. Exactly one of ``instructions`` / ``operations``
    must be given; explicit ``required_signer_addresses`` / ``errors``
    override the metadata inherited from composed parts."""
    if (instructions is None) == (operations is None):
        raise ValueError(
            f"Transaction '{name}' must provide exactly one of instructions or operations"
        )
    if operations is not None:
        parts = [operation.transaction for operation in operations]
    else:
        parts = [
            entry.transaction
            if isinstance(entry, PreparedInstruction)
            else create_prepared_transaction_body(name=name, instructions=[entry])
            for entry in instructions or ()
        ]
    transaction = create_prepared_transaction_body(
        name=name,
        instructions=[
            instruction for part in parts for instruction in part.instructions
        ],
        required_signer_addresses=(
            required_signer_addresses
            if required_signer_addresses is not None
            else [
                address
                for part in parts
                for address in part.required_signer_addresses
            ]
        ),
        errors=(
            errors
            if errors is not None
            else [error for part in parts for error in part.errors]
        ),
    )
    return PreparedTransaction(
        name=name,
        transaction=transaction,
        plan=_create_plan(name, artifacts, [transaction]),
        artifacts=artifacts,
    )


def _coerce_transaction_body(
    value: Union[PreparedTransactionBody, Mapping[str, Any]],
) -> PreparedTransactionBody:
    if isinstance(value, PreparedTransactionBody):
        return create_prepared_transaction_body(
            name=value.name,
            instructions=value.instructions,
            required_signer_addresses=value.required_signer_addresses,
            errors=value.errors,
        )
    return create_prepared_transaction_body(**dict(value))


def create_prepared_flow(
    *,
    name: str,
    transactions: Sequence[Union[PreparedTransactionBody, Mapping[str, Any]]],
    artifacts: Any = None,
) -> PreparedFlow:
    bodies = [_coerce_transaction_body(transaction) for transaction in transactions]
    return PreparedFlow(
        name=name,
        plan=_create_plan(name, artifacts, bodies),
        artifacts=artifacts,
    )


def prepend_transaction_instructions(
    transaction: PreparedTransactionBody,
    instructions: Sequence[BuiltInstruction],
) -> PreparedTransactionBody:
    return create_prepared_transaction_body(
        name=transaction.name,
        instructions=[*instructions, *transaction.instructions],
        required_signer_addresses=[
            *_infer_signer_addresses(tuple(instructions)),
            *transaction.required_signer_addresses,
        ],
        errors=transaction.errors,
    )


def append_transaction_instructions(
    transaction: PreparedTransactionBody,
    instructions: Sequence[BuiltInstruction],
) -> PreparedTransactionBody:
    return create_prepared_transaction_body(
        name=transaction.name,
        instructions=[*transaction.instructions, *instructions],
        required_signer_addresses=[
            *transaction.required_signer_addresses,
            *_infer_signer_addresses(tuple(instructions)),
        ],
        errors=transaction.errors,
    )


def append_flow_transactions(
    flow: PreparedFlow,
    transactions: Sequence[Union[PreparedTransactionBody, Mapping[str, Any]]],
) -> PreparedFlow:
    return create_prepared_flow(
        name=flow.name,
        artifacts=flow.artifacts,
        transactions=[*flow.plan.transactions, *transactions],
    )


def prepend_flow_transaction_instructions(
    flow: PreparedFlow,
    transaction_index: int,
    instructions: Sequence[BuiltInstruction],
) -> PreparedFlow:
    if not 0 <= transaction_index < len(flow.plan.transactions):
        raise ValueError(
            f"Flow '{flow.name}' has no transaction at index {transaction_index}"
        )
    transactions = list(flow.plan.transactions)
    transactions[transaction_index] = prepend_transaction_instructions(
        transactions[transaction_index], instructions
    )
    return create_prepared_flow(
        name=flow.name, artifacts=flow.artifacts, transactions=transactions
    )


# ---------------------------------------------------------------------------
# Receipts
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class OperationTransactionReceipt:
    """Receipt for one confirmed transaction of an operation."""

    transaction_index: int
    transaction_name: str
    signature: str
    slot: Optional[int] = None


@dataclass(frozen=True)
class OperationReceipt:
    """Receipt for a fully executed operation.

    Single receipt shape for all kinds (Rust idiom): flows carry every
    transaction receipt in ``transactions``; instruction/transaction
    operations carry exactly one (also reachable via :attr:`transaction`).
    """

    kind: str
    operation_name: str
    artifacts: Any
    signatures: Tuple[str, ...]
    transactions: Tuple[OperationTransactionReceipt, ...]
    callback_errors: Tuple["OperationCallbackError", ...] = ()

    @property
    def transaction(self) -> OperationTransactionReceipt:
        return self.transactions[0]


@dataclass(frozen=True)
class OperationExecutionEvent:
    """Event passed to execution observers; ``receipt`` is ``None`` for
    transaction-start events."""

    operation: PreparedOperation
    transaction: PreparedTransactionBody
    transaction_index: int
    receipt: Optional[OperationTransactionReceipt] = None


class OperationCallbackError(AreteError):
    """An observer callback failed. Observational only — never changes the
    transaction outcome."""

    def __init__(
        self,
        *,
        phase: str,
        operation: PreparedOperation,
        transaction: PreparedTransactionBody,
        transaction_index: int,
        receipt: Optional[OperationTransactionReceipt] = None,
        cause: Any = None,
    ) -> None:
        super().__init__(
            f"Operation '{operation.name}' {phase} callback failed for "
            f"transaction {transaction_index + 1} ({transaction.name})",
            "OPERATION_CALLBACK_FAILED",
        )
        self.phase = phase
        self.operation = operation
        self.transaction = transaction
        self.transaction_index = transaction_index
        self.receipt = receipt
        self.cause = cause


class TransactionExecutionError(AreteError):
    """Structured transaction failure raised by ``client.transaction`` (and
    optionally by adapters), carrying a classified
    :class:`TransactionFailureOutcome`."""

    def __init__(
        self, outcome: TransactionFailureOutcome, message: Optional[str] = None
    ) -> None:
        super().__init__(message or outcome.message, "TRANSACTION_FAILED")
        self.outcome = outcome
        self.cause = outcome.cause
        self.signature = outcome.signature
        self.slot = outcome.slot


class OperationExecutionError(AreteError):
    """Execution of a prepared operation failed.

    Carries which transaction failed, receipts for the transactions that had
    already completed, callback errors observed so far, and the structured
    :class:`TransactionFailureOutcome`.
    """

    def __init__(
        self,
        *,
        operation: PreparedOperation,
        failed_transaction: PreparedTransactionBody,
        failed_transaction_index: int,
        completed_receipts: Sequence[OperationTransactionReceipt] = (),
        callback_errors: Sequence[OperationCallbackError] = (),
        outcome: Optional[TransactionFailureOutcome] = None,
        cause: Any = None,
    ) -> None:
        resolved = get_transaction_failure_outcome(cause) or outcome
        if resolved is None:
            resolved = TransactionFailureOutcome.not_submitted("send", cause=cause)
        context = (
            f"Operation '{operation.name}' failed at transaction "
            f"{failed_transaction_index + 1} ({failed_transaction.name})"
        )
        detail = (
            str(cause)
            if isinstance(cause, BaseException) and str(cause)
            else cause if isinstance(cause, str) else resolved.message
        )
        super().__init__(
            f"{context}: {detail}" if detail else context, "OPERATION_FAILED"
        )
        self.operation = operation
        self.failed_transaction = failed_transaction
        self.failed_transaction_index = failed_transaction_index
        self.completed_receipts = tuple(completed_receipts)
        self.callback_errors = tuple(callback_errors)
        self.outcome = resolved
        self.signature = resolved.signature
        self.slot = resolved.slot
        self.cause = cause


def get_transaction_failure_outcome(
    error: Any,
) -> Optional[TransactionFailureOutcome]:
    """Find a structured transaction failure through nested error causes."""
    seen: Set[int] = set()
    current = error
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        if isinstance(current, TransactionFailureOutcome):
            return current
        outcome = getattr(current, "outcome", None)
        if isinstance(outcome, TransactionFailureOutcome):
            return outcome
        next_value = getattr(current, "cause", None)
        if next_value is None and isinstance(current, BaseException):
            next_value = current.__cause__
        current = next_value
    return None


# -- failure normalization (TS instructions/error-parser.ts) ----------------

_ERROR_CHAIN_KEYS = (
    "cause",
    "error",
    "err",
    "value",
    "data",
    "outcome",
    "transaction_error",
    "transactionError",
)

_WALLET_REJECTION_NAMES = frozenset(
    {
        "UserRejectedRequestError",
        "UserRejectError",
        "WalletRequestRejectedError",
    }
)

_WALLET_REJECTION_MESSAGES = (
    _re.compile(
        r"^(?:the )?user (?:rejected|declined|denied)(?: the)? "
        r"(?:request|transaction|signature request|wallet request)[.!]?$",
        _re.IGNORECASE,
    ),
    _re.compile(
        r"^(?:request|transaction) (?:was )?(?:rejected|declined|denied) "
        r"by (?:the )?user[.!]?$",
        _re.IGNORECASE,
    ),
)


def _lookup(current: Any, key: str) -> Any:
    if isinstance(current, Mapping):
        return current.get(key)
    return getattr(current, key, None)


def _is_walkable(value: Any) -> bool:
    """Objects the error walkers descend into (TS ``typeof x === 'object'``)."""
    if value is None or isinstance(value, (str, bytes, bytearray, int, float, bool)):
        return False
    return isinstance(value, Mapping) or hasattr(value, "__dict__")


def _int_or_none(value: Any) -> Optional[int]:
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    return None


def _next_cause(current: Any) -> Any:
    """The next link of a failure chain (``cause``, then ``__cause__``)."""
    next_value = _lookup(current, "cause")
    if next_value is None and isinstance(current, BaseException):
        next_value = current.__cause__
    return next_value


@dataclass(frozen=True)
class _ExtractedErrorCode:
    code: int
    source: str  # 'instruction-error' | 'program-error' | 'direct-code'


@dataclass(frozen=True)
class _InstructionErrorMatch:
    program_error: ErrorMetadata
    deterministic: bool


@dataclass(frozen=True)
class _TransactionContext:
    signature: Optional[str] = None
    slot: Optional[int] = None


def _extract_error_codes(
    error: Any, seen: Set[int], results: List[_ExtractedErrorCode]
) -> None:
    """Port of TS ``extractErrorCodes`` (error-parser.ts:118-176)."""
    if not _is_walkable(error) or id(error) in seen:
        return
    seen.add(id(error))

    # TS short-circuits on already-parsed carriers (InstructionError with a
    # programError, chain-failed TransactionExecutionError). Python's carrier
    # is the structured outcome itself, wherever it is attached.
    carried = error if isinstance(error, TransactionFailureOutcome) else None
    if carried is None:
        attached = _lookup(error, "outcome")
        if isinstance(attached, TransactionFailureOutcome):
            carried = attached
    if (
        carried is not None
        and carried.status == "chain-failed"
        and carried.program_error is not None
    ):
        results.append(
            _ExtractedErrorCode(carried.program_error.code, "program-error")
        )
        return

    instruction_error = _lookup(error, "InstructionError")
    if isinstance(instruction_error, (list, tuple)) and len(instruction_error) > 1:
        custom = _int_or_none(_lookup(instruction_error[1], "Custom"))
        if custom is not None:
            results.append(_ExtractedErrorCode(custom, "instruction-error"))

    program_error = _lookup(error, "program_error")
    if program_error is None:
        program_error = _lookup(error, "programError")
    if program_error is not None:
        code = _int_or_none(_lookup(program_error, "code"))
        if code is not None:
            results.append(_ExtractedErrorCode(code, "program-error"))

    direct = _int_or_none(_lookup(error, "code"))
    if direct is not None:
        results.append(_ExtractedErrorCode(direct, "direct-code"))

    for key in _ERROR_CHAIN_KEYS:
        _extract_error_codes(_lookup(error, key), seen, results)
    if isinstance(error, BaseException) and error.__cause__ is not None:
        _extract_error_codes(error.__cause__, seen, results)


def _parse_instruction_error_match(
    error: Any, errors: Sequence[ErrorMetadata] = ()
) -> Optional[_InstructionErrorMatch]:
    """Port of TS ``parseInstructionErrorMatch`` (error-parser.ts:178-215)."""
    if error is None:
        return None
    candidates: List[_ExtractedErrorCode] = []
    _extract_error_codes(error, set(), candidates)
    if not candidates:
        return None
    selected = (
        next((c for c in candidates if c.source == "instruction-error"), None)
        or next((c for c in candidates if c.source == "program-error"), None)
        or next(
            (c for c in candidates if lookup_program_error(c.code, errors) is not None),
            None,
        )
        or candidates[0]
    )
    metadata = lookup_program_error(selected.code, errors)
    return _InstructionErrorMatch(
        program_error=parse_program_error(selected.code, errors),
        deterministic=selected.source != "direct-code" or metadata is not None,
    )


def _extract_transaction_context(error: Any) -> _TransactionContext:
    """Port of TS ``extractTransactionContext`` (error-parser.ts:318-348).

    Recovers ``signature`` / ``slot`` from arbitrary adapter exceptions by
    walking the failure chain (and any structured outcome attached to it)."""
    seen: Set[int] = set()
    current = error
    signature: Optional[str] = None
    slot: Optional[int] = None
    while _is_walkable(current) and id(current) not in seen:
        seen.add(id(current))
        if signature is None:
            candidate = _lookup(current, "signature")
            if isinstance(candidate, str) and candidate:
                signature = candidate
        if slot is None:
            slot = _int_or_none(_lookup(current, "slot"))
        attached = (
            current
            if isinstance(current, TransactionFailureOutcome)
            else _lookup(current, "outcome")
        )
        if isinstance(attached, TransactionFailureOutcome):
            if signature is None and attached.signature:
                signature = attached.signature
            if slot is None:
                slot = attached.slot
        current = _next_cause(current)
    return _TransactionContext(signature=signature, slot=slot)


def _is_wallet_rejection(error: Any) -> bool:
    """Port of TS ``isWalletRejection`` (error-parser.ts:350-383)."""
    seen: Set[int] = set()
    current = error
    while _is_walkable(current) and id(current) not in seen:
        seen.add(id(current))
        code = _lookup(current, "code")
        if code == 4001 or code == "4001" or code == "ACTION_REJECTED":
            return True
        names = {str(_lookup(current, "name") or "")}
        if isinstance(current, BaseException):
            names.add(type(current).__name__)
        if names & _WALLET_REJECTION_NAMES:
            return True
        message = _lookup(current, "message")
        if not isinstance(message, str) and isinstance(current, BaseException):
            message = str(current)
        message = message.strip() if isinstance(message, str) else ""
        if any(pattern.match(message) for pattern in _WALLET_REJECTION_MESSAGES):
            return True
        next_value = _next_cause(current)
        current = next_value if next_value is not None else _lookup(current, "error")
    return False


def _chain_failed_outcome(
    cause: Any,
    existing: Optional[TransactionFailureOutcome],
    program_error: ErrorMetadata,
    errors: Sequence[ErrorMetadata] = (),
) -> TransactionFailureOutcome:
    """Port of TS ``asInstructionError`` (error-parser.ts:399-418)."""
    context = _extract_transaction_context(cause)
    if existing is not None and existing.status == "chain-failed":
        phase = existing.phase
        signature = context.signature or existing.signature
        slot = context.slot if context.slot is not None else existing.slot
        # An outcome that already carries a resolved program error keeps it
        # when the code is unknown to ``errors`` (TS returns such carriers
        # untouched); re-resolution must never downgrade a known name to the
        # synthetic ``CustomError<code>`` fallback.
        if (
            existing.program_error is not None
            and existing.program_error.code == program_error.code
            and lookup_program_error(program_error.code, errors) is None
        ):
            program_error = existing.program_error
        if (
            existing.program_error == program_error
            and existing.signature == signature
            and existing.slot == slot
        ):
            return existing
    else:
        phase = "chain"
        signature = context.signature
        slot = context.slot
    original_cause = (
        existing.cause if existing is not None and existing.cause is not None else cause
    )
    return TransactionFailureOutcome.chain_failed(
        phase=phase,
        signature=signature,
        slot=slot,
        program_error=program_error,
        message=format_program_error(program_error),
        cause=original_cause,
    )


def classify_execution_failure(
    cause: Any,
    errors: Sequence[ErrorMetadata] = (),
    fallback_phase: str = "send",
) -> TransactionFailureOutcome:
    """Classify a wallet/host failure into a structured outcome.

    Faithful port of TS ``normalizeTransactionError``
    (``instructions/error-parser.ts``) expressed in the Python
    outcome-as-data model, in ladder order:

    1. a deterministic program-error match anywhere in the failure chain
       (``InstructionError`` payloads, ``program_error`` carriers, or a raw
       ``code`` that resolves against ``errors``) is a ``chain-failed``
       outcome — the signature/slot recovered from the chain are attached and
       the program error is (re-)resolved against ``errors`` metadata;
    2. otherwise a structured outcome attached to the failure (or its cause
       chain) wins;
    3. otherwise a recognized wallet rejection is ``not-submitted``/``wallet``;
    4. otherwise a non-deterministic program-error match is ``chain-failed``;
    5. otherwise a signature recovered from the failure chain means the
       transaction was dispatched: ``submitted-unknown``/``confirmation``;
    6. otherwise an outcome-less :class:`WalletError` keeps its own message as
       ``not-submitted``/``fallback_phase``, and anything else is
       ``not-submitted``/``fallback_phase`` with the raw cause.

    Divergence from TS: the rejection *heuristic* (step 3) is skipped for
    :class:`WalletError`, because Python adapters own their classification and
    report rejections by attaching a ``not-submitted``/``wallet`` outcome; an
    outcome-less ``WalletError`` stays ``not-submitted``/``send``.
    """
    existing = get_transaction_failure_outcome(cause)
    match = _parse_instruction_error_match(cause, errors)

    if match is not None and match.deterministic:
        return _chain_failed_outcome(cause, existing, match.program_error, errors)
    if existing is not None:
        return existing
    if not isinstance(cause, WalletError) and _is_wallet_rejection(cause):
        return TransactionFailureOutcome.not_submitted("wallet", cause=cause)
    if match is not None:
        return _chain_failed_outcome(cause, existing, match.program_error, errors)

    context = _extract_transaction_context(cause)
    if context.signature:
        return TransactionFailureOutcome.submitted_unknown(
            context.signature,
            phase="confirmation",
            slot=context.slot,
            cause=cause,
        )
    if isinstance(cause, WalletError):
        return cause.into_outcome(fallback_phase)
    return TransactionFailureOutcome.not_submitted(fallback_phase, cause=cause)


# ---------------------------------------------------------------------------
# Signer registry (TS signer-registry.ts)
# ---------------------------------------------------------------------------


def infer_signer_address(value: Any) -> Optional[str]:
    """Best-effort address of an opaque signer: a non-empty string, or an
    object exposing ``address`` / ``public_key`` / ``pubkey``. Opaque signers
    without an inferable address never satisfy signer validation (fail
    closed)."""
    if isinstance(value, str):
        return value or None
    if value is None:
        return None
    for attr in ("address", "public_key", "pubkey"):
        candidate = (
            value.get(attr) if isinstance(value, Mapping) else getattr(value, attr, None)
        )
        if isinstance(candidate, str) and candidate:
            return candidate
    return None


class SignerRegistry:
    """Address-keyed registry of opaque signers. Registered addresses count
    toward fail-closed signer validation; registered signers are forwarded to
    the wallet adapter on every send."""

    def __init__(
        self, entries: Iterable[Tuple[str, Any]] = ()
    ) -> None:
        self._signers: Dict[str, Any] = {}
        for address, signer in entries:
            self.register(address, signer)

    def register(self, address: str, signer: Any) -> None:
        if not address:
            raise ValueError("Signer registry addresses must not be empty")
        self._signers[address] = signer

    def unregister(self, address: str) -> bool:
        if address in self._signers:
            del self._signers[address]
            return True
        return False

    def get(self, address: str) -> Any:
        return self._signers.get(address)

    def has(self, address: str) -> bool:
        return address in self._signers

    def addresses(self) -> Tuple[str, ...]:
        return tuple(self._signers.keys())

    def values(self) -> Tuple[Any, ...]:
        return tuple(self._signers.values())

    def entries(self) -> Tuple[Tuple[str, Any], ...]:
        return tuple(self._signers.items())

    def clear(self) -> None:
        self._signers.clear()

    def __len__(self) -> int:
        return len(self._signers)


def create_signer_registry(
    entries: Iterable[Tuple[str, Any]] = ()
) -> SignerRegistry:
    return SignerRegistry(entries)


# ---------------------------------------------------------------------------
# Execution
# ---------------------------------------------------------------------------


class OperationExecutionHost(Protocol):
    """Host that dispatches one transaction at a time (a connected client)."""

    @property
    def wallet(self) -> Optional[WalletAdapter]: ...

    @property
    def public_key(self) -> Optional[str]: ...

    async def transaction(
        self,
        instructions: Sequence[BuiltInstruction],
        *,
        wallet: Optional[WalletAdapter] = None,
        send: Optional[SendOptions] = None,
        errors: Optional[Sequence[ErrorMetadata]] = None,
        signers: Optional[Sequence[Any]] = None,
        transaction_transport: Optional[TransactionTransport] = None,
    ) -> SendResult: ...


Callback = Callable[[OperationExecutionEvent], Union[None, Awaitable[None]]]
CallbackErrorObserver = Callable[
    [OperationCallbackError], Union[None, Awaitable[None]]
]


async def _maybe_await(value: Any) -> Any:
    if _inspect.isawaitable(value):
        return await value
    return value


def _missing_signers(
    transaction: PreparedTransactionBody,
    host: Any,
    wallet: Optional[WalletAdapter],
    signers: Sequence[Any],
    signer_registry: Optional[SignerRegistry],
    available_signer_addresses: Optional[Sequence[str]],
) -> List[str]:
    available: Set[str] = set(available_signer_addresses or ())
    if signer_registry is not None:
        available.update(signer_registry.addresses())
    effective_wallet = wallet if wallet is not None else getattr(host, "wallet", None)
    declared = getattr(effective_wallet, "signer_addresses", None)
    if declared is not None and not callable(declared):
        available.update(str(address) for address in declared)
    # TS: `const walletAddress = wallet?.publicKey ?? host.publicKey` — the
    # host key is a *fallback* for an effective wallet without a public key,
    # never an addition. A per-call wallet override must not inherit the
    # client's default address, or validation stops failing closed.
    wallet_key = getattr(effective_wallet, "public_key", None)
    if not (isinstance(wallet_key, str) and wallet_key):
        wallet_key = getattr(host, "public_key", None)
    if isinstance(wallet_key, str) and wallet_key:
        available.add(wallet_key)
    for signer in signers:
        address = infer_signer_address(signer)
        if address:
            available.add(address)
    return [
        address
        for address in transaction.required_signer_addresses
        if address not in available
    ]


async def _run_callback(
    *,
    phase: str,
    operation: PreparedOperation,
    transaction: PreparedTransactionBody,
    transaction_index: int,
    receipt: Optional[OperationTransactionReceipt],
    callback: Optional[Callback],
    callback_errors: List[OperationCallbackError],
    on_callback_error: Optional[CallbackErrorObserver],
) -> None:
    if callback is None:
        return
    event = OperationExecutionEvent(
        operation=operation,
        transaction=transaction,
        transaction_index=transaction_index,
        receipt=receipt,
    )
    try:
        await _maybe_await(callback(event))
    except Exception as cause:
        error = OperationCallbackError(
            phase=phase,
            operation=operation,
            transaction=transaction,
            transaction_index=transaction_index,
            receipt=receipt,
            cause=cause,
        )
        callback_errors.append(error)
        if on_callback_error is not None:
            try:
                await _maybe_await(on_callback_error(error))
            except Exception:
                # The callback-error observer is also observational and must
                # not alter execution.
                pass


async def execute_prepared_operation(
    host: OperationExecutionHost,
    operation: PreparedOperation,
    *,
    wallet: Optional[WalletAdapter] = None,
    send: Optional[SendOptions] = None,
    signers: Optional[Sequence[Any]] = None,
    signer_registry: Optional[SignerRegistry] = None,
    available_signer_addresses: Optional[Sequence[str]] = None,
    transaction_transport: Optional[TransactionTransport] = None,
    on_transaction_start: Optional[Callback] = None,
    on_transaction_success: Optional[Callback] = None,
    on_callback_error: Optional[CallbackErrorObserver] = None,
) -> OperationReceipt:
    """Execute a prepared operation transaction-by-transaction through the
    host's wallet adapter.

    Per transaction, mirroring the TS executor's order exactly: (1) validate
    required signers against the union of ``available_signer_addresses``, the
    signer registry's addresses, the effective wallet's declared signer
    addresses, the effective wallet's public key (falling back to the host's
    public key only when that wallet has none), and addresses inferable from
    ``signers`` — failing closed *before* dispatch;
    (2) invoke ``on_transaction_start``; (3) dispatch via
    ``host.transaction``; (4) record the receipt; (5) invoke
    ``on_transaction_success``. Callbacks are observational: their failures
    are collected as :class:`OperationCallbackError` (forwarded to
    ``on_callback_error``) and never change the outcome.
    """
    receipts: List[OperationTransactionReceipt] = []
    callback_errors: List[OperationCallbackError] = []

    combined_signers: List[Any] = []
    seen_signers: Set[int] = set()
    for signer in [*(signer_registry.values() if signer_registry else ()), *(signers or ())]:
        if id(signer) not in seen_signers:
            seen_signers.add(id(signer))
            combined_signers.append(signer)

    def fail(
        transaction_index: int,
        transaction: PreparedTransactionBody,
        *,
        outcome: Optional[TransactionFailureOutcome] = None,
        cause: Any = None,
    ) -> OperationExecutionError:
        return OperationExecutionError(
            operation=operation,
            failed_transaction=transaction,
            failed_transaction_index=transaction_index,
            completed_receipts=receipts,
            callback_errors=callback_errors,
            outcome=outcome,
            cause=cause,
        )

    for transaction_index, transaction in enumerate(operation.plan.transactions):
        missing = _missing_signers(
            transaction,
            host,
            wallet,
            combined_signers,
            signer_registry,
            available_signer_addresses,
        )
        if missing:
            cause = ValueError(
                f"Missing signer(s) for {transaction.name}: {', '.join(missing)}"
            )
            raise fail(
                transaction_index,
                transaction,
                outcome=TransactionFailureOutcome.not_submitted(
                    "build", message=str(cause), cause=cause
                ),
                cause=cause,
            )

        await _run_callback(
            phase="transaction-start",
            operation=operation,
            transaction=transaction,
            transaction_index=transaction_index,
            receipt=None,
            callback=on_transaction_start,
            callback_errors=callback_errors,
            on_callback_error=on_callback_error,
        )

        try:
            result = await host.transaction(
                transaction.instructions,
                wallet=wallet,
                send=send,
                errors=list(transaction.errors),
                signers=combined_signers if combined_signers else None,
                transaction_transport=transaction_transport,
            )
        except Exception as cause:
            outcome = classify_execution_failure(cause, transaction.errors)
            # Normalize the cause so the enriched (program-error-resolved)
            # outcome is what nested unwrapping finds (TS
            # normalizeTransactionError before wrapping).
            normalized_cause: Any = cause
            if get_transaction_failure_outcome(cause) is not outcome:
                normalized_cause = TransactionExecutionError(outcome)
                normalized_cause.__cause__ = cause
            raise fail(
                transaction_index,
                transaction,
                outcome=outcome,
                cause=normalized_cause,
            ) from cause

        receipt = OperationTransactionReceipt(
            transaction_index=transaction_index,
            transaction_name=transaction.name,
            signature=result.signature,
            slot=getattr(result, "slot", None),
        )
        receipts.append(receipt)

        await _run_callback(
            phase="transaction-success",
            operation=operation,
            transaction=transaction,
            transaction_index=transaction_index,
            receipt=receipt,
            callback=on_transaction_success,
            callback_errors=callback_errors,
            on_callback_error=on_callback_error,
        )

    return OperationReceipt(
        kind=operation.kind,
        operation_name=operation.name,
        artifacts=operation.artifacts,
        signatures=tuple(receipt.signature for receipt in receipts),
        transactions=tuple(receipts),
        callback_errors=tuple(callback_errors),
    )


def unwrap_operation_execution_error(error: Any) -> Any:
    """Unwrap operation context, retaining structured transaction outcomes.

    Returns the innermost :class:`TransactionFailureOutcome` when one exists
    anywhere in the failure chain; otherwise returns the error unchanged.
    """
    if isinstance(error, OperationExecutionError):
        underlying = unwrap_operation_execution_error(error.cause)
        if isinstance(underlying, TransactionFailureOutcome):
            return underlying
        return error.outcome
    outcome = get_transaction_failure_outcome(error)
    return outcome if outcome is not None else error


# ---------------------------------------------------------------------------
# Inspection & description
# ---------------------------------------------------------------------------


def extract_program_error_code(value: Any) -> Optional[int]:
    """Best-effort custom program-error code extraction from raw simulation /
    RPC failure shapes (``{"InstructionError": [i, {"Custom": code}]}``,
    ``program_error.code``, nested ``err``/``error``/``value``/``cause``).
    Used only for unsigned inspection results — executed failures are
    classified by the wallet adapter."""
    seen: Set[int] = set()

    def lookup(current: Any, key: str) -> Any:
        if isinstance(current, Mapping):
            return current.get(key)
        return getattr(current, key, None)

    def walk(current: Any) -> Optional[int]:
        if current is None or id(current) in seen:
            return None
        if not isinstance(current, Mapping) and not hasattr(current, "__dict__"):
            return None
        seen.add(id(current))
        instruction_error = lookup(current, "InstructionError")
        if isinstance(instruction_error, (list, tuple)) and len(instruction_error) > 1:
            detail = instruction_error[1]
            custom = (
                detail.get("Custom") if isinstance(detail, Mapping) else None
            )
            if isinstance(custom, int) and not isinstance(custom, bool):
                return custom
        program_error = lookup(current, "program_error")
        code = (
            program_error.code
            if isinstance(program_error, ErrorMetadata)
            else lookup(program_error, "code") if program_error is not None else None
        )
        if isinstance(code, int) and not isinstance(code, bool):
            return code
        direct = lookup(current, "code")
        if isinstance(direct, int) and not isinstance(direct, bool):
            return direct
        for key in ("cause", "error", "err", "value", "data", "outcome"):
            found = walk(lookup(current, key))
            if found is not None:
                return found
        return None

    return walk(value)


@dataclass(frozen=True)
class OperationInspection:
    """Result of unsigned prepared-operation inspection."""

    description: Mapping[str, Any]
    transaction: Any
    program_error: Optional[ErrorMetadata]


async def inspect_prepared_operation(
    wallet: Optional[Any],
    operation: PreparedOperation,
    options: Optional[Mapping[str, Any]] = None,
    context: Optional[WalletExecutionContext] = None,
) -> OperationInspection:
    """Inspect one prepared instruction/transaction without signing or
    submission. Multi-transaction flows are intentionally unsupported."""
    if operation.kind == "flow":
        raise ValueError(
            f"Cannot inspect flow '{operation.name}': flow inspection is not supported"
        )
    if len(operation.plan.transactions) != 1:
        raise ValueError(
            f"Cannot inspect operation '{operation.name}': multi-transaction "
            "operation inspection is not supported"
        )
    inspect_transaction = getattr(wallet, "inspect_transaction", None)
    if wallet is None or not callable(inspect_transaction):
        raise WalletError(
            "Wallet adapter does not support unsigned transaction inspection"
        )

    transaction = operation.plan.transactions[0]
    description = describe_prepared_operation(operation)
    if context is not None:
        inspection = await _maybe_await(
            inspect_transaction(transaction.instructions, options, context)
        )
    else:
        inspection = await _maybe_await(
            inspect_transaction(transaction.instructions, options)
        )
    raw_error = (
        inspection.error
        if isinstance(inspection, TransactionInspectionResult)
        else lookup_inspection_error(inspection)
    )
    code = extract_program_error_code(raw_error if raw_error is not None else inspection)
    program_error = (
        parse_program_error(code, transaction.errors) if code is not None else None
    )
    return OperationInspection(
        description=description,
        transaction=inspection,
        program_error=program_error,
    )


def lookup_inspection_error(inspection: Any) -> Any:
    if isinstance(inspection, Mapping):
        return inspection.get("error")
    return getattr(inspection, "error", None)


def to_json_value(value: Any, _ancestors: Optional[Set[int]] = None) -> Any:
    """Convert an arbitrary value to a JSON-safe structure (TS ``toJsonValue``).

    ``bytes`` become integer lists; sets/tuples become lists; dataclasses and
    mappings become dicts; non-finite floats become ``None``; circular values
    raise ``ValueError``.
    """
    if value is None or isinstance(value, (bool, str, int)):
        return value
    if isinstance(value, float):
        return value if _math.isfinite(value) else None
    ancestors = _ancestors if _ancestors is not None else set()
    if id(value) in ancestors:
        raise ValueError("Cannot convert a circular value to JSON")
    ancestors.add(id(value))
    try:
        if isinstance(value, (bytes, bytearray, memoryview)):
            return list(bytes(value))
        if isinstance(value, Mapping):
            return {
                str(key): to_json_value(entry, ancestors)
                for key, entry in value.items()
            }
        if isinstance(value, (list, tuple, set, frozenset)):
            return [to_json_value(entry, ancestors) for entry in value]
        if dataclasses.is_dataclass(value) and not isinstance(value, type):
            return {
                item.name: to_json_value(getattr(value, item.name), ancestors)
                for item in dataclasses.fields(value)
            }
        return str(value)
    finally:
        ancestors.discard(id(value))


def describe_prepared_operation(operation: PreparedOperation) -> Dict[str, Any]:
    """JSON-safe description of a prepared operation (snake_case keys)."""
    return {
        "kind": operation.kind,
        "name": operation.name,
        "artifacts": to_json_value(operation.artifacts),
        "transactions": [
            {
                "name": transaction.name,
                "required_signer_addresses": list(
                    transaction.required_signer_addresses
                ),
                "errors": [
                    {"code": error.code, "name": error.name, "msg": error.msg}
                    for error in transaction.errors
                ],
                "instructions": [
                    {
                        "program_id": instruction.program_id,
                        "accounts": [
                            {
                                "pubkey": account.pubkey,
                                "is_signer": account.is_signer,
                                "is_writable": account.is_writable,
                            }
                            for account in instruction.accounts
                        ],
                        "data": list(instruction.data),
                    }
                    for instruction in transaction.instructions
                ],
            }
            for transaction in operation.plan.transactions
        ],
    }


def format_prepared_operation(operation: PreparedOperation) -> str:
    lines = [
        f"{operation.kind}: {operation.name}",
        f"Transactions: {len(operation.plan.transactions)}",
    ]
    for index, transaction in enumerate(operation.plan.transactions):
        count = len(transaction.instructions)
        plural = "" if count == 1 else "s"
        lines.append(f"  {index + 1}. {transaction.name} ({count} instruction{plural})")
        if transaction.required_signer_addresses:
            lines.append(
                f"    Signers: {', '.join(transaction.required_signer_addresses)}"
            )
    return "\n".join(lines)
