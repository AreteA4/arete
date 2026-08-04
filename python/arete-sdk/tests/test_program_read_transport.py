"""Tests for arete.program_read_transport against the program-read-http/v1 contract.

Fixture cases are a verbatim copy of
``typescript/core/src/program-read-contract-v1.fixture.json``.
"""

from __future__ import annotations

import json

import pytest

from arete.errors import AreteError
from arete.http import AuthTokenTarget
from arete.program_read_transport import (
    HOSTED_BINDING,
    LOCAL_HTTP,
    PROGRAM_READ_CONTRACT_VERSION,
    HostedBindingTransportDef,
    HttpAuthMetadata,
    HttpProgramReadTransport,
    LocalHttpTransportDef,
    ProgramReadBinding,
    ProgramReadDescriptor,
    ProgramReleaseReference,
    UnavailableProgramReadTransport,
    program_read_descriptor_from_wire,
    program_read_descriptor_to_wire,
    request_path,
    validate_program_read_descriptor,
)
from arete.read import ProgramReadRequest, ReadRequestError

FIXTURE = json.loads("""{
  "contractVersion": "program-read-http/v1",
  "success": {
    "rawValue": {
      "value": "decoded",
      "count": 7
    },
    "missing": null,
    "exists": {
      "exists": true
    },
    "batch": {
      "items": [
        {
          "address": "present",
          "status": "ok",
          "value": {
            "value": "decoded",
            "count": 7
          }
        },
        {
          "address": "missing",
          "status": "missing"
        },
        {
          "address": "broken",
          "status": "error",
          "error": {
            "code": "ACCOUNT_DECODE_FAILED"
          }
        }
      ]
    }
  },
  "errors": {
    "nested": {
      "error": {
        "code": "ACCOUNT_DECODE_FAILED"
      }
    },
    "refreshable": {
      "error": {
        "code": "TOKEN_EXPIRED"
      }
    },
    "nonRefreshable": {
      "error": {
        "code": "AUTH_REQUIRED"
      }
    }
  }
}""")

TEST_BINDING_ID = "prb_00000000000000000000000000000001"


def release() -> ProgramReleaseReference:
    return ProgramReleaseReference(
        program_release_hash="release-alpha", program_spec_hash="spec-alpha"
    )


def auth_metadata(**overrides) -> HttpAuthMetadata:
    fields = {
        "session_endpoint": "https://auth.example.test/session",
        "target_kind": "program-read-binding",
        "target_id": TEST_BINDING_ID,
        "scopes": ("read",),
    }
    fields.update(overrides)
    return HttpAuthMetadata(**fields)


def binding(endpoint: str = "https://reads.example.test", **auth_overrides) -> ProgramReadBinding:
    return ProgramReadBinding(
        endpoint=endpoint,
        program_read_binding_id=auth_overrides.pop("binding_id", TEST_BINDING_ID),
        auth=auth_metadata(**auth_overrides),
    )


def hosted_descriptor(
    the_binding: ProgramReadBinding = None, the_release: ProgramReleaseReference = None
) -> ProgramReadDescriptor:
    return ProgramReadDescriptor(
        release=the_release or release(),
        transport=HostedBindingTransportDef(binding=the_binding or binding()),
    )


def local_descriptor(the_release: ProgramReleaseReference = None) -> ProgramReadDescriptor:
    return ProgramReadDescriptor(
        release=the_release or release(), transport=LocalHttpTransportDef()
    )


class FakeHttpError(AreteError):
    """Duck-typed stand-in for arete.errors.HttpRequestError."""

    def __init__(self, status: int, body: str, headers=None) -> None:
        super().__init__(f"http {status}")
        self.status = status
        self.body = body
        self.headers = headers or {}


class FakeHttpClient:
    """Duck-typed stand-in for arete.http.HttpAuthClient.request_json."""

    def __init__(self, *results) -> None:
        self.calls = []
        self._results = list(results)

    async def request_json(
        self,
        method,
        url,
        *,
        json_body=None,
        params=None,
        headers=None,
        target=None,
        scopes=None,
    ):
        self.calls.append(
            {
                "method": method,
                "url": url,
                "json_body": json_body,
                "target": target,
                "scopes": tuple(scopes) if scopes is not None else None,
            }
        )
        result = self._results.pop(0)
        if isinstance(result, Exception):
            raise result
        return result


