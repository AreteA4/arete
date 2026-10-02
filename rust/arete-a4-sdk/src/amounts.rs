//! Token amount parsing and resolution.
//!
//! Port of `typescript/core/src/amounts.ts` using pure string math (no float
//! precision loss). Raw amounts are `u128` (base units); UI amounts are
//! non-negative decimal strings.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chain::{ChainClient, ChainError};
use crate::error::AreteError;

/// A token amount expressed either in raw base units or UI units.
///
/// Deserializes from exactly the shapes the TypeScript `AmountInput`
/// (`bigint | { ui: string | number } | { raw: bigint | string | number }`)
/// accepts, so an extension input field can be typed `AmountInput` directly:
///
/// - a bare integer: raw base units (TypeScript `bigint`);
/// - `{ "raw": … }`: an integer, an integral number, or a string that
///   JavaScript's `BigInt(…)` reads (decimal with an optional sign, or
///   `0x`/`0o`/`0b`; surrounding whitespace ignored; empty is `0`);
/// - `{ "ui": … }`: a decimal string, or a number, kept as the text
///   JavaScript's `String(number)` gives (`1e-7`, `1e+21`), so
///   [`parse_ui_amount_to_raw`] reports the TypeScript message.
///
/// Objects carry exactly one of `raw` or `ui`. A bare string or non-integer
/// number is not an amount. Raw amounts are unsigned here: a negative raw
/// value fails to deserialize, where TypeScript would accept the bigint.
///
/// Serializes as `{ "raw": "<decimal>" }` or `{ "ui": "<text>" }` (raw as a
/// string so no JSON reader loses precision), which deserializes back to the
/// same value.
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

impl Serialize for AmountInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Raw(raw) => map.serialize_entry("raw", &raw.to_string())?,
            Self::Ui(text) => map.serialize_entry("ui", text)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AmountInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(AmountInputVisitor)
    }
}

const AMOUNT_FIELDS: &[&str] = &["raw", "ui"];

fn negative_raw<E: de::Error>(value: impl fmt::Display) -> E {
    E::custom(format!(
        "raw amount {value} is negative; AmountInput raw amounts are unsigned"
    ))
}

fn raw_from_i128<E: de::Error>(value: i128) -> Result<u128, E> {
    u128::try_from(value).map_err(|_| negative_raw(value))
}

/// JavaScript's `BigInt(number)`: integral finite numbers only.
fn raw_from_f64<E: de::Error>(value: f64) -> Result<u128, E> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(E::custom(format!(
            "The number {} cannot be converted to a BigInt because it is not an integer",
            javascript_number_text(value)
        )));
    }
    if value < 0.0 {
        return Err(negative_raw(javascript_number_text(value)));
    }
    // 2^128 is exactly representable; every smaller integral f64 fits.
    if value >= 340_282_366_920_938_463_463_374_607_431_768_211_456.0 {
        return Err(E::custom(format!(
            "raw amount {} exceeds the u128 range",
            javascript_number_text(value)
        )));
    }
    Ok(value as u128)
}

