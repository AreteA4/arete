"""Tests for arete.read: AccountReader, query executors, ReadRequestError.

Fixture cases mirror ``typescript/core/src/program-read-contract-v1.fixture.json``.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, List

import pytest

from arete.errors import AreteError
from arete.read import (
    AccountBatchItem,
    AccountBatchResult,
    AccountReader,
    ProgramAccountReadDef,
    ProgramQueryDef,
    ProgramReadRequest,
    QueryExecutor,
    ReadRequestError,
    StackQueryDef,
    normalize_program_account_wire_keys,
)

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


class FakeTransport:
    """ProgramReadTransport double returning canned wire values."""

    def __init__(self, *results) -> None:
        self.requests = []
        self._results = list(results)

    async def read(self, request: ProgramReadRequest) -> Any:
        self.requests.append(request)
        result = self._results.pop(0)
        if isinstance(result, Exception):
            raise result
        return result


@dataclass(frozen=True)
class FixtureAccount:
    value: str
    count: int


def parse_fixture_account(data: Any) -> FixtureAccount:
    return FixtureAccount(value=data["value"], count=data["count"])


@dataclass(frozen=True)
class Inner:
    inner_value: str


@dataclass(frozen=True)
class NormalizedAccount:
    value_count: int
    inner: Inner
    items: List[Inner]


def parse_normalized_account(data: Any) -> NormalizedAccount:
    return NormalizedAccount(
        value_count=data["value_count"],
        inner=Inner(inner_value=data["inner"]["inner_value"]),
        items=[Inner(inner_value=item["inner_value"]) for item in data["items"]],
    )


# ---------------------------------------------------------------------------
# AccountReader.fetch
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_fetch_returns_raw_wire_value_without_parser() -> None:
    transport = FakeTransport(FIXTURE["success"]["rawValue"])
    reader = AccountReader("State", transport)
    value = await reader.fetch("present")
    assert value == {"value": "decoded", "count": 7}
    assert transport.requests == [ProgramReadRequest.fetch("State", "present")]


@pytest.mark.asyncio
async def test_fetch_missing_returns_none() -> None:
    transport = FakeTransport(FIXTURE["success"]["missing"])
    reader = AccountReader("State", transport, parse_fixture_account)
    assert await reader.fetch("missing") is None


@pytest.mark.asyncio
async def test_fetch_decodes_typed_account_via_parser() -> None:
    transport = FakeTransport(FIXTURE["success"]["rawValue"])
    reader = AccountReader("State", transport, parse_fixture_account)
    assert await reader.fetch("present") == FixtureAccount(value="decoded", count=7)


@pytest.mark.asyncio
async def test_fetch_retries_with_key_normalization_before_failing() -> None:
    camel = {
        "valueCount": 7,
        "inner": {"innerValue": "x"},
        "items": [{"innerValue": "y"}],
    }
    transport = FakeTransport(camel)
    reader = AccountReader("State", transport, parse_normalized_account)
    account = await reader.fetch("present")
    assert account == NormalizedAccount(
        value_count=7,
        inner=Inner(inner_value="x"),
        items=[Inner(inner_value="y")],
    )


@pytest.mark.asyncio
async def test_fetch_reports_schema_validation_failure() -> None:
    transport = FakeTransport({"unexpected": True})
    reader = AccountReader("State", transport, parse_fixture_account)
    with pytest.raises(AreteError) as excinfo:
        await reader.fetch("present")
    assert excinfo.value.message == "Program account read 'State' failed schema validation"
    assert excinfo.value.code == "SCHEMA_VALIDATION"


# ---------------------------------------------------------------------------
# AccountReader.exists
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_exists_translates_fixture_object_to_bool() -> None:
    transport = FakeTransport(FIXTURE["success"]["exists"])
    reader = AccountReader("State", transport)
    assert await reader.exists("present") is True
    assert transport.requests == [ProgramReadRequest.exists("State", "present")]


@pytest.mark.asyncio
async def test_exists_rejects_invalid_payload() -> None:
    transport = FakeTransport({"nope": 1})
    reader = AccountReader("State", transport)
    with pytest.raises(AreteError) as excinfo:
        await reader.exists("present")
    assert excinfo.value.code == "INVALID_RESPONSE"


# ---------------------------------------------------------------------------
# AccountReader.fetch_many
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_fetch_many_preserves_mixed_batch_statuses_and_order() -> None:
    transport = FakeTransport(FIXTURE["success"]["batch"])
    reader = AccountReader("State", transport)
    result = await reader.fetch_many(["present", "missing", "broken"])
    assert result == AccountBatchResult(
        items=(
            AccountBatchItem(
                address="present", status="ok", value={"value": "decoded", "count": 7}
            ),
            AccountBatchItem(address="missing", status="missing"),
            AccountBatchItem(
                address="broken", status="error", error_code="ACCOUNT_DECODE_FAILED"
            ),
        )
    )
    assert transport.requests == [
        ProgramReadRequest.fetch_many("State", ["present", "missing", "broken"])
    ]


@pytest.mark.asyncio
async def test_fetch_many_parses_only_ok_items() -> None:
    transport = FakeTransport(FIXTURE["success"]["batch"])
    reader = AccountReader("State", transport, parse_fixture_account)
    result = await reader.fetch_many(["present", "missing", "broken"])
    assert result.items[0].value == FixtureAccount(value="decoded", count=7)
    assert result.items[1] == AccountBatchItem(address="missing", status="missing")
    assert result.items[2].error_code == "ACCOUNT_DECODE_FAILED"


@pytest.mark.asyncio
async def test_fetch_many_rejects_invalid_batch_shape() -> None:
    transport = FakeTransport({"items": [{"address": "a", "status": "weird"}]})
    reader = AccountReader("State", transport)
    with pytest.raises(AreteError) as excinfo:
        await reader.fetch_many(["a"])
    assert excinfo.value.code == "INVALID_RESPONSE"


@pytest.mark.asyncio
async def test_reader_propagates_transport_errors() -> None:
    error = ReadRequestError(
        status=422, path="/p", body="{}", server_error_code="ACCOUNT_DECODE_FAILED"
    )
    transport = FakeTransport(error)
    reader = AccountReader("State", transport)
    with pytest.raises(ReadRequestError) as excinfo:
        await reader.fetch("broken")
    assert excinfo.value is error


# ---------------------------------------------------------------------------
# Definitions + ReadRequestError
# ---------------------------------------------------------------------------


def test_read_request_error_shape() -> None:
    error = ReadRequestError(
        status=422,
        path="/v1/releases/release-alpha/accounts/State/broken",
        body='{"error":{"code":"ACCOUNT_DECODE_FAILED"}}',
        server_error_code="ACCOUNT_DECODE_FAILED",
    )
    assert isinstance(error, AreteError)
    assert error.status == 422
    assert error.server_error_code == "ACCOUNT_DECODE_FAILED"
    assert str(error) == (
        "Read request to '/v1/releases/release-alpha/accounts/State/broken' "
        'failed (422): {"error":{"code":"ACCOUNT_DECODE_FAILED"}}'
    )


def test_definition_defaults() -> None:
    account = ProgramAccountReadDef(account="State")
    assert account.parser is None
    program_query = ProgramQueryDef(name="echo", path="/queries/echo")
    stack_query = StackQueryDef(name="status", path="/queries/status", method="GET")
    assert program_query.method == "POST"
    assert stack_query.method == "GET"


def test_account_reader_from_def_uses_definition_parser() -> None:
    definition = ProgramAccountReadDef(account="State", parser=parse_fixture_account)
    reader = AccountReader.from_def(definition, FakeTransport())
    assert reader.account == "State"


def test_normalize_program_account_wire_keys_matches_ts() -> None:
    assert normalize_program_account_wire_keys(
        {"valueCount": 7, "Nested": {"innerValue": "x"}, "items": [{"innerValue": "y"}]}
    ) == {"value_count": 7, "nested": {"inner_value": "x"}, "items": [{"inner_value": "y"}]}


# ---------------------------------------------------------------------------
# Query executors
# ---------------------------------------------------------------------------


class FakeHttpError(AreteError):
    def __init__(self, status: int, body: str, headers=None) -> None:
        super().__init__(f"http {status}")
        self.status = status
        self.body = body
        self.headers = headers or {}


class FakeHttpClient:
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


@pytest.mark.asyncio
async def test_query_executor_posts_json_params_and_parses_result() -> None:
    http = FakeHttpClient({"value": "ok"})
    executor = QueryExecutor("https://stack.example.test/api/", http)
    query = ProgramQueryDef(
        name="echo", path="/queries/echo", parser=lambda data: data["value"]
    )
    result = await executor.execute(query, {"limit": 2})
    assert result == "ok"
    assert http.calls == [
        {
            "method": "POST",
            "url": "https://stack.example.test/api/queries/echo",
            "json_body": {"limit": 2},
            "target": None,
            "scopes": ("read",),
        }
    ]


@pytest.mark.asyncio
async def test_query_executor_supports_get_without_body() -> None:
    http = FakeHttpClient({"value": "got"})
    executor = QueryExecutor("https://stack.example.test", http)
    query = StackQueryDef(name="status", path="queries/status", method="GET")
    result = await executor.execute_stack(query)
    assert result == {"value": "got"}
    assert http.calls[0]["method"] == "GET"
    assert http.calls[0]["url"] == "https://stack.example.test/queries/status"
    assert http.calls[0]["json_body"] is None


@pytest.mark.asyncio
async def test_query_executor_surfaces_read_request_error_with_top_level_code() -> None:
    body = '{"error":"missing","code":"not-found"}'
    http = FakeHttpClient(FakeHttpError(404, body))
    executor = QueryExecutor("https://stack.example.test", http)
    query = ProgramQueryDef(name="echo", path="/queries/echo")
    with pytest.raises(ReadRequestError) as excinfo:
        await executor.execute(query, {"limit": 1})
    error = excinfo.value
    assert error.status == 404
    assert error.path == "/queries/echo"
    assert error.body == body
    assert error.server_error_code == "not-found"
    assert str(error) == f"Read request to '/queries/echo' failed (404): {body}"


@pytest.mark.asyncio
async def test_query_executor_ignores_nested_body_codes() -> None:
    # TS read.ts getServerErrorCode consults only the top-level `code` field.
    http = FakeHttpClient(FakeHttpError(500, '{"error":{"code":"NESTED"}}'))
    executor = QueryExecutor("https://stack.example.test", http)
    query = ProgramQueryDef(name="echo", path="/queries/echo")
    with pytest.raises(ReadRequestError) as excinfo:
        await executor.execute(query, {})
    assert excinfo.value.server_error_code is None


@pytest.mark.asyncio
async def test_query_executor_prefers_x_error_code_header() -> None:
    http = FakeHttpClient(
        FakeHttpError(401, '{"code":"top"}', headers={"x-error-code": "token-expired"})
    )
    executor = QueryExecutor("https://stack.example.test", http)
    query = ProgramQueryDef(name="echo", path="/queries/echo")
    with pytest.raises(ReadRequestError) as excinfo:
        await executor.execute(query, {})
    assert excinfo.value.server_error_code == "token-expired"


@pytest.mark.asyncio
async def test_query_executor_reports_result_schema_validation() -> None:
    def strict_parser(data: Any) -> str:
        if not isinstance(data["value"], str):
            raise TypeError("value must be a string")
        return data["value"]

    http = FakeHttpClient({"value": 42})
    executor = QueryExecutor("https://stack.example.test", http)
    query = ProgramQueryDef(name="echo", path="/queries/echo", parser=strict_parser)
    with pytest.raises(AreteError) as excinfo:
        await executor.execute(query, {"limit": 1})
    assert excinfo.value.message == "Query 'echo' failed schema validation"
    assert excinfo.value.code == "QUERY_VALIDATION"


@pytest.mark.asyncio
async def test_query_executor_propagates_non_http_errors() -> None:
    http = FakeHttpClient(ValueError("network unavailable"))
    executor = QueryExecutor("https://stack.example.test", http)
    query = ProgramQueryDef(name="echo", path="/queries/echo")
    with pytest.raises(ValueError, match="network unavailable"):
        await executor.execute(query, {})
