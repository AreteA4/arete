//! Identifier sanitizers shared by the SDK generators.
//!
//! Generated SDKs turn user-authored names into identifiers: the stack name
//! (a StackManifest `name` only has to be non-empty, so a Stack Source may be
//! called `token-balances`), IDL program names, composition aliases and SDK
//! file stems. Each target language gets one sanitizer in this module, and
//! every generator site that derives an identifier from such a name goes
//! through it.
//!
//! # TypeScript and Rust
//!
//! 1. **Word boundaries.** A name made only of ASCII letters, digits and `_`
//!    is kept as written, so every name that already produced valid
//!    identifiers keeps producing byte-identical SDKs. Any other character
//!    (`-`, `.`, space, `/`, `:`, non-ASCII, ...) is a word boundary: the name
//!    is split there (and at `_`), the first letter of each word is
//!    upper-cased and the words are joined — `token-balances` becomes
//!    `TokenBalances`, `my.stack` becomes `MyStack`.
//! 2. **Case.** The case each site needs is then applied with the generators'
//!    established conversions: [`IdentifierCase::Preserve`] keeps the result
//!    as is, `Pascal` upper-cases the first letter of every `_`-separated
//!    word, `Camel` additionally lower-cases the first letter, and
//!    `ScreamingSnake` inserts `_` before every upper-case letter and
//!    upper-cases the whole identifier (`TokenBalances` -> `TOKEN_BALANCES`).
//! 3. **Leading digits.** An identifier that would start with a digit is
//!    prefixed with `A` (`a` for camelCase): `9lives` -> `A9lives`,
//!    `A9LIVES`, `a9lives`. This matches the prefix composition sessions
//!    already used for aliases and manifest names.
//! 4. **Empty names.** A name with no letters or digits becomes `Unnamed`
//!    (`unnamed`, `UNNAMED`).
//! 5. **Reserved words.** [`typescript::identifier`] and [`rust::identifier`]
//!    return a complete, bare identifier and append `_` to reserved words.
//!    The `identifier_stem` variants return the start of a longer identifier
//!    (`TOKEN_BALANCES` + `_STACK`), which is never reserved as a whole, so
//!    they leave reserved words alone.
//!
//! # Python
//!
//! Python generation already mapped every name through a snake_case
//! conversion that treats any non-alphanumeric character as a word boundary
//! (see [`python::snake_case`]); its rules are kept unchanged so existing
//! Python SDKs stay byte-identical.
//!
//! # Collisions
//!
//! Sanitizing is lossy: `token-balances` and `token_balances` both become
//! `TOKEN_BALANCES`. Generators therefore check the identifiers they declare
//! ([`IdentifierScope`], [`typescript::check_module_declarations`]) and
//! report a collision instead of emitting a module that silently merges or
//! redeclares two names.

use std::borrow::Cow;
use std::collections::BTreeMap;

/// The identifier convention a generator site needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierCase {
    /// The name's own casing (`OreStream` -> `OreStream`,
    /// `token_balances` -> `token_balances`, `token-balances` ->
    /// `TokenBalances`).
    Preserve,
    /// `OreStream`, `TokenBalances`.
    Pascal,
    /// `oreStream`, `tokenBalances`.
    Camel,
    /// `ORE_STREAM`, `TOKEN_BALANCES`.
    ScreamingSnake,
}

/// Reduce `raw` to identifier characters, keeping names that are already made
/// of ASCII letters, digits and `_` exactly as written.
fn identifier_base(raw: &str) -> Cow<'_, str> {
    if raw
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Cow::Borrowed(raw)
    } else {
        Cow::Owned(pascal_words(raw))
    }
}

