"""Transaction relay transport (``POST <base>/transactions/v1/*``).

Port of ``typescript/core/src/transactions.ts`` / Rust
``arete_sdk::transactions``: the :class:`TransactionTransport` protocol over
the six relay routes and :class:`HttpTransactionTransport`, authenticated via
:mod:`arete.http` with ``transaction:inspect`` / ``transaction:send`` scopes.
``send`` requests carry the predispatch marker and only replay after a token
refresh when the server proved the upstream dispatch was never attempted.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Protocol, Sequence

from arete.errors import AreteError
from arete.http import AuthTokenTarget, HttpAuthClient, HttpRequestError

INSPECT_SCOPE = "transaction:inspect"
SEND_SCOPE = "transaction:send"

_COMMITMENTS = ("processed", "confirmed", "finalized")


class TransactionTransportError(AreteError):
    """Structured transaction relay failure (full TS error body)."""

    def __init__(
        self,
        status: int,
        *,
        code: str,
        message: str,
        retryable: bool = False,
        request_id: Optional[str] = None,
        submission_state: Optional[str] = None,  # 'not_submitted' | 'unknown'
        signature: Optional[str] = None,
        details: Any = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.retryable = retryable
        self.request_id = request_id
        self.submission_state = submission_state
        self.signature = signature
        self.details = details


@dataclass(frozen=True)
class LatestBlockhashResult:
    blockhash: str
    context_slot: int
    last_valid_block_height: int


@dataclass(frozen=True)
class TransactionFeeResult:
    fee_lamports: Optional[int]
    context_slot: int


@dataclass(frozen=True)
class TransactionSimulationResult:
    context_slot: int
    err: Any = None
    logs: Optional[List[str]] = None
    units_consumed: Optional[int] = None
    accounts: Optional[List[Any]] = None
    # Loaded account data size the simulated transaction touched, in bytes.
    # Absent/null stays ``None``; ``"0"`` parses to ``0`` (the distinction is
    # load-bearing for transaction V1 budget estimation, SIMD-0385).
    loaded_accounts_data_size: Optional[int] = None


@dataclass(frozen=True)
class TransactionSendResult:
    signature: str


@dataclass(frozen=True)
class TransactionSignatureStatus:
    signature: str
    slot: Optional[int]
    confirmation_status: Optional[str]
    err: Any = None


@dataclass(frozen=True)
class TransactionAccountBalance:
    pubkey: str
    pre_balance: int
    post_balance: int

@dataclass(frozen=True)
class ConfirmedTransaction:
    signature: str
    slot: int
    block_time: Optional[int]
    err: Any
    accounts: tuple[TransactionAccountBalance, ...]
    transaction: Optional[Dict[str, Any]] = None
    meta: Optional[Dict[str, Any]] = None
    version: Any = None
    metadata_available: Optional[bool] = None

class TransactionTransport(Protocol):
    async def get(self, signature: str, *, commitment: Optional[str] = None, max_supported_transaction_version: Optional[int] = None) -> Optional[ConfirmedTransaction]: ...
    async def get_latest_blockhash(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> LatestBlockhashResult: ...

    async def get_fee_for_message(
        self,
        message: str,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TransactionFeeResult: ...

    async def simulate_transaction(
        self,
        transaction: str,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
        accounts: Optional[Sequence[str]] = None,
        inner_instructions: Optional[bool] = None,
        replace_recent_blockhash: Optional[bool] = None,
    ) -> TransactionSimulationResult: ...

    async def send_transaction(
        self,
        transaction: str,
        *,
        skip_preflight: Optional[bool] = None,
        preflight_commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TransactionSendResult: ...

    async def get_signature_status(
        self,
        signature: str,
        *,
        search_transaction_history: Optional[bool] = None,
    ) -> Optional[TransactionSignatureStatus]: ...

    async def get_block_height(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> int: ...


def _decimal(value: Optional[int]) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError("minContextSlot must be a non-negative integer")
    return str(value)


def _int_field(value: Any, name: str) -> int:
    if not isinstance(value, str) or not value or not value.isascii() or not value.isdigit():
        raise TransactionTransportError(
            0,
            code="invalid_response",
            message=f"Invalid decimal u64 field '{name}' in transaction response",
        )
    parsed = int(value)
    if parsed > 18446744073709551615:
        raise TransactionTransportError(0, code="invalid_response", message=f"{name} exceeds u64")
    return parsed


def _optional_int(value: Any, name: str) -> Optional[int]:
    if value is None:
        return None
    return _int_field(value, name)


def _request_body(entries: Dict[str, Any]) -> Dict[str, Any]:
    return {key: value for key, value in entries.items() if value is not None}


def _transport_error(status: Optional[int], body: Any) -> TransactionTransportError:
    parsed = body if isinstance(body, dict) else {}
    status = status or 0
    # Public errors are deliberately synthesized without reflecting raw bodies.
    return TransactionTransportError(
        status,
        code=parsed.get("code")
        if isinstance(parsed.get("code"), str)
        else "transaction_transport_error",
        message=parsed.get("message")
        if isinstance(parsed.get("message"), str)
        else f"Transaction request failed ({status})",
        retryable=parsed.get("retryable") is True,
        request_id=parsed.get("request_id", parsed.get("requestId")),
        submission_state=parsed.get("submission_state", parsed.get("submissionState")),
        signature=parsed.get("signature"),
        details=parsed.get("details"),
    )


class HttpTransactionTransport:
    """HTTP :class:`TransactionTransport` over ``<base>/transactions/v1``."""

    def __init__(
        self,
        base_url: str,
        auth_client: HttpAuthClient,
        *,
        target: Optional[AuthTokenTarget] = None,
    ) -> None:
        self._root = base_url.rstrip("/") + "/transactions/v1"
        self._auth = auth_client
        self._target = target

    async def _post(
        self, route: str, body: Dict[str, Any], scope: str
    ) -> Any:
        try:
            return await self._auth.request_json(
                "POST",
                f"{self._root}/{route}",
                json_body=_request_body(body),
                target=self._target,
                scopes=(scope,),
            )
        except HttpRequestError as e:
            raise _transport_error(
                getattr(e, "status", None), getattr(e, "body", None)
            ) from e

    async def get(self, signature: str, *, commitment: Optional[str] = None, max_supported_transaction_version: Optional[int] = None) -> Optional[ConfirmedTransaction]:
        if commitment is not None and commitment not in ("confirmed", "finalized"):
            raise ValueError("get accepts confirmed or finalized")
        if max_supported_transaction_version is not None and (isinstance(max_supported_transaction_version, bool) or not isinstance(max_supported_transaction_version, int) or not 0 <= max_supported_transaction_version <= 255):
            raise ValueError("Invalid maximum transaction version")
        body = await self._post("get", {"signature": signature, "commitment": commitment, "maxSupportedTransactionVersion": max_supported_transaction_version}, INSPECT_SCOPE)
        if "transaction" not in body:
            raise ValueError("Missing transaction field")
        tx = body["transaction"]
        if tx is None:
            return None
        if not isinstance(tx.get("signature"), str) or not isinstance(tx.get("accounts"), list):
            raise ValueError("Invalid transaction response")
        accounts = []
        for item in tx["accounts"]:
            if not isinstance(item.get("pubkey"), str):
                raise ValueError("Invalid transaction account pubkey")
            accounts.append(TransactionAccountBalance(item["pubkey"], _int_field(item.get("preBalance"), "preBalance"), _int_field(item.get("postBalance"), "postBalance")))
        block_time = tx.get("blockTime")
        if block_time is not None:
            import re
            if not isinstance(block_time, str) or re.fullmatch(r"-?[0-9]+", block_time) is None or not -9223372036854775808 <= int(block_time) <= 9223372036854775807:
                raise ValueError("Invalid blockTime")
            block_time = int(block_time)
        return ConfirmedTransaction(tx["signature"], _int_field(tx.get("slot"), "slot"), block_time, tx.get("err"), tuple(accounts), tx.get("transaction"), tx.get("meta"), tx.get("version"), tx.get("metadataAvailable"))

    async def get_latest_blockhash(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> LatestBlockhashResult:
        value = await self._post(
            "latest-blockhash",
            {"commitment": commitment, "minContextSlot": _decimal(min_context_slot)},
            INSPECT_SCOPE,
        )
        return LatestBlockhashResult(
            blockhash=str(value["blockhash"]),
            context_slot=_int_field(value.get("contextSlot"), "contextSlot"),
            last_valid_block_height=_int_field(
                value.get("lastValidBlockHeight"), "lastValidBlockHeight"
            ),
        )

    async def get_fee_for_message(
        self,
        message: str,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TransactionFeeResult:
        value = await self._post(
            "fee",
            {
                "message": message,
                "commitment": commitment,
                "minContextSlot": _decimal(min_context_slot),
            },
            INSPECT_SCOPE,
        )
        fee = value.get("feeLamports")
        return TransactionFeeResult(
            fee_lamports=None if fee is None else _int_field(fee, "feeLamports"),
            context_slot=_int_field(value.get("contextSlot"), "contextSlot"),
        )

    async def simulate_transaction(
        self,
        transaction: str,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
        accounts: Optional[Sequence[str]] = None,
        inner_instructions: Optional[bool] = None,
        replace_recent_blockhash: Optional[bool] = None,
    ) -> TransactionSimulationResult:
        value = await self._post(
            "simulate",
            {
                "transaction": transaction,
                "commitment": commitment,
                "minContextSlot": _decimal(min_context_slot),
                "accounts": {"addresses": list(accounts)} if accounts else None,
                "innerInstructions": inner_instructions,
                "replaceRecentBlockhash": replace_recent_blockhash,
            },
            INSPECT_SCOPE,
        )
        return TransactionSimulationResult(
            context_slot=_int_field(value.get("contextSlot"), "contextSlot"),
            err=value.get("err"),
            logs=value.get("logs"),
            units_consumed=_optional_int(value.get("unitsConsumed"), "unitsConsumed"),
            accounts=value.get("accounts"),
            loaded_accounts_data_size=_optional_int(
                value.get("loadedAccountsDataSize"), "loadedAccountsDataSize"
            ),
        )

    async def send_transaction(
        self,
        transaction: str,
        *,
        skip_preflight: Optional[bool] = None,
        preflight_commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TransactionSendResult:
        value = await self._post(
            "send",
            {
                "transaction": transaction,
                "skipPreflight": skip_preflight,
                "preflightCommitment": preflight_commitment,
                "minContextSlot": _decimal(min_context_slot),
            },
            SEND_SCOPE,
        )
        return TransactionSendResult(signature=str(value["signature"]))

    async def get_signature_status(
        self,
        signature: str,
        *,
        search_transaction_history: Optional[bool] = None,
    ) -> Optional[TransactionSignatureStatus]:
        value = await self._post(
            "signature-status",
            {
                "signature": signature,
                "searchTransactionHistory": search_transaction_history,
            },
            INSPECT_SCOPE,
        )
        status = value.get("status") if isinstance(value, dict) else None
        if not status:
            return None
        slot = status.get("slot")
        return TransactionSignatureStatus(
            signature=signature,
            slot=None if slot is None else _int_field(slot, "slot"),
            confirmation_status=status.get("confirmationStatus"),
            err=status.get("err"),
        )

    async def get_block_height(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> int:
        value = await self._post(
            "block-height",
            {"commitment": commitment, "minContextSlot": _decimal(min_context_slot)},
            INSPECT_SCOPE,
        )
        return _int_field(value.get("blockHeight"), "blockHeight")
