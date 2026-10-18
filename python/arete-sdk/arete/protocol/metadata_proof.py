"""RFC 8785 JSON canonicalization and protocol metadata wallet proofs.

Port of ``typescript/core/src/protocol/metadata-proof.ts``. A participant
wallet authorises a metadata patch by signing a fixed six-line message; the
hosted API rebuilds the same message from the request and checks the
signature. Signing itself is the caller's: pass the message bytes to the
wallet and hand the 64-byte Ed25519 signature to
:func:`protocol_metadata_proof`.
"""

from __future__ import annotations

import hashlib
import json
import math
import time
from dataclasses import dataclass
from typing import Any, Mapping, Optional

from arete.encoding import encode_base58

PROTOCOL_METADATA_PROOF_DOMAIN = "arete.protocol-metadata/v1"
"""First line of a metadata wallet proof message."""

PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS = 300
"""How far ahead a metadata proof may expire, in seconds."""

DEFAULT_PROOF_TTL_SECONDS = 240
"""The expiry :class:`arete.protocol.ProtocolClient` uses when none is given."""

_MAX_SAFE_INTEGER = 2**53 - 1


# ---------------------------------------------------------------------------
# RFC 8785 (JCS)
# ---------------------------------------------------------------------------


def canonicalize_json(value: Any) -> str:
    """Serialize a JSON value in the RFC 8785 JSON Canonicalization Scheme.

    Object members are sorted by the UTF-16 code units of their names, there
    is no whitespace, numbers take their ECMAScript ``JSON.stringify`` form and
    strings use the minimal JSON escapes.

    Raises :class:`TypeError` for anything that is not JSON: non-finite
    numbers, integers beyond the IEEE-754 safe range (send them as decimal
    strings), non-string object keys, other Python objects, and strings with
    lone surrogates.
    """
    return _canonicalize(value, "$")


def canonical_json_sha256(value: Any) -> str:
    """Lowercase hex SHA-256 of the JCS form of a JSON value."""
    return hashlib.sha256(canonicalize_json(value).encode("utf-8")).hexdigest()


def _has_lone_surrogate(value: str) -> bool:
    # Python strings hold code points; a surrogate code point is always lone.
    return any(0xD800 <= ord(character) <= 0xDFFF for character in value)


def _canonical_string(value: str, path: str) -> str:
    if _has_lone_surrogate(value):
        raise TypeError(f"{path} contains a lone surrogate and is not valid I-JSON")
    return json.dumps(value, ensure_ascii=False)


def _utf16_key(name: str) -> bytes:
    return name.encode("utf-16-be")


def _es_number(value: float, path: str) -> str:
    """ECMAScript Number::toString for a finite double."""
    if not math.isfinite(value):
        raise TypeError(f"{path} is not a finite number")
    if value == 0:
        return "0"
    sign = "-" if value < 0 else ""
    text = repr(abs(value))  # shortest round-trip digits
    mantissa, _, exponent_text = text.partition("e")
    exponent = int(exponent_text) if exponent_text else 0
    integer, _, fraction = mantissa.partition(".")
    digits = (integer + fraction).lstrip("0")
    scale = exponent - len(fraction)
    stripped = digits.rstrip("0")
    scale += len(digits) - len(stripped)
    digits = stripped
    k = len(digits)
    n = k + scale
    if k <= n <= 21:
        body = digits + "0" * (n - k)
    elif 0 < n <= 21:
        body = f"{digits[:n]}.{digits[n:]}"
    elif -6 < n <= 0:
        body = "0." + "0" * (-n) + digits
    else:
        e = n - 1
        exponent_part = f"e{'+' if e >= 0 else '-'}{abs(e)}"
        body = digits + exponent_part if k == 1 else f"{digits[0]}.{digits[1:]}{exponent_part}"
    return sign + body


