"""The Arete client: a connected handle binding one stack.

Python projection of ``typescript/core/src/client.ts`` (canonical §3):

    a4 = await Arete.connect(STACK, url=..., wallet=...)
    async with await Arete.connect(STACK) as a4: ...

Surface: ``views`` (six verbs), ``programs`` (§6 layers), ``chain`` /
``transactions`` (injectable transports; hosted gateway bindings on the
stack definition wire them automatically), ``wallet`` / ``set_wallet``,
``transaction(...)`` / ``execute(...)`` / ``inspect_operation(...)``,
connection lifecycle + observation hooks, and the ``processed_slot`` /
``wait_for_processed_slot`` reconciliation cursor.

``transport="http"`` skips the WebSocket entirely — point reads, chain
reads, and instruction execution work, while views/subscriptions raise a
``WEBSOCKET_DISABLED`` :class:`arete.errors.AreteError` fast.
"""

from __future__ import annotations

from dataclasses import replace as _dc_replace
from typing import (
    Any,
    Awaitable,
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
from arete.connection import ConnectionManager
from arete.errors import AreteError
from arete.extensions import apply_connected_stack_extensions
from arete.gateway import create_hosted_solana_gateway_transports
from arete.http import HttpAuthClient, derive_http_endpoint
from arete.instructions import BuiltInstruction, ErrorMetadata
from arete.operations import (
    OperationInspection,
    OperationReceipt,
    PreparedOperation,
    SendOptions,
    SignerRegistry,
    TransactionExecutionError,
    classify_execution_failure,
    execute_prepared_operation,
    inspect_prepared_operation,
)
from arete.program_read_transport import (
    HOSTED_BINDING,
    LOCAL_HTTP,
    HttpProgramReadTransport,
    ProgramReadDescriptor,
    UnavailableProgramReadTransport,
    validate_program_read_descriptor,
)
from arete.read import QueryExecutor
from arete.stack import (
    ConnectedProgram,
    ProgramDef,
    ProgramsNamespace,
    StackDef,
    with_programs,
)
from arete.store import Store
from arete.subscription import SubscriptionRegistry
from arete.transactions import HttpTransactionTransport, TransactionTransport
from arete.views import DEFAULT_INITIAL_DATA_TIMEOUT, ViewsNamespace
from arete.wallet import (
    SendResult,
    TransactionFailureOutcome,
    WalletAdapter,
    WalletExecutionContext,
)

__all__ = [
    "Arete",
    "TRANSPORTS",
    "validate_program_reads",
]

TRANSPORTS: Tuple[str, ...] = ("websocket", "http")

_EXECUTION_DEFAULT_KEYS = (
    "wallet",
    "send",
    "signers",
    "signer_registry",
    "available_signer_addresses",
    "transaction_transport",
    "on_transaction_start",
    "on_transaction_success",
    "on_callback_error",
)


def _validate_release_identity(
    program_name: str,
    definition: ProgramDef,
    descriptor: Optional[ProgramReadDescriptor],
) -> None:
    if (
        descriptor is not None
        and definition.program_spec_hash
        and descriptor.release.program_spec_hash != definition.program_spec_hash
    ):
        raise AreteError(
            f"Program '{program_name}' release programSpecHash "
            f"'{descriptor.release.program_spec_hash}' does not match definition "
            f"programSpecHash '{definition.program_spec_hash}'",
            "PROGRAM_RELEASE_MISMATCH",
        )


def validate_program_reads(
    stack: StackDef,
    overrides: Optional[Mapping[str, ProgramReadDescriptor]] = None,
    *,
    validate_descriptor_keys: bool = True,
) -> None:
    """Fail-closed program-read configuration validation (TS
    ``validateProgramReads``)."""
    program_keys = list(stack.programs)
    if validate_descriptor_keys and stack.program_reads:
        read_keys = list(stack.program_reads)
        missing = [key for key in program_keys if key not in read_keys]
        extra = [key for key in read_keys if key not in program_keys]
        if missing or extra:
            detail = ""
            if missing:
                detail += f"; missing: {', '.join(missing)}"
            if extra:
                detail += f"; unknown: {', '.join(extra)}"
            raise AreteError(
                f"Stack '{stack.name}' program_reads keys must exactly match "
                f"programs{detail}",
                "INVALID_CONFIG",
            )
    for name, descriptor in stack.program_reads.items():
        validate_program_read_descriptor(name, descriptor)
    for key in overrides or {}:
        if key not in program_keys:
            raise AreteError(
                f"Program read override '{key}' does not match a program in "
                f"stack '{stack.name}'",
                "INVALID_CONFIG",
            )
        validate_program_read_descriptor(key, (overrides or {})[key])
    for name, definition in stack.programs.items():
        _validate_release_identity(name, definition, stack.program_reads.get(name))
        _validate_release_identity(name, definition, (overrides or {}).get(name))


def _effective_program_read(
    stack: StackDef,
    name: str,
    overrides: Mapping[str, ProgramReadDescriptor],
) -> Optional[ProgramReadDescriptor]:
    return overrides.get(name) or stack.program_reads.get(name)


def _has_program_account_reads(definition: ProgramDef) -> bool:
    return bool(definition.accounts)


def _has_complete_independent_program_reads(
    stack: StackDef,
    overrides: Mapping[str, ProgramReadDescriptor],
    connect_http_url: Optional[str],
) -> bool:
    programs = list(stack.programs.items())
    if not programs:
        return False
    for name, definition in programs:
        if not _has_program_account_reads(definition):
            continue
        descriptor = _effective_program_read(stack, name, overrides)
        if descriptor is None:
            return False
        if descriptor.transport_kind == HOSTED_BINDING:
            continue
        if descriptor.transport_kind == LOCAL_HTTP and connect_http_url:
            continue
        return False
    return True


def _validate_local_program_read_endpoints(
    stack: StackDef,
    overrides: Mapping[str, ProgramReadDescriptor],
    connect_http_url: Optional[str],
) -> None:
    for name, definition in stack.programs.items():
        if not _has_program_account_reads(definition):
            continue
        descriptor = _effective_program_read(stack, name, overrides)
        if (
            descriptor is not None
            and descriptor.transport_kind == LOCAL_HTTP
            and not connect_http_url
        ):
            raise AreteError(
                f"Program '{name}' local HTTP transport requires the http_url "
                "connect option",
                "INVALID_CONFIG",
            )


def _has_runtime_auth_strategy(auth: Optional[AuthConfig]) -> bool:
    return auth is not None and bool(
        getattr(auth, "token", None)
        or getattr(auth, "get_token", None)
        or getattr(auth, "token_endpoint", None)
    )


def _compose_callbacks(
    default: Optional[Callable[[Any], Any]],
    override: Optional[Callable[[Any], Any]],
) -> Optional[Callable[[Any], Awaitable[None]]]:
    """Run the connect-time default callback and the per-call callback in
    order; both always run, the first raised error propagates."""
    if default is None and override is None:
        return None
    callbacks = [callback for callback in (default, override) if callback is not None]
    if len(callbacks) == 1:
        only = callbacks[0]

        async def single(value: Any) -> None:
            result = only(value)
            if hasattr(result, "__await__"):
                await result

        return single

    async def both(value: Any) -> None:
        errors: List[BaseException] = []
        for callback in callbacks:
            try:
                result = callback(value)
                if hasattr(result, "__await__"):
                    await result
            except Exception as error:  # noqa: PERF203 - mirror TS semantics
                errors.append(error)
        if errors:
            raise errors[0]

    return both


class Arete:
    """A connected client binding one stack. Use :meth:`connect`."""

    def __init__(
        self,
        stack: StackDef,
        *,
        websocket_url: Optional[str],
        http_base_url: Optional[str],
        connect_http_url: Optional[str],
        program_read_overrides: Mapping[str, ProgramReadDescriptor],
        auth: Optional[AuthConfig] = None,
        wallet: Optional[WalletAdapter] = None,
        chain: Optional[ChainClient] = None,
        transactions: Optional[TransactionTransport] = None,
        execution: Optional[Mapping[str, Any]] = None,
        auto_reconnect: bool = True,
        reconnect_intervals: Optional[Sequence[float]] = None,
        max_reconnect_attempts: Optional[int] = None,
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
        http_client: Any = None,
        connect_factory: Optional[Callable[..., Any]] = None,
    ) -> None:
        self._stack = stack
        self._wallet = wallet
        self._auth = auth
        self._http_base_url = http_base_url
        self._connect_http_url = connect_http_url
        self._program_read_overrides = dict(program_read_overrides)
        self._execution_defaults = self._validate_execution_defaults(execution)

        connection_kwargs: Dict[str, Any] = {"auto_reconnect": auto_reconnect}
        if websocket_url is not None and auth is not None:
            connection_kwargs["auth"] = auth
        if reconnect_intervals is not None:
            connection_kwargs["reconnect_intervals"] = tuple(reconnect_intervals)
        if max_reconnect_attempts is not None:
            connection_kwargs["max_reconnect_attempts"] = max_reconnect_attempts
        if connect_factory is not None:
            connection_kwargs["connect_factory"] = connect_factory
        self._connection = ConnectionManager(websocket_url, **connection_kwargs)

        self._store = Store()
        self._registry = SubscriptionRegistry(self._connection, self._store)
        self._connection.on_frame(self._store.handle_frame)
        self._connection.on_connection_state_change(
            lambda state, error=None: self._registry.handle_connection_state(state)
        )
        self._views = ViewsNamespace(
            self._registry, stack.views, initial_data_timeout
        )

        self._http = HttpAuthClient(
            auth=auth, websocket_url=websocket_url, http_client=http_client
        )
        self._hosted_read_clients: Dict[str, HttpAuthClient] = {}
        self._injected_http_client = http_client

        gateway = stack.gateway
        if gateway is not None and (chain is None or transactions is None):
            transports = create_hosted_solana_gateway_transports(
                gateway, auth=auth, http_client=http_client
            )
            chain = chain if chain is not None else transports.chain
            transactions = (
                transactions if transactions is not None else transports.transactions
            )
        if chain is None and http_base_url:
            chain = HttpChainClient(http_base_url, self._http)
        if transactions is None and http_base_url:
            transactions = HttpTransactionTransport(http_base_url, self._http)
        self._chain = chain
        self._transactions = transactions

        self._query_executor = (
            QueryExecutor(http_base_url, self._http) if http_base_url else None
        )
        self._aggregated_errors: Optional[List[ErrorMetadata]] = None
        self._programs = ProgramsNamespace(
            {
                name: ConnectedProgram(
                    name,
                    definition,
                    self,
                    self._create_program_transport(name, definition),
                    self._query_executor,
                )
                for name, definition in stack.programs.items()
            }
        )
        self.queries = {
            name: self._bind_stack_query(definition)
            for name, definition in stack.queries.items()
        }

    # -- connect -----------------------------------------------------------

    @classmethod
    async def connect(
        cls,
        stack: StackDef,
        *,
        url: Optional[str] = None,
        http_url: Optional[str] = None,
        transport: Optional[str] = None,
        auth: Optional[AuthConfig] = None,
        wallet: Optional[WalletAdapter] = None,
        programs: Optional[Mapping[str, ProgramDef]] = None,
        program_reads: Optional[Mapping[str, ProgramReadDescriptor]] = None,
        chain: Optional[ChainClient] = None,
        transactions: Optional[TransactionTransport] = None,
        execution: Optional[Mapping[str, Any]] = None,
        auto_connect: bool = True,
        auto_reconnect: bool = True,
        reconnect_intervals: Optional[Sequence[float]] = None,
        max_reconnect_attempts: Optional[int] = None,
        initial_data_timeout: Optional[float] = DEFAULT_INITIAL_DATA_TIMEOUT,
        http_client: Any = None,
        connect_factory: Optional[Callable[..., Any]] = None,
    ) -> "Arete":
        """Connect a client to ``stack``.

        ``transport`` is ``"websocket"`` (default) or ``"http"`` (skips the
        socket; view subscriptions fail fast with ``WEBSOCKET_DISABLED``).
        ``chain`` / ``transactions`` inject explicit transports (composition
        sessions); otherwise gateway bindings on the stack definition, or
        default HTTP transports over the resolved HTTP endpoint, are used.
        ``initial_data_timeout`` bounds every ``views.*.get`` / ``get_one``
        wait for an initial snapshot (Rust's ``AreteConfig`` option, default
        5s; ``None`` waits forever, per-call ``timeout`` overrides it).
        """
        if transport is not None and transport not in TRANSPORTS:
            raise AreteError(
                f"transport must be one of {TRANSPORTS}, got {transport!r}",
                "INVALID_CONFIG",
            )
        validate_program_reads(stack, None)
        effective_stack = with_programs(stack, programs)
        overrides = dict(program_reads or {})
        validate_program_reads(
            effective_stack, overrides, validate_descriptor_keys=False
        )

        requested_url = url if url is not None else (stack.endpoints.ws or None)
        connect_http_url = (
            http_url if http_url is not None else (stack.endpoints.http or None)
        )
        _validate_local_program_read_endpoints(
            effective_stack, overrides, connect_http_url
        )
        independently_readable = _has_complete_independent_program_reads(
            effective_stack, overrides, connect_http_url
        )
        implicit_program_only_http = (
            transport is None
            and not requested_url
            and independently_readable
            and not stack.views
        )
        http_only = transport == "http" or implicit_program_only_http
        websocket_url = None if http_only else requested_url

        http_base = http_url
        if http_base is None:
            http_base = stack.endpoints.http
        if http_base is None and requested_url:
            http_base = derive_http_endpoint(requested_url)

        if not http_only and not websocket_url:
            raise AreteError(
                "WebSocket URL is required (provide the url option or define "
                "endpoints.ws in the stack)",
                "INVALID_CONFIG",
            )
        if http_only and not http_base and not independently_readable:
            raise AreteError(
                'HTTP endpoint is required for transport: "http" (provide the '
                "http_url option or define endpoints.http in the stack)",
                "INVALID_CONFIG",
            )

        client = cls(
            effective_stack,
            websocket_url=websocket_url,
            http_base_url=http_base or None,
            connect_http_url=connect_http_url,
            program_read_overrides=overrides,
            auth=auth,
            wallet=wallet,
            chain=chain,
            transactions=transactions,
            execution=execution,
            auto_reconnect=auto_reconnect,
            reconnect_intervals=reconnect_intervals,
            max_reconnect_attempts=max_reconnect_attempts,
            initial_data_timeout=initial_data_timeout,
            http_client=http_client,
            connect_factory=connect_factory,
        )
        if not http_only and auto_connect:
            await client._connection.connect()
        return apply_connected_stack_extensions(client, effective_stack)

    # -- internals ---------------------------------------------------------

    @staticmethod
    def _validate_execution_defaults(
        execution: Optional[Mapping[str, Any]]
    ) -> Dict[str, Any]:
        if execution is None:
            return {}
        unknown = set(execution) - set(_EXECUTION_DEFAULT_KEYS)
        if unknown:
            raise AreteError(
                "Unknown execution default option(s): " + ", ".join(sorted(unknown)),
                "INVALID_CONFIG",
            )
        return dict(execution)

    def _hosted_read_auth_client(self, binding: Any) -> Tuple[HttpAuthClient, bool]:
        """Auth client + authenticated flag for a hosted read binding.

        A configured runtime strategy wins; bindings that do not require auth
        run unauthenticated when no runtime strategy exists; otherwise tokens
        are minted from the binding's session endpoint (mirror of the gateway
        binding auth rules)."""
        if _has_runtime_auth_strategy(self._auth):
            return self._http, True
        if binding.auth.required is False:
            return self._http, False
        identity = f"session-endpoint:{binding.auth.session_endpoint}"
        client = self._hosted_read_clients.get(identity)
        if client is None:
            base = self._auth if self._auth is not None else AuthConfig()
            client = HttpAuthClient(
                auth=_dc_replace(base, token_endpoint=binding.auth.session_endpoint),
                websocket_url=None,
                http_client=self._injected_http_client,
            )
            self._hosted_read_clients[identity] = client
        return client, True

    def _create_program_transport(self, name: str, definition: ProgramDef) -> Any:
        if not _has_program_account_reads(definition):
            return UnavailableProgramReadTransport(
                f"Program '{name}' has no generated account readers"
            )
        descriptor = _effective_program_read(
            self._stack, name, self._program_read_overrides
        )
        _validate_release_identity(name, definition, descriptor)
        if descriptor is None:
            return UnavailableProgramReadTransport(
                f"Program '{name}' has no release-aware read descriptor"
            )
        if descriptor.transport_kind == LOCAL_HTTP:
            return HttpProgramReadTransport.local_http(
                self._connect_http_url or "", descriptor.release, self._http
            )
        binding = descriptor.binding
        assert binding is not None
        http, authenticated = self._hosted_read_auth_client(binding)
        return HttpProgramReadTransport.hosted(
            binding, descriptor.release, http, authenticated=authenticated
        )

    def _bind_stack_query(self, definition: Any) -> Callable[..., Any]:
        async def run(params: Any = None) -> Any:
            if self._query_executor is None:
                raise AreteError(
                    f"Stack '{self._stack.name}' has no HTTP endpoint; stack "
                    "queries require http_url or endpoints.http",
                    "INVALID_CONFIG",
                )
            return await self._query_executor.execute_stack(definition, params)

        return run

    def _aggregate_errors(self) -> List[ErrorMetadata]:
        """Error metadata from every handler in the stack, deduped by code
        (first wins)."""
        if self._aggregated_errors is None:
            seen: set = set()
            collected: List[ErrorMetadata] = []
            for definition in self._stack.programs.values():
                for handler in definition.raw_instructions.values():
                    for error in handler.errors or ():
                        if error.code not in seen:
                            seen.add(error.code)
                            collected.append(error)
                for error in definition.errors:
                    if error.code not in seen:
                        seen.add(error.code)
                        collected.append(error)
            self._aggregated_errors = collected
        return self._aggregated_errors

    # -- surface -----------------------------------------------------------

    @property
    def stack_name(self) -> str:
        return self._stack.name

    @property
    def views(self) -> ViewsNamespace:
        return self._views

    @property
    def programs(self) -> ProgramsNamespace:
        return self._programs

    @property
    def chain(self) -> ChainClient:
        if self._chain is None:
            raise AreteError(
                f"Stack '{self._stack.name}' has no HTTP endpoint; chain reads "
                "require http_url, endpoints.http, or an injected chain client",
                "INVALID_CONFIG",
            )
        return self._chain

    @property
    def transactions(self) -> TransactionTransport:
        if self._transactions is None:
            raise AreteError(
                f"Stack '{self._stack.name}' has no HTTP endpoint; the "
                "transaction relay requires http_url, endpoints.http, or an "
                "injected transaction transport",
                "INVALID_CONFIG",
            )
        return self._transactions

    @property
    def wallet(self) -> Optional[WalletAdapter]:
        """The default wallet adapter, if one was configured."""
        return self._wallet

    @property
    def public_key(self) -> Optional[str]:
        """The connected wallet address, if a default wallet is configured."""
        wallet = self._wallet
        return getattr(wallet, "public_key", None) if wallet is not None else None

    def set_wallet(self, wallet: Optional[WalletAdapter]) -> None:
        """Set (or clear) the default wallet adapter used for execution."""
        self._wallet = wallet

    # -- execution ---------------------------------------------------------

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
        """Sign and send pre-built instructions as a single transaction.

        Build instructions with ``client.programs.<program>.raw.<name>.build``
        and compose them here; RPC/compilation/confirmation are owned by the
        wallet adapter. On failure raises
        :class:`arete.operations.TransactionExecutionError` carrying the
        classified :class:`arete.wallet.TransactionFailureOutcome`; every
        adapter failure (structured or not) runs through
        :func:`arete.operations.classify_execution_failure`, so chain failures
        are parsed against ``errors`` when given, otherwise against error
        metadata aggregated from every handler in the stack.
        """
        adapter = wallet if wallet is not None else self._wallet
        if adapter is None:
            raise TransactionExecutionError(
                TransactionFailureOutcome.not_submitted(
                    "wallet", message="Wallet required to sign and send transaction"
                )
            )
        options = SendOptions.coerce(send)
        if signers is not None:
            options = options.with_signers(signers)
        context = WalletExecutionContext(
            transaction_transport=(
                transaction_transport
                if transaction_transport is not None
                else self._transactions
            )
        )
        try:
            result = await adapter.sign_and_send(list(instructions), options, context)
        except Exception as cause:
            outcome = classify_execution_failure(
                cause, errors if errors is not None else self._aggregate_errors()
            )
            if isinstance(cause, TransactionExecutionError) and outcome is cause.outcome:
                raise
            raise TransactionExecutionError(outcome) from cause
        if isinstance(result, SendResult):
            return result
        return SendResult(
            signature=result["signature"]
            if isinstance(result, Mapping)
            else result.signature,
            slot=result.get("slot")
            if isinstance(result, Mapping)
            else getattr(result, "slot", None),
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
        """Run a prepared operation through the wallet: fail-closed signer
        validation, per-transaction callbacks, receipts with signatures.
        Connect-time ``execution`` defaults merge under per-call options."""
        defaults = self._execution_defaults
        merged_send: Any = None
        if defaults.get("send") is not None or send is not None:
            merged_send = SendOptions.coerce(defaults.get("send")).merged(
                SendOptions.coerce(send) if send is not None else None
            )
        return await execute_prepared_operation(
            self,
            prepared,
            wallet=wallet if wallet is not None else defaults.get("wallet"),
            send=merged_send,
            signers=signers if signers is not None else defaults.get("signers"),
            signer_registry=(
                signer_registry
                if signer_registry is not None
                else defaults.get("signer_registry")
            ),
            available_signer_addresses=(
                available_signer_addresses
                if available_signer_addresses is not None
                else defaults.get("available_signer_addresses")
            ),
            transaction_transport=(
                transaction_transport
                if transaction_transport is not None
                else defaults.get("transaction_transport")
            ),
            on_transaction_start=_compose_callbacks(
                defaults.get("on_transaction_start"), on_transaction_start
            ),
            on_transaction_success=_compose_callbacks(
                defaults.get("on_transaction_success"), on_transaction_success
            ),
            on_callback_error=_compose_callbacks(
                defaults.get("on_callback_error"), on_callback_error
            ),
        )

    async def inspect_operation(
        self,
        prepared: PreparedOperation,
        *,
        wallet: Optional[WalletAdapter] = None,
        inspect: Optional[Mapping[str, Any]] = None,
        transaction_transport: Optional[TransactionTransport] = None,
    ) -> OperationInspection:
        """Inspect one prepared instruction/transaction without signing or
        submitting it."""
        return await inspect_prepared_operation(
            wallet if wallet is not None else self._wallet,
            prepared,
            inspect,
            WalletExecutionContext(
                transaction_transport=(
                    transaction_transport
                    if transaction_transport is not None
                    else self._transactions
                )
            ),
        )

    # -- connection lifecycle ---------------------------------------------

    @property
    def connection_state(self) -> str:
        return self._connection.connection_state

    def is_connected(self) -> bool:
        return self._connection.is_connected()

    async def connect_socket(self) -> None:
        """(Re)open the WebSocket (raises ``WEBSOCKET_DISABLED`` in http-only
        mode)."""
        await self._connection.connect()

    async def disconnect(self) -> None:
        """Release all subscriptions and close the connection."""
        self._registry.clear()
        await self._connection.disconnect()

    async def aclose(self) -> None:
        """Disconnect and release owned HTTP resources."""
        await self.disconnect()
        await self._http.aclose()
        for client in self._hosted_read_clients.values():
            await client.aclose()

    async def __aenter__(self) -> "Arete":
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        await self.aclose()

    def on_connection_state_change(
        self, callback: Callable[[str, Optional[str]], None]
    ) -> Callable[[], None]:
        return self._connection.on_connection_state_change(callback)

    def on_frame(self, callback: Callable[[Any], None]) -> Callable[[], None]:
        return self._connection.on_frame(callback)

    def on_socket_issue(self, callback: Callable[[Any], None]) -> Callable[[], None]:
        return self._connection.on_socket_issue(callback)

    # -- processed slot cursor --------------------------------------------

    @property
    def processed_slot(self) -> Optional[int]:
        """Highest Solana slot whose streamed frame has been processed."""
        return self._connection.processed_slot

    async def wait_for_processed_slot(
        self, slot: int, *, timeout: Optional[float] = None
    ) -> int:
        """Wait until a frame at or beyond ``slot`` has been processed — the
        reconciliation primitive used after writes."""
        return await self._connection.wait_for_processed_slot(slot, timeout=timeout)

    # -- escape hatches ----------------------------------------------------

    def get_connection(self) -> ConnectionManager:
        return self._connection

    def get_subscription_registry(self) -> SubscriptionRegistry:
        return self._registry

    def __repr__(self) -> str:
        return (
            f"<Arete stack={self._stack.name!r} "
            f"state={self._connection.connection_state}>"
        )
