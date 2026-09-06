//! Portable local aliases for registry dependencies.
//!
//! A dependency has four distinct identities: the remote lookup string sent to
//! the registry unchanged (an alias such as `My-Program`, a stable `upr_...`
//! reference, or a deployed stack name), the local alias that names the
//! dependency in `arete.toml` and the generated module/crate/package, the
//! semver requirement, and the immutable lock. This module owns the second
//! one: a deterministic, cross-language identifier derived from the lookup so
//! regeneration on another machine yields the same names.

use anyhow::{bail, Result};

use super::manifest::DependencyKind;

/// Prefix applied when a normalized alias would start with a digit, which
/// Rust crate/module names and Python packages reject.
const LEADING_DIGIT_PREFIX: &str = "pkg-";
/// Suffix applied when the identifier form of an alias is a reserved word in
/// one of the generated languages.
const RESERVED_SUFFIX: &str = "-pkg";
const MAX_ALIAS_LENGTH: usize = 64;

/// Reserved identifiers across Rust, TypeScript/JavaScript, and Python. The
/// identifier form of an alias (`-` mapped to `_`) must not collide with any of
/// them because generated code uses it for modules, crates, and packages.
const RESERVED_IDENTIFIERS: &[&str] = &[
    // Rust keywords (strict, reserved, and weak).
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "try",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "gen",
    "union",
    "core",
    "std",
    "alloc",
    "proc_macro",
    "test",
    "main",
    "lib",
    // TypeScript / JavaScript reserved and strict-mode words.
    "case",
    "catch",
    "class",
    "debugger",
    "default",
    "delete",
    "export",
    "extends",
    "finally",
    "function",
    "import",
    "instanceof",
    "new",
    "null",
    "switch",
    "this",
    "throw",
    "var",
    "void",
    "with",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "arguments",
    "eval",
    "undefined",
    "any",
    "boolean",
    "number",
    "string",
    "symbol",
    "never",
    "unknown",
    "object",
    "bigint",
    "declare",
    "namespace",
    "module",
    "require",
    "readonly",
    "keyof",
    "infer",
    // Python keywords, soft keywords, and standard-library shadowing hazards.
    "and",
    "assert",
    "def",
    "del",
    "elif",
    "except",
    "from",
    "global",
    "is",
    "lambda",
    "nonlocal",
    "not",
    "or",
    "pass",
    "raise",
    "none",
    "typing",
    "dataclasses",
    "asyncio",
    "json",
    "enum_",
    "types",
    "builtins",
    "sys",
    "os",
];

/// Derive the deterministic local alias for a registry lookup string.
///
/// Rules, applied in order:
/// 1. lower-case ASCII; every run of characters outside `[a-z0-9]` becomes one `-`;
/// 2. leading and trailing `-` are trimmed; an empty result becomes `dependency`;
/// 3. a leading digit is prefixed with `pkg-`;
/// 4. an identifier form (`-` → `_`) that is reserved in Rust, TypeScript, or
///    Python receives the `-pkg` suffix;
/// 5. the result is truncated to 64 bytes without a trailing `-`.
///
/// The lookup itself is never modified; callers send it to the registry as
/// written. Stable resource IDs such as `upr_AbC-…` therefore stay exact
/// lookups while their alias becomes `upr-abc-…`.
pub fn derive_local_alias(lookup: &str) -> String {
    let mut normalized = String::with_capacity(lookup.len());
    let mut pending_separator = false;
    for character in lookup.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            pending_separator = false;
            normalized.push(character.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    if normalized.is_empty() {
        normalized.push_str("dependency");
    }
    if normalized
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        normalized.insert_str(0, LEADING_DIGIT_PREFIX);
    }
    if is_reserved_identifier(&normalized) {
        normalized.push_str(RESERVED_SUFFIX);
    }
    if normalized.len() > MAX_ALIAS_LENGTH {
        normalized.truncate(MAX_ALIAS_LENGTH);
        while normalized.ends_with('-') {
            normalized.pop();
        }
    }
    normalized
}

