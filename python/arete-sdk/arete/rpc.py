"""Direct Solana JSON-RPC implementation of :class:`arete.transactions.TransactionTransport`.

Arete's relay stays the default everywhere; this is the explicit escape hatch
for a caller who wants to talk to their own provider. It satisfies the same
protocol :class:`arete.transactions.HttpTransactionTransport` does, so the
compiler, budget planner, inspection, signing and confirmation above it are
unchanged -- only the wire underneath differs.

It adds no dependency (the SDK already carries ``httpx``), needs no Solana
library, and does not raise the base Python minimum. Provider endpoint and
credentials are configured here and never mixed with Arete authentication: no
Arete token reaches an RPC provider, and no provider header reaches Arete.

Two wire differences the relay hides:

* the relay encodes ``u64`` as decimal strings and returns flat bodies, while
  native JSON-RPC uses JSON numbers, wraps most results in
  ``{"context": ..., "value": ...}`` and reports failures as an ``error``
  member inside a ``200 OK`` body. Nothing here reuses the relay's decimal
  parsing;
* an absent simulation metric stays absent. ``unitsConsumed`` and
  ``loadedAccountsDataSize`` are ``None`` when the node did not report them
  and ``0`` when it measured zero, because a V1 budget derived from a missing
  metric would be a guess rather than a measurement.

Usage::

    from arete import Arete
    from arete.rpc import RpcTransactionTransport

    rpc = RpcTransactionTransport("https://api.devnet.solana.com")
    a4 = await Arete.connect(api_key="hspk_…", transactions=rpc)
"""

from __future__ import annotations

import itertools
from typing import Any, Dict, Optional, Sequence

import httpx

from arete.transactions import (
    LatestBlockhashResult,
    TransactionFeeResult,
    TransactionSendResult,
    TransactionSignatureStatus,
    TransactionSimulationResult,
    TransactionTransportError,
)

__all__ = ["RpcTransactionTransport"]

#: JSON-RPC error codes that prove a ``sendTransaction`` never reached the
#: cluster, so the caller may rebuild rather than reconcile. Anything else --
#: a transport failure, a gateway error, an unrecognized code -- leaves the
#: submission state unknown, which is what keeps the locally derived
#: signature authoritative in the wallet adapter.
_NOT_SUBMITTED_RPC_CODES = frozenset(
    {
        -32002,  # SendTransactionPreflightFailure: the node simulated and refused.
        -32003,  # TransactionSignatureVerificationFailure.
        -32602,  # Invalid params: the node never parsed the transaction.
    }
)


def _config(**entries: Any) -> Dict[str, Any]:
    """A JSON-RPC config object with the unset members dropped."""
    return {key: value for key, value in entries.items() if value is not None}


def _invalid(method: str, detail: str) -> TransactionTransportError:
    return TransactionTransportError(
        0,
        code="invalid_response",
        message=f"RPC {method!r}: {detail}",
    )


