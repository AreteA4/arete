//! Token amount parsing and resolution.
//!
//! Port of `typescript/core/src/amounts.ts` using pure string math (no float
//! precision loss). Raw amounts are `u128` (base units); UI amounts are
//! non-negative decimal strings.

use thiserror::Error;

use crate::chain::{ChainClient, ChainError};

/// A token amount expressed either in raw base units or UI units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmountInput {
    /// Raw base units; never requires the mint's decimals to resolve.
    Raw(u128),
    /// Human decimal string (e.g. `"1.5"`); scaled by the mint's decimals.
    Ui(String),
}

impl From<u64> for AmountInput {
    fn from(value: u64) -> Self {
        Self::Raw(value as u128)
    }
}

impl From<u128> for AmountInput {
    fn from(value: u128) -> Self {
        Self::Raw(value)
    }
}

impl From<&str> for AmountInput {
    fn from(value: &str) -> Self {
        Self::Ui(value.to_string())
    }
}

impl From<String> for AmountInput {
    fn from(value: String) -> Self {
        Self::Ui(value)
    }
}

/// Errors produced by amount parsing and resolution.
#[derive(Debug, Error)]
pub enum AmountError {
    /// The UI amount is not a non-negative decimal number.
    #[error("Invalid UI amount: {0}")]
    InvalidUiAmount(String),

    /// The UI amount has non-zero digits below the mint's precision.
    #[error("UI amount {value} has more fractional digits than the mint's {decimals} decimals")]
    ExcessFractionalDigits { value: String, decimals: u8 },

    /// The scaled amount does not fit in `u128`.
    #[error("UI amount {0} exceeds the supported range")]
    Overflow(String),

    /// The mint exists but reports no decimals on the read endpoint.
    #[error("Mint {0} is missing decimals on the configured read endpoint.")]
    MissingDecimals(String),

    /// The chain read failed.
    #[error(transparent)]
    Chain(#[from] ChainError),
}

/// Input for [`resolve_amount`] / [`resolve_amount_to_raw`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmountResolutionInput {
    pub mint: String,
    pub amount: AmountInput,
    /// Known decimals; when present the chain is never consulted.
    pub decimals: Option<u8>,
}

/// A resolved amount: raw base units plus the decimals used to scale it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAmount {
    pub raw: u128,
    pub decimals: u8,
}

/// Converts a UI amount (`"1.5"`) to raw base units using string math.
///
/// Trailing zero fraction digits beyond the mint's decimals are accepted;
/// non-zero excess digits are rejected. Negative and malformed inputs error.
pub fn parse_ui_amount_to_raw(value: &str, decimals: u8) -> Result<u128, AmountError> {
    let trimmed = value.trim();

    // Mirror of the TS validation: /^\d+(?:\.\d+)?$/
    let (whole_part, fraction_part) = match trimmed.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (trimmed, ""),
    };
    let valid = !whole_part.is_empty()
        && whole_part.bytes().all(|byte| byte.is_ascii_digit())
        && (!trimmed.contains('.')
            || (!fraction_part.is_empty()
                && fraction_part.bytes().all(|byte| byte.is_ascii_digit())))
        && trimmed.bytes().filter(|byte| *byte == b'.').count() <= 1;
    if !valid {
        return Err(AmountError::InvalidUiAmount(value.to_string()));
    }

    let decimals_usize = decimals as usize;
    if fraction_part.len() > decimals_usize {
        let excess = &fraction_part[decimals_usize..];
        if excess.bytes().any(|byte| (b'1'..=b'9').contains(&byte)) {
            return Err(AmountError::ExcessFractionalDigits {
                value: value.to_string(),
                decimals,
            });
        }
    }

    // Concatenate whole + fraction padded/truncated to `decimals` digits and
    // parse the resulting integer: whole * 10^decimals + fraction.
    let mut digits = String::with_capacity(whole_part.len() + decimals_usize);
    digits.push_str(whole_part);
    if fraction_part.len() >= decimals_usize {
        digits.push_str(&fraction_part[..decimals_usize]);
    } else {
        digits.push_str(fraction_part);
        digits.extend(std::iter::repeat_n(
            '0',
            decimals_usize - fraction_part.len(),
        ));
    }

    let normalized = digits.trim_start_matches('0');
    if normalized.is_empty() {
        return Ok(0);
    }
    normalized
        .parse::<u128>()
        .map_err(|_| AmountError::Overflow(value.to_string()))
}

/// Formats raw base units as a UI decimal string (inverse of
/// [`parse_ui_amount_to_raw`]); trailing fraction zeros are trimmed.
pub fn format_raw_to_ui(raw: u128, decimals: u8) -> String {
    let digits = raw.to_string();
    let decimals = decimals as usize;
    if decimals == 0 {
        return digits;
    }

    let (whole, fraction) = if digits.len() > decimals {
        let split = digits.len() - decimals;
        (digits[..split].to_string(), digits[split..].to_string())
    } else {
        (
            "0".to_string(),
            format!("{digits:0>decimals$}", decimals = decimals),
        )
    };

    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        whole
    } else {
        format!("{whole}.{fraction}")
    }
}

