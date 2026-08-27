"""Chain read client (``/chain/*`` HTTP routes).

Port of ``typescript/core/src/chain.ts`` / Rust ``arete_sdk::chain``: the
:class:`ChainClient` protocol with the ten chain read methods and
:class:`HttpChainClient`, the HTTP implementation authenticated through
:mod:`arete.http` with the ``read`` scope.

``u64`` values are decimal strings on the wire (validated as ``^\\d+$`` within
u64) and native ``int`` in Python; account data is base64.
"""

from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass
from typing import Any, Dict, Optional, Protocol
from urllib.parse import quote

from arete.errors import AreteError
from arete.http import AuthTokenTarget, HttpAuthClient, HttpRequestError

U64_MAX = 18_446_744_073_709_551_615

# Server-side cap on one POST /chain/accounts batch.
MAX_CHAIN_BATCH_ADDRESSES = 100

# encodeURIComponent-safe characters (JS parity).
_URI_COMPONENT_SAFE = "-_.!~*'()"


class ChainError(AreteError):
    """A chain read request failed or returned an invalid response."""

    def __init__(
        self,
        message: str,
        *,
        status: Optional[int] = None,
        path: Optional[str] = None,
        body: Any = None,
        code: Optional[str] = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.path = path
        self.body = body
        self.code = code


@dataclass(frozen=True)
class ChainClock:
    slot: int
    unix_timestamp: int
    epoch: Optional[int] = None
    leader_schedule_epoch: Optional[int] = None


@dataclass(frozen=True)
class MintAccountInfo:
    address: str
    owner_program: str
    decimals: Optional[int] = None
    supply: Optional[str] = None
    mint_authority: Optional[str] = None
    freeze_authority: Optional[str] = None


@dataclass(frozen=True)
class TokenAccountInfo:
    address: str
    owner_program: str
    mint: Optional[str] = None
    owner: Optional[str] = None
    amount: Optional[str] = None
    ui_amount_string: Optional[str] = None


@dataclass(frozen=True)
class TokenBalanceInfo:
    exists: bool
    owner: str
    mint: str
    amount: str  # raw amount as a decimal string (mirrors the TS surface)
    context_slot: int
    address: Optional[str] = None
    token_program: Optional[str] = None
    decimals: Optional[int] = None
    ui_amount_string: Optional[str] = None


@dataclass(frozen=True)
class NativeBalanceInfo:
    lamports: int
    context_slot: int


@dataclass(frozen=True)
class RawAccountInfo:
    address: str
    owner_program: str
    lamports: int
    executable: bool
    data: bytes


class ChainClient(Protocol):
    """Read access to Solana chain state through the stack's ``/chain/*`` routes."""

    async def exists(self, address: str) -> bool: ...

    async def lamports(self, address: str) -> int: ...

    async def native_balance(
        self, address: str, *, min_context_slot: Optional[int] = None
    ) -> NativeBalanceInfo: ...

    async def minimum_balance_for_rent_exemption(self, space: int) -> int: ...

    async def clock(self) -> ChainClock: ...

    async def account(self, address: str) -> Optional[RawAccountInfo]: ...

    async def accounts(
        self, addresses: list[str]
    ) -> list[RawAccountInfo | None]: ...

    async def mint(self, address: str) -> Optional[MintAccountInfo]: ...

    async def token_account(self, address: str) -> Optional[TokenAccountInfo]: ...

    async def balance(
        self,
        *,
        owner: str,
        mint: str,
        token_program: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TokenBalanceInfo: ...


def _encode_component(value: str) -> str:
    return quote(value, safe=_URI_COMPONENT_SAFE)


def _parse_decimal_u64(value: Any, name: str, path: str) -> int:
    if not isinstance(value, str) or not value or not value.isascii() or not value.isdigit():
        raise ChainError(
            f"Invalid chain response for '{path}': {name} must be a decimal u64 string",
            path=path,
        )
    parsed = int(value)
    if parsed > U64_MAX:
        raise ChainError(
            f"Invalid chain response for '{path}': {name} exceeds u64", path=path
        )
    return parsed


def _parse_raw_account(body: Any, path: str) -> RawAccountInfo:
    try:
        data = base64.b64decode(body["data"], validate=True)
    except (binascii.Error, ValueError) as e:
        raise ChainError(
            f"Invalid chain response for '{path}': account data is not valid base64: {e}",
            path=path,
        ) from e
    return RawAccountInfo(
        address=body["address"],
        owner_program=body["ownerProgram"],
        lamports=body["lamports"],
        executable=body["executable"],
        data=data,
    )


def _serialize_context_slot(value: int) -> str:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0 or value > U64_MAX:
        raise ValueError("minContextSlot must be a non-negative integer within u64")
    return str(value)


def _with_context_slot(
    body: Dict[str, Any], min_context_slot: Optional[int]
) -> Dict[str, Any]:
    if min_context_slot is not None:
        body["minContextSlot"] = _serialize_context_slot(min_context_slot)
    return body


class HttpChainClient:
    """HTTP :class:`ChainClient` over a stack base URL.

    ``target`` (a ``solana-gateway-binding`` :class:`AuthTokenTarget`) makes
    every request mint targeted tokens — used by hosted gateway bindings.
    """

    def __init__(
        self,
        base_url: str,
        auth_client: HttpAuthClient,
        *,
        target: Optional[AuthTokenTarget] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._auth = auth_client
        self._target = target

    def _url(self, path: str) -> str:
        return self._base_url + (path if path.startswith("/") else "/" + path)

    async def _request(self, method: str, path: str, body: Any = None) -> Any:
        try:
            return await self._auth.request_json(
                method,
                self._url(path),
                json_body=body,
                target=self._target,
                scopes=("read",),
            )
        except HttpRequestError as e:
            status = getattr(e, "status", None)
            raise ChainError(
                f"Read request to '{path}' failed ({status}): {e}",
                status=status,
                path=path,
                body=getattr(e, "body", None),
                code=getattr(e, "code", None),
            ) from e

    async def exists(self, address: str) -> bool:
        body = await self._request("GET", f"/chain/exists/{_encode_component(address)}")
        return bool(body["exists"])

    async def lamports(self, address: str) -> int:
        body = await self._request("GET", f"/chain/lamports/{_encode_component(address)}")
        return body["lamports"]

    async def native_balance(
        self, address: str, *, min_context_slot: Optional[int] = None
    ) -> NativeBalanceInfo:
        path = "/chain/native-balance"
        payload = _with_context_slot({"address": address}, min_context_slot)
        body = await self._request("POST", path, payload)
        return NativeBalanceInfo(
            lamports=_parse_decimal_u64(body.get("lamports"), "lamports", path),
            context_slot=_parse_decimal_u64(body.get("contextSlot"), "contextSlot", path),
        )

    async def minimum_balance_for_rent_exemption(self, space: int) -> int:
        body = await self._request(
            "GET", f"/chain/rent-exemption/{_encode_component(str(space))}"
        )
        return body["lamports"]

    async def clock(self) -> ChainClock:
        body = await self._request("GET", "/chain/clock")
        return ChainClock(
            slot=body["slot"],
            epoch=body.get("epoch"),
            leader_schedule_epoch=body.get("leaderScheduleEpoch"),
            unix_timestamp=body["unixTimestamp"],
        )

    async def account(self, address: str) -> Optional[RawAccountInfo]:
        path = f"/chain/accounts/{_encode_component(address)}"
        body = await self._request("GET", path)
        return None if body is None else _parse_raw_account(body, path)

    async def accounts(
        self, addresses: list[str]
    ) -> list[RawAccountInfo | None]:
        requested = list(addresses)
        if len(requested) > MAX_CHAIN_BATCH_ADDRESSES:
            raise ValueError(
                f"addresses exceeds the {MAX_CHAIN_BATCH_ADDRESSES}-address "
                "limit for one batch"
            )
        if not requested:
            return []
        path = "/chain/accounts"
        body = await self._request("POST", path, {"addresses": requested})
        # Items are positionally aligned with the requested addresses.
        return [
            None if item is None else _parse_raw_account(item, path)
            for item in body["items"]
        ]

    async def mint(self, address: str) -> Optional[MintAccountInfo]:
        body = await self._request(
            "GET", f"/chain/mints/{_encode_component(address)}"
        )
        if body is None:
            return None
        return MintAccountInfo(
            address=body["address"],
            owner_program=body["ownerProgram"],
            decimals=body.get("decimals"),
            supply=body.get("supply"),
            mint_authority=body.get("mintAuthority"),
            freeze_authority=body.get("freezeAuthority"),
        )

    async def token_account(self, address: str) -> Optional[TokenAccountInfo]:
        body = await self._request(
            "GET", f"/chain/token-accounts/{_encode_component(address)}"
        )
        if body is None:
            return None
        return TokenAccountInfo(
            address=body["address"],
            owner_program=body["ownerProgram"],
            mint=body.get("mint"),
            owner=body.get("owner"),
            amount=body.get("amount"),
            ui_amount_string=body.get("uiAmountString"),
        )

    async def balance(
        self,
        *,
        owner: str,
        mint: str,
        token_program: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TokenBalanceInfo:
        path = "/chain/balances"
        payload: Dict[str, Any] = {"owner": owner, "mint": mint}
        if token_program is not None:
            payload["tokenProgram"] = token_program
        payload = _with_context_slot(payload, min_context_slot)
        body = await self._request("POST", path, payload)
        return TokenBalanceInfo(
            exists=body["exists"],
            address=body.get("address"),
            owner=body["owner"],
            mint=body["mint"],
            token_program=body.get("tokenProgram"),
            amount=body["amount"],
            decimals=body.get("decimals"),
            ui_amount_string=body.get("uiAmountString"),
            context_slot=_parse_decimal_u64(body.get("contextSlot"), "contextSlot", path),
        )
