//! JavaScript `String.prototype.localeCompare` equivalent (ICU default collation).
//!
//! Mirror of `python/arete-sdk/arete/subscription.py` (`collation_key` /
//! `locale_compare`) and of the TypeScript call sites it replicates
//! (`typescript/core/src/subscription.ts` lines 57 and 123,
//! `typescript/core/src/query-store.ts` lines 64 and 387).
//!
//! The canonical ordering rule (`docs/internal/sdk-core-api.md` §2) requires
//! that wherever the SDK surface sorts strings — canonical subscription
//! identity (filter keys) and store ordering including the key tie-break — the
//! order matches JS `localeCompare`, **not** code-point/byte order. Sorting by
//! `str`'s natural [`Ord`] would emit a different `filters` key order (and
//! therefore a different canonical identity) whenever keys differ only by case
//! or carry non-ASCII letters, and would order list results differently from
//! TypeScript for the mixed-case base58 keys that Solana addresses produce
//! constantly.
//!
//! [`collation_key`] builds a three-level UCA-style sort key (primary = base
//! letter with case and diacritics removed, secondary = diacritics, tertiary =
//! case with lowercase first). Sorting by that key is equivalent to sorting
//! with [`locale_compare`].
//!
//! # Replicated exactly (verified against Node v23 `localeCompare`)
//!
//! * All printable ASCII, in ICU order — whitespace and punctuation before
//!   digits before letters, with the exact ICU punctuation sequence
//!   (`_ - , ; : ! ? . ' " ( ) [ ] { } @ * / \ & # % ` ^ + < = > | ~ $`). This
//!   covers base58 keys and dotted filter paths, the only inputs the SDK
//!   produces in practice.
//! * Case as a tertiary difference with lowercase first (`a` < `A`,
//!   `test` < `Test`, `aBc1` < `apple` < `Bqq` < `Zap1`).
//! * Canonically decomposable accented Latin as a secondary difference, in
//!   ICU's diacritic order (`a` < `á` < `à` < `ă` < `â` < `ǎ` < `å` < `ä` <
//!   `a̋` < `ã` < `ȧ` …), so `etat` < `état` and `resume` < `résumé`.
//! * Level-by-level comparison over the whole string (a secondary difference
//!   anywhere loses to a primary difference anywhere).
//! * Canonical equivalence: NFC and NFD spellings compare equal.
//! * Completely ignorable characters (`Cc`/`Cf`: NUL, soft hyphen, ZWSP)
//!   contribute nothing, so `"a\u{200b}b" == "ab"`.
//! * Non-Latin *letters* fold by case, so Greek/Cyrillic order alphabetically
//!   (`α` < `Ω`) rather than by code point.
//! * A small Latin fold table: `ß`/`æ`/`œ` expand to `ss`/`ae`/`oe` and sort
//!   just after them; `ø đ ł ŧ` sort as stroked `o d l t`.
//! * Empty and equal strings (`"" < "a"`, `"" == ""`).
//!
//! # Approximated (sign may differ from ICU; none of these occur in wire data)
//!
//! * Non-ASCII punctuation, symbols and non-Latin scripts order by code point
//!   within their band, not by DUCET weight — so `U+3000` sorts after ASCII
//!   punctuation instead of with whitespace, and CJK/Greek/Cyrillic relative
//!   script order follows code point.
//! * Non-ASCII decimal digits sort after ASCII digits instead of interleaving
//!   by numeric value.
//! * Latin letters with no canonical decomposition and no fold-table entry
//!   (`ð þ ı ŋ`) fall back to the code-point tail of the letter band, so they
//!   sort after `z` instead of next to their base letter.
//! * Combining marks outside the modelled table order by code point, after all
//!   modelled marks.
//! * ICU contractions/expansions beyond the fold table above are not modelled.
//!
//! Ties return [`Ordering::Equal`] exactly as `localeCompare` returns `0`.
//! `Vec::sort_by`, TS `Array#sort` and Python `sorted` are all stable, so tied
//! elements keep input order in every SDK.

use std::cmp::Ordering;

use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

/// Printable ASCII punctuation/whitespace in ICU primary order.
const ASCII_PUNCTUATION_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

const fn build_ascii_punctuation_rank() -> [i8; 128] {
    let mut table = [-1i8; 128];
    let bytes = ASCII_PUNCTUATION_ORDER.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        table[bytes[index] as usize] = index as i8;
        index += 1;
    }
    table
}

