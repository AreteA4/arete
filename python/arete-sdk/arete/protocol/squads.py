"""Account Squads: the Squads v4 multisigs an Arete account has attached.

Port of ``typescript/core/src/protocol/squads.ts``.

Admin steps of a protocol are proposed to a multisig attached to the
deployment owner's account, by a proposer key that belongs to the account and
may only initiate. The account attaches each multisig once (or creates one)
and adds the proposer as an Initiate-only member; production deployments
refuse a Squads step whose multisig is not an ``active`` attachment
(``protocol-squads-setup-required``).

The easiest path is the dashboard (:data:`SQUADS_DASHBOARD_URL`), which signs
with any standard browser wallet, hardware wallets included. This client is
for scripts and hot wallets: every write returns an unsigned transaction for
one wallet to sign, and :meth:`SquadsClient.submit` sends and confirms it.
Arete never holds or asks for a member key.
"""

from __future__ import annotations

import base64
import os
from typing import Any, Dict, List, Literal, Optional, Sequence, TypedDict, Union
from urllib.parse import quote

import httpx

from arete.auth import AuthConfig, classify_api_key, resolve_auth_config
from arete.errors import AreteError
from arete.protocol.client import (
    ARETE_API_URL_ENV,
    _maybe_await,
    _read_json,
    _response_error,
    _segment,
)
from arete.protocol.types import ProtocolTransactionFormat

SQUADS_DASHBOARD_URL = "https://app.arete.run/dashboard/squads"
"""The dashboard page that sets up and manages the account's multisigs."""

SQUADS_SETUP_REQUIRED_CODE = "protocol-squads-setup-required"
"""The wire code of a deployment refused because its multisig is not set up."""

_DEFAULT_API_URL = "https://api.arete.run/api"


def squads_dashboard_url(multisig: Optional[str] = None) -> str:
    """The dashboard page for the account's multisigs, or for one of them."""
    trimmed = multisig.strip() if isinstance(multisig, str) else ""
    return f"{SQUADS_DASHBOARD_URL}/{quote(trimmed, safe='')}" if trimmed else SQUADS_DASHBOARD_URL


# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------

SquadsSetup = Literal["notStarted", "ready", "needsAttention"]
"""``notStarted``: nothing attached; ``ready``: one is ``active``;
``needsAttention``: none is."""

SquadsAttachmentState = Literal["pending", "active", "archived"]

SquadsAttachmentProblem = Literal[
    "not_member",
    "permissions_invalid",
    "squads_multisig_missing",
    "squads_multisig_invalid",
    "create_unconfirmed",
]
"""Why an attachment is not ``active``."""

SquadsPermission = Literal["initiate", "vote", "execute"]


class SquadsAccountProposer(TypedDict):
    """The account's proposer. It proposes; it never votes or executes."""

    pubkey: str
    permissions: Literal["initiate"]
    lamports: Optional[int]
    """For information; ``None`` when the chain did not answer."""


class SquadsMember(TypedDict):
    pubkey: str
    mask: int
    """The Squads permission mask (Initiate 1, Vote 2, Execute 4)."""
    permissions: List[SquadsPermission]


class SquadsAttachmentDeployment(TypedDict):
    """A production deployment whose Squads steps propose to the multisig."""

    id: str
    protocolId: str
    protocolName: str
    protocolSlug: str
    status: str
    steps: List[str]


class SquadsNotificationDeployment(TypedDict, total=False):
    id: str
    protocolName: str
    steps: List[str]


class SquadsNotification(TypedDict):
    """An ``archived`` attachment that was ``active``, with the deployments
    that used it then. It goes away once the attachment is active again or
    forgotten."""

    multisig: str
    problem: Optional[str]
    problemDescription: Optional[str]
    deployments: List[SquadsNotificationDeployment]
    since: str
    dashboardUrl: str


class SquadsAttachment(TypedDict):
    """A multisig attached to the account, refreshed from the chain."""

    address: str
    label: Optional[str]
    state: SquadsAttachmentState
    problem: Optional[str]
    problemDescription: Optional[str]
    """The problem as a plain sentence."""
    createdVia: Literal["page", "created", "deployment"]
    stateChangedAt: str
    createdAt: str
    threshold: Optional[int]
    members: Optional[List[SquadsMember]]
    proposerMask: Optional[int]
    vault: str
    squadsUrl: str
    dashboardUrl: str
    deploymentConfigSnippet: str
    deployments: List[SquadsAttachmentDeployment]


