"""Stack binding model and the connected program runtime.

The data half (:class:`StackDef` / :class:`ProgramDef`) is what generated
Python stack bindings instantiate: pure data + pure functions, no I/O —
mirror of the TS ``StackDefinition`` / ``ProgramSdkDefinition``
(``typescript/core/src/types.ts``) and Rust ``program.rs``.

The runtime half (:class:`ConnectedProgram`, :class:`ProgramsNamespace`)
binds a :class:`ProgramDef` to a connected client:

- ``programs.<name>.raw.<ix>.build(**params)`` — pure instruction building
  (fail-closed kwargs, payer defaulting to the client wallet).
- ``programs.<name>.pdas.<name>.derive(**seeds)`` — typed PDA factories.
- ``programs.<name>.accounts.<name>`` — :class:`arete.read.AccountReader`
  over the program's release-addressed read transport.
- ``programs.<name>.instructions/.transactions/.flows.<path>.prepare(**input)``
  — semantic operations created by extensions with access to the fully
  connected program.
- ``programs.<name>.errors`` / ``.parse_error(code)`` — error metadata.

Namespaces are attribute-accessed objects (dynamic ``__getattr__`` over the
definition maps with helpful ``AttributeError``\\ s).
"""

from __future__ import annotations

import inspect as _inspect
import logging
from dataclasses import dataclass, field
from typing import (
    Any,
    Callable,
    Dict,
    Mapping,
    Optional,
    Tuple,
)

from arete.errors import ProgramKeyConflictError

logger = logging.getLogger(__name__)
from arete.instructions import (
    AccountRefSeed,
    ArgRefSeed,
    BuiltInstruction,
    ErrorMetadata,
    InstructionHandler,
    PdaConfig,
    derive_pda,
    parse_program_error,
)
from arete.gateway import HostedSolanaGatewayBindings
from arete.program_read_transport import ProgramReadDescriptor
from arete.read import (
    AccountReader,
    ProgramAccountReadDef,
    ProgramQueryDef,
    ProgramReadTransport,
    QueryExecutor,
    StackQueryDef,
)
from arete.views import ViewDef

__all__ = [
    "StackEndpoints",
    "StackRelease",
    "StackDef",
    "ProgramDef",
    "ProgramOperations",
    "ProgramOperationContext",
    "Operation",
    "instruction_operation",
    "transaction_operation",
    "flow_operation",
    "normalize_program_operations",
    "AttrNamespace",
    "OperationNamespace",
    "RawInstruction",
    "PdaFactory",
    "ConnectedProgram",
    "ProgramsNamespace",
    "compare_program_identity",
    "same_program",
    "with_programs",
]


@dataclass(frozen=True)
class StackEndpoints:
    """Deployment endpoints of a stack."""

    ws: str = ""
    http: Optional[str] = None


@dataclass(frozen=True)
class StackRelease:
    """The exact served version a generated stack was built for: one live
    alias of one StackManifest.

    Generated only from a hosted StackManifest. It is sent with the WebSocket
    session request for ``endpoints.ws`` so the session endpoint can route the
    client to that version.
    """

    #: ``arete:h1:stack-manifest:sha256:<64 hex>``
    stack_manifest_hash: str
    #: The StackManifest live alias this stack serves.
    live_alias: str

    def __post_init__(self) -> None:
        if not isinstance(self.stack_manifest_hash, str) or not self.stack_manifest_hash:
            raise ValueError("StackRelease.stack_manifest_hash must be a non-empty string")
        if not isinstance(self.live_alias, str) or not self.live_alias:
            raise ValueError("StackRelease.live_alias must be a non-empty string")


