"""Cross-language vectors for the protocol client: webhook signatures,
metadata proofs, RFC 8785 canonicalization and the SSE parser."""

from __future__ import annotations

import json
from pathlib import Path

import httpx
import pytest

from arete.protocol import (
    ServerSentEventParser,
    WebhookVerificationError,
    canonicalize_json,
    protocol_metadata_proof,
    protocol_metadata_proof_message,
    sign_webhook,
    verify_webhook,
)

VECTORS = Path(__file__).resolve().parents[3] / "test-vectors"


def load(name: str):
    document = json.loads((VECTORS / name).read_text(encoding="utf-8"))
    assert document["schema"] in (
        "arete.protocol-client-vector/v1",
        "arete.hash-vectors/v1",
    )
    return document


WEBHOOK_CASES = load("protocol-client/webhook-signature-v1.json")["cases"]
PROOF_CASES = load("protocol-client/metadata-proof-v1.json")["cases"]


@pytest.mark.parametrize("case", WEBHOOK_CASES, ids=lambda case: case["name"])
def test_webhook_vectors(case):
    data = case["input"]
    expected = case["expected"]
    if "signature" in expected:
        assert sign_webhook(data["secret"], data["timestamp"], data["body"]) == expected["signature"]
        assert (
            sign_webhook(data["secret"].encode(), data["timestamp"], data["body"].encode())
            == expected["signature"]
        )
        return
    if "error" in expected:
        with pytest.raises(WebhookVerificationError) as raised:
            verify_webhook(data["secret"], data["body"], data["headers"], now=data["now"])
        assert raised.value.code == expected["error"]
        return
    verified = verify_webhook(data["secret"], data["body"], data["headers"], now=data["now"])
    assert verified.id == expected["id"]
    assert verified.timestamp == expected["timestamp"]
    assert verified.payload == json.loads(data["body"])


def test_webhook_headers_are_case_insensitive_and_bodies_may_be_bytes():
    case = WEBHOOK_CASES[1]["input"]
    headers = httpx.Headers({key.title(): value for key, value in case["headers"].items()})
    verified = verify_webhook(case["secret"], case["body"].encode(), headers, now=case["now"])
    assert verified.id == "dlv_1"
    mixed = {key.upper(): value for key, value in case["headers"].items()}
    assert verify_webhook(case["secret"], case["body"], mixed, now=case["now"]).id == "dlv_1"
    signature = case["headers"]["arete-webhook-signature"]
    with pytest.raises(WebhookVerificationError) as raised:
        verify_webhook(
            case["secret"],
            "not json",
            {**case["headers"], "arete-webhook-signature": f"v1={sign_webhook(case['secret'], 1790000000, 'not json')}"},
            now=case["now"],
        )
    assert raised.value.code == "invalid-payload"
    assert signature.startswith("v1=")


@pytest.mark.parametrize("case", PROOF_CASES, ids=lambda case: case["name"])
def test_metadata_proof_vectors(case):
    data = case["input"]
    build = lambda: protocol_metadata_proof_message(  # noqa: E731
        deployment_id=data["deploymentId"],
        instance_id=data["instanceId"],
        wallet=data["wallet"],
        patch=data["patch"],
        expires_at=data["expiresAt"],
        now=data["now"],
    )
    error = case["expected"].get("error")
    if error is not None:
        # `expires-at` is a ValueError, `invalid-field` a TypeError.
        with pytest.raises(ValueError if error == "expires-at" else TypeError):
            build()
        return
    assert canonicalize_json(data["patch"]) == case["expected"]["canonicalPatch"]
    message = build()
    assert message.patch_sha256 == case["expected"]["patchSha256"]
    assert message.message == case["expected"]["message"]
    assert message.bytes == case["expected"]["message"].encode()