def local_transport(http: FakeHttpClient) -> HttpProgramReadTransport:
    return HttpProgramReadTransport.local_http(
        "https://stack.example.test/api", release(), http
    )


# ---------------------------------------------------------------------------
# Contract fixture: request/response pairs
# ---------------------------------------------------------------------------


def test_contract_version() -> None:
    assert PROGRAM_READ_CONTRACT_VERSION == FIXTURE["contractVersion"]


@pytest.mark.asyncio
async def test_fetch_issues_get_on_the_release_addressed_path() -> None:
    http = FakeHttpClient(FIXTURE["success"]["rawValue"])
    value = await local_transport(http).read(ProgramReadRequest.fetch("State", "present"))
    assert value == {"value": "decoded", "count": 7}
    assert http.calls == [
        {
            "method": "GET",
            "url": "https://stack.example.test/api/v1/releases/release-alpha/accounts/State/present",
            "json_body": None,
            "target": None,
            "scopes": ("read",),
        }
    ]


@pytest.mark.asyncio
async def test_fetch_missing_returns_none() -> None:
    http = FakeHttpClient(FIXTURE["success"]["missing"])
    value = await local_transport(http).read(ProgramReadRequest.fetch("State", "missing"))
    assert value is None


@pytest.mark.asyncio
async def test_exists_addresses_the_exists_suffix() -> None:
    http = FakeHttpClient(FIXTURE["success"]["exists"])
    value = await local_transport(http).read(ProgramReadRequest.exists("State", "present"))
    assert value == {"exists": True}
    assert http.calls[0]["url"].endswith(
        "/v1/releases/release-alpha/accounts/State/present/exists"
    )
    assert http.calls[0]["method"] == "GET"


@pytest.mark.asyncio
async def test_fetch_many_posts_addresses_to_the_account_root() -> None:
    http = FakeHttpClient(FIXTURE["success"]["batch"])
    value = await local_transport(http).read(
        ProgramReadRequest.fetch_many("State", ["present", "missing", "broken"])
    )
    assert value == FIXTURE["success"]["batch"]
    assert http.calls == [
        {
            "method": "POST",
            "url": "https://stack.example.test/api/v1/releases/release-alpha/accounts/State",
            "json_body": {"addresses": ["present", "missing", "broken"]},
            "target": None,
            "scopes": ("read",),
        }
    ]


@pytest.mark.asyncio
async def test_preserves_typed_release_hashes_and_encodes_path_segments() -> None:
    release_hash = (
        "arete:h1:program-release:sha256:"
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    )
    http = FakeHttpClient({"value": "typed"})
    transport = HttpProgramReadTransport.local_http(
        "http://127.0.0.1:8899/local/api/",
        ProgramReleaseReference(
            program_release_hash=release_hash, program_spec_hash="spec-alpha"
        ),
        http,
    )
    await transport.read(ProgramReadRequest.fetch("State", "addr/one two+"))
    assert http.calls[0]["url"] == (
        "http://127.0.0.1:8899/local/api/v1/releases/"
        f"{release_hash}/accounts/State/addr%2Fone%20two%2B"
    )


def test_request_path_encodes_like_javascript() -> None:
    assert request_path(release(), ProgramReadRequest.fetch("State", "a b/c:d+e")) == (
        "/v1/releases/release-alpha/accounts/State/a%20b%2Fc%3Ad%2Be"
    )
    assert request_path(release(), ProgramReadRequest.fetch("State", "é")) == (
        "/v1/releases/release-alpha/accounts/State/%C3%A9"
    )
    assert request_path(
        release(), ProgramReadRequest.fetch("State", "abc-AZ_09.!~*'()")
    ).endswith("/abc-AZ_09.!~*'()")