/// JavaScript's `StrWhiteSpaceChar` (WhiteSpace and LineTerminator).
fn is_javascript_whitespace(character: char) -> bool {
    // Tab, line feed, vertical tab, form feed and carriage return are
    // U+0009..U+000D.
    matches!(character, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}')
        || matches!(character, '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}')
        || matches!(character, '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// JavaScript's `BigInt(string)` (`StringToBigInt`), restricted to the
/// unsigned values a raw amount holds.
fn raw_from_text<E: de::Error>(text: &str) -> Result<u128, E> {
    let invalid = || E::custom(format!("Cannot convert {text} to a BigInt"));
    let trimmed = text.trim_matches(is_javascript_whitespace);
    if trimmed.is_empty() {
        return Ok(0);
    }
    let prefixed = |prefix: [&str; 2]| {
        prefix
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
    };
    let (digits, radix, negative) = if let Some(digits) = prefixed(["0x", "0X"]) {
        (digits, 16, false)
    } else if let Some(digits) = prefixed(["0o", "0O"]) {
        (digits, 8, false)
    } else if let Some(digits) = prefixed(["0b", "0B"]) {
        (digits, 2, false)
    } else if let Some(digits) = trimmed.strip_prefix('-') {
        (digits, 10, true)
    } else {
        (trimmed.strip_prefix('+').unwrap_or(trimmed), 10, false)
    };
    if digits.is_empty() || !digits.chars().all(|digit| digit.is_digit(radix)) {
        return Err(invalid());
    }
    let significant = digits.trim_start_matches('0');
    if negative && !significant.is_empty() {
        return Err(negative_raw(trimmed));
    }
    if significant.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(significant, radix)
        .map_err(|_| E::custom(format!("raw amount {trimmed} exceeds the u128 range")))
}

/// JavaScript's `String(number)`: the shortest round-trip digits, in the
/// exponent form JavaScript uses below 1e-6 and from 1e21.
fn javascript_number_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    if (1e-6..1e21).contains(&value.abs()) {
        return format!("{value}");
    }
    let text = format!("{value:e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => text,
    }
}

struct AmountInputVisitor;

impl<'de> Visitor<'de> for AmountInputVisitor {
    type Value = AmountInput;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a raw integer amount, { raw } or { ui }")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<AmountInput, E> {
        Ok(AmountInput::Raw(u128::from(value)))
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<AmountInput, E> {
        Ok(AmountInput::Raw(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<AmountInput, E> {
        raw_from_i128(i128::from(value)).map(AmountInput::Raw)
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<AmountInput, E> {
        raw_from_i128(value).map(AmountInput::Raw)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<AmountInput, A::Error> {
        let Some(key) = map.next_key::<String>()? else {
            return Err(de::Error::custom("an amount needs `raw` or `ui`"));
        };
        let amount = match key.as_str() {
            "raw" => AmountInput::Raw(map.next_value::<RawAmountValue>()?.0),
            "ui" => AmountInput::Ui(map.next_value::<UiAmountValue>()?.0),
            other => return Err(de::Error::unknown_field(other, AMOUNT_FIELDS)),
        };
        if let Some(extra) = map.next_key::<String>()? {
            return Err(de::Error::custom(format!(
                "an amount has exactly one of `raw` or `ui`, not `{extra}` too"
            )));
        }
        Ok(amount)
    }
}

/// The value of `{ raw }`.
struct RawAmountValue(u128);

impl<'de> Deserialize<'de> for RawAmountValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RawVisitor;

        impl Visitor<'_> for RawVisitor {
            type Value = RawAmountValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a raw amount: an integer, or a string BigInt reads")
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<RawAmountValue, E> {
                Ok(RawAmountValue(u128::from(value)))
            }

            fn visit_u128<E: de::Error>(self, value: u128) -> Result<RawAmountValue, E> {
                Ok(RawAmountValue(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<RawAmountValue, E> {
                raw_from_i128(i128::from(value)).map(RawAmountValue)
            }

            fn visit_i128<E: de::Error>(self, value: i128) -> Result<RawAmountValue, E> {
                raw_from_i128(value).map(RawAmountValue)
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<RawAmountValue, E> {
                raw_from_f64(value).map(RawAmountValue)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<RawAmountValue, E> {
                raw_from_text(value).map(RawAmountValue)
            }
        }

        deserializer.deserialize_any(RawVisitor)
    }
}

/// The value of `{ ui }`.
struct UiAmountValue(String);

impl<'de> Deserialize<'de> for UiAmountValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UiVisitor;

        impl Visitor<'_> for UiVisitor {
            type Value = UiAmountValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a UI amount: a decimal string or a number")
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(value.to_string()))
            }

            fn visit_u128<E: de::Error>(self, value: u128) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(value.to_string()))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(value.to_string()))
            }

            fn visit_i128<E: de::Error>(self, value: i128) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(value.to_string()))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(javascript_number_text(value)))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<UiAmountValue, E> {
                Ok(UiAmountValue(value.to_string()))
            }
        }

        deserializer.deserialize_any(UiVisitor)
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

