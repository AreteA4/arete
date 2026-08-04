"""Program and stack HTTP read primitives.

Python projection of ``typescript/core/src/read.ts`` plus the account/query
surfaces from ``typescript/core/src/client.ts`` (``createAccountReader``,
``createQueryExecutor``, ``parseProgramAccountValue``,
``normalizeProgramAccountWireKeys``). Rust sibling: ``arete_sdk::read``.
Wire contract: ``program-read-http/v1`` (sdk-core-api.md §2.2/§8).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import (
    Any,
    Callable,
    Generic,
    Mapping,
    Optional,
    Protocol,
    Sequence,
    Tuple,
    TypeVar,
    Union,
)

from arete.errors import AreteError

T = TypeVar("T")

READ_SCOPES: Tuple[str, ...] = ("read",)


class ReadRequestError(AreteError):
    """Failed (non-2xx) HTTP read (TS ``ReadRequestError``)."""

    def __init__(
        self,
        *,
        status: int,
        path: str,
        body: str,
        server_error_code: Optional[str] = None,
    ) -> None:
        super().__init__(f"Read request to '{path}' failed ({status}): {body}")
        self.status = status
        self.path = path
        self.body = body
        self.server_error_code = server_error_code


def _coded_error(message: str, code: str) -> AreteError:
    """TS ``new AreteError(message, code)``."""
    return AreteError(message, code)


# ---------------------------------------------------------------------------
# Read requests + transport seam (TS `ProgramReadRequest` / `ProgramReadTransport`)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProgramReadRequest:
    """One program read operation. ``operation`` is ``"fetch" | "fetch_many" | "exists"``."""

    operation: str
    account: str
    address: Optional[str] = None
    addresses: Optional[Tuple[str, ...]] = None

    @classmethod
    def fetch(cls, account: str, address: str) -> "ProgramReadRequest":
        return cls("fetch", account, address=address)

    @classmethod
    def fetch_many(cls, account: str, addresses: Sequence[str]) -> "ProgramReadRequest":
        return cls("fetch_many", account, addresses=tuple(addresses))

    @classmethod
    def exists(cls, account: str, address: str) -> "ProgramReadRequest":
        return cls("exists", account, address=address)


class ProgramReadTransport(Protocol):
    """Structural transport interface; returns the raw decoded JSON wire value."""

    async def read(self, request: ProgramReadRequest) -> Any: ...


# ---------------------------------------------------------------------------
# Generated read definitions (TS `programAccountRead` / `programQuery` / `stackQuery`)
# ---------------------------------------------------------------------------

Parser = Callable[[Any], T]


@dataclass(frozen=True)
class ProgramAccountReadDef(Generic[T]):
    """Generated account read definition; ``parser`` replaces the TS zod schema."""

    account: str
    parser: Optional[Parser[T]] = None


@dataclass(frozen=True)
class ProgramQueryDef(Generic[T]):
    """Generated program-scoped query definition (method ``"GET" | "POST"``)."""

    name: str
    path: str
    method: str = "POST"
    parser: Optional[Parser[T]] = None


@dataclass(frozen=True)
class StackQueryDef(Generic[T]):
    """Generated stack-scoped query definition (method ``"GET" | "POST"``)."""

    name: str
    path: str
    method: str = "POST"
    parser: Optional[Parser[T]] = None


# ---------------------------------------------------------------------------
# Account value parsing (TS `parseProgramAccountValue` + key normalization)
# ---------------------------------------------------------------------------


def _camel_to_snake(key: str) -> str:
    out = []
    for index, ch in enumerate(key):
        if ch.isascii() and ch.isupper():
            if index != 0:
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def normalize_program_account_wire_keys(value: Any) -> Any:
    """Recursively rewrite camelCase object keys to snake_case (TS port)."""
    if isinstance(value, list):
        return [normalize_program_account_wire_keys(item) for item in value]
    if isinstance(value, dict):
        return {
            _camel_to_snake(key): normalize_program_account_wire_keys(nested)
            for key, nested in value.items()
        }
    return value


def _parse_account_value(account: str, parser: Optional[Parser[T]], value: Any) -> T:
    if parser is None:
        return value
    try:
        return parser(value)
    except Exception:
        pass
    try:
        return parser(normalize_program_account_wire_keys(value))
    except Exception:
        raise _coded_error(
            f"Program account read '{account}' failed schema validation",
            "SCHEMA_VALIDATION",
        ) from None


# ---------------------------------------------------------------------------
# HTTP error coercion (shared by the transport and query executors)
# ---------------------------------------------------------------------------


def _header_error_code(headers: Any) -> Optional[str]:
    if headers is None:
        return None
    try:
        items = headers.items()
    except AttributeError:
        return None
    for key, value in items:
        if isinstance(key, str) and key.lower() == "x-error-code" and isinstance(value, str):
            return value
    return None


def _body_error_code(body: str, nested: bool) -> Optional[str]:
    try:
        parsed = json.loads(body)
    except (TypeError, ValueError):
        return None
    if not isinstance(parsed, dict):
        return None
    if nested:
        error = parsed.get("error")
        if isinstance(error, dict) and isinstance(error.get("code"), str):
            return error["code"]
    code = parsed.get("code")
    return code if isinstance(code, str) else None


def coerce_read_request_error(
    error: Exception,
    path: str,
    *,
    nested_body_code: bool = True,
) -> Optional[ReadRequestError]:
    """Translate an HTTP-layer exception into :class:`ReadRequestError`.

    Duck-typed against :class:`arete.errors.HttpRequestError`: requires an
    integer ``status`` (or ``status_code``); reads the response text from
    ``body``/``response_body``/``text``. Server error code precedence follows
    TS ``responseErrorCode``: ``X-Error-Code`` header, then the body's nested
    ``error.code`` (program reads only), then top-level ``code``, then a
    ``code``-like attribute on the exception. Returns ``None`` when the
    exception is not an HTTP response error (network failures propagate).
    """
    if isinstance(error, ReadRequestError):
        return error
    status: Any = getattr(error, "status", None)
    if isinstance(status, bool) or not isinstance(status, int):
        status = getattr(error, "status_code", None)
    if isinstance(status, bool) or not isinstance(status, int):
        return None
    body = ""
    for attr in ("body", "response_body", "text"):
        candidate = getattr(error, attr, None)
        if candidate is None:
            continue
        if isinstance(candidate, bytes):
            body = candidate.decode("utf-8", "replace")
        elif isinstance(candidate, str):
            body = candidate
        else:
            # HttpRequestError.body may carry the decoded JSON body.
            body = json.dumps(candidate, separators=(",", ":"))
        break
    code = _header_error_code(getattr(error, "headers", None))
    if code is None:
        code = _body_error_code(body, nested_body_code)
    if code is None:
        for attr in ("server_error_code", "error_code", "code"):
            candidate = getattr(error, attr, None)
            if isinstance(candidate, str):
                code = candidate
                break
    return ReadRequestError(status=status, path=path, body=body, server_error_code=code)


# ---------------------------------------------------------------------------
# Typed account reader (TS `createAccountReader`, Rust `AccountReader<T>`)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class AccountBatchItem(Generic[T]):
    """One item of a batched read; ``status`` is ``"ok" | "missing" | "error"``."""

    address: str
    status: str
    value: Optional[T] = None
    error_code: Optional[str] = None


@dataclass(frozen=True)
class AccountBatchResult(Generic[T]):
    items: Tuple[AccountBatchItem[T], ...]


class AccountReader(Generic[T]):
    """Typed reader over one generated program account (``fetch/fetch_many/exists``)."""

    def __init__(
        self,
        account: str,
        transport: ProgramReadTransport,
        parser: Optional[Parser[T]] = None,
    ) -> None:
        self._account = account
        self._transport = transport
        self._parser = parser

    @classmethod
    def from_def(
        cls,
        definition: ProgramAccountReadDef[T],
        transport: ProgramReadTransport,
    ) -> "AccountReader[T]":
        return cls(definition.account, transport, definition.parser)

    @property
    def account(self) -> str:
        return self._account

    async def fetch(self, address: str) -> Optional[T]:
        """Fetch one decoded account; a ``null`` wire body means missing → ``None``."""
        value = await self._transport.read(ProgramReadRequest.fetch(self._account, address))
        if value is None:
            return None
        return _parse_account_value(self._account, self._parser, value)

    async def fetch_many(self, addresses: Sequence[str]) -> AccountBatchResult[T]:
        """Fetch a mixed batch; per-address ``ok``/``missing``/``error`` statuses and order are preserved."""
        value = await self._transport.read(
            ProgramReadRequest.fetch_many(self._account, addresses)
        )
        raw_items = value.get("items") if isinstance(value, Mapping) else None
        if not isinstance(raw_items, list):
            raise self._invalid_response("batch")
        items = []
        for raw in raw_items:
            if not isinstance(raw, Mapping) or not isinstance(raw.get("address"), str):
                raise self._invalid_response("batch")
            address = raw["address"]
            status = raw.get("status")
            if status == "ok":
                items.append(
                    AccountBatchItem(
                        address=address,
                        status="ok",
                        value=_parse_account_value(self._account, self._parser, raw.get("value")),
                    )
                )
            elif status == "missing":
                items.append(AccountBatchItem(address=address, status="missing"))
            elif status == "error":
                error = raw.get("error")
                code = error.get("code") if isinstance(error, Mapping) else None
                if not isinstance(code, str):
                    raise self._invalid_response("batch")
                items.append(
                    AccountBatchItem(address=address, status="error", error_code=code)
                )
            else:
                raise self._invalid_response("batch")
        return AccountBatchResult(items=tuple(items))

    async def exists(self, address: str) -> bool:
        """Existence probe (``…/<address>/exists`` → ``{"exists": bool}``)."""
        value = await self._transport.read(ProgramReadRequest.exists(self._account, address))
        exists = value.get("exists") if isinstance(value, Mapping) else None
        if not isinstance(exists, bool):
            raise self._invalid_response("exists")
        return exists

    def _invalid_response(self, operation: str) -> AreteError:
        return _coded_error(
            f"Program account read '{self._account}' returned an invalid {operation} response",
            "INVALID_RESPONSE",
        )


# ---------------------------------------------------------------------------
# Query executors (TS `createQueryExecutor` + `readJson`, Rust `QueryExecutor`)
# ---------------------------------------------------------------------------


def resolve_read_url(http_base: str, path: str) -> str:
    base = http_base[:-1] if http_base.endswith("/") else http_base
    return f"{base}{path}" if path.startswith("/") else f"{base}/{path}"


class QueryExecutor:
    """Executes stack- and program-scoped queries against the stack HTTP base URL.

    ``http`` is :class:`arete.http.HttpAuthClient`-shaped (``request_json``);
    token strategy and refresh-replay-once live there.
    """

    def __init__(self, http_base: str, http: Any) -> None:
        self._http_base = http_base
        self._http = http

    async def execute(
        self,
        query: Union[ProgramQueryDef[T], StackQueryDef[T]],
        params: Any = None,
    ) -> T:
        """Execute a program-scoped query (JSON params, ``read`` scope)."""
        return await self._run(query.name, query.path, query.method, query.parser, params)

    async def execute_stack(self, query: StackQueryDef[T], params: Any = None) -> T:
        """Execute a stack-scoped query."""
        return await self._run(query.name, query.path, query.method, query.parser, params)

    async def _run(
        self,
        name: str,
        path: str,
        method: str,
        parser: Optional[Parser[T]],
        params: Any,
    ) -> T:
        url = resolve_read_url(self._http_base, path)
        json_body = params if method == "POST" else None
        try:
            value = await self._http.request_json(
                method,
                url,
                json_body=json_body,
                scopes=READ_SCOPES,
            )
        except Exception as error:
            # Query reads consult only the top-level body `code` (TS read.ts).
            read_error = coerce_read_request_error(error, path, nested_body_code=False)
            if read_error is None:
                raise
            raise read_error from error
        if parser is None:
            return value
        try:
            return parser(value)
        except Exception:
            raise _coded_error(
                f"Query '{name}' failed schema validation",
                "QUERY_VALIDATION",
            ) from None