# ---------------------------------------------------------------------------
# Targeted tokens
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_hosted_transport_forwards_the_targeted_token_descriptor() -> None:
    http = FakeHttpClient(FIXTURE["success"]["rawValue"])
    transport = HttpProgramReadTransport.hosted(binding(), release(), http)
    await transport.read(ProgramReadRequest.fetch("State", "present"))
    assert http.calls[0]["target"] == AuthTokenTarget(
        kind="program-read-binding",
        target_id=TEST_BINDING_ID,
        release_hash="release-alpha",
        scopes=("read",),
    )
    assert http.calls[0]["url"] == (
        "https://reads.example.test/v1/releases/release-alpha/accounts/State/present"
    )


@pytest.mark.asyncio
async def test_hosted_unauthenticated_omits_the_target() -> None:
    http = FakeHttpClient({"value": "open"})
    transport = HttpProgramReadTransport.hosted(
        binding(), release(), http, authenticated=False
    )
    await transport.read(ProgramReadRequest.fetch("State", "address"))
    assert http.calls[0]["target"] is None


# ---------------------------------------------------------------------------
# Error translation
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_header_error_code_wins_over_nested_body_code() -> None:
    nested = json.dumps(FIXTURE["errors"]["nested"])
    http = FakeHttpClient(
        FakeHttpError(422, nested, headers={"X-Error-Code": "ACCOUNT_OWNER_MISMATCH"})
    )
    with pytest.raises(ReadRequestError) as excinfo:
        await local_transport(http).read(ProgramReadRequest.fetch("State", "broken"))
    error = excinfo.value
    assert error.status == 422
    assert error.path == "/v1/releases/release-alpha/accounts/State/broken"
    assert error.body == nested
    assert error.server_error_code == "ACCOUNT_OWNER_MISMATCH"
    assert str(error) == (
        "Read request to '/v1/releases/release-alpha/accounts/State/broken' "
        f"failed (422): {nested}"
    )


@pytest.mark.asyncio
async def test_nested_body_code_is_read_without_header() -> None:
    http = FakeHttpClient(FakeHttpError(422, json.dumps(FIXTURE["errors"]["nested"])))
    with pytest.raises(ReadRequestError) as excinfo:
        await local_transport(http).read(ProgramReadRequest.fetch("State", "broken"))
    assert excinfo.value.server_error_code == "ACCOUNT_DECODE_FAILED"


@pytest.mark.asyncio
async def test_top_level_body_code_is_a_fallback() -> None:
    http = FakeHttpClient(FakeHttpError(500, '{"code":"PLAIN"}'))
    with pytest.raises(ReadRequestError) as excinfo:
        await local_transport(http).read(ProgramReadRequest.fetch("State", "broken"))
    assert excinfo.value.server_error_code == "PLAIN"


@pytest.mark.asyncio
async def test_non_http_errors_propagate_unchanged() -> None:
    http = FakeHttpClient(ValueError("network unavailable"))
    with pytest.raises(ValueError, match="network unavailable"):
        await local_transport(http).read(ProgramReadRequest.fetch("State", "address"))


@pytest.mark.asyncio
async def test_unavailable_transport_raises_invalid_config() -> None:
    transport = UnavailableProgramReadTransport(
        "Program 'alpha' has no release-aware read descriptor"
    )
    with pytest.raises(AreteError) as excinfo:
        await transport.read(ProgramReadRequest.fetch("State", "address"))
    assert excinfo.value.code == "INVALID_CONFIG"
    assert excinfo.value.message == "Program 'alpha' has no release-aware read descriptor"


# ---------------------------------------------------------------------------
# Descriptor validation
# ---------------------------------------------------------------------------


ACCEPTED_DESCRIPTORS = [
    ("local http", lambda: local_descriptor()),
    ("hosted https", lambda: hosted_descriptor()),
    (
        "hosted http localhost",
        lambda: hosted_descriptor(binding(endpoint="http://localhost:8899")),
    ),
    (
        "hosted http 127.0.0.1",
        lambda: hosted_descriptor(binding(endpoint="http://127.0.0.1:1234/prefix")),
    ),
    (
        "hosted loopback session endpoint",
        lambda: hosted_descriptor(
            binding(session_endpoint="http://127.0.0.1:9000/session")
        ),
    ),
]

