"""Protocol definitions, rejections, intents, instances and events.

Port of ``typescript/core/src/protocol/types.ts``. A generated protocol
module (``a4 install protocol <ref> --python``) builds a
:class:`ProtocolDefinition` and typed client classes over it; the hosted
API's JSON documents are returned as plain dictionaries typed with the
``TypedDict`` shapes here.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import (
    Any,
    Dict,
    Generic,
    List,
    Literal,
    Mapping,
    Optional,
    Tuple,
    TypedDict,
    TypeVar,
    Union,
)

ProtocolKeyClass = Literal["publishable", "secret"]
"""The class of API key a protocol client authenticates with."""

ProtocolStepScope = Literal["create", "instance", "participant"]
"""Which machine a step acts on: a new instance, an instance, or a participant."""

ProtocolStepAccess = Literal["public", "secret"]
"""``public`` steps accept publishable keys; ``secret`` steps need a secret key."""

ProtocolMachineKind = Literal["instance", "participant"]
ProtocolTransactionFormat = Literal["v1", "v0"]

ProtocolInteger = Union[str, int]
"""A ``u64``/``u128``/``i64``/unix-timestamp argument: a decimal string or an
``int`` (sent as a decimal string). The hosted API returns decimal strings."""

PROTOCOL_ACCESS_REJECTION_CODES: Tuple[str, ...] = (
    "step_requires_secret_key",
    "step_not_in_session_scope",
    "step_not_client_callable",
)
"""Access refusals a prepare can return instead of a guard rejection."""

ProtocolAccessRejectionCode = Literal[
    "step_requires_secret_key",
    "step_not_in_session_scope",
    "step_not_client_callable",
]

PROTOCOL_STATE_REJECTION_CODES: Tuple[str, ...] = (
    "instance_terminal",
    "participant_terminal",
)
"""Lifecycle rejections a prepare can return instead of a guard rejection.

``instance_terminal``: the instance is in a terminal state.
``participant_terminal``: the preparing wallet's participant is in a terminal
state. Both carry ``detail["state"]``; ``instance_terminal`` is checked first.
Step listings (``allowed_steps``, wallet actions) report the same codes as
``reason``.
"""

ProtocolStateRejectionCode = Literal[
    "instance_terminal",
    "participant_terminal",
]

EXECUTION_INTENT_SCHEMA_V1 = "arete.execution-intent/v1"

TCode = TypeVar("TCode", bound=str)


# ---------------------------------------------------------------------------
# Definitions
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProtocolStepDefinition:
    """Runtime description of one client-callable step."""

    scope: str
    access: str
    guards: Tuple[str, ...] = ()
    """Guard codes the step can reject with, in evaluation order."""
    args: Tuple[str, ...] = ()
    """Argument names."""


@dataclass(frozen=True)
class ProtocolMetadataFieldDefinition:
    """Who may write a metadata field, and whether instances filter by it."""

    writer: str
    filterable: bool = False


@dataclass(frozen=True)
class ProtocolDeployment:
    """Where a protocol deployment is served. Never carries credentials."""

    deployment_id: str
    access: str
    api_url: str
    """Base URL of the hosted API, e.g. ``https://api.arete.run/api``."""
    session_endpoint: str
    """Where session tokens are minted, e.g. ``https://api.arete.run/ws/sessions``."""


@dataclass(frozen=True)
class ProtocolLiveStack:
    """The protocol's own generated stack view that mirrors an instance live."""

    stack: str
    view: str
    key_account: str


@dataclass(frozen=True)
class ProtocolDefinition:
    """A generated protocol module's definition constant
    (``<NAME>_PROTOCOL``). Pass it to :class:`arete.protocol.ProtocolClient`."""

    name: str
    version: str
    package: str
    package_release_hash: str
    protocol_spec_hash: str
    protocol_release_hash: str
    deployment: ProtocolDeployment
    instance_visibility: str
    steps: Mapping[str, ProtocolStepDefinition] = field(default_factory=dict)
    """Client-callable steps only."""
    events: Mapping[str, str] = field(default_factory=dict)
    """Event name -> machine (``instance`` or ``participant``)."""
    instance_metadata: Mapping[str, ProtocolMetadataFieldDefinition] = field(
        default_factory=dict
    )
    participant_metadata: Mapping[str, ProtocolMetadataFieldDefinition] = field(
        default_factory=dict
    )
    live_stack: Optional[ProtocolLiveStack] = None
    stacks: Mapping[str, Any] = field(default_factory=dict)
    """Stacks the protocol reads (``StackDef``), hoisted so a session
    connects each once."""
    programs: Mapping[str, Any] = field(default_factory=dict)
    """Programs the protocol uses (``ProgramDef``), hoisted so a session
    serves each once."""
    kind: str = "protocol"


