//! Pure vault helpers.

/// Decimals of the vault's amounts.
pub const VAULT_DECIMALS: u8 = 6;

/// A whole-unit amount in raw units.
pub fn to_raw(ui: u64) -> u64 {
    ui * 10u64.pow(VAULT_DECIMALS as u32)
}
