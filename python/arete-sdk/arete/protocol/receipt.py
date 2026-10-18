"""Execution receipts: check a receipt offline, and check its claims against
the transaction itself.

Mirror of ``typescript/core/src/protocol/receipt.ts``; every SDK runs the
shared vectors in ``test-vectors/protocol-client/execution-receipt-v1.json``.

- :func:`verify_receipt` checks the receipt-key signatures. A receipt's body
  is every field except ``signatures``; each signature is Ed25519 over
  ``arete.protocol-execution-receipt/v1\\n<keyId>\\n<bodySha256>``, where
  ``bodySha256`` is the lowercase hex SHA-256 of the body in RFC 8785 (JCS)
  form. A retired key verifies only receipts issued before ``retiredAt``.
- :func:`verify_receipt_on_chain` checks a transaction fetched from any RPC:
  its first signature and message hash, and the attested Memo v2
  (``arete:p2:<shortId>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>``)
  that the attestation key signs.

Pure Python, stdlib only: the Ed25519 check follows RFC 8032 section 5.1.7.
"""

from __future__ import annotations

import base64
import binascii
import hashlib
import re
from dataclasses import dataclass
from typing import Any, List, Mapping, Optional, Tuple, Union

from arete.encoding import decode_base58, encode_base58
from arete.protocol.metadata_proof import canonicalize_json

PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1 = "arete.protocol-execution-receipt/v1"
PROTOCOL_RECEIPT_KEYS_SCHEMA_V1 = "arete.protocol-receipt-keys/v1"
MEMO_V2_PREFIX = "arete:p2:"
MEMO_PROGRAM_ID = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
_LEGACY_MEMO_PROGRAM_ID = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo"

RECEIPT_ERROR_CODES = (
    "invalid-receipt",
    "unknown-key",
    "key-retired",
    "bad-signature",
    "invalid-transaction",
    "transaction-failed",
    "slot-mismatch",
    "signature-mismatch",
    "message-hash-mismatch",
    "attestation-not-signer",
    "memo-missing",
    "memo-not-unique",
    "memo-invalid",
    "memo-mismatch",
    "attestation-signature-invalid",
)


@dataclass(frozen=True)
class ReceiptVerification:
    """The outcome of a check. ``key_id`` names the key that verified (for
    :func:`verify_receipt`); ``error`` is one of :data:`RECEIPT_ERROR_CODES`."""

    valid: bool
    key_id: Optional[str] = None
    error: Optional[str] = None
    message: Optional[str] = None


def _fail(code: str, message: str) -> ReceiptVerification:
    return ReceiptVerification(valid=False, error=code, message=message)


# ---------------------------------------------------------------------------
# Ed25519 (RFC 8032)
# ---------------------------------------------------------------------------

_P = 2**255 - 19
_L = 2**252 + 27742317777372353535851937790883648493
_D = (-121665 * pow(121666, _P - 2, _P)) % _P
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)

_Point = Tuple[int, int, int, int]


def _add(a: _Point, b: _Point) -> _Point:
    x1, y1, z1, t1 = a
    x2, y2, z2, t2 = b
    pa = (y1 - x1) * (y2 - x2) % _P
    pb = (y1 + x1) * (y2 + x2) % _P
    pc = 2 * t1 * t2 * _D % _P
    pd = 2 * z1 * z2 % _P
    e, f, g, h = pb - pa, pd - pc, pd + pc, pb + pa
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _multiply(scalar: int, point: _Point) -> _Point:
    result: _Point = (0, 1, 1, 0)
    while scalar > 0:
        if scalar & 1:
            result = _add(result, point)
        point = _add(point, point)
        scalar >>= 1
    return result


def _equal(a: _Point, b: _Point) -> bool:
    return (a[0] * b[2] - b[0] * a[2]) % _P == 0 and (a[1] * b[2] - b[1] * a[2]) % _P == 0