/// Split at every character that is not an ASCII letter or digit, upper-case
/// the first letter of each word and join the words.
fn pascal_words(raw: &str) -> String {
    raw.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(capitalize)
        .collect()
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// The generators' established PascalCase conversion.
fn pascal_case(value: &str) -> String {
    value.split(['_', '-', '.', ':']).map(capitalize).collect()
}

fn camel_case(value: &str) -> String {
    let pascal = pascal_case(value);
    let mut characters = pascal.chars();
    match characters.next() {
        Some(first) => first.to_lowercase().collect::<String>() + characters.as_str(),
        None => pascal,
    }
}

/// The generators' established SCREAMING_SNAKE_CASE conversion
/// (`OreStream` -> `ORE_STREAM`).
fn screaming_snake_case(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 4);
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            result.push('_');
        }
        result.extend(character.to_uppercase());
    }
    result
}

/// Shared TypeScript/Rust stem: word boundaries, case, leading digit and
/// empty-name handling (rules 1-4 of the module documentation).
fn stem(raw: &str, case: IdentifierCase) -> String {
    let base = identifier_base(raw);
    let mut identifier = match case {
        IdentifierCase::Preserve => base.into_owned(),
        IdentifierCase::Pascal => pascal_case(&base),
        IdentifierCase::Camel => camel_case(&base),
        IdentifierCase::ScreamingSnake => screaming_snake_case(&base),
    };
    if identifier.is_empty() {
        identifier.push_str(match case {
            IdentifierCase::Camel => "unnamed",
            IdentifierCase::ScreamingSnake => "UNNAMED",
            IdentifierCase::Preserve | IdentifierCase::Pascal => "Unnamed",
        });
    } else if identifier.starts_with(|character: char| character.is_ascii_digit()) {
        identifier.insert(
            0,
            if case == IdentifierCase::Camel {
                'a'
            } else {
                'A'
            },
        );
    }
    identifier
}

fn is_ascii_identifier(value: &str, extra: impl Fn(char) -> bool) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || extra(first))
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || extra(character)
        })
}

/// Records which source name produced each generated identifier so that two
/// different names that sanitize to the same identifier are reported instead
/// of being merged into one declaration.
#[derive(Debug)]
pub struct IdentifierScope {
    language: &'static str,
    owners: BTreeMap<String, String>,
}

impl IdentifierScope {
    pub fn new(language: &'static str) -> Self {
        Self {
            language,
            owners: BTreeMap::new(),
        }
    }

    /// Claim `identifier` for `owner` (a human-readable description such as
    /// `stack name 'token-balances'`). Claiming the same identifier again for
    /// the same owner is allowed.
    pub fn claim(&mut self, identifier: &str, owner: &str) -> Result<(), String> {
        match self.owners.get(identifier) {
            Some(existing) if existing != owner => Err(format!(
                "{} identifier `{}` is generated for both {} and {}; rename one of them so their generated names differ",
                self.language, identifier, existing, owner
            )),
            Some(_) => Ok(()),
            None => {
                self.owners
                    .insert(identifier.to_string(), owner.to_string());
                Ok(())
            }
        }
    }
}

pub mod typescript {
    //! TypeScript identifiers.

    use super::{is_ascii_identifier, stem, IdentifierCase};
    use std::collections::BTreeMap;

    /// Words that cannot be used as a TypeScript binding or type name.
    const RESERVED: &[&str] = &[
        // ECMAScript reserved words.
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "new",
        "null",
        "return",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        // Reserved in strict mode (every ES module is strict).
        "arguments",
        "await",
        "eval",
        "implements",
        "interface",
        "let",
        "package",
        "private",
        "protected",
        "public",
        "static",
        "yield",
        // Not allowed as a type alias or interface name.
        "any",
        "bigint",
        "boolean",
        "never",
        "number",
        "object",
        "string",
        "symbol",
        "undefined",
        "unknown",
    ];

    pub fn is_reserved_word(value: &str) -> bool {
        RESERVED.contains(&value)
    }

    /// Whether `value` can be used as a bare TypeScript identifier.
    pub fn is_identifier(value: &str) -> bool {
        is_ascii_identifier(value, |character| character == '$') && !is_reserved_word(value)
    }

