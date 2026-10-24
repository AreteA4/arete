"""Async client for a hosted Arete Protocol deployment.

Port of ``typescript/core/src/protocol/client.ts``. The client mints
protocol sessions from the app's own key through the deployment's session
endpoint (``targetKind: "protocol-deployment"``), prepares steps, submits the
wallet-signed transactions, reads instances, participants and execution
intents, updates metadata, and follows an instance's event stream.

Read calls use a ``protocol:read`` session. The first prepare or submit mints
a session that also carries ``protocol:submit`` and ``protocol:prepare:*``
(secret keys) or ``protocol:prepare:<step>`` for each step a publishable key
may prepare. A refused token is refreshed and the request replayed once.

Submit is never retried by the client beyond that one auth replay: an intent
in ``submitted_unknown`` may still land, so read it with :meth:`get_intent`
instead of submitting again.
"""

from __future__ import annotations

import asyncio
import base64
import codecs
import inspect
import json
import os
import time
import uuid
from dataclasses import replace
from typing import (
    Any,
    AsyncIterator,
    Awaitable,
    Callable,
    Dict,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
    Union,
)
from urllib.parse import quote

import httpx

from arete.auth import (
    AuthConfig,
    AuthErrorCode,
    classify_api_key,
    resolve_auth_config,
    should_refresh_token,
)
from arete.errors import AreteError, AuthError, HttpRequestError
from arete.http import PROTOCOL_DEPLOYMENT_KIND, AuthTokenTarget, HttpAuthClient
from arete.protocol.handle import ProtocolInstanceHandle
from arete.protocol.metadata_proof import (
    DEFAULT_PROOF_TTL_SECONDS,
    ProtocolMetadataProofMessage,
    protocol_metadata_proof,
    protocol_metadata_proof_message,
)
from arete.protocol.sse import ServerSentEvent, ServerSentEventParser
from arete.protocol.types import (
    PROTOCOL_ACCESS_REJECTION_CODES,
    PROTOCOL_STATE_REJECTION_CODES,
    SQUADS_PROPOSER_MISSING_EVENT,
    ExecutionIntentV1,
    ProtocolDefinition,
    ProtocolInstanceDetail,
    ProtocolInstancePage,
    ProtocolIntentState,
    ProtocolMetadataState,
    ProtocolMetadataUpdateResult,
    ProtocolParticipantDetail,
    ProtocolPrepared,
    ProtocolPrepareResult,
    ProtocolRejection,
    ProtocolSquadsProposer,
    ProtocolSquadsStatus,
    ProtocolStepAvailability,
    ProtocolStreamEvent,
    ProtocolWalletActions,
)

PROTOCOL_READ_SCOPE = "protocol:read"
PROTOCOL_SUBMIT_SCOPE = "protocol:submit"
PROTOCOL_PREPARE_ANY_SCOPE = "protocol:prepare:*"

MAX_RECONNECT_DELAY_SECONDS = 30.0
_STREAM_EVENT_TYPES = ("transition", "participant", "observation")
_ACCESS_CODES = frozenset(PROTOCOL_ACCESS_REJECTION_CODES)
_STATE_CODES = frozenset(PROTOCOL_STATE_REJECTION_CODES)

# Seam for tests: the reconnect delay of event streams.
_sleep = asyncio.sleep

SignMessage = Callable[[bytes], Union[bytes, Awaitable[bytes]]]
"""Signs a metadata proof message with the participant wallet's Ed25519 key
and returns the 64-byte signature."""

OnStreamError = Callable[[Exception], Union[None, Awaitable[None]]]


def is_protocol_access_rejection_code(code: str) -> bool:
    """True when ``code`` is one of the access refusals of a prepare."""
    return code in _ACCESS_CODES