@dataclass
class ProgramDef:
    """Generated, portable description of one program SDK bundled with a
    stack. Pure data + pure functions; no I/O.

    ``create_operations`` is the semantic-operation factory installed by
    extensions (:func:`arete.extensions.extend_program`): it receives a
    :class:`ProgramOperationContext` (chain + live wallet + the fully
    connected program) and returns :class:`ProgramOperations` (or a mapping
    with ``instructions`` / ``transactions`` / ``flows`` keys) whose leaves
    are :class:`Operation` values.

    ``create_read`` is the program read factory installed the same way (TS
    ``createRead``): it receives the same context and returns a mapping of
    async callables, surfaced as the connected program's ``read`` namespace
    before ``create_operations`` runs, so operations can use it.
    """

    name: str
    program_id: str
    raw_instructions: Dict[str, InstructionHandler] = field(default_factory=dict)
    pdas: Dict[str, PdaConfig] = field(default_factory=dict)
    accounts: Dict[str, ProgramAccountReadDef] = field(default_factory=dict)
    queries: Dict[str, ProgramQueryDef] = field(default_factory=dict)
    errors: Tuple[ErrorMetadata, ...] = ()
    addresses: Dict[str, Any] = field(default_factory=dict)
    constants: Dict[str, Any] = field(default_factory=dict)
    defaults: Dict[str, Any] = field(default_factory=dict)
    math: Dict[str, Any] = field(default_factory=dict)
    create_operations: Optional[
        Callable[["ProgramOperationContext"], Any]
    ] = None
    create_read: Optional[
        Callable[["ProgramOperationContext"], Mapping[str, Any]]
    ] = None
    # Provenance hashes (pin-validated by the extensions pipeline).
    program_spec_hash: Optional[str] = None
    sdk_definition_hash: Optional[str] = None
    # Package release this program SDK was generated from, when known. Two
    # definitions carrying the same one are the same program (see
    # :func:`same_program`); local builds leave it unset.
    package_release_hash: Optional[str] = None
    # Managed-hosting transports for standalone program sessions.
    gateway: Optional[HostedSolanaGatewayBindings] = None


@dataclass
class StackDef:
    """Generated, portable description of one stack: name, endpoints, view
    definitions, program definitions, program-read descriptors, optional
    hosted Solana gateway bindings, and extension namespaces.

    ``views`` maps entity group name → ``{view_name: ViewDef}`` (the shape
    :class:`arete.views.ViewsNamespace` consumes). ``program_reads`` keys must
    exactly match ``programs`` keys when present. ``gateway`` bindings wire
    default chain + transaction transports at connect time. ``release`` is
    the served version the stack was generated for (hosted StackManifests
    only).
    """

    name: str
    endpoints: StackEndpoints = field(default_factory=StackEndpoints)
    views: Dict[str, Dict[str, ViewDef]] = field(default_factory=dict)
    programs: Dict[str, ProgramDef] = field(default_factory=dict)
    program_reads: Dict[str, ProgramReadDescriptor] = field(default_factory=dict)
    queries: Dict[str, StackQueryDef] = field(default_factory=dict)
    gateway: Optional[HostedSolanaGatewayBindings] = None
    # Extension namespaces (installed by arete.extensions.extend_stack).
    addresses: Dict[str, Any] = field(default_factory=dict)
    constants: Dict[str, Any] = field(default_factory=dict)
    defaults: Dict[str, Any] = field(default_factory=dict)
    math: Dict[str, Any] = field(default_factory=dict)
    read_arg_counts: Dict[str, Any] = field(default_factory=dict)
    create_read: Optional[Callable[[Any], Mapping[str, Any]]] = None
    create_flows: Optional[Callable[[Any], Mapping[str, Any]]] = None
    # Served version for the WebSocket session request (hosted only).
    release: Optional[StackRelease] = None


def _non_empty(value: Any) -> Optional[str]:
    return value if isinstance(value, str) and value else None


