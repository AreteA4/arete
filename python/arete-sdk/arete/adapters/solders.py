"""Solana transaction adapter backed by ``solders`` (optional ``solana`` extra).

    pip install 'arete-sdk[solana]'

The core SDK stays RPC-free and dependency-free; this module is the only place
in the package that imports a Solana library. It builds, signs and submits
legacy, v0 and transaction-V1 (SIMD-0385) transactions through the Arete
:class:`arete.transactions.TransactionTransport` relay.

Semantics come from the shared wallet contract (:mod:`arete.wallet`) and are
not re-implemented here: option coercion, the version/fee mutual exclusion and
the resource merge all run through :class:`arete.wallet.SendOptions`.

Version-specific budgeting is the one real difference between the versions.
V1 carries compute budget and priority fee inline in the message
(:class:`solders.message.TransactionConfig`); legacy and v0 express the same
ceilings as ``ComputeBudget`` instructions, which this adapter prepends itself
-- a caller-supplied ``ComputeBudget`` instruction is rejected so the typed
configuration stays the single source of truth.
"""

from __future__ import annotations

import asyncio
import base64
import logging
import time
from dataclasses import dataclass, replace
from typing import Any, Dict, List, Optional, Sequence, Tuple

from arete.instructions import BuiltInstruction
from arete.transactions import TransactionTransport, TransactionTransportError
from arete.wallet import (
    CONFIRMATION_LEVELS,
    MAX_TRANSACTION_BYTES,
    V1_MAX_ACCOUNTS,
    V1_MAX_INSTRUCTIONS,
    V1_MAX_SIGNATURES,
    V1_MAX_TRANSACTION_BYTES,
    SendOptions,
    SendResult,
    TransactionFailureOutcome,
    TransactionInspectionResult,
    TransactionResourceOptions,
    WalletError,
    WalletExecutionContext,
    ensure_transaction_version_supported,
)

try:
    from solders.compute_budget import ID as COMPUTE_BUDGET_PROGRAM_ID
    from solders.compute_budget import (
        request_heap_frame,
        set_compute_unit_limit,
        set_compute_unit_price,
        set_loaded_accounts_data_size_limit,
    )
    from solders.hash import Hash
    from solders.instruction import AccountMeta, Instruction
    from solders.keypair import Keypair
    from solders.message import (
        Message,
        MessageV0,
        MessageV1,
        TransactionConfig,
        to_bytes_versioned,
    )
    from solders.pubkey import Pubkey
    from solders.signature import Signature
    from solders.transaction import VersionedTransaction
except ImportError as cause:  # exercised by the blocked-import test
    raise ImportError(
        "arete.adapters.solders requires the optional 'solana' extra: install it "
        "with `pip install 'arete-sdk[solana]'` (solders >= 0.29, Python >= 3.10). "
        "The rest of the SDK works without it."
    ) from cause

__all__ = [
    "SoldersAdapterConfig",
    "SoldersWalletAdapter",
    "to_instruction",
]

#: Every version this adapter can actually build, so an explicit
#: ``transaction_version=1`` is honoured instead of failing closed.
SUPPORTED_TRANSACTION_VERSIONS: Tuple[Any, ...] = ("legacy", 0, 1)

#: Which transport an operation runs on, mirroring the TypeScript adapters'
#: ``AdapterTransportSelection = 'auto' | 'direct' | TransactionTransport``.
#: ``"auto"`` keeps Arete first: the invoking client's transport, then the
#: configured one when the adapter is used standalone. ``"direct"`` inverts
#: that, so the configured transport wins even under a connected client --
#: covering TypeScript's explicit ``'direct'`` and explicit-transport arms,
#: which are the same field here.
TRANSPORT_SELECTIONS: Tuple[str, ...] = ("auto", "direct")

logger = logging.getLogger(__name__)

_COMPUTE_BUDGET_ADDRESS = str(COMPUTE_BUDGET_PROGRAM_ID)
_CONFIRMATION_RANK = {level: rank for rank, level in enumerate(CONFIRMATION_LEVELS)}

# Runtime heap request constraints (``request_heap_frame``): a multiple of 1 KiB
# between 32 KiB and 256 KiB. A malformed request fails the transaction on
# chain, so it is rejected while building instead.
_HEAP_ALIGNMENT = 1024
_MIN_HEAP_BYTES = 32 * 1024
_MAX_HEAP_BYTES = 256 * 1024

