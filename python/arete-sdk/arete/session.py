"""Multi-stack / multi-program sessions.

Python port of ``typescript/core/src/session.ts`` (canonical §9):

    session = await create_session(
        stacks={"ore": ORE_STACK},
        programs={"spl": SPL_PROGRAM},
        wallet=wallet,
    )
    session.stacks.ore.views...
    session.programs.ore.raw...
    await session.execute(prepared)

Semantics:

- Every member gets its own :class:`arete.client.Arete` client (connection +
  store); standalone programs become synthetic HTTP-only stacks reusing the
  exact same machinery.
- Programs bundled by stack members are promoted onto ``session.programs``
  by reference; on key collisions the first-connected stack wins (with a
  warning). Explicit standalone programs always win over promoted keys.
- ``mode="composition"`` requires generated or explicit ``chain`` +
  ``transactions`` transports and forbids shared fallback endpoints — chain
  reads and program reads never inherit a live member's HTTP endpoint.
- The execution host is the first connected member; ``set_wallet`` fans out
  to every member; ``close()`` disconnects all.
"""

from __future__ import annotations

import warnings
from typing import (
    Any,
    Callable,
    Dict,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
)

from arete.auth import AuthConfig
from arete.chain import ChainClient, HttpChainClient
from arete.client import Arete
from arete.errors import AreteError
from arete.gateway import create_hosted_solana_gateway_transports
from arete.http import HttpAuthClient
from arete.instructions import BuiltInstruction, ErrorMetadata
from arete.operations import (
    OperationReceipt,
    PreparedOperation,
    SignerRegistry,
)
from arete.program_read_transport import (
    HOSTED_BINDING,
    ProgramReadDescriptor,
    validate_program_read_descriptor,
)
from arete.stack import (
    AttrNamespace,
    ConnectedProgram,
    ProgramDef,
    StackDef,
    StackEndpoints,
    with_programs,
)
from arete.transactions import TransactionTransport
from arete.wallet import SendResult, WalletAdapter

__all__ = ["Session", "SessionError", "create_session"]

_MEMBER_OPTION_KEYS = (
    "url",
    "http_url",
    "transport",
    "auth",
    "auto_connect",
    "auto_reconnect",
    "programs",
    "program_read",
    "program_reads",
)


class SessionError(AreteError):
    """Invalid session composition or configuration."""

    def __init__(self, message: str, code: str = "INVALID_CONFIG") -> None:
        super().__init__(message, code)


def _validate_member_options(key: str, options: Mapping[str, Any]) -> None:
    unknown = set(options) - set(_MEMBER_OPTION_KEYS)
    if unknown:
        raise SessionError(
            f"Unknown option(s) for session member '{key}': "
            + ", ".join(sorted(unknown))
        )


def _resolve_member_program_reads(
    stack: StackDef,
    member: Mapping[str, Any],
    session_program_read: Optional[ProgramReadDescriptor],
    session_program_reads: Mapping[str, ProgramReadDescriptor],
) -> Optional[Dict[str, ProgramReadDescriptor]]:
    """Layered per-program override resolution: session-wide single override,
    session per-program, member-wide single override, member per-program —
    later layers win."""
    member_read = member.get("program_read")
    member_reads = member.get("program_reads") or {}
    resolved: Dict[str, ProgramReadDescriptor] = {}
    for name in stack.programs:
        override = None
        for layer in (
            session_program_read,
            session_program_reads.get(name),
            member_read,
            member_reads.get(name),
        ):
            if layer is not None:
                override = layer
        if override is not None:
            resolved[name] = override
    return resolved or None


def _program_as_stack(
    name: str,
    program: ProgramDef,
    descriptor: Optional[ProgramReadDescriptor],
) -> StackDef:
    return StackDef(
        name=name,
        endpoints=StackEndpoints(ws=""),
        views={},
        programs={name: program},
        program_reads={name: descriptor} if descriptor is not None else {},
        gateway=program.gateway,
    )