def compare_program_identity(left: ProgramDef, right: ProgramDef) -> str:
    """Canonical §9 program identity, never by name (TS
    ``compareProgramIdentity``):

    - ``"same"``: the same definition object, or both carry a
      ``package_release_hash`` and the hashes are equal;
    - ``"unproven"``: at least one has no ``package_release_hash`` (a local
      build, or a program extended outside its generated SDK) but both carry
      the same ``program_spec_hash``;
    - ``"different"``: both carry package releases that differ, or the program
      specs differ or are missing.
    """
    if left is right:
        return "same"
    left_release = _non_empty(left.package_release_hash)
    right_release = _non_empty(right.package_release_hash)
    if left_release is not None and right_release is not None:
        return "same" if left_release == right_release else "different"
    left_spec = _non_empty(left.program_spec_hash)
    if left_spec is not None and left_spec == _non_empty(right.program_spec_hash):
        return "unproven"
    return "different"


def same_program(left: ProgramDef, right: ProgramDef) -> bool:
    """Whether two definitions are provably the same program SDK: the same
    definition object, or both carry the same ``package_release_hash``.
    Names never decide it."""
    return compare_program_identity(left, right) == "same"


def program_identity_label(program: ProgramDef) -> str:
    """How a conflict or warning message names a program's identity."""
    release = _non_empty(program.package_release_hash)
    if release is not None:
        return f"package release {release}"
    spec = _non_empty(program.program_spec_hash)
    if spec is not None:
        return f"program spec {spec}, no package release"
    return "no program identity"


_warned_identity_messages: set = set()


def warn_program_identity_once(message: str) -> None:
    """Log a program identity warning once per distinct message."""
    if message in _warned_identity_messages:
        return
    _warned_identity_messages.add(message)
    logger.warning("%s", message)


def with_programs(
    stack: StackDef, attached: Optional[Mapping[str, ProgramDef]]
) -> StackDef:
    """A copy of ``stack`` with additional program SDKs attached (TS
    ``withPrograms``), matched by :func:`compare_program_identity` under a key
    the stack already provides:

    - the same program keeps the stack's definition;
    - the same program spec without a provable identity match uses the
      attached program, with one logged warning;
    - anything else raises :class:`arete.errors.ProgramKeyConflictError`.

    The ``stack`` definition itself is never changed.
    """
    if not attached:
        return stack
    import copy

    merged: Dict[str, ProgramDef] = dict(attached)
    for name, definition in stack.programs.items():
        other = merged.get(name)
        if other is None:
            merged[name] = definition
            continue
        match = compare_program_identity(definition, other)
        if match == "unproven":
            warn_program_identity_once(
                f"programs.{name} uses the program attached to stack '{stack.name}': "
                f"it has the same program spec as the stack's '{name}' program but "
                "could not be proven identical (stack: "
                f"{program_identity_label(definition)}; attached: "
                f"{program_identity_label(other)})."
            )
            continue
        if match == "different":
            raise ProgramKeyConflictError(
                f"Program key '{name}' conflicts with stack '{stack.name}': the stack "
                f"already provides a different '{name}' program SDK (stack: "
                f"{program_identity_label(definition)}; attached: "
                f"{program_identity_label(other)}). Use the stack's program at "
                f"programs.{name}, or attach the other program under a different key, "
                "one the stack does not use.",
                key=name,
                stacks=[stack.name],
            )
        merged[name] = definition
    cloned = copy.copy(stack)
    cloned.programs = merged
    return cloned


# ---------------------------------------------------------------------------
# Attribute namespaces
# ---------------------------------------------------------------------------