# ---------------------------------------------------------------------------
# Prepare results
# ---------------------------------------------------------------------------


class ProtocolPreparedTransaction(TypedDict):
    """One partially signed transaction for the wallet to sign."""

    index: int
    transactionBase64: str


class IntentDeploymentV1(TypedDict):
    deploymentId: str
    revision: int
    releaseHash: str


class IntentTransactionV1(TypedDict):
    index: int
    messageSha256: str
    signers: List[str]


class ExecutionIntentV1(TypedDict, total=False):
    """What a prepare committed to: transactions, signers, fees and expiry.

    ``signers`` entries carry a ``class`` key; ``fees`` is ``None`` when the
    step charges none.
    """

    schema: str
    executionId: str
    deployment: IntentDeploymentV1
    step: str
    instanceId: Optional[str]
    participantId: Optional[str]
    wallet: str
    role: Optional[str]
    callerAccount: str
    sessionKeyClass: str
    transactionFormat: str
    inputSha256: str
    resourceHashes: List[str]
    providerSnapshotHash: str
    fees: Optional[Dict[str, Any]]
    signers: List[Dict[str, Any]]
    transactions: List[IntentTransactionV1]
    blockhash: str
    lastValidBlockHeight: str
    expiresAt: int
    createdAt: int


@dataclass(frozen=True)
class ProtocolPrepared:
    """A step prepared: the execution intent and the partially signed
    transactions for the wallet to sign."""

    intent: ExecutionIntentV1
    transactions: List[ProtocolPreparedTransaction]

    @property
    def ok(self) -> Literal[True]:
        return True

    @property
    def execution_id(self) -> str:
        return str(self.intent.get("executionId", ""))


@dataclass(frozen=True)
class ProtocolRejection(Generic[TCode]):
    """A step that did not prepare: a guard failed, access was refused, or
    the instance or participant is in a terminal state.

    ``rejected`` is the guard code, one of
    :data:`PROTOCOL_ACCESS_REJECTION_CODES` or one of
    :data:`PROTOCOL_STATE_REJECTION_CODES`.
    """

    rejected: TCode
    step: str
    guard: Optional[str] = None
    """Source of the guard expression that failed."""
    detail: Optional[Dict[str, Any]] = None
    """The evaluated operands of the guard, or ``{"state": ...}`` for a state
    rejection."""

    @property
    def ok(self) -> Literal[False]:
        return False

    @property
    def is_access_refusal(self) -> bool:
        return self.rejected in PROTOCOL_ACCESS_REJECTION_CODES

    @property
    def is_state_rejection(self) -> bool:
        """True when the instance or the wallet's participant has ended."""
        return self.rejected in PROTOCOL_STATE_REJECTION_CODES


ProtocolPrepareResult = Union[ProtocolPrepared, ProtocolRejection[Any]]


# ---------------------------------------------------------------------------
# Execution intents
# ---------------------------------------------------------------------------

ProtocolIntentStatus = Literal[
    "prepared",
    "submitting",
    "partially_completed",
    "completed",
    "chain_failed",
    "submitted_unknown",
    "expired",
    "rejected",
]


class ProtocolIntentTransactionState(TypedDict, total=False):
    index: int
    signature: Optional[str]
    state: str
    error: Optional[str]


class _ProtocolIntentStateOptional(TypedDict, total=False):
    #: Where the signed receipt of a completed intent can be fetched, when
    #: the API issues one.
    receiptUrl: str


class ProtocolIntentState(_ProtocolIntentStateOptional):
    """An execution intent's state. ``submitted_unknown`` means the outcome
    is not known yet: read it again with ``get_intent``, never submit the same
    transactions again."""

    executionId: str
    state: str
    transactions: List[ProtocolIntentTransactionState]


# ---------------------------------------------------------------------------
# Instances and participants
# ---------------------------------------------------------------------------


class ProtocolStepArgDescriptor(TypedDict, total=False):
    """One declared argument of a step: its name, whether it may be omitted,
    and its field schema as the spec declares it (``enum``, ``minimum``,
    ``maximum``, ``maxLength``, ``items``, ``maxItems``, ``properties`` and
    ``required`` appear when the spec sets them)."""

    name: str
    type: str
    """The schema's JSON type: ``string``, ``integer``, ``boolean``, ``array``
    or ``object``. Solana amounts and keys are strings with a ``format``."""
    format: str
    """The string format, e.g. ``u64``, ``i64``, ``pubkey`` or ``unixTimestamp``."""
    optional: bool
    description: str