/// An amount that cannot be resolved is an extension input error
/// ([`AreteError::InvalidInput`], the message alone); a failed chain read
/// converts as [`ChainError`] does.
impl From<AmountError> for AreteError {
    fn from(error: AmountError) -> Self {
        match error {
            AmountError::Chain(error) => error.into(),
            other => AreteError::InvalidInput(other.to_string()),
        }
    }
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
        async fn accounts(
            &self,
            _addresses: &[String],
        ) -> Result<Vec<Option<RawAccountInfo>>, ChainError> {
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

    fn amount(value: serde_json::Value) -> Result<AmountInput, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    fn raw(value: u128) -> Result<AmountInput, String> {
        Ok(AmountInput::Raw(value))
    }

    fn ui(text: &str) -> Result<AmountInput, String> {
        Ok(AmountInput::Ui(text.to_string()))
    }

    #[test]
    fn amount_input_deserializes_a_bare_bigint_as_raw() {
        use serde_json::json;
        assert_eq!(amount(json!(0)), raw(0));
        assert_eq!(amount(json!(5_000_000)), raw(5_000_000));
        assert_eq!(amount(json!(u64::MAX)), raw(u128::from(u64::MAX)));
        // Formats with native 128-bit integers keep the full range.
        use serde::de::value::{Error, I128Deserializer, U128Deserializer};
        assert_eq!(
            AmountInput::deserialize(U128Deserializer::<Error>::new(u128::MAX)).unwrap(),
            AmountInput::Raw(u128::MAX)
        );
        assert_eq!(
            AmountInput::deserialize(I128Deserializer::<Error>::new(i128::MAX)).unwrap(),
            AmountInput::Raw(i128::MAX as u128)
        );
        assert!(AmountInput::deserialize(I128Deserializer::<Error>::new(-1)).is_err());
    }

    #[test]
    fn amount_input_deserializes_raw_objects_as_bigint_reads_them() {
        use serde_json::json;
        assert_eq!(amount(json!({ "raw": 7 })), raw(7));
        assert_eq!(amount(json!({ "raw": "25" })), raw(25));
        // `BigInt(string)`: surrounding whitespace, sign, radix prefixes,
        // leading zeros, and the empty string as zero.
        assert_eq!(amount(json!({ "raw": "  42\n" })), raw(42));
        assert_eq!(amount(json!({ "raw": "\u{feff}42\u{a0}" })), raw(42));
        assert_eq!(amount(json!({ "raw": "+9" })), raw(9));
        assert_eq!(amount(json!({ "raw": "-0" })), raw(0));
        assert_eq!(amount(json!({ "raw": "007" })), raw(7));
        assert_eq!(amount(json!({ "raw": "0x1F" })), raw(31));
        assert_eq!(amount(json!({ "raw": "0O17" })), raw(15));
        assert_eq!(amount(json!({ "raw": "0b101" })), raw(5));
        assert_eq!(amount(json!({ "raw": "" })), raw(0));
        assert_eq!(amount(json!({ "raw": " " })), raw(0));
        assert_eq!(
            amount(json!({ "raw": "340282366920938463463374607431768211455" })),
            raw(u128::MAX)
        );
        // `BigInt(number)`: integral numbers only.
        assert_eq!(amount(json!({ "raw": 5.0 })), raw(5));
        assert_eq!(amount(json!({ "raw": 1e21 })), raw(10u128.pow(21)));
    }

    #[test]
    fn amount_input_rejects_raw_values_bigint_rejects_with_its_message() {
        use serde_json::json;
        for (value, message) in [
            (json!({ "raw": "abc" }), "Cannot convert abc to a BigInt"),
            (json!({ "raw": "1.5" }), "Cannot convert 1.5 to a BigInt"),
            (json!({ "raw": "1e3" }), "Cannot convert 1e3 to a BigInt"),
            (
                json!({ "raw": "1_000" }),
                "Cannot convert 1_000 to a BigInt",
            ),
            (json!({ "raw": "0x" }), "Cannot convert 0x to a BigInt"),
            (json!({ "raw": "-0x1" }), "Cannot convert -0x1 to a BigInt"),
            (json!({ "raw": "0b2" }), "Cannot convert 0b2 to a BigInt"),
            (json!({ "raw": "+" }), "Cannot convert + to a BigInt"),
            (
                json!({ "raw": 1.5 }),
                "The number 1.5 cannot be converted to a BigInt because it is not an integer",
            ),
            (
                json!({ "raw": 1e-7 }),
                "The number 1e-7 cannot be converted to a BigInt because it is not an integer",
            ),
        ] {
            let error = amount(value.clone()).unwrap_err();
            assert!(error.contains(message), "{value}: {error}");
        }
    }

    #[test]
    fn amount_input_rejects_raw_values_outside_u128() {
        use serde_json::json;
        for value in [
            json!(-1),
            json!({ "raw": -1 }),
            json!({ "raw": "-5" }),
            json!({ "raw": -2.0 }),
        ] {
            let error = amount(value.clone()).unwrap_err();
            assert!(error.contains("is negative"), "{value}: {error}");
        }
        for value in [
            json!({ "raw": "340282366920938463463374607431768211456" }),
            json!({ "raw": 1e39 }),
        ] {
            let error = amount(value.clone()).unwrap_err();
            assert!(error.contains("exceeds the u128 range"), "{value}: {error}");
        }
    }

    #[test]
    fn amount_input_deserializes_ui_objects_with_javascript_number_text() {
        use serde_json::json;
        assert_eq!(amount(json!({ "ui": "1.5" })), ui("1.5"));
        assert_eq!(amount(json!({ "ui": " 0.25 " })), ui(" 0.25 "));
        assert_eq!(amount(json!({ "ui": "one" })), ui("one"));
        assert_eq!(amount(json!({ "ui": 2 })), ui("2"));
        assert_eq!(amount(json!({ "ui": -3 })), ui("-3"));
        assert_eq!(amount(json!({ "ui": 1.5 })), ui("1.5"));
        assert_eq!(amount(json!({ "ui": 0.000001 })), ui("0.000001"));
        assert_eq!(amount(json!({ "ui": 1e-7 })), ui("1e-7"));
        assert_eq!(amount(json!({ "ui": 1.5e-7 })), ui("1.5e-7"));
        assert_eq!(amount(json!({ "ui": 1e16 })), ui("10000000000000000"));
        assert_eq!(amount(json!({ "ui": 1e21 })), ui("1e+21"));
        assert_eq!(amount(json!({ "ui": 1.2345e25 })), ui("1.2345e+25"));

        // The text then fails exactly as TypeScript's parse does.
        let AmountInput::Ui(text) = amount(json!({ "ui": 1e-7 })).unwrap() else {
            unreachable!()
        };
        assert_eq!(
            parse_ui_amount_to_raw(&text, 9).unwrap_err().to_string(),
            "Invalid UI amount: 1e-7"
        );
    }

    #[test]
    fn amount_input_rejects_shapes_typescript_does_not_accept() {
        use serde_json::json;
        for value in [
            json!("5"),
            json!(1.5),
            json!(true),
            json!(null),
            json!([1]),
            json!({}),
            json!({ "amount": 1 }),
            json!({ "raw": 1, "ui": "1" }),
            json!({ "ui": "1", "decimals": 9 }),
            json!({ "raw": null }),
            json!({ "raw": true }),
            json!({ "raw": [1] }),
            json!({ "ui": null }),
            json!({ "ui": false }),
            json!({ "ui": { "value": "1" } }),
        ] {
            assert!(amount(value.clone()).is_err(), "{value} should not decode");
        }
    }

    #[test]
    fn amount_input_serializes_to_a_shape_it_deserializes() {
        use serde_json::json;
        for (input, wire) in [
            (AmountInput::Raw(42), json!({ "raw": "42" })),
            (
                AmountInput::Raw(u128::MAX),
                json!({ "raw": "340282366920938463463374607431768211455" }),
            ),
            (AmountInput::Ui("1.5".to_string()), json!({ "ui": "1.5" })),
        ] {
            assert_eq!(serde_json::to_value(&input).unwrap(), wire);
            assert_eq!(amount(wire).unwrap(), input);
        }
    }

    #[test]
    fn amount_input_works_as_a_typed_input_field() {
        #[derive(Debug, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DeployInput {
            amount_per_square: AmountInput,
            #[serde(default)]
            deposit: Option<AmountInput>,
        }

        let input: DeployInput = serde_json::from_value(serde_json::json!({
            "amountPerSquare": { "ui": "0.005" },
            "deposit": 1_000,
        }))
        .unwrap();
        assert_eq!(
            input.amount_per_square,
            AmountInput::Ui("0.005".to_string())
        );
        assert_eq!(input.deposit, Some(AmountInput::Raw(1_000)));
        assert_eq!(
            to_raw_amount(&input.amount_per_square, 9).unwrap(),
            5_000_000
        );
    }

    #[test]
    fn amount_errors_convert_to_invalid_input() {
        let error: AreteError = to_raw_amount(&AmountInput::from("one"), 9)
            .unwrap_err()
            .into();
        assert!(matches!(error, AreteError::InvalidInput(_)));
        assert_eq!(error.to_string(), "Invalid UI amount: one");

        let error: AreteError = AmountError::Chain(ChainError::InvalidResponse {
            path: "/chain/mints/A".to_string(),
            message: "bad".to_string(),
        })
        .into();
        assert!(matches!(error, AreteError::Serialization(_)));
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