REJECTED_DESCRIPTORS = [
    (
        "empty release hash",
        lambda: hosted_descriptor(
            the_release=ProgramReleaseReference("", "spec-alpha")
        ),
    ),
    (
        "whitespace spec hash",
        lambda: hosted_descriptor(
            the_release=ProgramReleaseReference("release-alpha", "   ")
        ),
    ),
    (
        "local with empty release",
        lambda: local_descriptor(ProgramReleaseReference("", "spec-alpha")),
    ),
    (
        "insecure endpoint scheme",
        lambda: hosted_descriptor(binding(endpoint="http://reads.example.test")),
    ),
    ("unparseable endpoint", lambda: hosted_descriptor(binding(endpoint="not a url"))),
    (
        "insecure session endpoint scheme",
        lambda: hosted_descriptor(
            binding(session_endpoint="http://auth.example.test/session")
        ),
    ),
    (
        "empty session endpoint",
        lambda: hosted_descriptor(binding(session_endpoint="")),
    ),
    (
        "short binding id",
        lambda: hosted_descriptor(
            binding(binding_id="prb_too-short", target_id="prb_too-short")
        ),
    ),
    (
        "invalid binding id character",
        lambda: hosted_descriptor(
            binding(
                binding_id="prb_" + "0" * 31 + "!",
                target_id="prb_" + "0" * 31 + "!",
            )
        ),
    ),
    (
        "wrong auth target kind",
        lambda: hosted_descriptor(binding(target_kind="solana-gateway-binding")),
    ),
    (
        "mismatched auth target id",
        lambda: hosted_descriptor(
            binding(target_id="prb_00000000000000000000000000000002")
        ),
    ),
]


@pytest.mark.parametrize(
    "make_descriptor", [entry[1] for entry in ACCEPTED_DESCRIPTORS],
    ids=[entry[0] for entry in ACCEPTED_DESCRIPTORS],
)
def test_accepts_valid_descriptors(make_descriptor) -> None:
    validate_program_read_descriptor("alpha", make_descriptor())


@pytest.mark.parametrize(
    "make_descriptor", [entry[1] for entry in REJECTED_DESCRIPTORS],
    ids=[entry[0] for entry in REJECTED_DESCRIPTORS],
)
def test_rejects_invalid_descriptors(make_descriptor) -> None:
    with pytest.raises(AreteError) as excinfo:
        validate_program_read_descriptor("alpha", make_descriptor())
    assert excinfo.value.code == "INVALID_CONFIG"


def test_validation_error_messages_match_typescript() -> None:
    with pytest.raises(AreteError) as excinfo:
        validate_program_read_descriptor(
            "alpha", local_descriptor(ProgramReleaseReference("", ""))
        )
    assert excinfo.value.message == "Program 'alpha' read descriptor requires a complete release"

    with pytest.raises(AreteError) as excinfo:
        validate_program_read_descriptor(
            "alpha", hosted_descriptor(binding(endpoint="http://reads.example.test"))
        )
    assert excinfo.value.message == (
        "Program 'alpha' hosted binding requires secure endpoints, a canonical "
        "binding ID, and matching program-read-binding auth metadata"
    )

    with pytest.raises(AreteError) as excinfo:
        validate_program_read_descriptor(
            "alpha",
            ProgramReadDescriptor(
                release=release(),
                transport=LocalHttpTransportDef(endpoint_source="stack-http"),
            ),
        )
    assert excinfo.value.message == (
        "Program 'alpha' local HTTP transport must use endpointSource 'connect-http-url'"
    )


def test_rejects_unsupported_transport_kind() -> None:
    class WeirdTransport:
        kind = "carrier-pigeon"

    with pytest.raises(AreteError) as excinfo:
        validate_program_read_descriptor(
            "alpha",
            ProgramReadDescriptor(release=release(), transport=WeirdTransport()),
        )
    assert excinfo.value.message == (
        "Program 'alpha' read descriptor has an unsupported transport"
    )