/// Whether the identifier form of `alias` collides with a reserved word in a
/// generated language.
pub fn is_reserved_identifier(alias: &str) -> bool {
    let identifier = alias.replace('-', "_");
    RESERVED_IDENTIFIERS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(&identifier))
}

/// Validate an alias that will name generated Rust, TypeScript, and Python
/// output. Manifest validation enforces the character set; this adds the
/// cross-language identifier rules so an explicit `--alias` fails before any
/// file, manifest, or lock is written.
pub fn validate_local_alias(alias: &str, kind: DependencyKind) -> Result<()> {
    let valid_shape = !alias.is_empty()
        && alias.len() <= MAX_ALIAS_LENGTH
        && alias
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && alias.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
        && !alias.ends_with(['-', '_'])
        && !alias.contains("--")
        && !alias.contains("__");
    if !valid_shape {
        bail!(
            "{kind} alias '{alias}' must be 1-64 lowercase ASCII letters, digits, '-' or '_', start with a letter, and not end with or repeat separators (try the derived alias '{}')",
            derive_local_alias(alias)
        );
    }
    if is_reserved_identifier(alias) {
        bail!(
            "{kind} alias '{alias}' is a reserved identifier in a generated language (try '{}')",
            derive_local_alias(alias)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_deterministic_and_portable() {
        let cases: &[(&str, &str)] = &[
            ("ore", "ore"),
            ("My-Program", "my-program"),
            ("Plan004_Shared", "plan004-shared"),
            ("upr_AbC123-xyz_9", "upr-abc123-xyz-9"),
            ("UPR_ONLYCAPS", "upr-onlycaps"),
            ("demo.stack/one", "demo-stack-one"),
            ("  spaced   name ", "spaced-name"),
            ("1inch", "pkg-1inch"),
            ("42", "pkg-42"),
            ("---", "dependency"),
            ("", "dependency"),
            ("émoji ✨ stack", "moji-stack"),
            ("default", "default-pkg"),
            ("Class", "class-pkg"),
            ("import", "import-pkg"),
            ("proc-macro", "proc-macro-pkg"),
            ("None", "none-pkg"),
            ("self", "self-pkg"),
            ("a--b__c", "a-b-c"),
        ];
        for (lookup, expected) in cases {
            let alias = derive_local_alias(lookup);
            assert_eq!(&alias, expected, "lookup {lookup:?}");
            assert_eq!(
                derive_local_alias(&alias),
                alias,
                "derivation is idempotent for {lookup:?}"
            );
            validate_local_alias(&alias, DependencyKind::Program)
                .unwrap_or_else(|error| panic!("derived alias {alias:?} must validate: {error}"));
        }
    }

    #[test]
    fn colliding_lookups_normalize_identically_and_are_detectable() {
        assert_eq!(
            derive_local_alias("My_Program"),
            derive_local_alias("my-program")
        );
        assert_eq!(
            derive_local_alias("MY.PROGRAM"),
            derive_local_alias("my program")
        );
    }

    #[test]
    fn long_lookups_are_bounded_without_trailing_separator() {
        let long = format!("{}-tail", "a".repeat(70));
        let alias = derive_local_alias(&long);
        assert_eq!(alias.len(), 64);
        assert!(!alias.ends_with('-'));
        validate_local_alias(&alias, DependencyKind::Stack).unwrap();
    }

    #[test]
    fn explicit_aliases_must_be_valid_identifiers_in_every_language() {
        for alias in ["ore", "my-program", "upr-abc", "a1", "snake_case"] {
            validate_local_alias(alias, DependencyKind::Program).unwrap();
        }
        for alias in [
            "Default",
            "1abc",
            "class",
            "trailing-",
            "double--dash",
            "",
            "UPR_X",
            "fn",
            "self",
            "import",
        ] {
            assert!(
                validate_local_alias(alias, DependencyKind::Stack).is_err(),
                "{alias:?} must be rejected"
            );
        }
    }
}