    /// The start of a longer TypeScript identifier derived from `raw`
    /// (`TOKEN_BALANCES` in `TOKEN_BALANCES_STACK`). Never empty and never
    /// starts with a digit; see the module documentation for the rules.
    pub fn identifier_stem(raw: &str, case: IdentifierCase) -> String {
        stem(raw, case)
    }

    /// A complete TypeScript identifier derived from `raw`; reserved words
    /// get a trailing `_`.
    pub fn identifier(raw: &str, case: IdentifierCase) -> String {
        let mut identifier = identifier_stem(raw, case);
        if is_reserved_word(&identifier) {
            identifier.push('_');
        }
        identifier
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Binding {
        /// `const`, `let`, `var`.
        Variable,
        /// `function` (overload signatures repeat the name).
        Function,
        /// `type` alias.
        TypeAlias,
        /// `interface` (declarations merge).
        Interface,
        /// `class` / `enum`: a value and a type.
        ClassOrEnum,
        /// An imported binding.
        Import,
    }

    impl Binding {
        fn is_value(self) -> bool {
            matches!(
                self,
                Self::Variable | Self::Function | Self::ClassOrEnum | Self::Import
            )
        }

        fn is_type(self) -> bool {
            matches!(
                self,
                Self::TypeAlias | Self::Interface | Self::ClassOrEnum | Self::Import
            )
        }

        fn conflicts_with(self, other: Self) -> bool {
            if self == Self::Import || other == Self::Import {
                return true;
            }
            let both_values = self.is_value()
                && other.is_value()
                && !(self == Self::Function && other == Self::Function);
            let alias_and_type = (self == Self::TypeAlias && other.is_type())
                || (other == Self::TypeAlias && self.is_type());
            both_values || alias_and_type || (self == other && self == Self::ClassOrEnum)
        }
    }

    fn leading_identifier(value: &str) -> Option<&str> {
        let end = value
            .char_indices()
            .find(|(_, character)| {
                !(character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
            })
            .map(|(index, _)| index)
            .unwrap_or(value.len());
        (end > 0).then(|| &value[..end])
    }

    fn import_bindings(clause: &str, bindings: &mut Vec<(String, Binding)>) {
        let clause = clause.trim().trim_start_matches("type ").trim();
        let (default, named) = match clause.find('{') {
            Some(open) => (
                clause[..open].trim().trim_end_matches(',').trim(),
                clause[open + 1..]
                    .split('}')
                    .next()
                    .unwrap_or_default()
                    .trim(),
            ),
            None => (clause, ""),
        };
        if let Some(namespace) = default.strip_prefix("* as ") {
            bindings.push((namespace.trim().to_string(), Binding::Import));
        } else if !default.is_empty() {
            bindings.push((default.to_string(), Binding::Import));
        }
        for specifier in named.split(',') {
            let specifier = specifier.trim().trim_start_matches("type ").trim();
            let local = specifier
                .rsplit_once(" as ")
                .map(|(_, local)| local.trim())
                .unwrap_or(specifier);
            if !local.is_empty() {
                bindings.push((local.to_string(), Binding::Import));
            }
        }
    }

    /// Top-level bindings a generated module declares or imports. Generated
    /// modules start every top-level statement in column 0; single-line
    /// imports are the only form the generators emit.
    fn top_level_bindings(module: &str) -> Vec<(String, Binding)> {
        let mut bindings = Vec::new();
        for line in module.lines() {
            if let Some(import) = line.strip_prefix("import ") {
                if let Some((clause, _)) = import.rsplit_once(" from ") {
                    import_bindings(clause, &mut bindings);
                }
                continue;
            }
            let declaration = line.strip_prefix("export ").unwrap_or(line);
            let declaration = declaration.strip_prefix("declare ").unwrap_or(declaration);
            let declaration = declaration.strip_prefix("async ").unwrap_or(declaration);
            let (binding, rest) = [
                ("const ", Binding::Variable),
                ("let ", Binding::Variable),
                ("var ", Binding::Variable),
                ("function ", Binding::Function),
                ("type ", Binding::TypeAlias),
                ("interface ", Binding::Interface),
                ("class ", Binding::ClassOrEnum),
                ("enum ", Binding::ClassOrEnum),
            ]
            .into_iter()
            .find_map(|(keyword, binding)| {
                declaration
                    .strip_prefix(keyword)
                    .map(|rest| (binding, rest))
            })
            .unzip();
            if let (Some(binding), Some(name)) = (binding, rest.and_then(leading_identifier)) {
                bindings.push((name.to_string(), binding));
            }
        }
        bindings
    }

    /// Names a generated module exports (declarations with `export`).
    pub fn exported_names(module: &str) -> Vec<String> {
        module
            .lines()
            .filter(|line| line.starts_with("export "))
            .flat_map(|line| top_level_bindings(line).into_iter().map(|(name, _)| name))
            .collect()
    }

    /// Report a generated module whose top-level declarations or imports
    /// bind the same name twice (which `tsc` rejects), e.g. because two
    /// different source names sanitized to the same identifier. `context`
    /// describes the module in the error.
    pub fn check_module_declarations(module: &str, context: &str) -> Result<(), String> {
        let mut seen: BTreeMap<String, Vec<Binding>> = BTreeMap::new();
        for (name, binding) in top_level_bindings(module) {
            let previous = seen.entry(name.clone()).or_default();
            if previous
                .iter()
                .any(|existing| existing.conflicts_with(binding))
            {
                return Err(format!(
                    "{context} would declare the TypeScript identifier `{name}` more than once; two source names (stack, program, entity or alias) generate the same identifier after sanitizing — rename one of them"
                ));
            }
            previous.push(binding);
        }
        Ok(())
    }

    /// Names a module imports by name (`X` in `import { X as Y } from ...`).
    fn named_imports(module: &str) -> Vec<String> {
        module
            .lines()
            .filter_map(|line| line.strip_prefix("import "))
            .filter_map(|import| import.rsplit_once(" from ").map(|(clause, _)| clause))
            .filter_map(|clause| {
                let open = clause.find('{')?;
                Some(clause[open + 1..].split('}').next().unwrap_or_default())
            })
            .flat_map(|named| named.split(','))
            .map(|specifier| {
                let specifier = specifier.trim().trim_start_matches("type ").trim();
                specifier
                    .split_once(" as ")
                    .map(|(imported, _)| imported.trim())
                    .unwrap_or(specifier)
                    .to_string()
            })
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// Report names a wrapper module declares that also come from a module it
    /// star-re-exports: TypeScript silently lets the local declaration win,
    /// hiding the re-exported one. A wrapper that deliberately replaces an
    /// export imports the original by name (`import { X as X_CORE }`); those
    /// names are allowed.
    pub fn check_star_reexport_shadowing(
        wrapper: &str,
        reexported: &str,
        context: &str,
    ) -> Result<(), String> {
        let exported = exported_names(reexported);
        let replaced = named_imports(wrapper);
        for (name, binding) in top_level_bindings(wrapper) {
            if binding != Binding::Import && exported.contains(&name) && !replaced.contains(&name) {
                return Err(format!(
                    "{context} declares the TypeScript identifier `{name}`, which the re-exported core module already exports; two source names (stack, program, entity or alias) generate the same identifier after sanitizing — rename one of them"
                ));
            }
        }
        Ok(())
    }

    /// Escape text for a `/** ... */` or `//` comment.
    pub fn comment_text(value: &str) -> String {
        value.replace("*/", "*\\/").replace(['\n', '\r'], " ")
    }

    /// A single-quoted TypeScript string literal.
    pub fn single_quoted(value: &str) -> String {
        let mut literal = String::with_capacity(value.len() + 2);
        literal.push('\'');
        for character in value.chars() {
            match character {
                '\\' => literal.push_str("\\\\"),
                '\'' => literal.push_str("\\'"),
                '\n' => literal.push_str("\\n"),
                '\r' => literal.push_str("\\r"),
                '\u{2028}' => literal.push_str("\\u2028"),
                '\u{2029}' => literal.push_str("\\u2029"),
                character => literal.push(character),
            }
        }
        literal.push('\'');
        literal
    }
}

pub mod rust {
    //! Rust identifiers.

    use super::{is_ascii_identifier, stem, IdentifierCase};

    /// Strict, reserved and edition-2018+ keywords (plus `union`, which the
    /// generators have always treated as reserved).
    pub fn is_keyword(value: &str) -> bool {
        matches!(
            value,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "union"
                | "unsafe"
                | "use"
                | "where"
                | "while"
                | "abstract"
                | "become"
                | "box"
                | "do"
                | "final"
                | "macro"
                | "override"
                | "priv"
                | "typeof"
                | "unsized"
                | "virtual"
                | "yield"
                | "try"
        )
    }

    /// Whether `value` can be used as a bare Rust identifier.
    pub fn is_identifier(value: &str) -> bool {
        value != "_" && is_ascii_identifier(value, |_| false) && !is_keyword(value)
    }

    /// The start of a longer Rust identifier derived from `raw`
    /// (`TokenBalances` in `TokenBalancesStack`). Never empty and never
    /// starts with a digit; see the module documentation for the rules.
    pub fn identifier_stem(raw: &str, case: IdentifierCase) -> String {
        stem(raw, case)
    }

    /// A complete Rust identifier derived from `raw`; keywords and `_` get a
    /// trailing `_`.
    pub fn identifier(raw: &str, case: IdentifierCase) -> String {
        let mut identifier = identifier_stem(raw, case);
        if identifier == "_" || is_keyword(&identifier) {
            identifier.push('_');
        }
        identifier
    }

    /// A Cargo package name derived from `raw`: characters other than ASCII
    /// letters, digits, `-` and `_` become `-`, and a name that would not
    /// start with a letter or `_` is prefixed with `a`. Valid package names
    /// are returned unchanged.
    pub fn package_name(raw: &str) -> String {
        let mut name = String::with_capacity(raw.len() + 1);
        for character in raw.chars() {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                name.push(character);
            } else if !name.ends_with('-') {
                name.push('-');
            }
        }
        let name = name.trim_matches('-');
        if name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_') {
            name.to_string()
        } else {
            format!("a{name}")
        }
    }
}

pub mod python {
    //! Python identifiers. These are the rules Python generation has always
    //! used, kept byte-for-byte.