def _canonicalize(value: Any, path: str) -> str:
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, int):
        if abs(value) > _MAX_SAFE_INTEGER:
            raise TypeError(
                f"{path} is an integer beyond 2^53 - 1; convert it to a decimal string first"
            )
        return str(int(value))
    if isinstance(value, float):
        return _es_number(value, path)
    if isinstance(value, str):
        return _canonical_string(value, path)
    if isinstance(value, (list, tuple)):
        items = [_canonicalize(item, f"{path}[{index}]") for index, item in enumerate(value)]
        return "[" + ",".join(items) + "]"
    if isinstance(value, Mapping):
        members = []
        for key in sorted(value.keys(), key=lambda key: _utf16_key(key) if isinstance(key, str) else b""):
            if not isinstance(key, str):
                raise TypeError(f"{path} has a non-string key {key!r}")
            members.append(
                f"{_canonical_string(key, path)}:{_canonicalize(value[key], f'{path}.{key}')}"
            )
        return "{" + ",".join(members) + "}"
    raise TypeError(f"{path} is not a JSON value ({type(value).__name__})")


# ---------------------------------------------------------------------------
# Metadata proofs
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProtocolMetadataProofMessage:
    """The message a wallet signs to authorise a metadata patch."""

    message: str
    """The UTF-8 message, lines joined by ``\\n``."""
    bytes: bytes
    """``message`` as bytes, for the wallet's ``sign_message``."""
    patch_sha256: str
    """Lowercase hex SHA-256 of the patch's JCS form."""
    expires_at: int


def protocol_metadata_proof_message(
    *,
    deployment_id: str,
    instance_id: str,
    wallet: str,
    patch: Mapping[str, Any],
    expires_at: int,
    now: Optional[int] = None,
) -> ProtocolMetadataProofMessage:
    """Build the message a wallet signs to authorise a metadata patch::

        arete.protocol-metadata/v1
        <deployment id>
        <instance id>
        <wallet>
        <lowercase hex SHA-256 of the patch's RFC 8785 canonical JSON>
        <expires_at, unix seconds>

    ``expires_at`` must be in the future and at most 300 seconds ahead of
    ``now`` (the current time by default).
    """
    for name, value in (
        ("deployment_id", deployment_id),
        ("instance_id", instance_id),
        ("wallet", wallet),
    ):
        if not isinstance(value, str) or not value or "\n" in value or "\r" in value:
            raise TypeError(f"{name} must be a non-empty single-line string")
    if (
        isinstance(expires_at, bool)
        or not isinstance(expires_at, int)
        or expires_at <= 0
        or expires_at > _MAX_SAFE_INTEGER
    ):
        raise TypeError("expires_at must be unix seconds")
    current = int(time.time()) if now is None else now
    if expires_at <= current or expires_at - current > PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS:
        raise ValueError(
            "expires_at must be in the future and at most "
            f"{PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS} seconds ahead"
        )
    if not isinstance(patch, Mapping):
        raise TypeError("patch must be a JSON object")
    patch_sha256 = canonical_json_sha256(patch)
    message = "\n".join(
        [
            PROTOCOL_METADATA_PROOF_DOMAIN,
            deployment_id,
            instance_id,
            wallet,
            patch_sha256,
            str(expires_at),
        ]
    )
    return ProtocolMetadataProofMessage(
        message=message,
        bytes=message.encode("utf-8"),
        patch_sha256=patch_sha256,
        expires_at=expires_at,
    )


def protocol_metadata_proof(
    *, wallet: str, signature: bytes, expires_at: int
) -> dict:
    """Turn a wallet's 64-byte Ed25519 signature into the metadata update
    ``proof`` (``{"wallet", "signature" (base58), "expiresAt"}``)."""
    if not isinstance(signature, (bytes, bytearray)) or len(signature) != 64:
        raise TypeError("signature must be the 64-byte Ed25519 signature")
    return {
        "wallet": wallet,
        "signature": encode_base58(bytes(signature)),
        "expiresAt": expires_at,
    }