#: Per-transaction compute-unit maximum. Also the ceiling a *provisional* V1
#: message declares for a limit the caller omitted, so the simulation reports
#: real consumption instead of failing against a zero budget (SIMD-0385 makes
#: an unset V1 config bit request the *minimum*, not a default).
MAX_COMPUTE_UNIT_LIMIT = 1_400_000

#: Loaded-account-data maximum (64 MiB), used the same way.
MAX_LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024

#: The runtime accounts for loaded data in pages of this size, so an
#: estimated data budget is rounded up to a whole page plus one page of
#: headroom: a snapshot measurement leaves no room for account growth
#: between simulation and execution.
LOADED_ACCOUNTS_DATA_PAGE_BYTES = 32 * 1024

#: Wire keys of the two version-bound V1 budgets, for error messages.
_COMPUTE_UNIT_LIMIT_KEY = "computeUnitLimit"
_LOADED_DATA_KEY = "loadedAccountsDataSizeLimit"


def _estimated_compute_unit_limit(units_consumed: int, margin: int) -> int:
    """Measured consumption plus the configured margin, positive and bounded
    by the protocol maximum."""
    return min(max(units_consumed + margin, 1), MAX_COMPUTE_UNIT_LIMIT)


def _estimated_loaded_accounts_data_size(loaded_bytes: int) -> int:
    """Measured loaded data rounded up to a whole page, plus one page of
    headroom, positive and bounded by the protocol maximum."""
    page = LOADED_ACCOUNTS_DATA_PAGE_BYTES
    padded = (-(-loaded_bytes // page) + 1) * page
    return min(max(padded, 1), MAX_LOADED_ACCOUNTS_DATA_SIZE)


def to_instruction(built: BuiltInstruction) -> Instruction:
    """Convert an Arete :class:`~arete.instructions.BuiltInstruction`."""
    return Instruction(
        Pubkey.from_string(built.program_id),
        bytes(built.data),
        [
            AccountMeta(
                Pubkey.from_string(account.pubkey),
                account.is_signer,
                account.is_writable,
            )
            for account in built.accounts
        ],
    )


def _failure(outcome: TransactionFailureOutcome) -> WalletError:
    return WalletError.from_outcome(outcome)


def _not_submitted(
    message: str, *, phase: str = "build", cause: Any = None
) -> WalletError:
    return _failure(
        TransactionFailureOutcome.not_submitted(phase, message=message, cause=cause)
    )


def _resource_config(
    resources: Optional[TransactionResourceOptions],
) -> Optional[TransactionConfig]:
    """The V1 inline budget, or ``None`` when nothing was requested.

    ``priority_fee`` is the total-lamport priority fee of the V1 contract; the
    other three are byte/unit ceilings.
    """
    if resources is None:
        return None
    fields = (
        resources.priority_fee_lamports,
        resources.compute_unit_limit,
        resources.loaded_accounts_data_size_limit,
        resources.heap_size,
    )
    if all(value is None for value in fields):
        return None
    return TransactionConfig(
        priority_fee=resources.priority_fee_lamports,
        compute_unit_limit=resources.compute_unit_limit,
        loaded_accounts_data_size_limit=resources.loaded_accounts_data_size_limit,
        heap_size=resources.heap_size,
    )


def _compute_budget_instructions(
    resources: Optional[TransactionResourceOptions],
) -> List[Instruction]:
    """The legacy/v0 rendering of the same ceilings."""
    if resources is None:
        return []
    instructions: List[Instruction] = []
    if resources.compute_unit_limit is not None:
        instructions.append(set_compute_unit_limit(resources.compute_unit_limit))
    if resources.compute_unit_price_micro_lamports is not None:
        instructions.append(
            set_compute_unit_price(resources.compute_unit_price_micro_lamports)
        )
    if resources.heap_size is not None:
        instructions.append(request_heap_frame(resources.heap_size))
    if resources.loaded_accounts_data_size_limit is not None:
        instructions.append(
            set_loaded_accounts_data_size_limit(
                resources.loaded_accounts_data_size_limit
            )
        )
    return instructions


def _compile(
    version: Any,
    payer: Pubkey,
    instructions: Sequence[Instruction],
    blockhash: Hash,
    resources: Optional[TransactionResourceOptions],
) -> Any:
    if version == 1:
        return MessageV1.try_compile(
            payer, list(instructions), blockhash, _resource_config(resources)
        )
    budgeted = _compute_budget_instructions(resources) + list(instructions)
    if version == "legacy":
        return Message.new_with_blockhash(budgeted, payer, blockhash)
    return MessageV0.try_compile(payer, budgeted, [], blockhash)


def _unsigned(message: Any) -> VersionedTransaction:
    """A provisional transaction with placeholder signatures.

    Signatures are fixed-width, so this serializes to the exact size of the
    finished transaction: every wire limit can be enforced before anything is
    signed, and the same bytes are what simulation should see.
    """
    return VersionedTransaction.populate(
        message, [Signature.default()] * message.header.num_required_signatures
    )


def _b64(payload: bytes) -> str:
    return base64.b64encode(payload).decode("ascii")


def _non_negative(value: Any, name: str, *, integer: bool = False) -> None:
    """Reject a configured quantity that only fails once it is used.

    ``not value >= 0`` rather than ``value < 0`` so NaN is rejected too.
    """
    allowed = int if integer else (int, float)
    if isinstance(value, bool) or not isinstance(value, allowed) or not value >= 0:
        kind = "integer" if integer else "number"
        raise ValueError(f"{name} must be a non-negative {kind}, got {value!r}")


def _report_signature_mismatch(local: str, echoed: Optional[str]) -> None:
    """Report, never adopt, a relay signature that is not the one signed here.

    The adapter signs the final bytes and derives the signature from them, so
    the local one is authoritative. Reconciling an echoed signature would poll
    a transaction this adapter never submitted: it can report some other
    transaction's status, or never confirm the one that actually went out.

    Logged rather than warned. This runs after the transaction may already be
    on the wire, and ``warnings.warn`` raises under ``-W error``; a raised
    warning here would lose the signature and let the executor classify a
    submitted transaction as never sent, which is the misclassification that
    invites paying twice.
    """
    if echoed and echoed != local:
        logger.warning(
            "The relay reported signature %s for a transaction signed as %s; "
            "the locally derived signature is authoritative and is the one "
            "being reconciled",
            echoed,
            local,
        )


@dataclass(frozen=True)
class SoldersAdapterConfig:
    """Configuration for :class:`SoldersWalletAdapter`.

    ``transport`` is the transport ``transport_selection`` resolves to: the
    fallback under ``"auto"`` (the default, where the executing client's own
    relay wins so a client-driven send keeps using the client's connection),
    and the required backend under ``"direct"``, where it wins over the
    client -- the explicit escape hatch, typically an
    :class:`arete.rpc.RpcTransactionTransport`. TypeScript's third selection
    arm, an explicitly supplied transport, is this same field. The backend is
    chosen once, before the operation; a failure on it never falls back to
    the other.

    ``transaction_version`` and ``resources`` are the adapter's defaults; a
    per-send :class:`arete.wallet.SendOptions` merges over them under the shared
    contract (an override naming either fee clears the other).

    With ``estimate_resources`` the adapter simulates a provisional unsigned
    transaction and fills in the compute-unit limit and loaded-accounts-data-size
    limit that the caller left unset. An explicitly configured value is never
    re-estimated. Transaction V1 requires both before signing (SIMD-0385): pass
    them, or turn estimation on.
    """

    keypair: Keypair
    transport: Optional[TransactionTransport] = None
    transport_selection: str = "auto"
    signers: Tuple[Keypair, ...] = ()
    confirmation_level: str = "confirmed"
    transaction_version: Optional[Any] = None
    resources: Optional[TransactionResourceOptions] = None
    estimate_resources: bool = False
    compute_unit_margin: int = 1_000
    confirmation_timeout: float = 60.0
    poll_interval: float = 0.5

    def __post_init__(self) -> None:
        object.__setattr__(self, "signers", tuple(self.signers))
        # Range-checked here, before anything is signed. ``confirmation_timeout``
        # and ``poll_interval`` are only consumed after the single submission, so
        # a value the arithmetic or the sleep cannot use raises once the
        # transaction is already on the relay -- and an exception carrying no
        # signature is classified not-submitted, which invites a duplicate send
        # of a payment that already landed. A negative interval is no better: it
        # turns reconciliation into an unthrottled poll loop.
        _non_negative(self.confirmation_timeout, "confirmation_timeout")
        _non_negative(self.poll_interval, "poll_interval")
        _non_negative(self.compute_unit_margin, "compute_unit_margin", integer=True)
        # Same shape: ``SendOptions`` treats ``None`` as "unset" and lets it
        # through, but this is the fallback a per-send option falls back TO, so
        # an unset one reaches the confirmation-rank lookup after the send.
        if self.confirmation_level not in CONFIRMATION_LEVELS:
            raise ValueError(
                f"confirmation_level must be one of {CONFIRMATION_LEVELS}, got "
                f"{self.confirmation_level!r}"
            )
        if self.transport_selection not in TRANSPORT_SELECTIONS:
            raise ValueError(
                f"transport_selection must be one of {TRANSPORT_SELECTIONS}, got "
                f"{self.transport_selection!r}"
            )


@dataclass(frozen=True)
class _Plan:
    """Everything the unsigned build produced, shared by inspect and send."""

    version: Any
    commitment: str
    transport: TransactionTransport
    options: SendOptions
    resources: Optional[TransactionResourceOptions]
    message: Any
    unsigned: VersionedTransaction
    last_valid_block_height: Optional[int]


class SoldersWalletAdapter:
    """:class:`arete.wallet.WalletAdapter` over local ``solders`` keypairs.

    Signing is local and non-interactive: there is no wallet prompt to reach,
    and :meth:`inspect_transaction` never signs or submits regardless.
    """

    supported_transaction_versions: Tuple[Any, ...] = SUPPORTED_TRANSACTION_VERSIONS

    def __init__(self, config: SoldersAdapterConfig) -> None:
        self._config = config
        # Validated once, here: a bad confirmation level or an incompatible
        # version/fee default fails at construction, not mid-send.
        self._defaults = SendOptions(
            confirmation_level=config.confirmation_level,
            transaction_version=config.transaction_version,
            resources=config.resources,
        ).validate()
        keypairs: Dict[str, Keypair] = {}
        for keypair in (config.keypair, *config.signers):
            keypairs.setdefault(str(keypair.pubkey()), keypair)
        self._keypairs = keypairs
        self.public_key: str = str(config.keypair.pubkey())
        self.signer_addresses: Tuple[str, ...] = tuple(keypairs)

    # -- planning ----------------------------------------------------------

    def _transport(
        self, context: Optional[WalletExecutionContext]
    ) -> TransactionTransport:
        """Pick the backend once, before anything is built.

        ``"auto"`` keeps Arete first -- the connection that owns the session
        wins, and the configured transport is the standalone fallback.
        ``"direct"`` inverts that for the explicit escape hatch. Neither
        falls back to the other after a failure.
        """
        if self._config.transport_selection == "direct":
            if self._config.transport is None:
                raise _not_submitted(
                    "transport_selection='direct' requires "
                    "SoldersAdapterConfig(transport=...) -- for example an "
                    "arete.rpc.RpcTransactionTransport"
                )
            return self._config.transport
        transport = context.transaction_transport if context is not None else None
        if transport is None:
            transport = self._config.transport
        if transport is None:
            raise _not_submitted(
                "No transaction transport is available: send through a connected "
                "Arete client or set SoldersAdapterConfig(transport=...)"
            )
        return transport

    def resolve_send_options(self, options: Any) -> SendOptions:
        """Merge this adapter's defaults under a per-call override.

        The public adapter hook: a caller that validates options before
        dispatch (``Arete.transaction`` / ``Arete.execute`` /
        ``inspect_prepared_operation``) resolves them here first, so the
        version/fee pair is checked against the version this adapter will
        actually compile. Without it a V1-defaulted adapter rejects a valid
        ``priorityFeeLamports`` as a v0 mismatch before it is ever reached.
        """
        return self._effective_options(options)

    def _effective_options(self, options: Any) -> SendOptions:
        # ``merged`` validates the effective version/fee pair; a per-call
        # mapping is coerced first so camelCase resource keys and unknown
        # resource keys behave exactly as they do in the core contract.
        effective = self._defaults.merged(SendOptions.coerce(options))
        ensure_transaction_version_supported(self, effective.transaction_version)
        return effective

    def _convert(
        self, instructions: Sequence[BuiltInstruction]
    ) -> List[Instruction]:
        if not instructions:
            raise _not_submitted("A transaction requires at least one instruction")
        for built in instructions:
            if built.program_id == _COMPUTE_BUDGET_ADDRESS:
                raise _not_submitted(
                    "Caller-supplied ComputeBudget instructions are rejected: use the "
                    "typed resource options (computeUnitLimit, heapSize, "
                    "loadedAccountsDataSizeLimit, priorityFeeLamports for V1, "
                    "computeUnitPriceMicroLamports for legacy/v0) so the adapter owns "
                    "the budget for every version"
                )
        try:
            return [to_instruction(built) for built in instructions]
        except Exception as cause:
            raise _not_submitted(
                f"Instruction is not valid for a Solana transaction: {cause}",
                cause=cause,
            ) from cause

    def _check_lookup_tables(self, options: SendOptions, version: Any) -> None:
        for key in options.extra:
            if "lookup" not in str(key).lower():
                continue
            if version == 1:
                raise _not_submitted(
                    f"Transaction V1 does not support address lookup tables "
                    f"(rejected send option {key!r}): V1 resolves accounts inline"
                )
            raise _not_submitted(
                f"This adapter does not compile address lookup tables "
                f"(rejected send option {key!r})"
            )

    def _check_resources(
        self, resources: Optional[TransactionResourceOptions]
    ) -> None:
        heap_size = None if resources is None else resources.heap_size
        if heap_size is None:
            return
        if (
            heap_size % _HEAP_ALIGNMENT
            or heap_size < _MIN_HEAP_BYTES
            or heap_size > _MAX_HEAP_BYTES
        ):
            raise _not_submitted(
                f"heapSize must be a multiple of {_HEAP_ALIGNMENT} between "
                f"{_MIN_HEAP_BYTES} and {_MAX_HEAP_BYTES} bytes, got {heap_size}"
            )

    def _enforce_limits(self, version: Any, plan_message: Any, wire: int) -> None:
        limit = V1_MAX_TRANSACTION_BYTES if version == 1 else MAX_TRANSACTION_BYTES
        if wire > limit:
            raise _not_submitted(
                f"Encoded transaction is {wire} bytes, over the "
                f"{limit}-byte limit for version {version!r}"
            )
        if version != 1:
            return
        signatures = plan_message.header.num_required_signatures
        if signatures > V1_MAX_SIGNATURES:
            raise _not_submitted(
                f"Transaction V1 allows at most {V1_MAX_SIGNATURES} required "
                f"signatures, got {signatures}"
            )
        accounts = len(plan_message.account_keys)
        if accounts > V1_MAX_ACCOUNTS:
            raise _not_submitted(
                f"Transaction V1 allows at most {V1_MAX_ACCOUNTS} accounts, "
                f"got {accounts}"
            )
        instructions = len(plan_message.instructions)
        if instructions > V1_MAX_INSTRUCTIONS:
            raise _not_submitted(
                f"Transaction V1 allows at most {V1_MAX_INSTRUCTIONS} top-level "
                f"instructions, got {instructions}"
            )

    async def _plan(
        self,
        instructions: Sequence[BuiltInstruction],
        options: Any,
        context: Optional[WalletExecutionContext],
        *,
        signing: bool,
    ) -> _Plan:
        """Build the provisional unsigned transaction. Never signs, never sends."""
        options = self._effective_options(options)
        version = (
            0 if options.transaction_version is None else options.transaction_version
        )
        commitment = options.confirmation_level or self._config.confirmation_level
        transport = self._transport(context)
        self._check_lookup_tables(options, version)
        converted = self._convert(instructions)
        resources = options.resources
        self._check_resources(resources)
        payer = self._config.keypair.pubkey()

        try:
            latest = await transport.get_latest_blockhash(commitment=commitment)
            blockhash = Hash.from_string(latest.blockhash)
        except WalletError:
            raise
        except Exception as cause:
            raise _not_submitted(
                f"Could not fetch a recent blockhash: {cause}", cause=cause
            ) from cause

        resources = await self._resolve_resources(
            transport,
            version,
            payer,
            converted,
            blockhash,
            resources,
            commitment,
            signing,
        )
        self._check_resources(resources)
        message = self._compile(version, payer, converted, blockhash, resources)
        unsigned = _unsigned(message)
        self._enforce_limits(version, message, len(bytes(unsigned)))
        return _Plan(
            version=version,
            commitment=commitment,
            transport=transport,
            options=options,
            resources=resources,
            message=message,
            unsigned=unsigned,
            last_valid_block_height=latest.last_valid_block_height,
        )

    def _compile(
        self,
        version: Any,
        payer: Pubkey,
        instructions: Sequence[Instruction],
        blockhash: Hash,
        resources: Optional[TransactionResourceOptions],
    ) -> Any:
        try:
            return _compile(version, payer, instructions, blockhash, resources)
        except Exception as cause:
            raise _not_submitted(
                f"Could not compile a version {version!r} message: {cause}",
                cause=cause,
            ) from cause

    async def _resolve_resources(
        self,
        transport: TransactionTransport,
        version: Any,
        payer: Pubkey,
        instructions: Sequence[Instruction],
        blockhash: Hash,
        resources: Optional[TransactionResourceOptions],
        commitment: str,
        signing: bool,
    ) -> Optional[TransactionResourceOptions]:
        """Resolve the budgets the built message will carry.

        An explicit caller value is authoritative and is never re-estimated
        or raised. An unset one is measured by simulating a *provisional*
        message, then derived with headroom and protocol bounds.

        V1 is the strict case (SIMD-0385): an omitted compute-unit limit
        requests **zero** compute units and an omitted loaded-accounts limit
        requests **zero** bytes, so a final V1 message missing either could
        only fail on chain. Both are therefore always resolved for V1 --
        supplied by the caller, or measured, whatever ``estimate_resources``
        says -- and a metric the simulation never reported is refused by
        name rather than left unset. legacy/v0 express the same ceilings as
        ``ComputeBudget`` instructions, where an omitted one means the
        runtime's own default, so there estimation stays opt-in and an
        unmeasurable ceiling is simply left to the runtime.

        Inspection (``signing=False``) never refuses: an unresolved V1
        ceiling becomes the protocol maximum, which is exactly the
        provisional message whose reported metrics let the caller pin the
        budgets themselves.
        """
        needed = [
            key
            for key, value in (
                (
                    _COMPUTE_UNIT_LIMIT_KEY,
                    None if resources is None else resources.compute_unit_limit,
                ),
                (
                    _LOADED_DATA_KEY,
                    None
                    if resources is None
                    else resources.loaded_accounts_data_size_limit,
                ),
            )
            if value is None
        ]
        if not needed:
            return resources

        updates: Dict[str, int] = {}
        unmeasured: Dict[str, str] = {}
        # V1 signing must resolve both budgets, so it always measures.
        # Inspection stays a single round trip unless estimation is
        # configured: the maxima below are the provisional message whose
        # reported metrics are the point of inspecting.
        if self._config.estimate_resources or (signing and version == 1):
            simulation = await self._simulate_provisional(
                transport,
                version,
                payer,
                instructions,
                blockhash,
                resources,
                commitment,
            )
            if _COMPUTE_UNIT_LIMIT_KEY in needed:
                if simulation.units_consumed is None:
                    unmeasured[_COMPUTE_UNIT_LIMIT_KEY] = "unitsConsumed"
                else:
                    updates["compute_unit_limit"] = _estimated_compute_unit_limit(
                        simulation.units_consumed, self._config.compute_unit_margin
                    )
            if _LOADED_DATA_KEY in needed:
                if simulation.loaded_accounts_data_size is None:
                    unmeasured[_LOADED_DATA_KEY] = "loadedAccountsDataSize"
                else:
                    updates["loaded_accounts_data_size_limit"] = (
                        _estimated_loaded_accounts_data_size(
                            simulation.loaded_accounts_data_size
                        )
                    )
        else:
            unmeasured = {key: "" for key in needed}

        if version == 1 and unmeasured:
            if signing:
                raise self._unresolved_v1_budgets(unmeasured)
            # Provisional: the maxima stand in for what is still unresolved.
            if _COMPUTE_UNIT_LIMIT_KEY in unmeasured:
                updates["compute_unit_limit"] = MAX_COMPUTE_UNIT_LIMIT
            if _LOADED_DATA_KEY in unmeasured:
                updates["loaded_accounts_data_size_limit"] = (
                    MAX_LOADED_ACCOUNTS_DATA_SIZE
                )
        if not updates:
            # legacy/v0 only: an unmeasured ceiling is the runtime's default.
            return resources
        base = resources if resources is not None else TransactionResourceOptions()
        return replace(base, **updates)

    @staticmethod
    def _unresolved_v1_budgets(unmeasured: Dict[str, str]) -> WalletError:
        """Why each still-unresolved V1 budget cannot be signed for."""
        return _not_submitted(
            "Transaction version 1 requires "
            + " and ".join(unmeasured)
            + ": an omitted V1 budget requests the minimum (SIMD-0385), so the "
            "transaction could only fail on chain. "
            + "; ".join(
                f"{option} was not supplied and the simulation reported no {metric}"
                for option, metric in unmeasured.items()
            )
            + ". Pass the value explicitly, or use a relay whose simulation "
            "reports it."
        )

    async def _simulate_provisional(
        self,
        transport: TransactionTransport,
        version: Any,
        payer: Pubkey,
        instructions: Sequence[Instruction],
        blockhash: Hash,
        resources: Optional[TransactionResourceOptions],
        commitment: str,
    ) -> Any:
        """Simulate the message whose budgets are the ones being measured.

        For V1 the ceilings the caller omitted declare the protocol maxima
        here: signature verification being off does not lift a resource
        limit, so a provisional message carrying the SIMD-0385 minimum could
        not produce an ordinary successful execution estimate. The caller's
        own values are preserved untouched.

        A failure is a build failure: nothing was signed, so nothing can be
        reported as submitted.
        """
        provisional = resources
        if version == 1:
            base = resources if resources is not None else TransactionResourceOptions()
            provisional = replace(
                base,
                compute_unit_limit=(
                    base.compute_unit_limit
                    if base.compute_unit_limit is not None
                    else MAX_COMPUTE_UNIT_LIMIT
                ),
                loaded_accounts_data_size_limit=(
                    base.loaded_accounts_data_size_limit
                    if base.loaded_accounts_data_size_limit is not None
                    else MAX_LOADED_ACCOUNTS_DATA_SIZE
                ),
            )
        message = self._compile(version, payer, instructions, blockhash, provisional)
        try:
            simulation = await transport.simulate_transaction(
                _b64(bytes(_unsigned(message))),
                commitment=commitment,
                replace_recent_blockhash=False,
            )
        except WalletError:
            raise
        except Exception as cause:
            raise _not_submitted(
                f"Could not estimate transaction resources: {cause}", cause=cause
            ) from cause
        if simulation.err is not None:
            # Deliberately not attached as a cause: an unsigned simulation error
            # must never be classified as a submitted transaction.
            raise _not_submitted(
                "Unsigned resource estimation simulation failed: "
                f"{simulation.err!r}"
            )
        return simulation

    # -- inspection --------------------------------------------------------

    async def inspect_transaction(
        self,
        instructions: Sequence[BuiltInstruction],
        options: Any = None,
        context: Optional[WalletExecutionContext] = None,
    ) -> TransactionInspectionResult:
        """Fee and simulation for the unsigned transaction. Never signs or sends."""
        plan = await self._plan(instructions, options, context, signing=False)
        # A relay outage or a malformed response must not escape as a raw
        # transport or parse error: the wallet contract is WalletError, and
        # inspection never signed anything, so it is a build failure.
        try:
            fee = await plan.transport.get_fee_for_message(
                _b64(to_bytes_versioned(plan.message)), commitment=plan.commitment
            )
            simulation = await plan.transport.simulate_transaction(
                _b64(bytes(plan.unsigned)), commitment=plan.commitment
            )
        except WalletError:
            raise
        except Exception as cause:
            raise _not_submitted(
                f"Could not inspect the unsigned transaction: {cause}", cause=cause
            ) from cause
        return TransactionInspectionResult(
            fee_lamports=fee.fee_lamports,
            logs=None if simulation.logs is None else tuple(simulation.logs),
            compute_units_consumed=simulation.units_consumed,
            context_slot=simulation.context_slot,
            error=simulation.err,
            loaded_accounts_data_size=simulation.loaded_accounts_data_size,
            extra={
                "transactionVersion": plan.version,
                "feeContextSlot": fee.context_slot,
                "wireBytes": len(bytes(plan.unsigned)),
                "resources": (
                    {} if plan.resources is None else plan.resources.to_wire()
                ),
            },
        )

    # -- signing and sending -----------------------------------------------

    def _sign(self, plan: _Plan) -> VersionedTransaction:
        """Sign with every required signer. Local only: nothing prompts."""
        try:
            per_send = {
                str(signer.pubkey()): signer for signer in (plan.options.signers or ())
            }
        except AttributeError as cause:
            raise _failure(
                TransactionFailureOutcome.not_submitted(
                    "wallet",
                    message=(
                        "Per-send signers must be solders keypairs exposing "
                        f"pubkey(): {cause}"
                    ),
                    cause=cause,
                )
            ) from cause
        signers = []
        missing = []
        for account in plan.message.account_keys[
            : plan.message.header.num_required_signatures
        ]:
            address = str(account)
            keypair = per_send.get(address) or self._keypairs.get(address)
            if keypair is None:
                missing.append(address)
            else:
                signers.append(keypair)
        if missing:
            raise _failure(
                TransactionFailureOutcome.not_submitted(
                    "wallet",
                    message=(
                        "Missing signer(s) for transaction: " + ", ".join(missing)
                    ),
                )
            )
        try:
            return VersionedTransaction(plan.message, signers)
        except Exception as cause:
            raise _failure(
                TransactionFailureOutcome.not_submitted(
                    "wallet",
                    message=f"Could not sign the transaction: {cause}",
                    cause=cause,
                )
            ) from cause

    async def sign_and_send(
        self,
        instructions: Sequence[BuiltInstruction],
        options: Any = None,
        context: Optional[WalletExecutionContext] = None,
    ) -> SendResult:
        plan = await self._plan(instructions, options, context, signing=True)
        transaction = self._sign(plan)
        signature = str(transaction.signatures[0])

        # Exactly one submission. Anything ambiguous is reconciled by signature;
        # the transaction is never rebuilt, re-signed or resent.
        try:
            sent = await plan.transport.send_transaction(
                _b64(bytes(transaction)),
                skip_preflight=bool(plan.options.skip_preflight),
                preflight_commitment=plan.commitment,
            )
        except TransactionTransportError as cause:
            if cause.submission_state == "not_submitted":
                raise _not_submitted(
                    f"The relay rejected the transaction: {cause.message}",
                    phase="send",
                    cause=cause,
                ) from cause
            _report_signature_mismatch(signature, cause.signature)
            raise _failure(
                TransactionFailureOutcome.submitted_unknown(
                    signature, phase="send", cause=cause
                )
            ) from cause
        except Exception as cause:
            raise _failure(
                TransactionFailureOutcome.submitted_unknown(
                    signature, phase="send", cause=cause
                )
            ) from cause

        _report_signature_mismatch(signature, sent.signature)
        return await self._reconcile(plan, signature)

    async def _reconcile(self, plan: _Plan, signature: str) -> SendResult:
        """Poll the submitted signature until it settles, expires or times out."""
        wanted = _CONFIRMATION_RANK[plan.commitment]
        deadline = time.monotonic() + self._config.confirmation_timeout
        while True:
            try:
                status = await plan.transport.get_signature_status(
                    signature, search_transaction_history=True
                )
                if status is not None:
                    if status.err is not None:
                        raise _failure(
                            TransactionFailureOutcome.chain_failed(
                                phase="chain",
                                signature=signature,
                                slot=status.slot,
                                cause=status.err,
                            )
                        )
                    reached = _CONFIRMATION_RANK.get(
                        status.confirmation_status or "", -1
                    )
                    if reached >= wanted:
                        return SendResult(signature=signature, slot=status.slot)
                if plan.last_valid_block_height is not None:
                    height = await plan.transport.get_block_height(
                        commitment=plan.commitment
                    )
                    if height > plan.last_valid_block_height:
                        raise _failure(
                            TransactionFailureOutcome.submitted_unknown(
                                signature,
                                phase="confirmation",
                                slot=None if status is None else status.slot,
                                message=(
                                    f"Transaction {signature} was submitted but its "
                                    "blockhash expired before confirmation"
                                ),
                            )
                        )
            except WalletError:
                raise
            except Exception as cause:
                raise _failure(
                    TransactionFailureOutcome.submitted_unknown(
                        signature, phase="confirmation", cause=cause
                    )
                ) from cause
            if time.monotonic() >= deadline:
                raise _failure(
                    TransactionFailureOutcome.submitted_unknown(
                        signature,
                        phase="confirmation",
                        message=(
                            f"Transaction {signature} was submitted but its status "
                            f"was still unknown after "
                            f"{self._config.confirmation_timeout}s"
                        ),
                    )
                )
            await asyncio.sleep(self._config.poll_interval)