    pub fn is_keyword(value: &str) -> bool {
        matches!(
            value,
            "False"
                | "None"
                | "True"
                | "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "try"
                | "while"
                | "with"
                | "yield"
        )
    }

    /// snake_case: every character other than an ASCII letter or digit, and
    /// every upper-case letter, starts a new word; words are lower-cased and
    /// joined with `_`. A leading digit gets a `value_` prefix and keywords a
    /// trailing `_` (`OreStream` -> `ore_stream`, `token-balances` ->
    /// `token_balances`, `9lives` -> `value_9lives`, `class` -> `class_`).
    pub fn snake_case(value: &str) -> String {
        let mut result = String::new();
        let mut separator = false;
        for character in value.chars() {
            if character.is_ascii_alphanumeric() {
                if separator && !result.is_empty() {
                    result.push('_');
                }
                separator = false;
                if character.is_ascii_uppercase() {
                    if !result.is_empty() && !result.ends_with('_') {
                        result.push('_');
                    }
                    result.push(character.to_ascii_lowercase());
                } else {
                    result.push(character.to_ascii_lowercase());
                }
            } else {
                separator = true;
            }
        }
        if result
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            result.insert_str(0, "value_");
        }
        if is_keyword(&result) {
            result.push('_');
        }
        result
    }

