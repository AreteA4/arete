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

_U32_MAX = 0xFFFF_FFFF
_COMPUTE_BUDGET_ADDRESS = str(COMPUTE_BUDGET_PROGRAM_ID)
_CONFIRMATION_RANK = {level: rank for rank, level in enumerate(CONFIRMATION_LEVELS)}

# Runtime heap request constraints (``request_heap_frame``): a multiple of 1 KiB
# between 32 KiB and 256 KiB. A malformed request fails the transaction on
# chain, so it is rejected while building instead.
_HEAP_ALIGNMENT = 1024
_MIN_HEAP_BYTES = 32 * 1024
_MAX_HEAP_BYTES = 256 * 1024


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


@dataclass(frozen=True)
class SoldersAdapterConfig:
    """Configuration for :class:`SoldersWalletAdapter`.

    ``transport`` is the fallback relay used when the executing client does not
    supply one through :class:`arete.wallet.WalletExecutionContext`; the context
    transport always wins so a client-driven send keeps using the client's
    relay.

    ``transaction_version`` and ``resources`` are the adapter's defaults; a
    per-send :class:`arete.wallet.SendOptions` merges over them under the shared
    contract (an override naming either fee clears the other).

    With ``estimate_resources`` the adapter simulates the unsigned transaction
    and fills in the compute-unit limit and loaded-accounts-data-size limit that
    the caller left unset. An explicitly configured value is never re-estimated.
    """

    keypair: Keypair
    transport: Optional[TransactionTransport] = None
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
        transport = context.transaction_transport if context is not None else None
        if transport is None:
            transport = self._config.transport
        if transport is None:
            raise _not_submitted(
                "No transaction transport is available: send through a connected "
                "Arete client or set SoldersAdapterConfig(transport=...)"
            )
        return transport

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

        message = self._compile(version, payer, converted, blockhash, resources)
        if self._config.estimate_resources:
            estimated = await self._estimate(
                transport, message, resources, commitment
            )
            if estimated is not resources:
                resources = estimated
                self._check_resources(resources)
                message = self._compile(
                    version, payer, converted, blockhash, resources
                )
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

    async def _estimate(
        self,
        transport: TransactionTransport,
        message: Any,
        resources: Optional[TransactionResourceOptions],
        commitment: str,
    ) -> Optional[TransactionResourceOptions]:
        """Fill unset compute ceilings from a simulation of the unsigned message.

        An explicit value is authoritative and skips the round trip entirely.
        A simulation failure is a build failure: the transaction was never
        signed, so it can never be reported as submitted. The simulated message
        is the pre-estimate one, so the estimate is a lower bound on the final
        message's own cost -- that is what ``compute_unit_margin`` covers.
        """
        wanted = resources is None or (
            resources.compute_unit_limit is None
            or resources.loaded_accounts_data_size_limit is None
        )
        if not wanted:
            return resources
        try:
            simulation = await transport.simulate_transaction(
                _b64(bytes(_unsigned(message))),
                commitment=commitment,
                replace_recent_blockhash=False,
            )
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
        updates: Dict[str, int] = {}
        if (
            resources is None or resources.compute_unit_limit is None
        ) and simulation.units_consumed is not None:
            updates["compute_unit_limit"] = min(
                simulation.units_consumed + self._config.compute_unit_margin, _U32_MAX
            )
        if (
            resources is None or resources.loaded_accounts_data_size_limit is None
        ) and simulation.loaded_accounts_data_size is not None:
            updates["loaded_accounts_data_size_limit"] = (
                simulation.loaded_accounts_data_size
            )
        if not updates:
            # A relay that reports no metrics leaves the request untouched.
            return resources
        base = resources if resources is not None else TransactionResourceOptions()
        return replace(base, **updates)

    # -- inspection --------------------------------------------------------

    async def inspect_transaction(
        self,
        instructions: Sequence[BuiltInstruction],
        options: Any = None,
        context: Optional[WalletExecutionContext] = None,
    ) -> TransactionInspectionResult:
        """Fee and simulation for the unsigned transaction. Never signs or sends."""
        plan = await self._plan(instructions, options, context)
        fee = await plan.transport.get_fee_for_message(
            _b64(to_bytes_versioned(plan.message)), commitment=plan.commitment
        )
        simulation = await plan.transport.simulate_transaction(
            _b64(bytes(plan.unsigned)), commitment=plan.commitment
        )
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
        plan = await self._plan(instructions, options, context)
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
            raise _failure(
                TransactionFailureOutcome.submitted_unknown(
                    cause.signature or signature, phase="send", cause=cause
                )
            ) from cause
        except Exception as cause:
            raise _failure(
                TransactionFailureOutcome.submitted_unknown(
                    signature, phase="send", cause=cause
                )
            ) from cause

        return await self._reconcile(plan, sent.signature or signature)

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