class SquadsConfigProposal(TypedDict):
    """A config proposal Arete built for the multisig."""

    kind: Literal["addProposer", "removeProposer"]
    transactionIndex: int
    proposal: str
    status: str
    approvals: List[str]
    threshold: Optional[int]
    squadsUrl: str


class SquadsStepProposal(TypedDict):
    """A proposal the account's proposer made for a deployment's admin step."""

    deploymentId: str
    step: str
    instanceId: Optional[str]
    status: str
    transactionIndex: int
    proposal: str
    createdAt: str
    squadsUrl: str


class SquadsAttachmentDetail(SquadsAttachment):
    proposer: str
    """The account's proposer pubkey."""
    configProposals: List[SquadsConfigProposal]
    proposals: List[SquadsStepProposal]


class SquadsAccountStatus(TypedDict):
    setup: SquadsSetup
    proposer: SquadsAccountProposer
    dashboardUrl: str
    multisigs: List[SquadsAttachment]


class SquadsPreparedTransaction(TypedDict):
    """An unsigned transaction for one wallet (``signer``, which also pays)
    to sign. Pass the signed bytes and ``transactionId`` to ``submit``."""

    transactionId: str
    kind: Literal["createMultisig", "addProposer", "approve", "execute", "removeProposer"]
    transactionBase64: str
    transactionFormat: ProtocolTransactionFormat
    signer: str
    lastValidBlockHeight: int
    multisig: str
    transactionIndex: Optional[int]
    proposal: Optional[str]
    next: str


class SquadsSubmitResult(TypedDict):
    """``pending``: sent, not confirmed yet; submit the same body again.
    ``expired``: prepare a new transaction."""

    transactionId: str
    signature: str
    status: Literal["confirmed", "failed", "pending", "expired"]
    error: Optional[str]
    multisig: SquadsAttachment


class SquadsCreateMember(TypedDict):
    pubkey: str
    permissions: List[SquadsPermission]


class SquadsSetupRequiredDetails(TypedDict, total=False):
    """``details`` of a ``protocol-squads-setup-required`` refusal."""

    step: str
    multisig: str
    state: SquadsAttachmentState
    problem: Optional[str]
    proposer: str
    setupUrl: str
    """The multisig's dashboard page, where the proposer is added."""
    commands: Dict[str, str]


class SquadsSetupRequired(TypedDict):
    message: str
    details: SquadsSetupRequiredDetails


# ---------------------------------------------------------------------------
# The 409 deploy gate
# ---------------------------------------------------------------------------


def _normalized(code: Any) -> Optional[str]:
    return code.strip().lower().replace("_", "-") if isinstance(code, str) else None


def _from_body(body: Any, code: Optional[str], fallback: Optional[str]) -> Optional[SquadsSetupRequired]:
    if not isinstance(body, dict) or not isinstance(body.get("details"), dict):
        return None
    if _normalized(body.get("code")) != SQUADS_SETUP_REQUIRED_CODE and code != SQUADS_SETUP_REQUIRED_CODE:
        return None
    message = body.get("message")
    if not isinstance(message, str):
        message = body.get("error") if isinstance(body.get("error"), str) else None
    return {
        "message": message or fallback or "The Squads multisig is not set up for your account",
        "details": body["details"],
    }


def squads_setup_required(error: Any) -> Optional[SquadsSetupRequired]:
    """The step, multisig and dashboard link of a
    ``protocol-squads-setup-required`` refusal (deployment create or
    activation), from an :class:`AreteError` or a raw response body;
    ``None`` for anything else."""
    if isinstance(error, AreteError):
        details = error.details if isinstance(error.details, dict) else {}
        body = getattr(error, "body", None)
        if body is None:
            body = details.get("body")
        code = _normalized(details.get("wire_error_code")) or _normalized(error.code)
        return _from_body(body, code, str(error))
    return _from_body(error, None, None)


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------


def _resolve_api_url(api_url: Optional[str]) -> str:
    configured = api_url.strip() if isinstance(api_url, str) else ""
    if configured:
        return configured.rstrip("/")
    origin = (os.environ.get(ARETE_API_URL_ENV) or "").strip().rstrip("/")
    if origin:
        return (origin[: -len("/api")] if origin.endswith("/api") else origin) + "/api"
    return _DEFAULT_API_URL


def _proposal_index(value: Any) -> str:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise AreteError("transaction_index must be a non-negative integer", "INVALID_CONFIG")
    return str(value)


def _member_body(member: Any, transaction_format: Optional[str], what: str) -> Dict[str, Any]:
    if not isinstance(member, str) or not member.strip():
        raise AreteError(f"{what} requires a member", "INVALID_CONFIG")
    body: Dict[str, Any] = {"member": member.strip()}
    if transaction_format is not None:
        body["transactionFormat"] = transaction_format
    return body


