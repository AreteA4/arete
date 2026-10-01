"""SPL token helpers (port of ``typescript/core/src/spl.ts``).

Program-address constants, associated token account derivation, and token
program resolution. PDA derivation is delegated to
``arete.instructions.pda.find_program_address`` (imported lazily — the
instruction runtime is an independent module).
"""

from __future__ import annotations

from typing import Optional

from arete.chain import ChainClient
from arete.encoding import decode_base58

SPL_TOKEN_PROGRAM_ADDRESS = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
TOKEN_2022_PROGRAM_ADDRESS = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
ASSOCIATED_TOKEN_PROGRAM_ADDRESS = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
SYSTEM_PROGRAM_ADDRESS = "11111111111111111111111111111111"

def _public_key_seed(address: str) -> bytes:
    """Decode a base58 address into its 32 seed bytes (TS
    ``createPublicKeySeed``), raising ``ValueError`` with the TypeScript
    messages: ``Invalid base58 character: <c>`` and
    ``Invalid public key length: expected 32, got <n>``."""
    decoded = decode_base58(address)
    if len(decoded) != 32:
        raise ValueError(f"Invalid public key length: expected 32, got {len(decoded)}")
    return decoded


async def resolve_token_program_address(
    chain: ChainClient, mint: str, override: Optional[str] = None
) -> str:
    """Resolve the token program owning ``mint`` (explicit override wins and
    skips the chain read)."""
    if override:
        return override
    mint_account = await chain.mint(mint)
    if mint_account is None:
        raise ValueError(f"Mint account not found while resolving token program: {mint}")
    if mint_account.owner_program not in (
        SPL_TOKEN_PROGRAM_ADDRESS,
        TOKEN_2022_PROGRAM_ADDRESS,
    ):
        raise ValueError(
            f"Mint {mint} is owned by unsupported token program "
            f"{mint_account.owner_program}"
        )
    return mint_account.owner_program


def derive_associated_token_account(
    *, owner: str, mint: str, token_program: Optional[str] = None
) -> str:
    """Derive the associated token account address for ``owner`` + ``mint``.

    ``token_program`` defaults to the SPL Token program when it is ``None``
    (TypeScript ``??``); any other value, the empty string included, is a
    seed. Each address must be a base58 32-byte public key: the error is a
    ``ValueError`` with the TypeScript message, checked in the order owner,
    token program, mint.
    """
    from arete.instructions.pda import find_program_address

    address, _bump = find_program_address(
        [
            _public_key_seed(owner),
            _public_key_seed(
                SPL_TOKEN_PROGRAM_ADDRESS if token_program is None else token_program
            ),
            _public_key_seed(mint),
        ],
        ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
    )
    return address