class AttrNamespace:
    """Attribute access over a definition map with helpful errors.

    Nested mappings are wrapped lazily into nested namespaces; every other
    value is returned as-is.
    """

    def __init__(self, label: str, entries: Mapping[str, Any]) -> None:
        self._label = label
        self._entries = dict(entries)

    def __getattr__(self, name: str) -> Any:
        if name.startswith("_"):
            raise AttributeError(name)
        if name not in self._entries:
            available = ", ".join(sorted(self._entries)) or "none"
            raise AttributeError(
                f"{self._label} has no entry '{name}' (available: {available})"
            )
        value = self._entries[name]
        if isinstance(value, Mapping):
            value = type(self)(f"{self._label}.{name}", value)
            self._entries[name] = value
        return value

    def __contains__(self, name: object) -> bool:
        return name in self._entries

    def __iter__(self):
        return iter(self._entries)

    def __len__(self) -> int:
        return len(self._entries)

    def __dir__(self):
        return [*super().__dir__(), *self._entries]

    def __repr__(self) -> str:
        return f"<{self._label}: {', '.join(sorted(self._entries)) or 'empty'}>"


class OperationNamespace(AttrNamespace):
    """Attribute namespace whose leaves are semantic :class:`Operation`
    values (``instructions`` / ``transactions`` / ``flows``)."""


# ---------------------------------------------------------------------------
# Semantic operations (TS program-instructions.ts)
# ---------------------------------------------------------------------------

_PREPARED_KINDS = ("instruction", "transaction", "flow")


class Operation:
    """A semantic operation: ``kind`` plus an async ``prepare(**input)``
    returning the matching prepared value."""

    def __init__(self, kind: str, prepare: Callable[..., Any]) -> None:
        if kind not in _PREPARED_KINDS:
            raise ValueError(f"Unknown operation kind '{kind}'")
        self.kind = kind
        self._prepare = prepare

    async def prepare(self, **input: Any) -> Any:
        result = self._prepare(**input)
        if _inspect.isawaitable(result):
            result = await result
        result_kind = getattr(result, "kind", None)
        if result_kind != self.kind:
            raise TypeError(
                f"{self.kind} operation prepared a "
                f"'{result_kind or type(result).__name__}' value; expected a "
                f"prepared {self.kind}"
            )
        return result

    def __repr__(self) -> str:
        return f"<{self.kind} operation {getattr(self._prepare, '__name__', '?')}>"


def instruction_operation(prepare: Callable[..., Any]) -> Operation:
    """Wrap a prepare callable returning a ``PreparedInstruction``."""
    return Operation("instruction", prepare)


def transaction_operation(prepare: Callable[..., Any]) -> Operation:
    """Wrap a prepare callable returning a ``PreparedTransaction``."""
    return Operation("transaction", prepare)


def flow_operation(prepare: Callable[..., Any]) -> Operation:
    """Wrap a prepare callable returning a ``PreparedFlow``."""
    return Operation("flow", prepare)


@dataclass
class ProgramOperations:
    """Semantic operations of one program, grouped by cardinality. Values may
    nest (mapping → nested namespace); leaves are :class:`Operation`\\ s."""

    instructions: Dict[str, Any] = field(default_factory=dict)
    transactions: Dict[str, Any] = field(default_factory=dict)
    flows: Dict[str, Any] = field(default_factory=dict)


def normalize_program_operations(value: Any) -> ProgramOperations:
    """Accept :class:`ProgramOperations` or a mapping with any of the three
    cardinality keys; anything else fails closed."""
    if value is None:
        return ProgramOperations()
    if isinstance(value, ProgramOperations):
        return value
    if isinstance(value, Mapping):
        unknown = set(value) - {"instructions", "transactions", "flows"}
        if unknown:
            raise TypeError(
                "Program operations mapping has unknown keys: "
                + ", ".join(sorted(unknown))
            )
        return ProgramOperations(
            instructions=dict(value.get("instructions") or {}),
            transactions=dict(value.get("transactions") or {}),
            flows=dict(value.get("flows") or {}),
        )
    raise TypeError(
        f"create_operations must return ProgramOperations or a mapping, got "
        f"{type(value).__name__}"
    )


class ProgramOperationContext:
    """Context given to ``create_operations`` and ``create_read``: chain
    reads, the live wallet, and the fully connected program."""

    def __init__(self, client: Any, program: "ConnectedProgram") -> None:
        self._client = client
        self.program = program

    @property
    def chain(self) -> Any:
        return self._client.chain

    @property
    def wallet(self) -> Any:
        return self._client.wallet