def _decompress(data: bytes) -> Optional[_Point]:
    if len(data) != 32:
        return None
    y = int.from_bytes(data, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    if y >= _P:
        return None
    x2 = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P) % _P
    if x2 == 0:
        if sign:
            return None
        x = 0
    else:
        x = pow(x2, (_P + 3) // 8, _P)
        if (x * x - x2) % _P != 0:
            x = x * _SQRT_M1 % _P
        if (x * x - x2) % _P != 0:
            return None
        if (x & 1) != sign:
            x = _P - x
    return (x, y, 1, x * y % _P)


_GY = 4 * pow(5, _P - 2, _P) % _P
_G = _decompress(_GY.to_bytes(32, "little"))


def ed25519_verify(public_key: bytes, message: bytes, signature: bytes) -> bool:
    """Whether ``signature`` is a valid Ed25519 signature of ``message``."""
    if len(public_key) != 32 or len(signature) != 64 or _G is None:
        return False
    a = _decompress(public_key)
    r = _decompress(signature[:32])
    if a is None or r is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= _L:
        return False
    h = int.from_bytes(hashlib.sha512(signature[:32] + public_key + message).digest(), "little") % _L
    return _equal(_multiply(s, _G), _add(r, _multiply(h, a)))


# ---------------------------------------------------------------------------
# Offline check
# ---------------------------------------------------------------------------


def receipt_canonical_body(receipt: Mapping[str, Any]) -> str:
    """The receipt body (``signatures`` removed) in RFC 8785 (JCS) form."""
    if not isinstance(receipt, Mapping):
        raise TypeError("a receipt is a JSON object")
    return canonicalize_json({key: value for key, value in receipt.items() if key != "signatures"})


def receipt_signing_message(key_id: str, body_sha256: str) -> bytes:
    """The exact bytes a receipt key signs."""
    return f"{PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1}\n{key_id}\n{body_sha256}".encode()


def _b64(text: Any) -> Optional[bytes]:
    if not isinstance(text, str):
        return None
    try:
        return base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError):
        return None


def _b58(text: Any) -> Optional[bytes]:
    if not isinstance(text, str):
        return None
    try:
        return decode_base58(text)
    except ValueError:
        return None


def verify_receipt(receipt: Mapping[str, Any], keys: Mapping[str, Any]) -> ReceiptVerification:
    """Verify a receipt's signatures against the published key set
    (``GET /api/protocol-receipts/keys``), offline.

    The receipt is valid when any signature verifies with a key that was not
    retired when the receipt was issued. Pass the receipt exactly as served.
    """
    if not isinstance(receipt, Mapping) or receipt.get("schema") != PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1:
        return _fail("invalid-receipt", "not an arete.protocol-execution-receipt/v1")
    issued_at = receipt.get("issuedAt")
    if not isinstance(issued_at, int) or isinstance(issued_at, bool):
        return _fail("invalid-receipt", "issuedAt must be unix seconds")
    entries = receipt.get("signatures")
    signatures = [
        entry
        for entry in (entries if isinstance(entries, list) else [])
        if isinstance(entry, Mapping)
        and isinstance(entry.get("keyId"), str)
        and isinstance(entry.get("signature"), str)
    ]
    if not signatures:
        return _fail("invalid-receipt", "the receipt carries no signatures")
    try:
        body = receipt_canonical_body(receipt)
    except (TypeError, ValueError) as error:
        return _fail("invalid-receipt", str(error))
    body_sha256 = hashlib.sha256(body.encode()).hexdigest()
    published = keys.get("keys") if isinstance(keys, Mapping) else None
    published = published if isinstance(published, list) else []
    bad = retired = False
    for entry in signatures:
        key = next(
            (k for k in published if isinstance(k, Mapping) and k.get("keyId") == entry["keyId"]),
            None,
        )
        if key is None:
            continue
        retired_at = key.get("retiredAt")
        if isinstance(retired_at, int) and issued_at >= retired_at:
            retired = True
            continue
        signature = _b64(entry["signature"])
        public_key = _b58(key.get("publicKey"))
        message = receipt_signing_message(entry["keyId"], body_sha256)
        if signature is not None and public_key is not None and ed25519_verify(public_key, message, signature):
            return ReceiptVerification(valid=True, key_id=entry["keyId"])
        bad = True
    if bad:
        return _fail("bad-signature", "no receipt signature verifies")
    if retired:
        return _fail("key-retired", "the receipt was signed by a key retired before it was issued")
    return _fail("unknown-key", "no receipt signature names a published key")


# ---------------------------------------------------------------------------
# On-chain check
# ---------------------------------------------------------------------------