def _validate_composition_program_reads(
    stack: StackDef,
    member: Mapping[str, Any],
    session_program_read: Optional[ProgramReadDescriptor],
    session_program_reads: Mapping[str, ProgramReadDescriptor],
) -> None:
    overrides = _resolve_member_program_reads(
        stack, member, session_program_read, session_program_reads
    ) or {}
    for name, program in stack.programs.items():
        descriptor = overrides.get(name) or stack.program_reads.get(name)
        if descriptor is not None:
            validate_program_read_descriptor(name, descriptor)
        if not program.accounts:
            continue
        binding = descriptor.binding if descriptor is not None else None
        if (
            descriptor is None
            or descriptor.transport_kind != HOSTED_BINDING
            or binding is None
            or not binding.endpoint.strip()
            or not binding.program_read_binding_id.strip()
            or binding.auth.target_kind != "program-read-binding"
            or binding.auth.target_id != binding.program_read_binding_id
            or not binding.auth.session_endpoint.strip()
        ):
            raise SessionError(
                f"Composition session program '{name}' requires a complete "
                "hosted-binding descriptor or override"
            )


class Session:
    """A connected multi-member session. Use :func:`create_session`."""

    def __init__(
        self,
        *,
        stacks: Mapping[str, Arete],
        programs: Mapping[str, ConnectedProgram],
        member_clients: Sequence[Arete],
        wallet: Optional[WalletAdapter],
        signer_registry: SignerRegistry,
        chain: Optional[ChainClient],
        transactions: Optional[TransactionTransport],
        execution: Mapping[str, Any],
    ) -> None:
        self._stack_clients = dict(stacks)
        self._program_map = dict(programs)
        self._members: Tuple[Arete, ...] = tuple(member_clients)
        self._wallet = wallet
        self._signer_registry = signer_registry
        self._chain = chain
        self._transactions = transactions
        self._execution = dict(execution)
        self.stacks = AttrNamespace("session.stacks", self._stack_clients)
        self.programs = AttrNamespace("session.programs", self._program_map)

    @property
    def wallet(self) -> Optional[WalletAdapter]:
        """One wallet governs execution across every member."""
        return self._wallet

    @property
    def signer_registry(self) -> SignerRegistry:
        return self._signer_registry

    @property
    def chain(self) -> ChainClient:
        if self._chain is not None:
            return self._chain
        return self._execution_host.chain

    @property
    def transactions(self) -> TransactionTransport:
        if self._transactions is not None:
            return self._transactions
        return self._execution_host.transactions

    @property
    def _execution_host(self) -> Arete:
        return self._members[0]

    def set_wallet(self, wallet: Optional[WalletAdapter]) -> None:
        """Fan the wallet out to every member client."""
        self._wallet = wallet
        for client in self._members:
            client.set_wallet(wallet)

    def _combined_signers(
        self, configured: Optional[Sequence[Any]]
    ) -> Optional[List[Any]]:
        signers: List[Any] = []
        seen: set = set()
        for signer in [*self._signer_registry.values(), *(configured or ())]:
            if id(signer) not in seen:
                seen.add(id(signer))
                signers.append(signer)
        return signers or None

    async def transaction(
        self,
        instructions: Sequence[BuiltInstruction],
        *,
        wallet: Optional[WalletAdapter] = None,
        send: Any = None,
        errors: Optional[Sequence[ErrorMetadata]] = None,
        signers: Optional[Sequence[Any]] = None,
        transaction_transport: Optional[TransactionTransport] = None,
    ) -> SendResult:
        """Sign and send through the execution host with the session's
        registered signers and transaction transport."""
        defaults = self._execution
        from arete.wallet import SendOptions

        merged_send: Any = None
        if defaults.get("send") is not None or send is not None:
            merged_send = SendOptions.coerce(defaults.get("send")).merged(
                SendOptions.coerce(send) if send is not None else None
            )
        configured = signers if signers is not None else defaults.get("signers")
        return await self._execution_host.transaction(
            instructions,
            wallet=wallet if wallet is not None else defaults.get("wallet"),
            send=merged_send,
            errors=errors,
            signers=self._combined_signers(configured),
            transaction_transport=(
                transaction_transport
                if transaction_transport is not None
                else self._transactions
            ),
        )

    async def execute(
        self,
        prepared: PreparedOperation,
        *,
        wallet: Optional[WalletAdapter] = None,
        send: Any = None,
        signers: Optional[Sequence[Any]] = None,
        signer_registry: Optional[SignerRegistry] = None,
        available_signer_addresses: Optional[Sequence[str]] = None,
        transaction_transport: Optional[TransactionTransport] = None,
        on_transaction_start: Optional[Callable[[Any], Any]] = None,
        on_transaction_success: Optional[Callable[[Any], Any]] = None,
        on_callback_error: Optional[Callable[[Any], Any]] = None,
    ) -> OperationReceipt:
        """Execute a prepared operation on the execution host with the
        session's signer registry counted toward signer validation."""
        defaults = self._execution
        return await self._execution_host.execute(
            prepared,
            wallet=wallet if wallet is not None else defaults.get("wallet"),
            send=send if send is not None else defaults.get("send"),
            signers=signers if signers is not None else defaults.get("signers"),
            signer_registry=(
                signer_registry
                if signer_registry is not None
                else self._signer_registry
            ),
            available_signer_addresses=(
                available_signer_addresses
                if available_signer_addresses is not None
                else defaults.get("available_signer_addresses")
            ),
            transaction_transport=(
                transaction_transport
                if transaction_transport is not None
                else defaults.get("transaction_transport") or self._transactions
            ),
            on_transaction_start=(
                on_transaction_start
                if on_transaction_start is not None
                else defaults.get("on_transaction_start")
            ),
            on_transaction_success=(
                on_transaction_success
                if on_transaction_success is not None
                else defaults.get("on_transaction_success")
            ),
            on_callback_error=(
                on_callback_error
                if on_callback_error is not None
                else defaults.get("on_callback_error")
            ),
        )

    async def close(self) -> None:
        """Disconnect every member client."""
        for client in self._members:
            await client.aclose()

    async def __aenter__(self) -> "Session":
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        await self.close()

    def __repr__(self) -> str:
        return (
            f"<Session stacks=[{', '.join(sorted(self._stack_clients))}] "
            f"programs=[{', '.join(sorted(self._program_map))}]>"
        )