const ASCII_PUNCTUATION_RANK: [i8; 128] = build_ascii_punctuation_rank();

/// Combining marks in ICU secondary order (acute before grave before breve …).
const SECONDARY_MARK_ORDER: [char; 24] = [
    '\u{0313}', '\u{0314}', '\u{0301}', '\u{0300}', '\u{0306}', '\u{0302}', '\u{030c}', '\u{030a}',
    '\u{0308}', '\u{030b}', '\u{0303}', '\u{0307}', '\u{0338}', '\u{0327}', '\u{0328}', '\u{0304}',
    '\u{0335}', '\u{0309}', '\u{030f}', '\u{0311}', '\u{031b}', '\u{0323}', '\u{0326}', '\u{0331}',
];

/// Sentinel appended by an expansion so that `"ss" < "ß"` / `"ae" < "æ"` the
/// way ICU's tertiary expansion weights do. Sorts after every modelled mark.
const EXPANSION_MARK: char = '\u{ffff}';

/// Unmodelled marks trail the modelled ones.
const UNMODELLED_MARK_BASE: u32 = 0x10000;

const BAND_PUNCTUATION: u8 = 0; // whitespace, punctuation, symbols
const BAND_DIGIT: u8 = 1;
const BAND_LETTER: u8 = 2; // letters and everything not otherwise classified

/// Latin letters ICU treats as expansions of, or stroked forms of, ASCII bases.
fn latin_fold(character: char) -> Option<&'static str> {
    Some(match character {
        'ß' => "ss\u{ffff}",
        'ẞ' => "SS\u{ffff}",
        'æ' => "ae\u{ffff}",
        'Æ' => "AE\u{ffff}",
        'œ' => "oe\u{ffff}",
        'Œ' => "OE\u{ffff}",
        'ø' => "o\u{0338}",
        'Ø' => "O\u{0338}",
        'đ' => "d\u{0335}",
        'Đ' => "D\u{0335}",
        'ł' => "l\u{0335}",
        'Ł' => "L\u{0335}",
        'ŧ' => "t\u{0335}",
        'Ŧ' => "T\u{0335}",
        _ => return None,
    })
}

/// Three-level UCA-style sort key: primary, then secondary, then tertiary.
///
/// The derived [`Ord`] compares the levels in order, so sorting a slice by this
/// key is equivalent to sorting it with [`locale_compare`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CollationKey {
    primary: Vec<(u8, u32)>,
    secondary: Vec<u32>,
    tertiary: Vec<u8>,
}

/// `char::to_lowercase` restricted to single-character results (`İ` expands).
fn lowercased(character: char) -> char {
    let mut lowered = character.to_lowercase();
    match (lowered.next(), lowered.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

fn ascii_punctuation_rank(character: char) -> Option<u32> {
    let code = character as u32;
    if code >= 128 {
        return None;
    }
    match ASCII_PUNCTUATION_RANK[code as usize] {
        -1 => None,
        rank => Some(rank as u32),
    }
}

fn primary_weight(character: char) -> (u8, u32) {
    let lowered = lowercased(character);
    if let Some(rank) = ascii_punctuation_rank(lowered) {
        return (BAND_PUNCTUATION, rank);
    }
    if lowered.general_category() == GeneralCategory::DecimalNumber {
        return (BAND_DIGIT, lowered as u32);
    }
    match lowered.general_category_group() {
        GeneralCategoryGroup::Punctuation
        | GeneralCategoryGroup::Symbol
        | GeneralCategoryGroup::Separator => (BAND_PUNCTUATION, lowered as u32),
        _ => (BAND_LETTER, lowered as u32),
    }
}

fn secondary_weight(mark: char) -> u32 {
    if mark == EXPANSION_MARK {
        return SECONDARY_MARK_ORDER.len() as u32 + 1;
    }
    match SECONDARY_MARK_ORDER.iter().position(|entry| *entry == mark) {
        Some(index) => index as u32 + 1,
        None => UNMODELLED_MARK_BASE + mark as u32,
    }
}

/// NFD, then the Latin fold table applied to the decomposed characters.
fn folded_nfd(text: &str) -> impl Iterator<Item = char> + '_ {
    text.nfd()
        .flat_map(|character| match latin_fold(character) {
            Some(expansion) => FoldedChars::Expansion(expansion.chars()),
            None => FoldedChars::Single(Some(character)),
        })
}

enum FoldedChars {
    Single(Option<char>),
    Expansion(std::str::Chars<'static>),
}

impl Iterator for FoldedChars {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            FoldedChars::Single(character) => character.take(),
            FoldedChars::Expansion(chars) => chars.next(),
        }
    }
}