def is_protocol_state_rejection_code(code: str) -> bool:
    """True when ``code`` is a lifecycle rejection: the instance, or the
    preparing wallet's participant, is in a terminal state."""
    return code in _STATE_CODES


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _segment(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value:
        raise AreteError(f"{name} must be a non-empty string", "INVALID_CONFIG")
    # encodeURIComponent's unreserved set.
    return quote(value, safe="!~*'()")


def _normalize_wire_code(code: str) -> str:
    return code.strip().lower()


def _is_refresh_code(code: Any) -> bool:
    if not isinstance(code, str) or not code.strip():
        return False
    known = AuthErrorCode.from_wire_known(code.strip().replace("_", "-"))
    return known is not None and should_refresh_token(known)


def _read_json(content: bytes) -> Any:
    if not content:
        return None
    try:
        return json.loads(content)
    except (ValueError, UnicodeDecodeError):
        try:
            return content.decode("utf-8")
        except UnicodeDecodeError:
            return None


def _wire_code_of(response: httpx.Response, body: Any) -> Optional[str]:
    header = response.headers.get("X-Error-Code")
    if header:
        return header.strip()
    if isinstance(body, dict) and isinstance(body.get("code"), str):
        return body["code"]
    return None


def _response_error(what: str, response: httpx.Response, body: Any) -> HttpRequestError:
    wire = _wire_code_of(response, body)
    message: Optional[str] = None
    if isinstance(body, dict):
        for key in ("error", "message"):
            if isinstance(body.get(key), str):
                message = body[key]
                break
    if message is None:
        message = body if isinstance(body, str) and body else (response.reason_phrase or "request failed")
    return HttpRequestError(
        f"{what} returned {response.status_code}: {message}",
        code=wire or "PROTOCOL_REQUEST_FAILED",
        status=response.status_code,
        body=body,
        retryable=isinstance(body, dict) and body.get("retryable") is True,
        details={"status": response.status_code, "wire_error_code": wire, "body": body},
    )


def _runtime_key_class(auth: Optional[AuthConfig], declared: Optional[str]) -> str:
    if auth is not None and auth.secret_key is not None:
        return "secret"
    if auth is not None and auth.publishable_key is not None:
        return "secret" if classify_api_key(auth.publishable_key) == "secret" else "publishable"
    return declared or "publishable"


def _has_runtime_auth_strategy(auth: Optional[AuthConfig]) -> bool:
    return auth is not None and bool(auth.token or auth.get_token or auth.token_endpoint)


async def _maybe_await(value: Any) -> Any:
    if inspect.isawaitable(value):
        return await value
    return value


def _mint_refusal_code(error: AuthError) -> Optional[str]:
    """The access code a session mint refused a step with, if any."""
    details = error.details if isinstance(error.details, dict) else {}
    for key in ("wire_error_code", "body_code"):
        value = details.get(key)
        if isinstance(value, str) and _normalize_wire_code(value) in _ACCESS_CODES:
            return _normalize_wire_code(value)
    return None


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------


ARETE_API_URL_ENV = "ARETE_API_URL"
"""Environment variable read for the API origin when no ``api_url`` is given."""


def _non_empty(value: Optional[str]) -> Optional[str]:
    value = value.strip() if isinstance(value, str) else None
    return value or None


def resolve_protocol_endpoints(
    deployment: Any,
    *,
    api_url: Optional[str] = None,
    session_endpoint: Optional[str] = None,
    deployment_id: Optional[str] = None,
    environ: Optional[Mapping[str, str]] = None,
) -> Tuple[str, str, str]:
    """``(deployment_id, api_url, session_endpoint)`` a client calls.

    The options first, then ``ARETE_API_URL`` (the API origin, e.g.
    ``https://api.arete.run``), then the generated descriptor, so one
    generated client runs against another API by environment alone. An ``api_url`` (``<origin>/api``) without a
    ``session_endpoint`` carries its origin's session endpoint.
    """
    env = os.environ if environ is None else environ
    chosen_deployment = _non_empty(deployment_id) or deployment.deployment_id
    explicit_api = _non_empty(api_url)
    origin = None if explicit_api else _non_empty(env.get(ARETE_API_URL_ENV))
    if explicit_api:
        api = explicit_api.rstrip("/")
        base = api[: -len("/api")] if api.endswith("/api") else api
        sessions = f"{base}/ws/sessions"
    elif origin and origin.rstrip("/"):
        base = origin.rstrip("/")
        api = f"{base}/api"
        sessions = f"{base}/ws/sessions"
    else:
        api = deployment.api_url.rstrip("/")
        sessions = deployment.session_endpoint
    return chosen_deployment, api, _non_empty(session_endpoint) or sessions


class ProtocolClient:
    """Untyped client for a generated :class:`ProtocolDefinition`.

    Generated modules wrap it in typed publishable and secret clients; use it
    directly when the protocol's types do not matter.

    ``auth`` takes ``AuthConfig(publishable_key=...)`` (``public`` steps only)
    or ``AuthConfig(secret_key=...)`` (every client-callable step); ``token``,
    ``get_token``, ``token_endpoint`` and the ``ARETE_API_KEY`` / ``a4`` login
    fallback work too. ``key_class`` names the key class when ``auth`` cannot
    tell (a token or token provider); it defaults to ``publishable``.
    ``api_url`` (``<origin>/api``) or, when it is unset, ``ARETE_API_URL``
    (the API origin) switches environment; ``deployment_id`` calls another
    deployment than the generated one.
    ``steps`` limits which steps a publishable session may prepare (default:
    every ``public`` step).

    ``stacks`` and ``programs`` are connected members (``Arete`` clients and
    program SDKs) keyed like the definition's; :func:`arete.create_session`
    passes the session's. Instance handles follow the definition's live
    stack view through ``stacks``.
    """

    def __init__(
        self,
        definition: ProtocolDefinition,
        *,
        auth: Optional[AuthConfig] = None,
        key_class: Optional[str] = None,
        steps: Optional[Sequence[str]] = None,
        http_client: Optional[httpx.AsyncClient] = None,
        api_url: Optional[str] = None,
        session_endpoint: Optional[str] = None,
        deployment_id: Optional[str] = None,
        stacks: Optional[Mapping[str, Any]] = None,
        programs: Optional[Mapping[str, Any]] = None,
    ) -> None:
        if not isinstance(definition, ProtocolDefinition) or definition.kind != "protocol":
            raise AreteError(
                "ProtocolClient requires a generated protocol definition", "INVALID_CONFIG"
            )
        if key_class not in (None, "publishable", "secret"):
            raise AreteError("key_class must be 'publishable' or 'secret'", "INVALID_CONFIG")
        self.definition = definition
        self.deployment_id, self._api_url, endpoint = resolve_protocol_endpoints(
            definition.deployment,
            api_url=api_url,
            session_endpoint=session_endpoint,
            deployment_id=deployment_id,
        )

        resolved = resolve_auth_config(auth)
        self.key_class: str = _runtime_key_class(resolved, key_class)
        if resolved is not None and resolved.token is not None and not resolved.token:
            raise AuthError("Authentication token is empty", AuthErrorCode.TOKEN_INVALID_FORMAT)
        self._static_token = resolved is not None and bool(resolved.token)
        # Keys mint through the generated session endpoint unless the app
        # brought its own token strategy.
        if not _has_runtime_auth_strategy(resolved):
            resolved = (
                replace(resolved, token_endpoint=endpoint)
                if resolved is not None
                else AuthConfig(token_endpoint=endpoint)
            )
        self._owns_http = http_client is None
        self._http = http_client or httpx.AsyncClient(timeout=30.0)
        self._auth = HttpAuthClient(auth=resolved, http_client=self._http)
        self._target = AuthTokenTarget(kind=PROTOCOL_DEPLOYMENT_KIND, target_id=self.deployment_id)

        self._read_scopes = (PROTOCOL_READ_SCOPE,)
        if self.key_class == "secret":
            prepare_scopes = [PROTOCOL_PREPARE_ANY_SCOPE]
        else:
            preparable = (
                list(steps)
                if steps is not None
                else [name for name, step in definition.steps.items() if step.access == "public"]
            )
            prepare_scopes = [f"protocol:prepare:{step}" for step in preparable]
        self._transaction_scopes = (
            PROTOCOL_READ_SCOPE,
            PROTOCOL_SUBMIT_SCOPE,
            *prepare_scopes,
        )
        self._filterable_instance_fields = frozenset(
            name for name, field in definition.instance_metadata.items() if field.filterable
        )
        self._streams: "set[ProtocolEventStream]" = set()
        self._handles: "set[ProtocolInstanceHandle]" = set()
        self._live_stacks: Mapping[str, Any] = dict(stacks) if stacks is not None else {}
        self._member_programs: Optional[Mapping[str, Any]] = (
            dict(programs) if programs is not None else None
        )

    @property
    def stacks(self) -> Mapping[str, Any]:
        """The connected stacks the client was given, else the protocol's
        stack definitions (for ``arete.create_session(stacks=...)``)."""
        return self._live_stacks if self._live_stacks else self.definition.stacks

    @property
    def programs(self) -> Mapping[str, Any]:
        """The connected programs the client was given, else the protocol's
        program definitions (for ``arete.create_session(programs=...)``)."""
        if self._member_programs is not None:
            return self._member_programs
        return self.definition.programs

    async def __aenter__(self) -> "ProtocolClient":
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        """Close every instance handle and event stream, and the HTTP client
        if the client made it."""
        for handle in list(self._handles):
            await handle.aclose()
        for stream in list(self._streams):
            await stream.aclose()
        if self._owns_http:
            await self._http.aclose()

    # ------------------------------------------------------------------
    # Authenticated HTTP

    async def session_token(self, scopes: Sequence[str], *, force_refresh: bool = False) -> Optional[str]:
        """Mint (or reuse) a protocol session covering ``scopes``."""
        return await self._auth.get_token(
            target=self._target, scopes=list(scopes), force_refresh=force_refresh
        )

    async def _is_refreshable(self, response: httpx.Response) -> bool:
        if _is_refresh_code(response.headers.get("X-Error-Code")):
            return True
        if response.status_code != 401:
            return False
        body = _read_json(await response.aread())
        return isinstance(body, dict) and _is_refresh_code(body.get("code"))

    async def _send(
        self,
        method: str,
        url: str,
        *,
        scopes: Sequence[str],
        json_body: Any = None,
        params: Optional[Mapping[str, str]] = None,
        headers: Optional[Mapping[str, str]] = None,
        stream: bool = False,
        force_refresh: bool = False,
    ) -> httpx.Response:
        content = None if json_body is None else json.dumps(json_body, separators=(",", ":"))

        async def attempt(force: bool) -> httpx.Response:
            token = await self.session_token(scopes, force_refresh=force)
            request_headers: Dict[str, str] = dict(headers or {})
            if token:
                request_headers["Authorization"] = f"Bearer {token}"
            if content is not None:
                request_headers["Content-Type"] = "application/json"
            request = self._http.build_request(
                method,
                url,
                content=content,
                params=dict(params) if params else None,
                headers=request_headers,
                # An event stream stays open between events.
                timeout=httpx.Timeout(30.0, read=None) if stream else httpx.USE_CLIENT_DEFAULT,
            )
            try:
                return await self._http.send(request, stream=stream)
            except httpx.HTTPError as error:
                raise HttpRequestError(
                    f"HTTP request to '{url}' failed: {error}",
                    code="PROTOCOL_REQUEST_FAILED",
                    status=0,
                ) from error

        response = await attempt(force_refresh)
        if not response.is_success and not self._static_token and await self._is_refreshable(response):
            await response.aclose()
            response = await attempt(True)
        return response

    async def _json(self, what: str, method: str, url: str, **kwargs: Any) -> Any:
        response = await self._send(method, url, **kwargs)
        body = _read_json(response.content)
        if not response.is_success:
            raise _response_error(what, response, body)
        return body

    def _instance_url(self, instance_id: str) -> str:
        return f"{self._api_url}/protocol-instances/{_segment(instance_id, 'instance')}"

    # ------------------------------------------------------------------
    # Steps

    async def prepare(
        self,
        step: str,
        *,
        wallet: str,
        instance: Optional[str] = None,
        args: Optional[Mapping[str, Any]] = None,
        transaction_format: str = "v1",
        idempotency_key: Optional[str] = None,
        client_context: Optional[Mapping[str, Any]] = None,
    ) -> ProtocolPrepareResult:
        """Ask the hosted API to build a step's transactions.

        Returns :class:`ProtocolPrepared` (the execution intent and the
        partially signed transactions for the wallet to sign) or
        :class:`ProtocolRejection` (a guard code or an access code). Reuse
        ``idempotency_key`` to make a retried prepare return the same intent.
        """
        definition = self.definition.steps.get(step)
        if definition is None:
            return ProtocolRejection(rejected="step_not_client_callable", step=step)
        if self.key_class == "publishable" and definition.access == "secret":
            return ProtocolRejection(rejected="step_requires_secret_key", step=step)
        if not isinstance(wallet, str) or not wallet:
            raise AreteError(f"{step}.prepare requires a wallet", "INVALID_CONFIG")
        if transaction_format not in ("v1", "v0"):
            raise AreteError("transaction_format must be 'v1' or 'v0'", "INVALID_CONFIG")
        if definition.scope == "create":
            url = (
                f"{self._api_url}/protocol-deployments/{_segment(self.deployment_id, 'deployment_id')}"
                f"/steps/{_segment(step, 'step')}/prepare"
            )
        else:
            if not isinstance(instance, str) or not instance:
                raise AreteError(f"{step}.prepare requires an instance id", "INVALID_CONFIG")
            url = f"{self._instance_url(instance)}/steps/{_segment(step, 'step')}/prepare"
        body: Dict[str, Any] = {
            "idempotencyKey": idempotency_key or str(uuid.uuid4()),
            "wallet": wallet,
            "args": dict(args or {}),
            "transactionFormat": transaction_format,
        }
        if client_context is not None:
            body["clientContext"] = dict(client_context)

        try:
            response = await self._send(
                "POST", url, scopes=self._transaction_scopes, json_body=body
            )
        except AuthError as error:
            # The session mint refuses a step the key cannot prepare.
            code = _mint_refusal_code(error)
            if code is not None:
                return ProtocolRejection(rejected=code, step=step)
            raise

        payload = _read_json(response.content)
        if response.is_success:
            if (
                not isinstance(payload, dict)
                or not isinstance(payload.get("intent"), dict)
                or not isinstance(payload.get("transactions"), list)
            ):
                raise HttpRequestError(
                    "Prepare returned an unexpected body",
                    code="PROTOCOL_REQUEST_FAILED",
                    status=response.status_code,
                    body=payload,
                )
            return ProtocolPrepared(intent=payload["intent"], transactions=payload["transactions"])
        if (
            response.status_code == 422
            and isinstance(payload, dict)
            and isinstance(payload.get("rejected"), str)
        ):
            guard = payload.get("guard")
            detail = payload.get("detail")
            return ProtocolRejection(
                rejected=payload["rejected"],
                step=payload["step"] if isinstance(payload.get("step"), str) else step,
                guard=guard if isinstance(guard, str) else None,
                detail=detail if isinstance(detail, dict) else None,
            )
        if response.status_code == 403:
            code = _wire_code_of(response, payload)
            if code is not None and _normalize_wire_code(code) in _ACCESS_CODES:
                detail = payload.get("detail") if isinstance(payload, dict) else None
                return ProtocolRejection(
                    rejected=_normalize_wire_code(code),
                    step=step,
                    detail=detail if isinstance(detail, dict) else None,
                )
        raise _response_error(f"Prepare of step '{step}'", response, payload)

    async def submit(
        self,
        intent: Union[ExecutionIntentV1, Mapping[str, Any], str, ProtocolPrepared],
        signed_transactions: Sequence[Union[str, bytes, bytearray]],
    ) -> ProtocolIntentState:
        """Submit the wallet-signed transactions of a prepared intent
        (base64 strings or raw bytes, in the prepared order).

        The client does not retry a submit. When the returned state is
        ``submitted_unknown``, read it again with :meth:`get_intent`; never
        resend the transactions.
        """
        if isinstance(intent, ProtocolPrepared):
            execution_id: Any = intent.execution_id
        elif isinstance(intent, Mapping):
            execution_id = intent.get("executionId")
        else:
            execution_id = intent
        if isinstance(signed_transactions, (str, bytes)) or not signed_transactions:
            raise AreteError("submit requires the signed transactions", "INVALID_CONFIG")
        encoded = [
            transaction
            if isinstance(transaction, str)
            else base64.b64encode(bytes(transaction)).decode("ascii")
            for transaction in signed_transactions
        ]
        return await self._json(
            "Submit",
            "POST",
            f"{self._api_url}/execution-intents/{_segment(execution_id, 'execution_id')}/submit",
            scopes=self._transaction_scopes,
            json_body={"signedTransactions": encoded},
        )

    @property
    def squads(self) -> "ProtocolSquads":
        """Squads onboarding for the calling account (its proposer and its
        membership status)."""
        return ProtocolSquads(self)

    def _squads_url(self, route: str) -> str:
        return (
            f"{self._api_url}/protocol-deployments/"
            f"{_segment(self.deployment_id, 'deployment_id')}/squads/{route}"
        )

    async def get_intent(self, execution_id: str) -> ProtocolIntentState:
        """Read an execution intent's state."""
        return await self._json(
            "Execution intent read",
            "GET",
            f"{self._api_url}/execution-intents/{_segment(execution_id, 'execution_id')}",
            scopes=self._read_scopes,
        )

    async def get_receipt(self, signature: str) -> Dict[str, Any]:
        """The signed execution receipt of a transaction this deployment
        landed. Check it with :func:`arete.protocol.verify_receipt` against
        :meth:`get_receipt_keys`, and with
        :func:`arete.protocol.verify_receipt_on_chain` against the transaction
        from any RPC. Raises with status 404 and code
        ``protocol-receipt-not-found`` when there is none."""
        return await self._json(
            "Protocol receipt read",
            "GET",
            f"{self._api_url}/protocol-receipts/{_segment(signature, 'signature')}",
            scopes=self._read_scopes,
        )

    async def get_receipt_keys(self) -> Dict[str, Any]:
        """The published receipt keys, including retired ones."""
        return await self._json(
            "Protocol receipt keys read",
            "GET",
            f"{self._api_url}/protocol-receipts/keys",
            scopes=self._read_scopes,
        )

    # ------------------------------------------------------------------
    # Instances and participants

    async def list_instances(
        self,
        *,
        state: Optional[str] = None,
        metadata: Optional[Mapping[str, Any]] = None,
        cursor: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> ProtocolInstancePage:
        """List the deployment's instances; ``metadata`` filters exactly on
        ``filterable`` instance metadata fields."""
        params: Dict[str, str] = {}
        if state is not None:
            params["state"] = state
        if cursor is not None:
            params["cursor"] = cursor
        if limit is not None:
            params["limit"] = str(limit)
        for name, value in (metadata or {}).items():
            if value is None:
                continue
            if name not in self._filterable_instance_fields:
                raise AreteError(
                    f"Instance metadata field '{name}' is not filterable", "INVALID_CONFIG"
                )
            params[f"metadata.{name}"] = (
                ("true" if value else "false") if isinstance(value, bool) else str(value)
            )
        return await self._json(
            "Protocol instance list",
            "GET",
            f"{self._api_url}/protocol-deployments/{_segment(self.deployment_id, 'deployment_id')}/instances",
            scopes=self._read_scopes,
            params=params,
        )

    async def get_instance(
        self, instance_id: str, *, wallet: Optional[str] = None
    ) -> ProtocolInstanceDetail:
        """Read an instance; with ``wallet``, ``allowedSteps`` is evaluated for it."""
        return await self._json(
            "Protocol instance read",
            "GET",
            self._instance_url(instance_id),
            scopes=self._read_scopes,
            params={"wallet": wallet} if wallet else None,
        )

    async def get_participant(self, instance_id: str, wallet: str) -> ProtocolParticipantDetail:
        return await self._json(
            "Protocol participant read",
            "GET",
            f"{self._instance_url(instance_id)}/participants/{_segment(wallet, 'wallet')}",
            scopes=self._read_scopes,
        )

    async def allowed_steps(self, instance_id: str, wallet: str) -> List[ProtocolStepAvailability]:
        """Which steps ``wallet`` can call on ``instance_id`` now, and why not."""
        _segment(wallet, "wallet")
        detail = await self.get_instance(instance_id, wallet=wallet)
        return list(detail.get("allowedSteps") or [])

    async def actions(
        self,
        wallet: str,
        *,
        instance: Optional[str] = None,
        cursor: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> ProtocolWalletActions:
        """What ``wallet`` can do on the deployment: the ``create`` steps it
        can call and, per instance it is related to (participant, creator or
        role holder), the steps it can call there, and why not.

        Paged by ``cursor`` (the API caps ``limit`` at 50); ``instance``
        narrows the read to one instance. Missing list fields are filled
        with empty lists and ``nextCursor`` with ``None``.
        """
        params: Dict[str, str] = {}
        if instance:
            params["instance"] = instance
        if cursor is not None:
            params["cursor"] = cursor
        if limit is not None:
            params["limit"] = str(limit)
        deployment = _segment(self.deployment_id, "deployment_id")
        body = await self._json(
            "Protocol wallet actions read",
            "GET",
            f"{self._api_url}/protocol-deployments/{deployment}/wallets/{_segment(wallet, 'wallet')}/actions",
            scopes=self._read_scopes,
            params=params,
        )
        body.setdefault("createSteps", [])
        body.setdefault("instances", [])
        body.setdefault("nextCursor", None)
        for entry in body["instances"]:
            entry.setdefault("relation", [])
            entry.setdefault("participantState", None)
            entry.setdefault("steps", [])
        return body

    # ------------------------------------------------------------------
    # Metadata

    async def get_metadata(
        self, instance_id: str, *, participant: Optional[str] = None
    ) -> ProtocolMetadataState:
        """An instance's metadata, or ``participant``'s metadata on it."""
        if participant is not None:
            participant_detail = await self.get_participant(instance_id, participant)
            return ProtocolMetadataState(
                metadata=dict(participant_detail.get("metadata") or {}), version=None
            )
        detail = await self.get_instance(instance_id)
        return ProtocolMetadataState(
            metadata=dict(detail.get("metadata") or {}), version=detail.get("metadataVersion")
        )

    def metadata_proof_message(
        self,
        instance_id: str,
        patch: Mapping[str, Any],
        *,
        wallet: str,
        expires_at: Optional[int] = None,
        now: Optional[int] = None,
    ) -> ProtocolMetadataProofMessage:
        """The message ``wallet`` signs to authorise ``patch``. ``expires_at``
        defaults to four minutes from now."""
        current = int(time.time()) if now is None else now
        return protocol_metadata_proof_message(
            deployment_id=self.deployment_id,
            instance_id=instance_id,
            wallet=wallet,
            patch=patch,
            expires_at=expires_at if expires_at is not None else current + DEFAULT_PROOF_TTL_SECONDS,
            now=current,
        )

    async def update_metadata(
        self,
        instance_id: str,
        patch: Mapping[str, Any],
        *,
        participant: Optional[str] = None,
        wallet_proof: Optional[Mapping[str, Any]] = None,
        sign: Optional[SignMessage] = None,
        wallet: Optional[str] = None,
        expires_at: Optional[int] = None,
    ) -> ProtocolMetadataUpdateResult:
        """Apply a JSON merge patch (``None`` removes a field) to an
        instance's metadata, or to ``participant``'s.

        Participant-writer fields need a wallet proof: pass ``wallet_proof``
        (see :func:`arete.protocol.protocol_metadata_proof`), or ``sign``, a
        callback that signs the proof message with ``wallet`` (default:
        ``participant``) and returns the 64-byte signature.
        """
        if not isinstance(patch, Mapping):
            raise AreteError("A metadata update requires a patch object", "INVALID_CONFIG")
        proof: Optional[Mapping[str, Any]] = wallet_proof
        if proof is None and sign is not None:
            signer = wallet or participant
            if not signer:
                raise AreteError(
                    "Signing a metadata proof requires the signing wallet", "INVALID_CONFIG"
                )
            message = self.metadata_proof_message(
                instance_id, patch, wallet=signer, expires_at=expires_at
            )
            signature = await _maybe_await(sign(message.bytes))
            proof = protocol_metadata_proof(
                wallet=signer, signature=signature, expires_at=message.expires_at
            )
        body: Dict[str, Any] = {"patch": dict(patch)}
        if participant is not None:
            body["participant"] = participant
        if proof is not None:
            body["proof"] = dict(proof)
        return await self._json(
            "Metadata update",
            "PATCH",
            f"{self._instance_url(instance_id)}/metadata",
            scopes=self._read_scopes,
            json_body=body,
        )

    # ------------------------------------------------------------------
    # Events

    def events(
        self,
        instance_id: str,
        *,
        last_event_id: Optional[str] = None,
        on_error: Optional[OnStreamError] = None,
    ) -> "ProtocolEventStream":
        """Follow an instance's event stream::

            async with client.events(instance_id) as stream:
                async for event in stream:
                    ...

        Reconnects and resumes after the last event id transparently, backing
        off 1s, 2s, 4s ... up to 30s after failures, which go to ``on_error``.
        A refusal the hosted API will repeat (a 4xx other than 408 and 429,
        such as an unknown instance) ends the iteration by raising it.
        """
        stream = ProtocolEventStream(
            self, f"{self._instance_url(instance_id)}/events", last_event_id, on_error
        )
        self._streams.add(stream)
        return stream


    def instance_handle(
        self, instance_id: str, *, wallet: Optional[str] = None
    ) -> ProtocolInstanceHandle:
        """A live instance, kept current from its event stream (and its live
        stack row, when the client has connected stacks)::

            async with client.instance_handle(instance_id) as handle:
                async for snapshot in handle:
                    print(snapshot.instance["state"], snapshot.live)

        ``await`` it instead to start it without a ``with`` block. With
        ``wallet``, ``allowedSteps`` is evaluated for it on every read.
        """
        return ProtocolInstanceHandle(self, instance_id, wallet=wallet)


class ProtocolSquads:
    """Squads onboarding from :attr:`ProtocolClient.squads`.

    Admin steps are proposed to a team's Squads v4 multisig by a proposer key
    that belongs to the calling account and may only initiate. Arete never
    votes or executes, and builds no Squads transactions: the team adds the
    proposer on the dashboard, in the Squads app, or from code with the
    squads-v4 program SDK."""

    def __init__(self, client: ProtocolClient) -> None:
        self._client = client

    async def proposer(self) -> ProtocolSquadsProposer:
        """The calling account's proposer and the permission it needs."""
        client = self._client
        return await client._json(
            "Squads proposer read",
            "GET",
            client._squads_url("proposer"),
            scopes=client._read_scopes,
        )

    async def status(self, multisig: str) -> ProtocolSquadsStatus:
        """Whether the proposer is an Initiate-only member of ``multisig``."""
        if not multisig:
            raise AreteError("status requires a multisig", "INVALID_CONFIG")
        client = self._client
        status: ProtocolSquadsStatus = await client._json(
            "Squads proposer status",
            "GET",
            client._squads_url("status"),
            scopes=client._read_scopes,
            params={"multisig": multisig},
        )
        status.setdefault("mask", None)
        return status


class _FatalStreamError(Exception):
    def __init__(self, error: Exception) -> None:
        super().__init__(str(error))
        self.error = error


class ProtocolEventStream:
    """An async iterator over an instance's events; see
    :meth:`ProtocolClient.events`."""

    def __init__(
        self,
        client: ProtocolClient,
        url: str,
        last_event_id: Optional[str],
        on_error: Optional[OnStreamError],
    ) -> None:
        self._client = client
        self._url = url
        self._last_event_id = last_event_id
        self._on_error = on_error
        self._closed = False
        self._force_refresh = False
        self._response: Optional[httpx.Response] = None
        self._iterator: Optional[AsyncIterator[ProtocolStreamEvent]] = None

    @property
    def last_event_id(self) -> Optional[str]:
        """The id of the last event received; reconnects resume after it."""
        return self._last_event_id

    @property
    def closed(self) -> bool:
        return self._closed

    def __aiter__(self) -> AsyncIterator[ProtocolStreamEvent]:
        if self._iterator is None:
            self._iterator = self._run()
        return self._iterator

    async def __aenter__(self) -> "ProtocolEventStream":
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._client._streams.discard(self)
        response, self._response = self._response, None
        if response is not None:
            await response.aclose()
        iterator = self._iterator
        if iterator is not None and hasattr(iterator, "aclose"):
            try:
                await iterator.aclose()  # type: ignore[attr-defined]
            except RuntimeError:
                pass  # closing from inside the iteration

    async def _report(self, error: Exception) -> None:
        if self._closed or self._on_error is None:
            return
        try:
            await _maybe_await(self._on_error(error))
        except Exception:  # noqa: BLE001 - a throwing callback must not end the stream
            pass

    async def _dispatch(self, event: ServerSentEvent) -> Optional[ProtocolStreamEvent]:
        try:
            data = json.loads(event.data)
        except ValueError:
            await self._report(
                AreteError(
                    f"Event stream sent invalid JSON for '{event.event}'",
                    "PROTOCOL_STREAM_INVALID",
                )
            )
            return None
        if event.event == "error":
            code = data.get("code") if isinstance(data, dict) else None
            code = code if isinstance(code, str) else "unknown"
            if _is_refresh_code(code) or "expired" in code.lower():
                self._force_refresh = True
                return None
            await self._report(
                AreteError(f"Event stream error: {code}", code, {"wire_error_code": code})
            )
            return None
        if not isinstance(data, dict):
            return None
        event_id = event.id or ""
        if event.event == "emit" and isinstance(data.get("event"), str):
            return ProtocolStreamEvent(type="emit", id=event_id, data=data, event=data["event"])
        if event.event in _STREAM_EVENT_TYPES or event.event == SQUADS_PROPOSER_MISSING_EVENT:
            return ProtocolStreamEvent(type=event.event, id=event_id, data=data)
        return None

    async def _run(self) -> AsyncIterator[ProtocolStreamEvent]:
        failures = 0
        try:
            while not self._closed:
                try:
                    headers = {"Accept": "text/event-stream", "Cache-Control": "no-cache"}
                    if self._last_event_id is not None:
                        headers["Last-Event-ID"] = self._last_event_id
                    response = await self._client._send(
                        "GET",
                        self._url,
                        scopes=self._client._read_scopes,
                        headers=headers,
                        stream=True,
                        force_refresh=self._force_refresh,
                    )
                    self._force_refresh = False
                    self._response = response
                    try:
                        if not response.is_success:
                            error = _response_error(
                                "Event stream", response, _read_json(await response.aread())
                            )
                            status = response.status_code
                            if 400 <= status < 500 and status not in (408, 429):
                                raise _FatalStreamError(error)
                            raise error
                        failures = 0
                        parser = ServerSentEventParser(self._last_event_id)
                        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
                        try:
                            async for chunk in response.aiter_bytes():
                                for raw in parser.push(decoder.decode(chunk)):
                                    if raw.id is not None:
                                        self._last_event_id = raw.id
                                    event = await self._dispatch(raw)
                                    if event is not None:
                                        yield event
                                    if self._closed:
                                        return
                        finally:
                            parser.end()
                    finally:
                        self._response = None
                        await response.aclose()
                    # The server ends streams periodically and when the session
                    # expires: reconnect after the last id.
                    if not self._closed:
                        await _sleep((parser.retry_ms or 0) / 1000)
                except _FatalStreamError as fatal:
                    self._closed = True
                    self._client._streams.discard(self)
                    raise fatal.error
                except Exception as error:  # noqa: BLE001 - reconnect on any failure
                    if self._closed:
                        return
                    await self._report(error)
                    failures += 1
                    await _sleep(min(MAX_RECONNECT_DELAY_SECONDS, float(2 ** (failures - 1))))
        finally:
            self._closed = True
            self._client._streams.discard(self)