    /// SCREAMING_SNAKE_CASE with the [`snake_case`] word rules
    /// (`token-balances` -> `TOKEN_BALANCES`, `9lives` -> `VALUE_9LIVES`).
    pub fn screaming_snake_case(value: &str) -> String {
        snake_case(value).to_uppercase()
    }

    /// Escape text interpolated into a `"""` docstring.
    pub fn docstring_text(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace("\"\"\"", "\\\"\\\"\\\"")
    }
}

#[cfg(test)]
mod tests {
    use super::IdentifierCase::{Camel, Pascal, Preserve, ScreamingSnake};
    use super::*;

    /// The conversions the generators applied to stack names before this
    /// module existed. Valid names must keep producing exactly these.
    fn legacy_typescript(raw: &str, case: IdentifierCase) -> String {
        match case {
            Preserve => raw.to_string(),
            Pascal => pascal_case(raw),
            Camel => camel_case(raw),
            ScreamingSnake => screaming_snake_case(raw),
        }
    }

    const EXISTING_NAMES: &[&str] = &[
        "OreStream",
        "ore_stream",
        "ore",
        "ORE",
        "Ore2",
        "ore2",
        "Token_Balances",
        "token_balances",
        "PumpfunStream",
        "OrderedStream",
        "a_b_c",
        "_private",
        "x",
    ];