/// Build the three-level sort key for `text`.
///
/// Sorting by this key is equivalent to sorting with [`locale_compare`]; use it
/// when the same string is compared repeatedly (a decorate-sort-undecorate pass
/// over a result set, for example).
pub fn collation_key(text: &str) -> CollationKey {
    let mut key = CollationKey::default();
    for character in folded_nfd(text) {
        if character == EXPANSION_MARK || canonical_combining_class(character) != 0 {
            // Marks are primary-ignorable: they only add a secondary weight.
            key.secondary.push(secondary_weight(character));
            key.tertiary.push(0);
            continue;
        }
        if matches!(
            character.general_category(),
            GeneralCategory::Control | GeneralCategory::Format
        ) {
            continue; // completely ignorable in DUCET
        }
        key.primary.push(primary_weight(character));
        key.secondary.push(0);
        key.tertiary.push(if character == lowercased(character) {
            0
        } else {
            1
        });
    }
    key
}

/// `left.localeCompare(right)` for the cases the SDK actually produces.
///
/// See the module documentation for the precise fidelity envelope against
/// Node's default ICU collator.
pub fn locale_compare(left: &str, right: &str) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    collation_key(left).cmp(&collation_key(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(left, right, sign(left.localeCompare(right)))`, captured verbatim from
    /// Node v23's default `en-US`/ICU collator — the reference TypeScript uses at
    /// `subscription.ts` 57/123 and `query-store.ts` 64/387. Same table as
    /// `python/arete-sdk/tests/test_collation.py::NODE_LOCALE_COMPARE`.
    const NODE_LOCALE_COMPARE: &[(&str, &str, Ordering)] = &[
        ("a", "A", Ordering::Less),
        ("A", "a", Ordering::Greater),
        ("a", "b", Ordering::Less),
        ("A", "b", Ordering::Less),
        ("b", "A", Ordering::Greater),
        ("aBc1", "apple", Ordering::Less),
        ("apple", "Bqq", Ordering::Less),
        ("Bqq", "Zap1", Ordering::Less),
        ("Zap1", "aBc1", Ordering::Greater),
        ("Test", "test", Ordering::Greater),
        ("test", "Test", Ordering::Less),
        ("aa", "aB", Ordering::Less),
        ("a1", "A1", Ordering::Less),
        ("abc", "abcd", Ordering::Less),
        ("abcd", "abc", Ordering::Greater),
        ("ZapfvUeQ", "aBc1xYz", Ordering::Greater),
        (
            "So11111111111111111111111111111111111111112",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            Ordering::Greater,
        ),
        ("état", "zone", Ordering::Less),
        ("zone", "état", Ordering::Greater),
        ("etat", "état", Ordering::Less),
        ("état", "Etat", Ordering::Greater),
        ("Etat", "ETAT", Ordering::Less),
        ("ETAT", "état", Ordering::Less),
        ("é", "e", Ordering::Greater),
        ("e", "é", Ordering::Less),
        ("é", "f", Ordering::Less),
        ("z", "é", Ordering::Greater),
        ("résumé", "resume", Ordering::Greater),
        ("resume", "résumé", Ordering::Less),
        ("a", "á", Ordering::Less),
        ("á", "à", Ordering::Less),
        ("à", "ă", Ordering::Less),
        ("ă", "â", Ordering::Less),
        ("â", "ǎ", Ordering::Less),
        ("ǎ", "å", Ordering::Less),
        ("å", "ä", Ordering::Less),
        ("ä", "ã", Ordering::Less),
        ("ã", "ȧ", Ordering::Less),
        ("ñ", "n", Ordering::Greater),
        ("ñ", "o", Ordering::Less),
        ("ç", "c", Ordering::Greater),
        ("ç", "d", Ordering::Less),
        ("ö", "o", Ordering::Greater),
        ("ö", "p", Ordering::Less),
        ("ü", "u", Ordering::Greater),
        ("ü", "v", Ordering::Less),
        ("ß", "ss", Ordering::Greater),
        ("ss", "ß", Ordering::Less),
        ("ß", "st", Ordering::Less),
        ("ß", "z", Ordering::Less),
        ("æ", "ae", Ordering::Greater),
        ("æ", "ad", Ordering::Greater),
        ("æ", "af", Ordering::Less),
        ("œ", "oe", Ordering::Greater),
        ("œ", "of", Ordering::Less),
        ("ø", "o", Ordering::Greater),
        ("ø", "ö", Ordering::Greater),
        ("ø", "p", Ordering::Less),
        ("ł", "l", Ordering::Greater),
        ("ł", "m", Ordering::Less),
        ("đ", "d", Ordering::Greater),
        ("đ", "e", Ordering::Less),
        ("Å", "å", Ordering::Greater),
        ("É", "é", Ordering::Greater),
        ("é", "e\u{301}", Ordering::Equal),
        ("e\u{301}", "é", Ordering::Equal),
        ("cafe\u{301}", "café", Ordering::Equal),
        ("1", "a", Ordering::Less),
        ("1", "A", Ordering::Less),
        ("9", "a", Ordering::Less),
        ("0", "Z", Ordering::Less),
        ("10", "9", Ordering::Less),
        ("9", "10", Ordering::Greater),
        ("2", "10", Ordering::Greater),
        ("a1", "a2", Ordering::Less),
        ("1a", "a1", Ordering::Less),
        ("0", "1", Ordering::Less),
        ("_", "a", Ordering::Less),
        ("_", "1", Ordering::Less),
        ("-", "_", Ordering::Greater),
        (".", "-", Ordering::Greater),
        (".", "_", Ordering::Greater),
        ("a.b", "a_b", Ordering::Greater),
        ("a_b", "a.b", Ordering::Less),
        ("a-b", "a.b", Ordering::Less),
        (" ", "_", Ordering::Less),
        (" ", "a", Ordering::Less),
        ("$", "a", Ordering::Less),
        ("$", "0", Ordering::Less),
        ("~", "0", Ordering::Less),
        ("(", ")", Ordering::Less),
        ("[", "{", Ordering::Less),
        ("@", "*", Ordering::Less),
        ("/", "\\", Ordering::Less),
        ("&", "#", Ordering::Less),
        ("%", "`", Ordering::Less),
        ("^", "+", Ordering::Less),
        ("<", "=", Ordering::Less),
        ("=", ">", Ordering::Less),
        ("|", "~", Ordering::Less),
        ("!", "?", Ordering::Less),
        (",", ";", Ordering::Less),
        (":", "!", Ordering::Less),
        ("'", "\"", Ordering::Less),
        ("a.b.c", "a.b", Ordering::Greater),
        ("view.field", "view_field", Ordering::Greater),
        ("owner.address", "Owner.Address", Ordering::Less),
        ("", "a", Ordering::Less),
        ("a", "", Ordering::Greater),
        ("", "", Ordering::Equal),
        ("a", "a", Ordering::Equal),
        ("état", "état", Ordering::Equal),
        ("", "_", Ordering::Less),
        ("", "0", Ordering::Less),
        ("a\u{200b}b", "ab", Ordering::Equal),
        ("ab", "a\u{200b}b", Ordering::Equal),
        ("a\u{ad}b", "ab", Ordering::Equal),
        ("a\u{0}b", "ab", Ordering::Equal),
        ("a\u{200b}", "a", Ordering::Equal),
        ("\u{200b}", "", Ordering::Equal),
        ("α", "Ω", Ordering::Less),
        ("Ω", "α", Ordering::Greater),
        ("α", "β", Ordering::Less),
        ("а", "Я", Ordering::Less),
    ];

    /// `(input, [...input].sort((a, b) => a.localeCompare(b)))`, same capture; the
    /// printable-ASCII list has its own test below.
    const NODE_SORTS: &[(&[&str], &[&str])] = &[
        (
            &["Zap1", "aBc1", "Bqq", "apple"],
            &["aBc1", "apple", "Bqq", "Zap1"],
        ),
        (&["état", "zone"], &["état", "zone"]),
        (&["zone", "état"], &["état", "zone"]),
        (
            &["etat", "Etat", "ETAT", "état", "zone", "Zone"],
            &["etat", "Etat", "ETAT", "état", "zone", "Zone"],
        ),
        (
            &["b", "B", "a", "A", "c", "C"],
            &["a", "A", "b", "B", "c", "C"],
        ),
        (
            &["9", "10", "A", "_", "é", "Z", "0", "a"],
            &["_", "0", "10", "9", "a", "A", "é", "Z"],
        ),
        (
            &[
                "owner.address",
                "owner_address",
                "owner-address",
                "Owner.Address",
            ],
            &[
                "owner_address",
                "owner-address",
                "owner.address",
                "Owner.Address",
            ],
        ),
        (&["", "a", "A", "0", "_"], &["", "_", "0", "a", "A"]),
        (
            &["mint", "Mint", "MINT", "mInt", "mint1", "mint0"],
            &["mint", "mInt", "Mint", "MINT", "mint0", "mint1"],
        ),
    ];

    #[test]
    fn locale_compare_matches_node() {
        for (left, right, expected) in NODE_LOCALE_COMPARE {
            assert_eq!(
                locale_compare(left, right),
                *expected,
                "locale_compare({left:?}, {right:?})"
            );
        }
    }

    #[test]
    fn collation_key_sorts_like_node() {
        for (source, expected) in NODE_SORTS {
            let mut sorted: Vec<&str> = source.to_vec();
            sorted.sort_by_key(|entry| collation_key(entry));
            assert_eq!(&sorted[..], *expected, "sorting {source:?}");
        }
    }

    #[test]
    fn printable_ascii_is_in_icu_order() {
        let printable: Vec<String> = (0x20u32..0x7f)
            .map(|code| char::from_u32(code).unwrap().to_string())
            .collect();
        let mut sorted: Vec<&String> = printable.iter().collect();
        sorted.sort_by_key(|entry| collation_key(entry));
        let expected: String = concat!(
            " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$",
            "0123456789",
            "aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ",
        )
        .to_string();
        let actual: String = sorted.into_iter().map(|entry| entry.as_str()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn locale_compare_is_antisymmetric_and_consistent_with_the_key() {
        let sample = [
            "", "a", "A", "b", "B", "z", "Z", "0", "1", "9", "_", "-", ".", " ", "é", "É", "e",
            "E", "ä", "ñ", "aBc1", "apple", "Bqq", "Zap1", "état", "zone",
        ];
        for left in sample {
            assert_eq!(locale_compare(left, left), Ordering::Equal);
            for right in sample {
                assert_eq!(
                    locale_compare(left, right),
                    locale_compare(right, left).reverse(),
                    "antisymmetry for {left:?} / {right:?}"
                );
            }
        }
        let mut ordered = sample.to_vec();
        ordered.sort_by_key(|entry| collation_key(entry));
        for pair in ordered.windows(2) {
            assert_ne!(locale_compare(pair[0], pair[1]), Ordering::Greater);
        }
    }

    #[test]
    fn canonical_equivalence_and_ignorables() {
        // NFC and NFD spellings of the same text collate equal, as in ICU.
        assert_eq!(locale_compare("caf\u{e9}", "cafe\u{301}"), Ordering::Equal);
        assert_eq!(collation_key("caf\u{e9}"), collation_key("cafe\u{301}"));
        // Completely ignorable characters contribute nothing.
        assert_eq!(locale_compare("a\u{200b}b", "ab"), Ordering::Equal);
        assert_eq!(locale_compare("a\u{ad}b", "ab"), Ordering::Equal);
    }

    /// The defect this module exists to fix: byte order and collation order
    /// disagree exactly here. Fails against `str::cmp`, passes against
    /// `locale_compare`.
    #[test]
    fn collation_order_differs_from_byte_order() {
        assert_eq!(locale_compare("état", "zone"), Ordering::Less);
        assert_eq!("état".cmp("zone"), Ordering::Greater);

        let mut by_collation = ["Zap1", "aBc1", "Bqq", "apple"];
        by_collation.sort_by(|left, right| locale_compare(left, right));
        assert_eq!(by_collation, ["aBc1", "apple", "Bqq", "Zap1"]);

        let mut by_bytes = ["Zap1", "aBc1", "Bqq", "apple"];
        by_bytes.sort();
        assert_eq!(by_bytes, ["Bqq", "Zap1", "aBc1", "apple"]);
    }
}