def test_metadata_proofs_check_expiry_and_lines():
    base = dict(deployment_id="d", instance_id="i", wallet="w", patch={}, now=1000)
    with pytest.raises(ValueError):
        protocol_metadata_proof_message(expires_at=1000, **base)
    with pytest.raises(ValueError):
        protocol_metadata_proof_message(expires_at=1301, **base)
    assert protocol_metadata_proof_message(expires_at=1300, **base).expires_at == 1300
    with pytest.raises(TypeError):
        protocol_metadata_proof_message(expires_at=1100, **{**base, "wallet": "a\nb"})
    with pytest.raises(TypeError):
        protocol_metadata_proof(wallet="w", signature=b"short", expires_at=1)


def test_jcs_matches_the_hash_vectors():
    checked = 0
    for vector in load("hash-v1.json")["hashVectors"]:
        if vector.get("profile") != "arete-jcs-v1" or vector["input"].get("encoding") != "utf8":
            continue
        value = json.loads(vector["input"]["data"])
        expected = bytes.fromhex(vector["expected"]["canonicalPayloadHex"]).decode("utf-8")
        assert canonicalize_json(value) == expected, vector["id"]
        checked += 1
    assert checked >= 10


@pytest.mark.parametrize(
    "value, expected",
    [
        (1e21, "1e+21"),
        (1e20, "100000000000000000000"),
        (1e-7, "1e-7"),
        (0.000001, "0.000001"),
        (-0.0, "0"),
        (1.5e300, "1.5e+300"),
        (123.456, "123.456"),
        (5e-324, "5e-324"),
        (2.0, "2"),
    ],
)
def test_jcs_numbers_use_the_ecmascript_form(value, expected):
    assert canonicalize_json(value) == expected


def test_jcs_refuses_non_json():
    for value in (float("nan"), 2**53, {1: "x"}, object(), "\ud800"):
        with pytest.raises(TypeError):
            canonicalize_json(value)


def test_sse_parser_follows_the_specification():
    parser = ServerSentEventParser("0")
    events = parser.push("data: a\ndata:b\n\nid\nevent: x\ndata\n\nid: bad\0id\nretry: x\nretry: 5\ndata: c")
    assert [(event.event, event.data, event.id) for event in events] == [
        ("message", "a\nb", "0"),
        ("x", "", ""),
    ]
    assert parser.retry_ms == 5
    assert parser.push("\n\n")[0].data == "c"
    parser.push("data: partial")
    parser.end()
    assert parser.push("\n\n") == []


RECEIPT_VECTORS = load("protocol-client/execution-receipt-v1.json")


@pytest.mark.parametrize("case", RECEIPT_VECTORS["verifyCases"], ids=lambda case: case["name"])
def test_receipt_verify_vectors(case):
    from arete.protocol import receipt_canonical_body, receipt_signing_message, verify_receipt

    expected = case["expected"]
    result = verify_receipt(case["receipt"], RECEIPT_VECTORS["keySet"])
    if expected["valid"]:
        assert result.valid and result.key_id == expected["keyId"], result
    else:
        assert not result.valid and result.error == expected["error"], result
    if "canonicalBody" in expected:
        assert receipt_canonical_body(case["receipt"]) == expected["canonicalBody"]
        assert (
            receipt_signing_message(expected["keyId"], expected["bodySha256"]).decode()
            == expected["signingMessage"]
        )


@pytest.mark.parametrize("case", RECEIPT_VECTORS["onChainCases"], ids=lambda case: case["name"])
def test_receipt_on_chain_vectors(case):
    from arete.protocol import verify_receipt_on_chain

    expected = case["expected"]
    result = verify_receipt_on_chain(case["receipt"], case["transaction"])
    if expected["valid"]:
        assert result.valid, result
    else:
        assert not result.valid and result.error == expected["error"], result


def test_receipt_on_chain_accepts_wire_bytes():
    import base64

    from arete.protocol import verify_receipt_on_chain

    case = next(c for c in RECEIPT_VECTORS["onChainCases"] if c["name"] == "legacy-wire")
    assert verify_receipt_on_chain(case["receipt"], base64.b64decode(case["transaction"])).valid