    #[test]
    fn existing_identifier_names_are_unchanged() {
        for name in EXISTING_NAMES {
            for case in [Preserve, Pascal, Camel, ScreamingSnake] {
                let legacy = legacy_typescript(name, case);
                assert_eq!(
                    typescript::identifier_stem(name, case),
                    legacy,
                    "{name} {case:?}"
                );
                assert_eq!(rust::identifier_stem(name, case), legacy, "{name} {case:?}");
            }
        }
    }

    #[test]
    fn separators_become_word_boundaries() {
        let cases = [
            (
                "token-balances",
                "TokenBalances",
                "TokenBalances",
                "tokenBalances",
                "TOKEN_BALANCES",
            ),
            ("my.stack", "MyStack", "MyStack", "myStack", "MY_STACK"),
            ("my stack", "MyStack", "MyStack", "myStack", "MY_STACK"),
            (
                "Ore-Stream",
                "OreStream",
                "OreStream",
                "oreStream",
                "ORE_STREAM",
            ),
            (
                "token_balances-v2",
                "TokenBalancesV2",
                "TokenBalancesV2",
                "tokenBalancesV2",
                "TOKEN_BALANCES_V2",
            ),
            ("a/b::c", "ABC", "ABC", "aBC", "A_B_C"),
            ("--x--", "X", "X", "x", "X"),
            ("café-bar", "CafBar", "CafBar", "cafBar", "CAF_BAR"),
        ];
        for (raw, preserve, pascal, camel, screaming) in cases {
            assert_eq!(
                typescript::identifier_stem(raw, Preserve),
                preserve,
                "{raw}"
            );
            assert_eq!(typescript::identifier_stem(raw, Pascal), pascal, "{raw}");
            assert_eq!(typescript::identifier_stem(raw, Camel), camel, "{raw}");
            assert_eq!(
                typescript::identifier_stem(raw, ScreamingSnake),
                screaming,
                "{raw}"
            );
        }
    }

    #[test]
    fn hyphenated_pascal_and_camel_match_the_legacy_conversions() {
        // PascalCase and camelCase already split on `-`, `.` and `:`; the
        // sanitizer must agree so program keys and type names don't move.
        for raw in ["pump-amm", "token-2022", "a.b", "ore::round", "x-9y"] {
            assert_eq!(typescript::identifier_stem(raw, Pascal), pascal_case(raw));
            assert_eq!(typescript::identifier_stem(raw, Camel), camel_case(raw));
        }
    }

    #[test]
    fn leading_digits_get_a_letter_prefix() {
        assert_eq!(typescript::identifier_stem("9lives", Preserve), "A9lives");
        assert_eq!(typescript::identifier_stem("9lives", Pascal), "A9lives");
        assert_eq!(typescript::identifier_stem("9lives", Camel), "a9lives");
        assert_eq!(
            typescript::identifier_stem("9lives", ScreamingSnake),
            "A9LIVES"
        );
        assert_eq!(
            typescript::identifier_stem("9-lives", ScreamingSnake),
            "A9_LIVES"
        );
        assert_eq!(typescript::identifier_stem("_9x", Pascal), "A9x");
        assert_eq!(rust::identifier_stem("9lives", Preserve), "A9lives");
    }

