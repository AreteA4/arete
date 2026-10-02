"""Vault program package extension (Python)."""

from arete import create_prepared_instruction, instruction_operation

from . import vault_math
from .models import Vault, vault_from_wire
from .programs import VAULT_PROGRAM_ID

TREASURY = "Treasury11111111111111111111111111111111111"


def create_operations(ctx):
    async def deposit(*, mint, amount, authority=None):
        authority = authority or ctx.wallet.public_key
        instruction = ctx.program.raw.deposit.build(
            authority=authority, vault=TREASURY, mint=mint, amount=amount
        )
        return create_prepared_instruction(
            name="treasury.deposit",
            instruction=instruction,
            artifacts={"authority": authority},
        )

    return {"instructions": {"treasury": {"deposit": instruction_operation(deposit)}}}


def vault_balance(payload) -> int:
    """The raw balance of a wire `Vault` account payload."""
    vault: Vault = vault_from_wire(payload)
    return vault.balance


def create_read(ctx):
    async def slot():
        return (await ctx.chain.clock()).slot

    async def vault(address):
        return await ctx.program.accounts.vault.fetch(address)

    async def balance(address):
        account = await vault(address)
        return None if account is None else account.balance

    return {"slot": slot, "vault": vault, "balance": balance}


PROGRAM_EXTENSIONS = {
    "addresses": {"treasury": lambda: TREASURY, "program": lambda: VAULT_PROGRAM_ID},
    "constants": {"treasury": TREASURY, "vault_decimals": vault_math.VAULT_DECIMALS},
    "math": {"to_raw": vault_math.to_raw, "vault_balance": vault_balance},
    "create_operations": create_operations,
    "create_read": create_read,
}