class _Failure(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass
class _Instruction:
    program: int
    accounts: List[int]
    data: bytes


@dataclass
class _Wire:
    signatures: List[bytes]
    message: bytes
    required_signers: int
    static_keys: List[bytes]
    instructions: List[_Instruction]


class _Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.at = 0

    def take(self, length: int) -> bytes:
        if self.at + length > len(self.data):
            raise ValueError("truncated transaction")
        out = self.data[self.at : self.at + length]
        self.at += length
        return out

    def byte(self) -> int:
        return self.take(1)[0]

    def compact(self) -> int:
        value = 0
        for index in range(3):
            byte = self.byte()
            value |= (byte & 0x7F) << (7 * index)
            if byte & 0x80 == 0:
                return value
        raise ValueError("invalid compact-u16")


_V1_KNOWN_CONFIG_BITS = 0b11111


def _decode_wire(data: bytes) -> _Wire:
    reader = _Reader(data)
    if data[:1] == b"\x81":
        reader.at = 1
        required = reader.take(3)[0]
        mask = int.from_bytes(reader.take(4), "little")
        if mask & ~_V1_KNOWN_CONFIG_BITS:
            raise ValueError("unsupported V1 configuration")
        reader.take(32)
        instruction_count = reader.byte()
        key_count = reader.byte()
        keys = [reader.take(32) for _ in range(key_count)]
        reader.take(bin(mask).count("1") * 4)
        headers = reader.take(instruction_count * 4)
        instructions = []
        for i in range(instruction_count):
            program, account_count = headers[i * 4], headers[i * 4 + 1]
            data_length = int.from_bytes(headers[i * 4 + 2 : i * 4 + 4], "little")
            accounts = list(reader.take(account_count))
            instructions.append(_Instruction(program, accounts, reader.take(data_length)))
        message = data[: reader.at]
        signatures = [reader.take(64) for _ in range(required)]
        if reader.at != len(data) or required > key_count:
            raise ValueError("malformed V1 transaction")
        return _Wire(signatures, message, required, keys, instructions)
    signature_count = reader.compact()
    signatures = [reader.take(64) for _ in range(signature_count)]
    message_start = reader.at
    if reader.at >= len(data):
        raise ValueError("truncated message")
    versioned = bool(data[reader.at] & 0x80)
    if versioned:
        if data[reader.at] != 0x80:
            raise ValueError("unsupported transaction version")
        reader.at += 1
    required = reader.take(3)[0]
    key_count = reader.compact()
    keys = [reader.take(32) for _ in range(key_count)]
    reader.take(32)
    instructions = []
    for _ in range(reader.compact()):
        program = reader.byte()
        accounts = list(reader.take(reader.compact()))
        instructions.append(_Instruction(program, accounts, reader.take(reader.compact())))
    if versioned:
        for _ in range(reader.compact()):
            reader.take(32)
            reader.take(reader.compact())
            reader.take(reader.compact())
    if reader.at != len(data) or signature_count != required or required > key_count:
        raise ValueError("malformed transaction")
    return _Wire(signatures, data[message_start:], required, keys, instructions)


_UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


def receipt_memo_id(uuid: str) -> Optional[str]:
    """The memo form of a lowercase UUID: base58 of its 16 bytes."""
    if not isinstance(uuid, str) or not _UUID.match(uuid):
        return None
    return encode_base58(bytes.fromhex(uuid.replace("-", "")))


TransactionInput = Union[str, bytes, Mapping[str, Any]]


def _transaction_bytes(transaction: TransactionInput) -> Tuple[bytes, Optional[int]]:
    def from_base64(text: Any) -> bytes:
        data = _b64(text)
        if data is None:
            raise _Failure("invalid-transaction", "the transaction is not base64")
        return data

    if isinstance(transaction, (bytes, bytearray)):
        return bytes(transaction), None
    if isinstance(transaction, str):
        return from_base64(transaction), None
    if not isinstance(transaction, Mapping):
        raise _Failure("invalid-transaction", "expected base64 wire bytes or a getTransaction result")
    meta = transaction.get("meta")
    error = meta.get("err") if isinstance(meta, Mapping) else None
    if error is not None:
        raise _Failure("transaction-failed", f"the transaction failed: {error}")
    slot = transaction.get("slot")
    if slot is not None and (not isinstance(slot, int) or isinstance(slot, bool)):
        raise _Failure("invalid-transaction", "slot must be an integer")
    pair = transaction.get("transaction")
    if not isinstance(pair, list) or len(pair) < 2 or pair[1] != "base64":
        raise _Failure(
            "invalid-transaction",
            'fetch the transaction with encoding "base64" so its message bytes are exact',
        )
    return from_base64(pair[0]), slot


def verify_receipt_on_chain(
    receipt: Mapping[str, Any], transaction: TransactionInput
) -> ReceiptVerification:
    """Check a receipt's claims against the transaction itself.

    ``transaction`` is the base64 wire transaction, its bytes, or the result
    of ``getTransaction`` with ``encoding: "base64"`` from any RPC (its
    ``slot`` and ``meta.err`` are checked too). This does not check the
    receipt's own signature: use :func:`verify_receipt` for that.
    """
    try:
        _check_on_chain(receipt, transaction)
    except _Failure as failure:
        return _fail(failure.code, str(failure))
    return ReceiptVerification(valid=True)


def _check_on_chain(receipt: Mapping[str, Any], transaction: TransactionInput) -> None:
    deployment = receipt.get("deployment") if isinstance(receipt, Mapping) else None
    position = receipt.get("transaction") if isinstance(receipt, Mapping) else None
    if not isinstance(deployment, Mapping) or not isinstance(position, Mapping):
        raise _Failure("invalid-receipt", "not an execution receipt")
    data, slot = _transaction_bytes(transaction)
    if slot is not None and str(slot) != receipt.get("slot"):
        raise _Failure("slot-mismatch", f"the transaction landed in slot {slot}, not {receipt.get('slot')}")
    try:
        tx = _decode_wire(data)
    except (ValueError, IndexError):
        raise _Failure("invalid-transaction", "not a legacy, v0 or V1 wire transaction") from None
    if not tx.signatures or encode_base58(tx.signatures[0]) != receipt.get("signature"):
        raise _Failure("signature-mismatch", "the transaction's signature is not the receipt's")
    if hashlib.sha256(tx.message).hexdigest() != receipt.get("messageSha256"):
        raise _Failure("message-hash-mismatch", "the message hash differs from the receipt's")
    attestation = _b58(receipt.get("attestationPubkey"))
    if attestation is None or len(attestation) != 32:
        raise _Failure("invalid-receipt", "attestationPubkey is not a public key")
    signers = tx.static_keys[: tx.required_signers]
    if attestation not in signers:
        raise _Failure("attestation-not-signer", "the attestation key does not sign the transaction")
    signer_index = signers.index(attestation)
    memo_programs = (decode_base58(MEMO_PROGRAM_ID), decode_base58(_LEGACY_MEMO_PROGRAM_ID))

    def program_of(ix: _Instruction) -> Optional[bytes]:
        return tx.static_keys[ix.program] if ix.program < len(tx.static_keys) else None

    listing = [
        ix
        for ix in tx.instructions
        if program_of(ix) in memo_programs
        and any(a < len(tx.static_keys) and tx.static_keys[a] == attestation for a in ix.accounts)
    ]
    if not listing:
        raise _Failure("memo-missing", "no memo instruction lists the attestation key")
    if len(listing) > 1:
        raise _Failure("memo-not-unique", "more than one memo instruction lists the attestation key")
    memo = listing[0]
    try:
        text: Optional[str] = memo.data.decode("utf-8")
    except UnicodeDecodeError:
        text = None
    parts = (
        text[len(MEMO_V2_PREFIX) :].split(":")
        if text is not None and text.startswith(MEMO_V2_PREFIX)
        else None
    )
    if program_of(memo) != memo_programs[0] or len(memo.accounts) != 1 or parts is None or len(parts) != 6:
        raise _Failure("memo-invalid", "the attestation key's instruction is not an attested Memo v2")
    short_id, _scope, instance, participant, step, memo_position = parts
    participant_id = receipt.get("participantId")
    expected_participant = "-" if participant_id is None else receipt_memo_id(participant_id)
    index, count = position.get("index"), position.get("count")
    matches = (
        short_id == deployment.get("shortId")
        and instance == receipt_memo_id(receipt.get("instanceId"))  # type: ignore[arg-type]
        and participant == expected_participant
        and step == receipt.get("step")
        and isinstance(index, int)
        and memo_position == f"{index + 1}/{count}"
    )
    if not matches:
        raise _Failure(
            "memo-mismatch",
            "the attested memo names a different deployment, instance, participant, step or position",
        )
    if not ed25519_verify(attestation, tx.message, tx.signatures[signer_index]):
        raise _Failure(
            "attestation-signature-invalid",
            "the attestation key's signature does not verify over the message",
        )
