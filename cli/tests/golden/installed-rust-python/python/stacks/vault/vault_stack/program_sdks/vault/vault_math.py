"""Pure vault helpers."""

VAULT_DECIMALS = 6


def to_raw(ui: int) -> int:
    return ui * 10**VAULT_DECIMALS