# ---------------------------------------------------------------------------
# Connected program runtime
# ---------------------------------------------------------------------------


class RawInstruction:
    """A raw instruction bound to a connected client: ``build(**params)`` is a
    pure prepare step returning a :class:`BuiltInstruction`.

    Params are IDL wire shape: arg-name keys serialize, account-name keys
    override addresses, ``resolve`` feeds PDA-only seeds; unknown params fail
    closed. Reserved keyword-only options: ``wallet`` (signer fallback address;
    defaults to the client wallet's public key), ``accounts`` (unvalidated
    escape-hatch overrides), ``remaining_accounts``.

    The fallback option is named ``wallet`` and not ``payer`` (matching the
    TypeScript ``BuildOptions.wallet``) because ``payer`` is a real IDL account
    name: a reserved kwarg by that name would shadow the ``payer`` account
    override that ``<Ix>Params`` advertises.
    """

    def __init__(
        self,
        name: str,
        handler: InstructionHandler,
        payer_provider: Callable[[], Optional[str]],
    ) -> None:
        self.name = name
        self.handler = handler
        self._payer_provider = payer_provider

    def build(
        self,
        *,
        wallet: Optional[str] = None,
        accounts: Optional[Mapping[str, str]] = None,
        remaining_accounts: Optional[Any] = None,
        **params: Any,
    ) -> BuiltInstruction:
        return self.handler.build(
            params,
            payer=wallet if wallet is not None else self._payer_provider(),
            accounts=accounts,
            remaining_accounts=remaining_accounts,
        )

    def __repr__(self) -> str:
        return f"<raw instruction {self.name}>"


class PdaFactory:
    """Typed PDA factory over :func:`arete.instructions.derive_pda`.

    ``derive(**seeds)`` takes seed values as keyword arguments named after the
    config's ``argRef`` / ``accountRef`` seed references; unknown keyword
    arguments fail closed.
    """

    def __init__(self, name: str, config: PdaConfig, program_id: str) -> None:
        self.name = name
        self.config = config
        self._program_id = config.program_id or program_id
        referenced = set()
        for seed in config.seeds:
            if isinstance(seed, ArgRefSeed):
                referenced.add(seed.arg.split(".", 1)[0])
            elif isinstance(seed, AccountRefSeed):
                referenced.add(seed.account)
        self._referenced = referenced

    def derive(self, **seeds: Any) -> Tuple[str, int]:
        """Derive ``(address, bump)`` from named seed values."""
        unknown = set(seeds) - self._referenced
        if unknown:
            expected = ", ".join(sorted(self._referenced)) or "none"
            raise TypeError(
                f"pdas.{self.name}.derive() got unexpected keyword argument(s) "
                f"{', '.join(sorted(unknown))} (expected: {expected})"
            )
        accounts = {
            key: value for key, value in seeds.items() if isinstance(value, str)
        }
        return derive_pda(
            self.config,
            args=seeds,
            accounts=accounts,
            program_id=self._program_id,
        )

    def __repr__(self) -> str:
        return f"<pda factory {self.name}>"


class _BoundQuery:
    """A program/stack query bound to the client's query executor."""

    def __init__(self, definition: Any, executor: Optional[QueryExecutor]) -> None:
        self._definition = definition
        self._executor = executor

    async def __call__(self, params: Any = None) -> Any:
        if self._executor is None:
            from arete.errors import AreteError

            raise AreteError(
                f"Query '{self._definition.name}' requires an HTTP endpoint "
                "(provide http_url or define endpoints.http in the stack)",
                "INVALID_CONFIG",
            )
        return await self._executor.execute(self._definition, params)


