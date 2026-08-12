"""Hosted Solana gateway transports (port of ``solana-gateway.ts``).

Generated gateway descriptors (``sgb_…`` bindings) become explicit
:class:`~arete.chain.ChainClient` and
:class:`~arete.transactions.TransactionTransport` implementations pointed at
the hosted gateway endpoints. Tokens are isolated by exact binding target and
scope: every capability mints ``solana-gateway-binding``-targeted tokens for
its own binding id, and ``transaction:send`` requests only replay after a
refresh when the response carries the ``X-Arete-Upstream-Attempted: false``
predispatch marker.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, replace
from typing import Any, Dict, Mapping, Optional
from urllib.parse import urlsplit

from arete.auth import AuthConfig
from arete.chain import ChainClient, HttpChainClient
from arete.errors import AreteError
from arete.http import SOLANA_GATEWAY_BINDING_KIND, AuthTokenTarget, HttpAuthClient
from arete.transactions import HttpTransactionTransport, TransactionTransport

CHAIN_REQUIRED_SCOPES = ("read",)
TRANSACTIONS_REQUIRED_SCOPES = ("transaction:inspect", "transaction:send")

_GATEWAY_BINDING_ID_RE = re.compile(r"^sgb_[A-Za-z0-9_-]{32}$")
_LOOPBACK_HOSTS = ("localhost", "127.0.0.1", "::1")


@dataclass(frozen=True)
class SolanaGatewayAuthMetadata:
    """Public auth metadata emitted for a hosted gateway binding
    (TS ``SolanaGatewayAuthMetadata``)."""

    required: bool
    mode: str
    session_endpoint: str
    jwks_url: str
    token_transport: str
    audience: str  # must be "arete:solana-gateway"
    target_kind: str  # must be "solana-gateway-binding"
    target_id: str
    scopes: tuple
    accepted_key_classes: tuple
    transaction_entitlement_required: bool

    @classmethod
    def from_dict(cls, value: Mapping[str, Any]) -> "SolanaGatewayAuthMetadata":
        return cls(
            required=value["required"],
            mode=value["mode"],
            session_endpoint=value["sessionEndpoint"],
            jwks_url=value["jwksUrl"],
            token_transport=value["tokenTransport"],
            audience=value["audience"],
            target_kind=value["targetKind"],
            target_id=value["targetId"],
            scopes=tuple(value["scopes"]),
            accepted_key_classes=tuple(value["acceptedKeyClasses"]),
            transaction_entitlement_required=value["transactionEntitlementRequired"],
        )


@dataclass(frozen=True)
class HostedSolanaGatewayCapabilityBinding:
    """One generated, non-inheriting hosted gateway capability binding
    (TS ``HostedSolanaGatewayCapabilityBinding``)."""

    endpoint: str
    auth_policy: str
    solana_gateway_binding_id: str
    cluster: str
    region: str
    auth: SolanaGatewayAuthMetadata

    @classmethod
    def from_dict(
        cls, value: Mapping[str, Any]
    ) -> "HostedSolanaGatewayCapabilityBinding":
        return cls(
            endpoint=value["endpoint"],
            auth_policy=value["authPolicy"],
            solana_gateway_binding_id=value["solanaGatewayBindingId"],
            cluster=value["cluster"],
            region=value["region"],
            auth=SolanaGatewayAuthMetadata.from_dict(value["auth"]),
        )


@dataclass(frozen=True)
class HostedSolanaGatewayBindings:
    """Generated gateway descriptors: one binding per capability."""

    chain: HostedSolanaGatewayCapabilityBinding
    transactions: HostedSolanaGatewayCapabilityBinding

    @classmethod
    def from_dict(cls, value: Mapping[str, Any]) -> "HostedSolanaGatewayBindings":
        return cls(
            chain=HostedSolanaGatewayCapabilityBinding.from_dict(value["chain"]),
            transactions=HostedSolanaGatewayCapabilityBinding.from_dict(
                value["transactions"]
            ),
        )


@dataclass(frozen=True)
class HostedSolanaGatewayTransports:
    chain: ChainClient
    transactions: TransactionTransport


def _is_secure_or_loopback_http_url(value: str) -> bool:
    try:
        parts = urlsplit(value)
    except ValueError:
        return False
    if not parts.netloc:
        return False
    if parts.scheme == "https":
        return True
    if parts.scheme == "http":
        host = parts.hostname or ""
        return host in _LOOPBACK_HOSTS
    return False


def validate_gateway_binding(
    capability: str,
    binding: HostedSolanaGatewayCapabilityBinding,
    required_scopes: tuple,
) -> None:
    """Validate one capability binding (port of the TS ``validateBinding``)."""
    auth = binding.auth
    complete = (
        _is_secure_or_loopback_http_url(binding.endpoint)
        and _GATEWAY_BINDING_ID_RE.match(binding.solana_gateway_binding_id) is not None
        and bool(binding.cluster.strip())
        and bool(binding.region.strip())
        and isinstance(auth.required, bool)
        and auth.mode == binding.auth_policy
        and _is_secure_or_loopback_http_url(auth.session_endpoint)
        and _is_secure_or_loopback_http_url(auth.jwks_url)
        and auth.token_transport == "bearer"
        and auth.audience == "arete:solana-gateway"
        and auth.target_kind == SOLANA_GATEWAY_BINDING_KIND
        and auth.target_id == binding.solana_gateway_binding_id
        and isinstance(auth.scopes, (list, tuple))
        and all(scope in auth.scopes for scope in required_scopes)
        and isinstance(auth.accepted_key_classes, (list, tuple))
        and isinstance(auth.transaction_entitlement_required, bool)
    )
    if not complete:
        raise AreteError(
            f"Hosted Solana gateway {capability} binding is incomplete or inconsistent"
        )


def _has_runtime_auth_strategy(auth: Optional[AuthConfig]) -> bool:
    return auth is not None and bool(
        auth.token or auth.get_token or auth.token_endpoint
    )


def _binding_auth_config(
    binding: HostedSolanaGatewayCapabilityBinding,
    runtime_auth: Optional[AuthConfig],
) -> Optional[AuthConfig]:
    """A configured runtime strategy wins; bindings that do not require auth
    keep whatever runtime auth exists; otherwise tokens are minted from the
    binding's session endpoint (keeping any publishable key and headers)."""
    if _has_runtime_auth_strategy(runtime_auth):
        return runtime_auth
    if not binding.auth.required:
        return runtime_auth
    base = runtime_auth if runtime_auth is not None else AuthConfig()
    return replace(base, token_endpoint=binding.auth.session_endpoint)