    #[test]
    fn empty_names_get_a_placeholder() {
        assert_eq!(typescript::identifier_stem("---", Pascal), "Unnamed");
        assert_eq!(typescript::identifier_stem("", Camel), "unnamed");
        assert_eq!(
            typescript::identifier_stem("...", ScreamingSnake),
            "UNNAMED"
        );
        assert_eq!(typescript::identifier_stem("_", Pascal), "Unnamed");
        // `_` alone is an identifier start the SCREAMING form keeps.
        assert_eq!(typescript::identifier_stem("_", ScreamingSnake), "_");
    }

    #[test]
    fn reserved_words_are_escaped_only_when_bare() {
        assert_eq!(typescript::identifier("class", Preserve), "class_");
        assert_eq!(typescript::identifier("delete", Camel), "delete_");
        assert_eq!(typescript::identifier("string", Preserve), "string_");
        assert_eq!(typescript::identifier_stem("class", Preserve), "class");
        assert_eq!(typescript::identifier("Class", Preserve), "Class");
        assert_eq!(rust::identifier("self", Preserve), "self_");
        assert_eq!(rust::identifier("Self", Pascal), "Self_");
        assert_eq!(rust::identifier("_", Preserve), "__");
        assert_eq!(rust::identifier_stem("Self", Pascal), "Self");
    }

    #[test]
    fn every_sanitized_identifier_is_valid() {
        let names = [
            "token-balances",
            "my.stack",
            "9lives",
            "my stack",
            "class",
            "---",
            "",
            "_",
            "Self",
            "ünïcode name",
            "a\"b'c",
            "ore",
            "OreStream",
            "$dollar",
        ];
        for raw in names {
            for case in [Preserve, Pascal, Camel, ScreamingSnake] {
                let ts = typescript::identifier(raw, case);
                assert!(typescript::is_identifier(&ts), "{raw:?} {case:?} -> {ts}");
                let rs = rust::identifier(raw, case);
                assert!(rust::is_identifier(&rs), "{raw:?} {case:?} -> {rs}");
                for suffix in ["_STACK", "Stack", "CoreStack"] {
                    let ts = typescript::identifier_stem(raw, case) + suffix;
                    assert!(typescript::is_identifier(&ts), "{raw:?} {case:?} -> {ts}");
                    let rs = rust::identifier_stem(raw, case) + suffix;
                    assert!(rust::is_identifier(&rs), "{raw:?} {case:?} -> {rs}");
                }
            }
            let py = python::screaming_snake_case(raw) + "_STACK";
            assert!(
                is_ascii_identifier(&py, |_| false) && !python::is_keyword(&py),
                "{raw:?} -> {py}"
            );
        }
    }

