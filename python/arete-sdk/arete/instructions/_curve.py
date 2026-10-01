"""Ed25519 on-curve check (and base58, re-exported from ``arete.encoding``).

Pure Python, stdlib only, so PDA derivation works offline with no native or
third-party dependencies. The on-curve check mirrors ``Point.fromHex`` in
``@noble/ed25519`` (the TypeScript SDK's reference): a 32-byte value is on the
curve iff its y coordinate is canonical (< p) and x² = (y² - 1) / (d·y² + 1)
has a square root in GF(p).
"""

from __future__ import annotations

# Base58 lives in ``arete.encoding``; re-exported here for the instruction
# modules that import it from this module.
from ..encoding import decode_base58, encode_base58  # noqa: F401


# Ed25519 field parameters (RFC 8032 section 5.1).
_P = 2**255 - 19
_D = (-121665 * pow(121666, _P - 2, _P)) % _P


def is_on_curve(point: bytes) -> bool:
    """Whether a 32-byte value is a valid compressed ed25519 point.

    A valid PDA must be OFF the curve (no private key may ever sign for it),
    so ``find_program_address`` rejects candidates for which this is true.
    Determined by attempting decompression per RFC 8032 field math.
    """
    if len(point) != 32:
        return False
    y = int.from_bytes(point, "little") & ((1 << 255) - 1)
    if y >= _P:
        return False
    y2 = y * y % _P
    u = (y2 - 1) % _P
    v = (_D * y2 + 1) % _P
    # Candidate square root of u/v: x = u·v³ · (u·v⁷)^((p-5)/8).
    x = u * pow(v, 3, _P) % _P * pow(u * pow(v, 7, _P) % _P, (_P - 5) // 8, _P) % _P
    vx2 = v * x % _P * x % _P
    # A root exists iff v·x² ≡ ±u (the -u case is fixed up by √-1).
    return vx2 == u or vx2 == (-u) % _P