/// Resolves an [`AmountInput`] to raw base units with known decimals.
pub fn to_raw_amount(amount: &AmountInput, decimals: u8) -> Result<u128, AmountError> {
    match amount {
        AmountInput::Raw(raw) => Ok(*raw),
        AmountInput::Ui(value) => parse_ui_amount_to_raw(value, decimals),
    }
}

/// Fetches a mint's decimals via the chain read endpoint, erroring when the
/// mint is missing or reports no decimals.
pub async fn get_mint_decimals(chain: &dyn ChainClient, mint: &str) -> Result<u8, AmountError> {
    let account = chain.mint(mint).await?;
    account
        .and_then(|account| account.decimals)
        .ok_or_else(|| AmountError::MissingDecimals(mint.to_string()))
}

/// Resolves an [`AmountInput`] to raw base units plus decimals, fetching the
/// mint's decimals only when unknown (explicit `decimals` never touch the
/// network).
pub async fn resolve_amount(
    chain: &dyn ChainClient,
    input: &AmountResolutionInput,
) -> Result<ResolvedAmount, AmountError> {
    let decimals = match input.decimals {
        Some(decimals) => decimals,
        None => get_mint_decimals(chain, &input.mint).await?,
    };
    Ok(ResolvedAmount {
        raw: to_raw_amount(&input.amount, decimals)?,
        decimals,
    })
}