    #[test]
    fn python_rules_are_unchanged() {
        assert_eq!(python::snake_case("OreStream"), "ore_stream");
        assert_eq!(python::snake_case("token-balances"), "token_balances");
        assert_eq!(python::snake_case("my.stack"), "my_stack");
        assert_eq!(python::snake_case("9lives"), "value_9lives");
        assert_eq!(python::snake_case("class"), "class_");
        assert_eq!(python::screaming_snake_case("ORE"), "O_R_E");
        assert_eq!(
            python::screaming_snake_case("token_balances"),
            "TOKEN_BALANCES"
        );
        assert_eq!(python::docstring_text(r#"a"""b\c"#), r#"a\"\"\"b\\c"#);
    }

    #[test]
    fn cargo_package_names() {
        assert_eq!(rust::package_name("OreStream-stack"), "OreStream-stack");
        assert_eq!(
            rust::package_name("token_balances-stack"),
            "token_balances-stack"
        );
        assert_eq!(
            rust::package_name("token-balances-stack"),
            "token-balances-stack"
        );
        assert_eq!(rust::package_name("my.stack-stack"), "my-stack-stack");
        assert_eq!(rust::package_name("my stack-stack"), "my-stack-stack");
        assert_eq!(rust::package_name("9lives-stack"), "a9lives-stack");
        assert_eq!(rust::package_name("_ore-stack"), "_ore-stack");
        assert_eq!(rust::package_name("-x"), "x");
        assert_eq!(rust::package_name("..."), "a");
    }

    #[test]
    fn scope_reports_collisions_with_both_sources() {
        let mut scope = IdentifierScope::new("TypeScript");
        scope
            .claim("TOKEN_BALANCES", "stack name 'token-balances'")
            .unwrap();
        scope
            .claim("TOKEN_BALANCES", "stack name 'token-balances'")
            .unwrap();
        let error = scope
            .claim("TOKEN_BALANCES", "program 'token_balances'")
            .unwrap_err();
        assert!(error.contains("`TOKEN_BALANCES`"), "{error}");
        assert!(error.contains("stack name 'token-balances'"), "{error}");
        assert!(error.contains("program 'token_balances'"), "{error}");
    }

    #[test]
    fn typescript_module_declaration_collisions() {
        let clean = "import { z } from 'zod';\nimport { pda, type ErrorMetadata } from '@usearete/sdk';\n\
export interface Foo {\n  a: string;\n}\nexport interface Foo {\n  b: string;\n}\n\
export const FooSchema = z.object({});\nexport type FooSchema = z.infer<typeof FooSchema>;\n\
function listView<T>(view: string): T;\nfunction listView<T>(view: string): T { return view as T; }\n\
const CORE = {\n  const: 1,\n};\nexport default CORE;\nexport * from './core.js';\n";
        typescript::check_module_declarations(clean, "module").unwrap();

        for duplicate in [
            "export const A = 1;\nexport const A = 2;\n",
            "export type A = string;\nexport interface A {}\n",
            "export type A = string;\nexport type A = number;\n",
            "import { A } from './a.js';\nexport interface A {}\n",
            "import A from './a.js';\nimport { B as A } from './b.js';\n",
            "export class A {}\nexport const A = 1;\n",
        ] {
            let error = typescript::check_module_declarations(duplicate, "module").unwrap_err();
            assert!(error.contains("`A`"), "{duplicate}: {error}");
        }
    }

    #[test]
    fn typescript_star_reexport_shadowing() {
        let core = "export const TOKEN_BALANCES_STACK_CORE = {};\nexport interface TokenBalancesStack {}\n";
        let entry = "import { TOKEN_BALANCES_STACK_CORE } from './core.js';\nexport * from './core.js';\n\
export const TOKEN_BALANCES_STACK = TOKEN_BALANCES_STACK_CORE;\nexport type TokenBalancesStack = typeof TOKEN_BALANCES_STACK;\n";
        let error = typescript::check_star_reexport_shadowing(entry, core, "entry").unwrap_err();
        assert!(error.contains("`TokenBalancesStack`"), "{error}");
        let core = "export const TOKEN_BALANCES_STACK_CORE = {};\n";
        typescript::check_star_reexport_shadowing(entry, core, "entry").unwrap();

        // A wrapper that imports the original under another name replaces it
        // on purpose.
        let core = "export const VAULT_PROGRAMS = {};\n";
        let entry = "import { VAULT_PROGRAMS as VAULT_PROGRAMS_CORE } from './core.js';\n\
export * from './core.js';\nexport const VAULT_PROGRAMS = VAULT_PROGRAMS_CORE;\n";
        typescript::check_star_reexport_shadowing(entry, core, "entry").unwrap();
    }

    #[test]
    fn typescript_literals_and_comments() {
        assert_eq!(typescript::single_quoted("ore-stream"), "'ore-stream'");
        assert_eq!(typescript::single_quoted("bob's\\x"), r"'bob\'s\\x'");
        assert_eq!(typescript::comment_text("a */ b"), "a *\\/ b");
        assert_eq!(typescript::comment_text("plain"), "plain");
    }
}