async def create_session(
    *,
    stacks: Optional[Mapping[str, StackDef]] = None,
    programs: Optional[Mapping[str, ProgramDef]] = None,
    mode: Optional[str] = None,
    program_reads: Optional[Mapping[str, ProgramReadDescriptor]] = None,
    wallet: Optional[WalletAdapter] = None,
    chain: Optional[ChainClient] = None,
    transactions: Optional[TransactionTransport] = None,
    auth: Optional[AuthConfig] = None,
    transport: Optional[str] = None,
    endpoints: Optional[Mapping[str, str]] = None,
    execution: Optional[Mapping[str, Any]] = None,
    signer_registry: Optional[SignerRegistry] = None,
    program_read: Optional[ProgramReadDescriptor] = None,
    program_read_overrides: Optional[Mapping[str, ProgramReadDescriptor]] = None,
    stack_options: Optional[Mapping[str, Mapping[str, Any]]] = None,
    program_options: Optional[Mapping[str, Mapping[str, Any]]] = None,
    http_client: Any = None,
    connect_factory: Optional[Callable[..., Any]] = None,
) -> Session:
    """Compose multiple stack and standalone-program clients behind one
    wallet.

    ``stacks`` / ``programs`` are the session members (stack bindings and
    standalone program definitions); ``program_reads`` holds generated
    descriptors keyed in parallel with standalone ``programs``.
    ``stack_options`` / ``program_options`` carry per-member connection
    overrides (``url``, ``http_url``, ``transport``, ``auth``,
    ``auto_connect``, ``auto_reconnect``, ``programs``, ``program_read``,
    ``program_reads``); ``program_read`` / ``program_read_overrides`` are the
    session-wide layers. ``mode="composition"`` requires generated or explicit
    ``chain`` + ``transactions`` and forbids ``endpoints`` fallback.
    """
    stack_entries = list((stacks or {}).items())
    program_entries = list((programs or {}).items())
    stack_options = stack_options or {}
    program_options = program_options or {}
    session_program_reads = dict(program_read_overrides or {})

    if not stack_entries and not program_entries:
        raise SessionError(
            "create_session requires at least one stack or program member"
        )
    if mode not in (None, "composition"):
        raise SessionError(f"Unknown session mode {mode!r}")
    composition = mode == "composition"
    if composition and endpoints is not None:
        raise SessionError(
            "composition sessions require per-member live endpoints, not "
            "shared fallback endpoints"
        )
    if composition and (chain is None or transactions is None):
        gateways = [stack.gateway for _, stack in stack_entries] + [
            program.gateway for _, program in program_entries
        ]
        generated_gateway = gateways[0] if gateways else None
        if generated_gateway is None or any(
            gateway != generated_gateway for gateway in gateways
        ):
            raise SessionError(
                "composition sessions require one consistent generated gateway "
                "or explicit chain and transaction transports"
            )
        generated_transports = create_hosted_solana_gateway_transports(
            generated_gateway,
            auth=auth,
            http_client=http_client,
        )
        chain = chain or generated_transports.chain
        transactions = transactions or generated_transports.transactions
    if program_reads:
        program_keys = [key for key, _ in program_entries]
        descriptor_keys = list(program_reads)
        if set(program_keys) != set(descriptor_keys):
            raise SessionError(
                "Session definition program_reads keys must exactly match "
                "standalone programs"
            )
    for key, member in {**dict(stack_options), **dict(program_options)}.items():
        _validate_member_options(key, member)

    if composition:
        for key, stack in stack_entries:
            member = stack_options.get(key, {})
            effective = with_programs(stack, member.get("programs"))
            _validate_composition_program_reads(
                effective, member, program_read, session_program_reads
            )
        for key, program in program_entries:
            _validate_composition_program_reads(
                _program_as_stack(key, program, (program_reads or {}).get(key)),
                program_options.get(key, {}),
                program_read,
                session_program_reads,
            )

    registry = signer_registry if signer_registry is not None else SignerRegistry()
    endpoints = endpoints or {}
    fallback_ws = endpoints.get("ws")
    fallback_http = endpoints.get("http")

    async def connect_member(
        stack: StackDef,
        member: Mapping[str, Any],
        *,
        force_http_only: bool,
        attached_programs: Optional[Mapping[str, ProgramDef]] = None,
    ) -> Arete:
        member_transport = member.get("transport")
        effective_transport = (
            member_transport
            if member_transport is not None
            else transport
            if transport is not None
            else ("http" if force_http_only else "websocket")
        )
        url = member.get("url") or (stack.endpoints.ws or fallback_ws)
        http_url = member.get("http_url") or fallback_http
        overrides = _resolve_member_program_reads(
            with_programs(stack, attached_programs),
            member,
            program_read,
            session_program_reads,
        )
        return await Arete.connect(
            stack,
            url=url,
            http_url=http_url,
            transport=effective_transport,
            auth=member.get("auth") or auth,
            wallet=wallet,
            programs=attached_programs,
            program_reads=overrides,
            chain=chain,
            transactions=transactions,
            auto_connect=member.get("auto_connect", True),
            auto_reconnect=member.get("auto_reconnect", True),
            http_client=http_client,
            connect_factory=connect_factory,
        )

    connected_stacks: List[Tuple[str, Arete]] = []
    for key, stack in stack_entries:
        member = stack_options.get(key, {})
        client = await connect_member(
            stack,
            member,
            force_http_only=False,
            attached_programs=member.get("programs"),
        )
        connected_stacks.append((key, client))

    connected_programs: List[Tuple[str, Arete]] = []
    for key, program in program_entries:
        synthetic = _program_as_stack(key, program, (program_reads or {}).get(key))
        client = await connect_member(
            synthetic,
            program_options.get(key, {}),
            force_http_only=True,
        )
        connected_programs.append((key, client))

    member_clients = [client for _, client in connected_stacks] + [
        client for _, client in connected_programs
    ]

    explicit_keys = {key for key, _ in connected_programs}
    promoted: Dict[str, ConnectedProgram] = {
        key: getattr(client.programs, key) for key, client in connected_programs
    }
    owners: Dict[str, Tuple[str, Optional[str]]] = {}
    if not composition:
        for stack_key, client in connected_stacks:
            for program_key, program in client.programs.items():
                if program_key in explicit_keys:
                    continue
                existing = owners.get(program_key)
                if existing is not None:
                    warnings.warn(
                        f"Program '{program_key}' is bundled by stacks "
                        f"'{existing[0]}' ({existing[1] or 'unknown program ID'}) "
                        f"and '{stack_key}' ({program.program_id or 'unknown program ID'}); "
                        f"session.programs.{program_key} uses '{existing[0]}' "
                        "because it was connected first",
                        stacklevel=2,
                    )
                    continue
                promoted[program_key] = program
                owners[program_key] = (stack_key, program.program_id)

    session_chain = chain
    if session_chain is None and fallback_http:
        session_chain = HttpChainClient(
            fallback_http,
            HttpAuthClient(auth=auth, websocket_url=None, http_client=http_client),
        )

    session_transactions = transactions
    if session_transactions is None:
        # The execution host's default transport, when it has one; otherwise
        # the Session.transactions property fails lazily on first use.
        session_transactions = member_clients[0]._transactions

    return Session(
        stacks=dict(connected_stacks),
        programs=promoted,
        member_clients=member_clients,
        wallet=wallet,
        signer_registry=registry,
        chain=session_chain,
        transactions=session_transactions,
        execution=execution or {},
    )