class ConnectedProgram:
    """One program SDK bound to a connected client (canonical §6 layers)."""

    def __init__(
        self,
        key: str,
        definition: ProgramDef,
        client: Any,
        transport: ProgramReadTransport,
        query_executor: Optional[QueryExecutor] = None,
    ) -> None:
        self.key = key
        self.name = definition.name
        self.program_id = definition.program_id
        self.PROGRAM_ID = definition.program_id
        self.definition = definition
        self.errors = tuple(definition.errors)
        self.program_spec_hash = definition.program_spec_hash

        def payer() -> Optional[str]:
            wallet = client.wallet
            return getattr(wallet, "public_key", None) if wallet is not None else None

        prefix = f"programs.{key}"
        self.raw = AttrNamespace(
            f"{prefix}.raw",
            {
                name: RawInstruction(name, handler, payer)
                for name, handler in definition.raw_instructions.items()
            },
        )
        self.pdas = AttrNamespace(
            f"{prefix}.pdas",
            {
                name: PdaFactory(name, config, definition.program_id)
                for name, config in definition.pdas.items()
            },
        )
        self.accounts = AttrNamespace(
            f"{prefix}.accounts",
            {
                name: AccountReader.from_def(account_def, transport)
                for name, account_def in definition.accounts.items()
            },
        )
        self.queries = AttrNamespace(
            f"{prefix}.queries",
            {
                name: _BoundQuery(query_def, query_executor)
                for name, query_def in definition.queries.items()
            },
        )
        self.addresses = AttrNamespace(f"{prefix}.addresses", definition.addresses)
        self.constants = AttrNamespace(f"{prefix}.constants", definition.constants)
        self.defaults = AttrNamespace(f"{prefix}.defaults", definition.defaults)
        self.math = AttrNamespace(f"{prefix}.math", definition.math)
        self.read = AttrNamespace(f"{prefix}.read", {})
        self.instructions = OperationNamespace(f"{prefix}.instructions", {})
        self.transactions = OperationNamespace(f"{prefix}.transactions", {})
        self.flows = OperationNamespace(f"{prefix}.flows", {})

        # TS order: the read namespace first, so operations can read through
        # `context.program.read`.
        context = ProgramOperationContext(client, self)
        if definition.create_read is not None:
            read = definition.create_read(context)
            if read is not None and not isinstance(read, Mapping):
                raise TypeError(
                    f"create_read must return a mapping, got {type(read).__name__}"
                )
            self.read = AttrNamespace(f"{prefix}.read", read or {})
        operations = ProgramOperations()
        if definition.create_operations is not None:
            operations = normalize_program_operations(
                definition.create_operations(context)
            )
        self.instructions = OperationNamespace(
            f"{prefix}.instructions", operations.instructions
        )
        self.transactions = OperationNamespace(
            f"{prefix}.transactions", operations.transactions
        )
        self.flows = OperationNamespace(f"{prefix}.flows", operations.flows)

    def parse_error(self, code: int) -> ErrorMetadata:
        """Resolve a program error code against this program's IDL errors."""
        return parse_program_error(code, self.errors)

    def __repr__(self) -> str:
        return f"<connected program {self.key} ({self.program_id})>"


class ProgramsNamespace:
    """``client.programs``: attribute access over the stack's connected
    program SDKs."""

    def __init__(self, programs: Mapping[str, ConnectedProgram]) -> None:
        self._programs = dict(programs)

    def __getattr__(self, name: str) -> ConnectedProgram:
        if name.startswith("_"):
            raise AttributeError(name)
        program = self._programs.get(name)
        if program is None:
            available = ", ".join(sorted(self._programs)) or "none"
            raise AttributeError(
                f"Stack has no program '{name}' (available: {available})"
            )
        return program

    def __contains__(self, name: object) -> bool:
        return name in self._programs

    def __iter__(self):
        return iter(self._programs)

    def __len__(self) -> int:
        return len(self._programs)

    def items(self):
        return self._programs.items()

    def __dir__(self):
        return [*super().__dir__(), *self._programs]