# ---------------------------------------------------------------------------
# Wire (de)serialization
# ---------------------------------------------------------------------------


def test_descriptor_wire_round_trip_matches_ts_shape() -> None:
    local_json = {
        "release": {
            "programReleaseHash": "release-alpha",
            "programSpecHash": "spec-alpha",
        },
        "transport": {"kind": "local-http", "endpointSource": "connect-http-url"},
    }
    local = program_read_descriptor_from_wire(local_json)
    assert local.transport_kind == LOCAL_HTTP
    assert local.release == release()
    assert local.binding is None
    assert program_read_descriptor_to_wire(local) == local_json

    hosted_json = {
        "release": {
            "programReleaseHash": "release-alpha",
            "programSpecHash": "spec-alpha",
        },
        "transport": {
            "kind": "hosted-binding",
            "binding": {
                "endpoint": "https://reads.example.test",
                "programReadBindingId": TEST_BINDING_ID,
                "auth": {
                    "sessionEndpoint": "https://auth.example.test/session",
                    "targetKind": "program-read-binding",
                    "targetId": TEST_BINDING_ID,
                    "scopes": ["read"],
                },
            },
        },
    }
    hosted = program_read_descriptor_from_wire(hosted_json)
    assert hosted.transport_kind == HOSTED_BINDING
    assert hosted.binding.program_read_binding_id == TEST_BINDING_ID
    assert hosted.binding.auth.scopes == ("read",)
    assert program_read_descriptor_to_wire(hosted) == hosted_json


def test_descriptor_from_wire_rejects_unknown_endpoint_source() -> None:
    bad = {
        "release": {
            "programReleaseHash": "release-alpha",
            "programSpecHash": "spec-alpha",
        },
        "transport": {"kind": "local-http", "endpointSource": "stack-http"},
    }
    with pytest.raises(AreteError) as excinfo:
        program_read_descriptor_from_wire(bad)
    assert excinfo.value.code == "INVALID_CONFIG"


def test_descriptor_from_wire_rejects_unknown_kind() -> None:
    bad = {
        "release": {
            "programReleaseHash": "release-alpha",
            "programSpecHash": "spec-alpha",
        },
        "transport": {"kind": "carrier-pigeon"},
    }
    with pytest.raises(AreteError):
        program_read_descriptor_from_wire(bad)


# ---------------------------------------------------------------------------
# from_descriptor wiring
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_from_descriptor_builds_a_local_transport() -> None:
    http = FakeHttpClient({"value": "local"})
    transport = HttpProgramReadTransport.from_descriptor(
        "alpha", local_descriptor(), http, connect_http_url="http://127.0.0.1:8879/local/api/"
    )
    await transport.read(ProgramReadRequest.fetch("State", "address"))
    assert http.calls[0]["url"] == (
        "http://127.0.0.1:8879/local/api/v1/releases/release-alpha/accounts/State/address"
    )
    assert http.calls[0]["target"] is None


def test_from_descriptor_requires_connect_http_url_for_local() -> None:
    with pytest.raises(AreteError) as excinfo:
        HttpProgramReadTransport.from_descriptor(
            "alpha", local_descriptor(), FakeHttpClient()
        )
    assert excinfo.value.code == "INVALID_CONFIG"


def test_from_descriptor_builds_a_hosted_transport_with_target() -> None:
    transport = HttpProgramReadTransport.from_descriptor(
        "alpha", hosted_descriptor(), FakeHttpClient()
    )
    assert transport.endpoint == "https://reads.example.test"
    assert transport.target == AuthTokenTarget(
        kind="program-read-binding",
        target_id=TEST_BINDING_ID,
        release_hash="release-alpha",
        scopes=("read",),
    )


def test_from_descriptor_validates_before_building() -> None:
    with pytest.raises(AreteError) as excinfo:
        HttpProgramReadTransport.from_descriptor(
            "alpha",
            hosted_descriptor(binding(endpoint="http://reads.example.test")),
            FakeHttpClient(),
        )
    assert excinfo.value.code == "INVALID_CONFIG"