/// Resolves an [`AmountInput`] to raw base units without forcing a decimals
/// fetch when the input is already raw.
pub async fn resolve_amount_to_raw(
    chain: &dyn ChainClient,
    input: &AmountResolutionInput,
) -> Result<u128, AmountError> {
    if let AmountInput::Raw(raw) = input.amount {
        return Ok(raw);
    }
    let decimals = match input.decimals {
        Some(decimals) => decimals,
        None => get_mint_decimals(chain, &input.mint).await?,
    };
    to_raw_amount(&input.amount, decimals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{
        ChainClock, ContextSlotOptions, MintAccountInfo, NativeBalanceInfo, RawAccountInfo,
        TokenAccountInfo, TokenBalanceInfo, TokenBalanceInput,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeChain {
        decimals: Option<u8>,
        mint_calls: AtomicUsize,
    }

    impl FakeChain {
        fn new(decimals: Option<u8>) -> Self {
            Self {
                decimals,
                mint_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChainClient for FakeChain {
        async fn exists(&self, _address: &str) -> Result<bool, ChainError> {
            unimplemented!()
        }
        async fn lamports(&self, _address: &str) -> Result<u64, ChainError> {
            unimplemented!()
        }
        async fn native_balance(
            &self,
            _address: &str,
            _options: ContextSlotOptions,
        ) -> Result<NativeBalanceInfo, ChainError> {
            unimplemented!()
        }
        async fn minimum_balance_for_rent_exemption(&self, _space: u64) -> Result<u64, ChainError> {
            unimplemented!()
        }
        async fn clock(&self) -> Result<ChainClock, ChainError> {
            unimplemented!()
        }
        async fn account(&self, _address: &str) -> Result<Option<RawAccountInfo>, ChainError> {
            unimplemented!()
        }
        async fn mint(&self, address: &str) -> Result<Option<MintAccountInfo>, ChainError> {
            self.mint_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(MintAccountInfo {
                address: address.to_string(),
                owner_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                decimals: self.decimals,
                supply: None,
                mint_authority: None,
                freeze_authority: None,
            }))
        }
        async fn token_account(
            &self,
            _address: &str,
        ) -> Result<Option<TokenAccountInfo>, ChainError> {
            unimplemented!()
        }
        async fn balance(
            &self,
            _input: &TokenBalanceInput,
            _options: ContextSlotOptions,
        ) -> Result<TokenBalanceInfo, ChainError> {
            unimplemented!()
        }
    }

    #[test]
    fn parses_decimal_strings_without_float_math() {
        assert_eq!(parse_ui_amount_to_raw("1.5", 6).unwrap(), 1_500_000);
        assert_eq!(parse_ui_amount_to_raw("0.000001", 6).unwrap(), 1);
        assert_eq!(parse_ui_amount_to_raw("100", 6).unwrap(), 100_000_000);
        assert_eq!(parse_ui_amount_to_raw("0", 6).unwrap(), 0);
        assert_eq!(
            parse_ui_amount_to_raw("12345678901234567890", 0).unwrap(),
            12_345_678_901_234_567_890
        );
    }

    #[test]
    fn accepts_trailing_zero_fraction_digits_beyond_mint_decimals() {
        assert_eq!(parse_ui_amount_to_raw("1.120000000", 6).unwrap(), 1_120_000);
    }

    #[test]
    fn rejects_malformed_and_negative_inputs() {
        for input in ["1.2.3", "abc", "-1", "", "1.", ".5", "1e5", "1 5"] {
            assert!(
                matches!(
                    parse_ui_amount_to_raw(input, 6),
                    Err(AmountError::InvalidUiAmount(_))
                ),
                "expected invalid: {input:?}"
            );
        }
    }

    #[test]
    fn rejects_non_zero_fraction_digits_below_mint_precision() {
        assert!(matches!(
            parse_ui_amount_to_raw("1.1234567", 6),
            Err(AmountError::ExcessFractionalDigits { decimals: 6, .. })
        ));
    }

    #[test]
    fn rejects_amounts_that_overflow_u128() {
        // u128::MAX + 1
        assert!(matches!(
            parse_ui_amount_to_raw("340282366920938463463374607431768211456", 0),
            Err(AmountError::Overflow(_))
        ));
        assert_eq!(
            parse_ui_amount_to_raw("340282366920938463463374607431768211455", 0).unwrap(),
            u128::MAX
        );
    }

    #[test]
    fn formats_raw_to_ui_as_the_inverse() {
        assert_eq!(format_raw_to_ui(1_500_000, 6), "1.5");
        assert_eq!(format_raw_to_ui(1, 6), "0.000001");
        assert_eq!(format_raw_to_ui(0, 6), "0");
        assert_eq!(format_raw_to_ui(100_000_000, 6), "100");
        assert_eq!(format_raw_to_ui(2_500_000, 6), "2.5");
        assert_eq!(format_raw_to_ui(5, 0), "5");
    }

    #[test]
    fn to_raw_amount_passes_raw_and_scales_ui() {
        assert_eq!(to_raw_amount(&AmountInput::Raw(42), 6).unwrap(), 42);
        assert_eq!(to_raw_amount(&AmountInput::from(25u64), 6).unwrap(), 25);
        assert_eq!(
            to_raw_amount(&AmountInput::from("2"), 6).unwrap(),
            2_000_000
        );
        assert_eq!(
            to_raw_amount(&AmountInput::from("0.25".to_string()), 8).unwrap(),
            25_000_000
        );
    }

    #[tokio::test]
    async fn get_mint_decimals_reads_from_chain() {
        let chain = FakeChain::new(Some(9));
        assert_eq!(get_mint_decimals(&chain, "MintA").await.unwrap(), 9);

        let chain = FakeChain::new(None);
        assert!(matches!(
            get_mint_decimals(&chain, "MintA").await,
            Err(AmountError::MissingDecimals(mint)) if mint == "MintA"
        ));
    }

    #[tokio::test]
    async fn resolve_amount_never_fetches_when_decimals_are_provided() {
        let chain = FakeChain::new(Some(6));
        let resolved = resolve_amount(
            &chain,
            &AmountResolutionInput {
                mint: "MintA".to_string(),
                amount: AmountInput::Ui("1.5".to_string()),
                decimals: Some(6),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            resolved,
            ResolvedAmount {
                raw: 1_500_000,
                decimals: 6,
            }
        );
        assert_eq!(chain.mint_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn resolve_amount_fetches_decimals_for_ui_and_unknown_raw() {
        let chain = FakeChain::new(Some(6));
        let resolved = resolve_amount(
            &chain,
            &AmountResolutionInput {
                mint: "MintA".to_string(),
                amount: AmountInput::Ui("1.5".to_string()),
                decimals: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(resolved.raw, 1_500_000);
        assert_eq!(chain.mint_calls.load(Ordering::SeqCst), 1);

        // Raw inputs with unknown decimals still resolve decimals (TS parity).
        let resolved = resolve_amount(
            &chain,
            &AmountResolutionInput {
                mint: "MintA".to_string(),
                amount: AmountInput::Raw(77),
                decimals: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            resolved,
            ResolvedAmount {
                raw: 77,
                decimals: 6,
            }
        );
        assert_eq!(chain.mint_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn resolve_amount_to_raw_never_fetches_for_raw_inputs() {
        let chain = FakeChain::new(None); // would error if consulted
        let raw = resolve_amount_to_raw(
            &chain,
            &AmountResolutionInput {
                mint: "MintA".to_string(),
                amount: AmountInput::Raw(42),
                decimals: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(raw, 42);
        assert_eq!(chain.mint_calls.load(Ordering::SeqCst), 0);

        let chain = FakeChain::new(Some(6));
        let raw = resolve_amount_to_raw(
            &chain,
            &AmountResolutionInput {
                mint: "MintA".to_string(),
                amount: AmountInput::Ui("1.5".to_string()),
                decimals: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(raw, 1_500_000);
        assert_eq!(chain.mint_calls.load(Ordering::SeqCst), 1);
    }
}