class SquadsClient:
    """The calling account's Squads multisigs, over ``<api>/account/squads``.

    Authenticate with a secret or agent key (``api_key=`` or
    ``AuthConfig(secret_key=...)``; ``ARETE_API_KEY`` and the ``a4`` login are
    the fallback) or a dashboard token (``token=``,
    ``AuthConfig(token=...)`` or ``AuthConfig(get_token=...)``). Publishable
    keys are refused. ``api_url`` is ``<origin>/api``; when it is unset,
    ``ARETE_API_URL`` (the API origin), then ``https://api.arete.run``::

        async with SquadsClient() as squads:
            status = await squads.status()
            print(status["setup"], status["dashboardUrl"])
    """

    def __init__(
        self,
        *,
        auth: Optional[AuthConfig] = None,
        api_key: Optional[str] = None,
        token: Optional[str] = None,
        api_url: Optional[str] = None,
        http_client: Optional[httpx.AsyncClient] = None,
    ) -> None:
        if sum(value is not None for value in (auth, api_key, token)) > 1:
            raise AreteError("Pass one of auth, api_key or token", "INVALID_CONFIG")
        if api_key is not None:
            auth = AuthConfig(secret_key=api_key)
        elif token is not None:
            auth = AuthConfig(token=token)
        resolved = resolve_auth_config(auth)
        self._static: Optional[str] = None
        self._get_token = None
        secret_only = (
            "Account Squads routes need a secret or agent key (api_key, ARETE_API_KEY "
            "or the a4 login) or a dashboard token"
        )
        if resolved is not None and resolved.token is not None:
            if not resolved.token:
                raise AreteError("Authentication token is empty", "INVALID_CONFIG")
            self._static = resolved.token
        elif resolved is not None and resolved.get_token is not None:
            self._get_token = resolved.get_token
        else:
            key = resolved.secret_key if resolved is not None else None
            publishable = resolved.publishable_key if resolved is not None else None
            if key is None and publishable is not None and classify_api_key(publishable) == "secret":
                key = publishable
            if key is not None and classify_api_key(key) == "publishable":
                key = None
                publishable = publishable or "publishable"
            if not key:
                if publishable is not None:
                    raise AreteError(
                        f"{secret_only}; publishable keys are refused. Use the dashboard "
                        f"({SQUADS_DASHBOARD_URL}) from a browser.",
                        "INVALID_CONFIG",
                    )
                raise AreteError(f"{secret_only}; none was configured.", "INVALID_CONFIG")
            self._static = key
        self._base = f"{_resolve_api_url(api_url)}/account/squads"
        self._owns_http = http_client is None
        self._http = http_client or httpx.AsyncClient(timeout=60.0)

    async def __aenter__(self) -> "SquadsClient":
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        if self._owns_http:
            await self._http.aclose()

    async def _bearer(self) -> str:
        if self._static is not None:
            return self._static
        result = await _maybe_await(self._get_token())  # type: ignore[misc]
        token = result if isinstance(result, str) else getattr(result, "token", None)
        if not token:
            raise AreteError("get_token returned an empty token", "INVALID_CONFIG")
        return token

    def _multisig_url(self, multisig: Any) -> str:
        value = multisig.strip() if isinstance(multisig, str) else multisig
        return f"{self._base}/multisigs/{_segment(value, 'multisig')}"

    async def _call(self, what: str, method: str, url: str, body: Any = None) -> Any:
        headers = {"Authorization": f"Bearer {await self._bearer()}"}
        response = await self._http.request(method, url, headers=headers, json=body)
        parsed = _read_json(response.content)
        if response.status_code >= 400:
            raise _response_error(what, response, parsed)
        return parsed

    async def status(self) -> SquadsAccountStatus:
        """The setup state, the proposer and every attachment (refreshed)."""
        return await self._call("Squads status", "GET", self._base)

    async def notifications(self) -> List[SquadsNotification]:
        """Attachments archived after being active, with the deployments
        that used them. No chain read."""
        return await self._call("Squads notifications", "GET", f"{self._base}/notifications")

    async def get(self, multisig: str) -> SquadsAttachmentDetail:
        """One attachment with its config proposals and Arete's proposals."""
        return await self._call("Squads multisig read", "GET", self._multisig_url(multisig))

    async def attach(self, address: str, *, label: Optional[str] = None) -> SquadsAttachment:
        """Attach an existing multisig: ``active`` if the proposer is already
        an Initiate-only member, else ``pending``."""
        if not isinstance(address, str) or not address.strip():
            raise AreteError("attach requires an address", "INVALID_CONFIG")
        body: Dict[str, Any] = {"address": address.strip()}
        if label is not None:
            body["label"] = label
        return await self._call("Squads attach", "POST", f"{self._base}/multisigs", body)

    async def create(
        self,
        *,
        members: Sequence[SquadsCreateMember],
        threshold: int,
        creator: str,
        label: Optional[str] = None,
        transaction_format: Optional[ProtocolTransactionFormat] = None,
    ) -> SquadsPreparedTransaction:
        """An unsigned ``multisigCreateV2`` with the proposer already an
        Initiate-only member (do not list it), for ``creator`` to sign and pay."""
        if not members or not isinstance(threshold, int) or not creator:
            raise AreteError("create requires members, a threshold and a creator", "INVALID_CONFIG")
        body: Dict[str, Any] = {
            "members": [
                {"pubkey": member["pubkey"], "permissions": list(member["permissions"])}
                for member in members
            ],
            "threshold": threshold,
            "creator": creator,
        }
        if label is not None:
            body["label"] = label
        if transaction_format is not None:
            body["transactionFormat"] = transaction_format
        return await self._call("Squads create", "POST", f"{self._base}/multisigs/create", body)

    async def add_proposer(
        self,
        multisig: str,
        *,
        member: str,
        transaction_format: Optional[ProtocolTransactionFormat] = None,
    ) -> SquadsPreparedTransaction:
        """An unsigned config proposal adding the proposer (Initiate only),
        for a member who can initiate."""
        body = _member_body(member, transaction_format, "add_proposer")
        return await self._call(
            "Squads add-proposer", "POST", f"{self._multisig_url(multisig)}/add-proposer", body
        )

    async def approve(
        self,
        multisig: str,
        transaction_index: int,
        *,
        member: str,
        transaction_format: Optional[ProtocolTransactionFormat] = None,
    ) -> SquadsPreparedTransaction:
        """An unsigned approval of config proposal ``transaction_index``, for
        a member who can vote."""
        body = _member_body(member, transaction_format, "approve")
        index = _proposal_index(transaction_index)
        return await self._call(
            "Squads approve",
            "POST",
            f"{self._multisig_url(multisig)}/proposals/{index}/approve",
            body,
        )

    async def execute(
        self,
        multisig: str,
        transaction_index: int,
        *,
        member: str,
        transaction_format: Optional[ProtocolTransactionFormat] = None,
    ) -> SquadsPreparedTransaction:
        """An unsigned execution of approved config proposal
        ``transaction_index``, for a member who can execute."""
        body = _member_body(member, transaction_format, "execute")
        index = _proposal_index(transaction_index)
        return await self._call(
            "Squads execute",
            "POST",
            f"{self._multisig_url(multisig)}/proposals/{index}/execute",
            body,
        )

    async def remove_proposer(
        self,
        multisig: str,
        *,
        member: str,
        transaction_format: Optional[ProtocolTransactionFormat] = None,
    ) -> SquadsPreparedTransaction:
        """An unsigned config proposal removing the proposer; once executed
        the attachment is ``archived``."""
        body = _member_body(member, transaction_format, "remove_proposer")
        return await self._call(
            "Squads remove-proposer", "POST", f"{self._multisig_url(multisig)}/remove-proposer", body
        )

    async def forget(self, multisig: str) -> None:
        """Forget a ``pending`` or ``archived`` attachment (an ``active`` one
        is refused: remove the proposer first)."""
        await self._call("Squads forget", "DELETE", self._multisig_url(multisig))

    async def submit(
        self, transaction_id: str, signed_transaction: Union[str, bytes, bytearray]
    ) -> SquadsSubmitResult:
        """Send a signed prepared transaction (base64 or bytes) and wait
        (about 30 seconds) for it to confirm."""
        if not isinstance(transaction_id, str) or not transaction_id:
            raise AreteError("submit requires a transaction_id", "INVALID_CONFIG")
        if isinstance(signed_transaction, (bytes, bytearray)):
            signed = base64.b64encode(bytes(signed_transaction)).decode("ascii")
        else:
            signed = signed_transaction
        if not isinstance(signed, str) or not signed:
            raise AreteError("submit requires the signed transaction", "INVALID_CONFIG")
        return await self._call(
            "Squads submit",
            "POST",
            f"{self._base}/transactions",
            {"transactionId": transaction_id, "signedTransactionBase64": signed},
        )