class ProtocolStepAvailability(TypedDict, total=False):
    """Whether a step can be called now, and why not."""

    step: str
    allowed: bool
    reason: str
    """Guard code or access code when not allowed."""
    detail: Dict[str, Any]
    argsDependent: List[str]
    """Guards that depend on arguments and were not evaluated."""
    scope: ProtocolStepScope
    """Which machine the step acts on. Absent from older API responses."""
    args: List[ProtocolStepArgDescriptor]
    """The step's typed arguments, as the protocol spec declares them."""


class ProtocolWalletInstanceActions(TypedDict, total=False):
    """One instance a wallet is related to, and the steps it can call there."""

    instanceId: str
    state: str
    relation: List[str]
    """How the wallet relates to the instance: ``participant``, ``creator``,
    or ``role:<name>`` for each role the wallet holds."""
    participantState: Optional[str]
    steps: List[ProtocolStepAvailability]


class ProtocolWalletActions(TypedDict, total=False):
    """What a wallet can do on a deployment: the create steps it can call
    and, per instance it is related to, the steps it can call there."""

    deploymentId: str
    wallet: str
    createSteps: List[ProtocolStepAvailability]
    instances: List[ProtocolWalletInstanceActions]
    nextCursor: Optional[str]
    """Pass as ``cursor`` to continue the instance list."""


class ProtocolInstanceSummary(TypedDict):
    id: str
    deploymentId: str
    revision: int
    state: str
    terminal: bool
    visibility: str
    accounts: Dict[str, str]
    roles: Dict[str, str]
    createdByWallet: str
    createdAt: str
    updatedAt: str
    closedAt: Optional[str]


class ProtocolInstancePage(TypedDict):
    instances: List[ProtocolInstanceSummary]
    nextCursor: Optional[str]


class ProtocolInstanceDetail(ProtocolInstanceSummary):
    releaseHash: str
    params: Dict[str, Any]
    lastErrorCode: Optional[str]
    derived: Any
    metadata: Dict[str, Any]
    metadataVersion: Optional[int]
    pendingTimers: List[Any]
    allowedSteps: List[ProtocolStepAvailability]


class ProtocolParticipantDetail(TypedDict):
    instanceId: str
    wallet: str
    participantId: Optional[str]
    state: Optional[str]
    metadata: Dict[str, Any]
    history: List[Any]
    allowedSteps: List[ProtocolStepAvailability]


@dataclass(frozen=True)
class ProtocolMetadataState:
    metadata: Dict[str, Any]
    version: Optional[int]


class ProtocolMetadataUpdateResult(TypedDict):
    instanceId: str
    participantId: Optional[str]
    version: int
    metadata: Dict[str, Any]


# ---------------------------------------------------------------------------
# Events
# ---------------------------------------------------------------------------


class ProtocolEventPayloadV1(TypedDict):
    """The JSON body of an emitted event, as delivered over webhooks."""

    id: str
    event: str
    deploymentId: str
    instanceId: str
    participantId: Optional[str]
    state: str
    occurredAt: str


SQUADS_PROPOSER_MISSING_EVENT = "arete.squads.proposerMissing"
"""System event: a Squads proposal waits for the integrator's proposer."""

ProtocolSquadsProposerProblem = Literal["not_member", "permissions_invalid"]


class ProtocolSquadsProposerMissingData(TypedDict):
    """Data of ``arete.squads.proposerMissing``: an admin step's proposal is
    parked because the calling account's proposer is not an Initiate-only
    member of the multisig. It is retried with backoff for up to 24 hours;
    adding the proposer (on the dashboard or in the Squads app) unblocks
    it."""

    id: int
    deploymentId: str
    instanceId: str
    outboxId: int
    step: str
    multisig: str
    proposer: str
    problem: ProtocolSquadsProposerProblem
    at: str


@dataclass(frozen=True)
class ProtocolStreamEvent:
    """One event from an instance's event stream.

    ``type`` is ``emit``, ``transition``, ``participant``, ``observation`` or
    a system event such as ``arete.squads.proposerMissing``
    (:data:`SQUADS_PROPOSER_MISSING_EVENT`); ``event`` is the spec's event
    name for ``emit`` events. ``id`` is the resume cursor.
    """

    type: str
    id: str
    data: Dict[str, Any]
    event: Optional[str] = None


# ---------------------------------------------------------------------------
# Squads onboarding
# ---------------------------------------------------------------------------


class ProtocolSquadsProposer(TypedDict):
    """The calling account's Squads proposer for a deployment."""

    pubkey: str
    permissions: Literal["initiate"]
    """Always ``initiate``: the proposer may propose and nothing else."""
    rootKeyId: Optional[str]
    """The root the proposer derives from; ``None`` for an environment key."""


class ProtocolSquadsStatus(TypedDict, total=False):
    """Whether the calling account's proposer can propose to a multisig."""

    ready: bool
    member: bool
    mask: Optional[int]
    threshold: int
    multisig: str
    proposer: str
    problem: ProtocolSquadsProposerProblem