def create_hosted_solana_gateway_transports(
    bindings: HostedSolanaGatewayBindings,
    *,
    auth: Optional[AuthConfig] = None,
    http_client: Optional[Any] = None,  # httpx.AsyncClient, injectable for tests
) -> HostedSolanaGatewayTransports:
    """Construct explicit hosted chain and transaction transports from
    generated gateway descriptors. Tokens are isolated by exact binding
    target and scope."""
    validate_gateway_binding("chain", bindings.chain, CHAIN_REQUIRED_SCOPES)
    validate_gateway_binding(
        "transactions", bindings.transactions, TRANSACTIONS_REQUIRED_SCOPES
    )

    # Token-minting state is shared between capabilities resolving to the same
    # strategy identity (runtime strategy, or the same session endpoint).
    clients: Dict[str, HttpAuthClient] = {}

    def auth_client_for(
        binding: HostedSolanaGatewayCapabilityBinding,
    ) -> HttpAuthClient:
        identity = (
            "runtime-auth-strategy"
            if _has_runtime_auth_strategy(auth)
            else f"session-endpoint:{binding.auth.session_endpoint}"
        )
        client = clients.get(identity)
        if client is None:
            client = HttpAuthClient(
                auth=_binding_auth_config(binding, auth),
                websocket_url=None,
                http_client=http_client,
            )
            clients[identity] = client
        return client

    def target_for(binding: HostedSolanaGatewayCapabilityBinding) -> AuthTokenTarget:
        return AuthTokenTarget(
            kind=SOLANA_GATEWAY_BINDING_KIND,
            target_id=binding.solana_gateway_binding_id,
        )

    chain = HttpChainClient(
        bindings.chain.endpoint,
        auth_client_for(bindings.chain),
        target=target_for(bindings.chain),
    )
    transactions = HttpTransactionTransport(
        bindings.transactions.endpoint,
        auth_client_for(bindings.transactions),
        target=target_for(bindings.transactions),
    )
    return HostedSolanaGatewayTransports(chain=chain, transactions=transactions)