def _u64(method: str, value: Any, field: str) -> int:
    """A required integer result. JSON numbers, never decimal strings."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise _invalid(method, f"{field!r} must be an integer, got {value!r}")
    return value


def _optional_u64(method: str, value: Any, field: str) -> Optional[int]:
    """An integer that may legitimately be absent. A present non-integer is a
    malformed response, never silently dropped to ``None``."""
    if value is None:
        return None
    return _u64(method, value, field)


class RpcTransactionTransport:
    """:class:`arete.transactions.TransactionTransport` over a node's JSON-RPC
    endpoint.

    ``headers`` carries provider credentials (an API key, say); they are sent
    to ``url`` and nowhere else. Pass ``http_client`` to share a pool or to
    impose a request timeout.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: Optional[Dict[str, str]] = None,
        http_client: Optional[httpx.AsyncClient] = None,
    ) -> None:
        self._url = url
        self._headers = dict(headers or {})
        self._owns_client = http_client is None
        self._http = http_client or httpx.AsyncClient()
        self._ids = itertools.count(1)

    @property
    def url(self) -> str:
        return self._url

    async def close(self) -> None:
        """Close the HTTP client, unless it was supplied by the caller."""
        if self._owns_client:
            await self._http.aclose()

    async def _call(self, method: str, params: Any) -> Any:
        try:
            response = await self._http.post(
                self._url,
                json={
                    "jsonrpc": "2.0",
                    "id": next(self._ids),
                    "method": method,
                    "params": params,
                },
                headers=self._headers or None,
            )
        except Exception as cause:
            raise TransactionTransportError(
                0,
                code="rpc_request_failed",
                message=f"RPC {method!r} request failed: {cause}",
                retryable=True,
            ) from cause

        status = response.status_code
        try:
            body = response.json()
        except Exception:
            body = None
        parsed = body if isinstance(body, dict) else {}

        # A node reports application failures inside a 200 body; a gateway or
        # rate limiter reports its own with a status and no envelope. Both
        # become the same typed transport error the relay produces.
        error = parsed.get("error")
        if isinstance(error, dict):
            raise self._rpc_error(method, status, error)
        if not 200 <= status < 300:
            raise TransactionTransportError(
                status,
                code="rpc_http_error",
                message=f"RPC {method!r} failed with HTTP {status}",
                retryable=status == 429 or 500 <= status < 600,
            )
        if "result" not in parsed:
            raise _invalid(method, "response carried neither 'result' nor 'error'")
        return parsed["result"]

    @staticmethod
    def _rpc_error(
        method: str, status: int, error: Dict[str, Any]
    ) -> TransactionTransportError:
        code = error.get("code")
        submission_state = None
        if method == "sendTransaction" and code in _NOT_SUBMITTED_RPC_CODES:
            submission_state = "not_submitted"
        return TransactionTransportError(
            status,
            code=f"rpc_{code}" if isinstance(code, int) else "rpc_error",
            message=(
                f"RPC {method!r} failed: "
                f"{error.get('message') if isinstance(error.get('message'), str) else 'no message'}"
            ),
            retryable=status == 429 or 500 <= status < 600,
            submission_state=submission_state,
            # A node never names a signature on an error, so there is nothing
            # here that could displace the one the caller signed.
            signature=None,
            details=error.get("data"),
        )

    @staticmethod
    def _context_value(method: str, result: Any) -> Any:
        """The ``{context, value}`` envelope, as ``(context_slot, value)``."""
        if not isinstance(result, dict):
            raise _invalid(method, f"result must be an object, got {result!r}")
        context = result.get("context")
        slot = context.get("slot") if isinstance(context, dict) else None
        if "value" not in result:
            raise _invalid(method, "missing 'value'")
        return _u64(method, slot, "context.slot"), result["value"]

    # -- reads -------------------------------------------------------------

    async def get_latest_blockhash(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> LatestBlockhashResult:
        method = "getLatestBlockhash"
        result = await self._call(
            method, [_config(commitment=commitment, minContextSlot=min_context_slot)]
        )
        context_slot, value = self._context_value(method, result)
        if not isinstance(value, dict) or not isinstance(value.get("blockhash"), str):
            raise _invalid(method, "missing 'value.blockhash'")
        return LatestBlockhashResult(
            blockhash=value["blockhash"],
            context_slot=context_slot,
            last_valid_block_height=_u64(
                method, value.get("lastValidBlockHeight"), "value.lastValidBlockHeight"
            ),
        )

    async def get_fee_for_message(
        self,
        message: str,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> TransactionFeeResult:
        method = "getFeeForMessage"
        result = await self._call(
            method,
            [message, _config(commitment=commitment, minContextSlot=min_context_slot)],
        )
        context_slot, value = self._context_value(method, result)
        # ``null`` is the node's answer for a message whose blockhash it no
        # longer holds -- not a malformed response, and not a zero fee.
        return TransactionFeeResult(
            fee_lamports=_optional_u64(method, value, "value"),
            context_slot=context_slot,
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
        method = "simulateTransaction"
        config = _config(
            encoding="base64",
            # Inspection and budget estimation simulate an unsigned message
            # carrying placeholder signatures, so verification stays off.
            sigVerify=False,
            commitment=commitment,
            minContextSlot=min_context_slot,
            innerInstructions=inner_instructions,
            replaceRecentBlockhash=replace_recent_blockhash,
            accounts=(
                {"encoding": "base64", "addresses": list(accounts)}
                if accounts
                else None
            ),
        )
        result = await self._call(method, [transaction, config])
        context_slot, value = self._context_value(method, result)
        if not isinstance(value, dict):
            raise _invalid(method, f"'value' must be an object, got {value!r}")
        return TransactionSimulationResult(
            context_slot=context_slot,
            err=value.get("err"),
            logs=value.get("logs"),
            units_consumed=_optional_u64(
                method, value.get("unitsConsumed"), "unitsConsumed"
            ),
            accounts=value.get("accounts"),
            loaded_accounts_data_size=_optional_u64(
                method, value.get("loadedAccountsDataSize"), "loadedAccountsDataSize"
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
        method = "sendTransaction"
        result = await self._call(
            method,
            [
                transaction,
                _config(
                    encoding="base64",
                    skipPreflight=skip_preflight,
                    preflightCommitment=preflight_commitment,
                    minContextSlot=min_context_slot,
                ),
            ],
        )
        if not isinstance(result, str) or not result:
            raise _invalid(method, "result must be a signature string")
        return TransactionSendResult(signature=result)

    async def get_signature_status(
        self,
        signature: str,
        *,
        search_transaction_history: Optional[bool] = None,
    ) -> Optional[TransactionSignatureStatus]:
        statuses = await self.get_signature_statuses(
            [signature], search_transaction_history=search_transaction_history
        )
        return statuses[0]

    async def get_signature_statuses(
        self,
        signatures: Sequence[str],
        *,
        search_transaction_history: Optional[bool] = None,
    ) -> list:
        """Batch status lookup, positionally aligned with ``signatures``."""
        method = "getSignatureStatuses"
        signatures = list(signatures)
        if not signatures:
            return []
        result = await self._call(
            method,
            [
                signatures,
                _config(searchTransactionHistory=search_transaction_history),
            ],
        )
        _, value = self._context_value(method, result)
        if not isinstance(value, list):
            raise _invalid(method, "'value' must be an array")
        # Callers read these positionally against their own list, so a length
        # mismatch would attribute one transaction's outcome to another.
        if len(value) != len(signatures):
            raise _invalid(
                method, f"expected {len(signatures)} statuses, got {len(value)}"
            )
        return [
            None
            if entry is None
            else TransactionSignatureStatus(
                signature=signature,
                slot=_optional_u64(method, entry.get("slot"), "slot"),
                confirmation_status=entry.get("confirmationStatus"),
                err=entry.get("err"),
            )
            for signature, entry in zip(signatures, value)
        ]

    async def get_block_height(
        self,
        *,
        commitment: Optional[str] = None,
        min_context_slot: Optional[int] = None,
    ) -> int:
        method = "getBlockHeight"
        result = await self._call(
            method, [_config(commitment=commitment, minContextSlot=min_context_slot)]
        )
        return _u64(method, result, "result")
