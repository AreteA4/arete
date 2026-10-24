"""Tests for arete.protocol.SquadsClient (port of
typescript/core/src/protocol/squads.test.ts)."""

from __future__ import annotations

import json
from typing import Any, Callable, List, Optional

import httpx
import pytest

from arete import AuthConfig
from arete.errors import AreteError, HttpRequestError
from arete.protocol import (
    SQUADS_DASHBOARD_URL,
    SQUADS_SETUP_REQUIRED_CODE,
    SquadsClient,
    squads_dashboard_url,
    squads_setup_required,
)

API = "https://api.example.test/api"
BASE = f"{API}/account/squads"
MULTISIG = "MsiG1111111111111111111111111111111111111111"
MEMBER = "So11111111111111111111111111111111111111112"
SECRET = "a4_sk_test_secret"


def recorder(answer: Optional[Callable[[httpx.Request], httpx.Response]] = None):
    calls: List[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(request)
        if answer is not None:
            return answer(request)
        if request.method == "DELETE":
            return httpx.Response(204)
        return httpx.Response(200, json={"ok": True})

    return calls, httpx.AsyncClient(transport=httpx.MockTransport(handler))


def body_of(request: httpx.Request) -> Any:
    return json.loads(request.content) if request.content else None


@pytest.mark.asyncio
async def test_calls_every_account_route_with_the_secret_key_as_bearer():
    calls, http = recorder()
    squads = SquadsClient(api_key=SECRET, api_url=API + "/", http_client=http)
    await squads.status()
    await squads.notifications()
    await squads.get(MULTISIG)
    await squads.attach(MULTISIG, label="Treasury")
    await squads.create(
        members=[{"pubkey": MEMBER, "permissions": ["initiate", "vote", "execute"]}],
        threshold=1,
        creator=MEMBER,
        label="Ops",
    )
    await squads.add_proposer(MULTISIG, member=MEMBER)
    await squads.approve(MULTISIG, 3, member=MEMBER, transaction_format="v1")
    await squads.execute(MULTISIG, 3, member=MEMBER)
    await squads.remove_proposer(MULTISIG, member=MEMBER)
    assert await squads.forget(MULTISIG) is None
    await squads.submit("tx-1", b"\x01\x02\x03\x04")

    assert [(c.method, str(c.url), body_of(c)) for c in calls] == [
        ("GET", BASE, None),
        ("GET", f"{BASE}/notifications", None),
        ("GET", f"{BASE}/multisigs/{MULTISIG}", None),
        ("POST", f"{BASE}/multisigs", {"address": MULTISIG, "label": "Treasury"}),
        (
            "POST",
            f"{BASE}/multisigs/create",
            {
                "members": [{"pubkey": MEMBER, "permissions": ["initiate", "vote", "execute"]}],
                "threshold": 1,
                "creator": MEMBER,
                "label": "Ops",
            },
        ),
        ("POST", f"{BASE}/multisigs/{MULTISIG}/add-proposer", {"member": MEMBER}),
        (
            "POST",
            f"{BASE}/multisigs/{MULTISIG}/proposals/3/approve",
            {"member": MEMBER, "transactionFormat": "v1"},
        ),
        ("POST", f"{BASE}/multisigs/{MULTISIG}/proposals/3/execute", {"member": MEMBER}),
        ("POST", f"{BASE}/multisigs/{MULTISIG}/remove-proposer", {"member": MEMBER}),
        ("DELETE", f"{BASE}/multisigs/{MULTISIG}", None),
        (
            "POST",
            f"{BASE}/transactions",
            {"transactionId": "tx-1", "signedTransactionBase64": "AQIDBA=="},
        ),
    ]
    assert {c.headers["authorization"] for c in calls} == {f"Bearer {SECRET}"}


@pytest.mark.asyncio
async def test_dashboard_token_provider_and_api_url_default(monkeypatch):
    calls, http = recorder()

    async def get_token():
        return "dash-token"

    monkeypatch.setenv("ARETE_API_URL", "https://staging.example.test/")
    squads = SquadsClient(auth=AuthConfig(get_token=get_token), http_client=http)
    await squads.status()
    monkeypatch.delenv("ARETE_API_URL")
    await SquadsClient(token="t", http_client=http).status()
    assert [str(c.url) for c in calls] == [
        "https://staging.example.test/api/account/squads",
        "https://api.arete.run/api/account/squads",
    ]
    assert calls[0].headers["authorization"] == "Bearer dash-token"


@pytest.mark.asyncio
async def test_refuses_publishable_keys_and_bad_input():
    with pytest.raises(AreteError, match="publishable keys are refused"):
        SquadsClient(auth=AuthConfig(publishable_key="a4_pk_test"))
    _, http = recorder()
    squads = SquadsClient(api_key=SECRET, api_url=API, http_client=http)
    with pytest.raises(AreteError, match="requires a member"):
        await squads.add_proposer(MULTISIG, member="")
    with pytest.raises(AreteError, match="non-negative integer"):
        await squads.approve(MULTISIG, -1, member=MEMBER)


@pytest.mark.asyncio
async def test_refusals_carry_the_wire_code():
    _, http = recorder(
        lambda request: httpx.Response(
            409, json={"code": "protocol-squads-multisig-active", "error": "remove the proposer first"}
        )
    )
    squads = SquadsClient(api_key=SECRET, api_url=API, http_client=http)
    with pytest.raises(HttpRequestError) as refused:
        await squads.forget(MULTISIG)
    assert refused.value.status == 409
    assert refused.value.code == "protocol-squads-multisig-active"
    assert "Squads forget returned 409: remove the proposer first" in str(refused.value)


def test_setup_required_reads_errors_and_bodies():
    body = {
        "code": SQUADS_SETUP_REQUIRED_CODE,
        "message": "Step 'finalize' proposes to Squads multisig M, which is not set up.",
        "details": {
            "step": "finalize",
            "multisig": MULTISIG,
            "state": "pending",
            "problem": "not_member",
            "proposer": MEMBER,
            "setupUrl": squads_dashboard_url(MULTISIG),
        },
    }
    error = HttpRequestError(
        "Deploy returned 409",
        code=SQUADS_SETUP_REQUIRED_CODE,
        status=409,
        body=body,
        details={"status": 409, "wire_error_code": SQUADS_SETUP_REQUIRED_CODE, "body": body},
    )
    found = squads_setup_required(error)
    assert found == {"message": body["message"], "details": body["details"]}
    assert squads_setup_required(body)["details"]["setupUrl"] == f"{SQUADS_DASHBOARD_URL}/{MULTISIG}"
    assert squads_setup_required({**body, "code": "other"}) is None
    assert squads_setup_required(ValueError("x")) is None
    assert squads_dashboard_url() == "https://app.arete.run/dashboard/squads"
