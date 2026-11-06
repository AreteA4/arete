//! Fuzz test: generated SDKs stay inert whatever text a stack or IDL holds.
//!
//! Every case puts a hostile string (a payload that tries to leave a string
//! literal, template literal, docstring or comment and call
//! `PWNED(0x1337)`) into one name or docs position of an IDL or stack spec,
//! then generates the TypeScript, Rust and Python SDKs. Each generator must
//! either refuse the input or emit code in which the payload stayed inside
//! its literal or comment:
//!
//! - Rust output must parse (`syn`), and no `(0x1337)` group may appear in
//!   its token stream outside a literal.
//! - TypeScript and Python output is lexed with every string literal and
//!   comment blanked out; no literal or comment may be left open, brackets
//!   must balance and `(0x1337)` may not appear in what remains. Python output
//!   is also parsed with `ast.parse` when a `python3` is available (parsing
//!   never runs the code).
//!
//! Free text (docs, error messages) must always generate; names that are not
//! identifiers must always be refused. Cases come from a fixed-seed generator,
//! so a failure reproduces exactly.

use std::collections::BTreeMap;
use std::path::PathBuf;

use arete_artifacts::{load_live_spec_v2, load_program_spec, load_stack_manifest_v2};
use arete_interpreter::ast::SerializableStackSpec;
use arete_interpreter::program_sdk::build_program_only_stack_spec_from_idl_bytes;
use arete_interpreter::public_artifacts::stack_spec_from_artifacts_v2;
use arete_interpreter::{python, rust, typescript};
use serde_json::json;

const PAYLOAD_CALL: &str = "PWNED(0x1337)";
/// Cases per IDL position (a small program SDK each) and per stack position
/// (the ORE stack SDK each).
const IDL_CASES_PER_POSITION: usize = 10;
const STACK_CASES_PER_POSITION: usize = 4;

/// Strings that end or open a literal, comment or substitution in at least
/// one of the generated languages, or that are merely not identifiers.
const BREAKOUTS: &[&str] = &[
    "'",
    "\"",
    "`",
    "\\",
    "\n",
    "\r",
    "\r\n",
    "\u{2028}",
    "\u{2029}",
    "\u{85}",
    "\0",
    "*/",
    "/*",
    "\"\"\"",
    "'''",
    "${",
    "{",
    "}",
    "}}",
    "#",
    "//",
    ";",
    "(",
    ")",
    " ",
    "-",
    ".",
    "/",
    ":",
    "$",
    "\u{202E}",
    "<",
    ">",
    "=",
    ",",
    "]",
    "\t",
    "\\n",
    "*/\n",
    "');",
    "\");",
    "`);",
    "\"\"\")",
    "-->",
    "\\\"",
    "\\'",
    "\r\n#",
    "\n//",
    "\n\"\"\"\n",
];

/// xorshift64*: deterministic, so every run checks the same cases.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }

    /// `x<breakout>PWNED(0x1337)<breakout>[<breakout>]`.
    fn hostile(&mut self) -> String {
        let mut value = format!(
            "x{}{PAYLOAD_CALL}{}",
            self.pick(BREAKOUTS),
            self.pick(BREAKOUTS)
        );
        if self.next().is_multiple_of(2) {
            value.push_str(self.pick(BREAKOUTS));
        }
        value
    }
}

/// Whether a position holds free text (must generate) or a name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Docs or an error message: always generates, payload stays inert.
    FreeText,
    /// A name the generators turn into identifiers: always refused.
    Identifier,
    /// A display name (program or stack name) the generators sanitize: may
    /// generate, payload stays inert.
    DisplayName,
}

#[derive(Clone, Copy, Debug)]
enum IdlPosition {
    ProgramName,
    Instruction,
    InstructionDocs,
    InstructionAccount,
    InstructionAccountDocs,
    Argument,
    Account,
    AccountDocs,
    TypeDocs,
    Field,
    FieldDocs,
    Enum,
    Variant,
    Error,
    ErrorMessage,
}

impl IdlPosition {
    const ALL: [Self; 15] = [
        Self::ProgramName,
        Self::Instruction,
        Self::InstructionDocs,
        Self::InstructionAccount,
        Self::InstructionAccountDocs,
        Self::Argument,
        Self::Account,
        Self::AccountDocs,
        Self::TypeDocs,
        Self::Field,
        Self::FieldDocs,
        Self::Enum,
        Self::Variant,
        Self::Error,
        Self::ErrorMessage,
    ];

    fn kind(self) -> Kind {
        match self {
            Self::InstructionDocs
            | Self::InstructionAccountDocs
            | Self::AccountDocs
            | Self::TypeDocs
            | Self::FieldDocs
            | Self::ErrorMessage => Kind::FreeText,
            Self::ProgramName => Kind::DisplayName,
            _ => Kind::Identifier,
        }
    }
}

/// An Anchor IDL with one instruction, account, struct, enum and error,
/// `hostile` placed at `position`.
fn idl(position: IdlPosition, hostile: &str) -> Vec<u8> {
    let at = |candidate: IdlPosition, default: &str| -> String {
        if std::mem::discriminant(&candidate) == std::mem::discriminant(&position) {
            hostile.to_string()
        } else {
            default.to_string()
        }
    };
    let account = at(IdlPosition::Account, "Vault");
    let choice = at(IdlPosition::Enum, "Choice");
    let document = json!({
        "address": "Demo111111111111111111111111111111111111111",
        "metadata": {"name": at(IdlPosition::ProgramName, "demo"), "version": "0.1.0", "spec": "0.1.0"},
        "instructions": [{
            "name": at(IdlPosition::Instruction, "deposit"),
            "docs": [at(IdlPosition::InstructionDocs, "Deposits funds.")],
            "discriminator": [1, 2, 3, 4, 5, 6, 7, 8],
            "accounts": [{
                "name": at(IdlPosition::InstructionAccount, "payer"),
                "writable": true,
                "signer": true,
                "docs": [at(IdlPosition::InstructionAccountDocs, "Pays.")]
            }],
            "args": [
                {"name": at(IdlPosition::Argument, "amount"), "type": "u64"},
                {"name": "choice", "type": {"defined": {"name": choice}}}
            ]
        }],
        "accounts": [{"name": account, "discriminator": [9, 9, 9, 9, 9, 9, 9, 9], "docs": [at(IdlPosition::AccountDocs, "A vault.")]}],
        "types": [
            {
                "name": account,
                "docs": [at(IdlPosition::TypeDocs, "Vault state.")],
                "type": {"kind": "struct", "fields": [{
                    "name": at(IdlPosition::Field, "balance"),
                    "docs": [at(IdlPosition::FieldDocs, "Balance.")],
                    "type": "u64"
                }]}
            },
            {"name": choice, "type": {"kind": "enum", "variants": [{"name": at(IdlPosition::Variant, "Yes")}, {"name": "No"}]}}
        ],
        "events": [],
        "errors": [{"code": 6000, "name": at(IdlPosition::Error, "Broken"), "msg": at(IdlPosition::ErrorMessage, "It broke.")}]
    });
    serde_json::to_vec(&document).unwrap()
}

#[derive(Clone, Copy, Debug)]
enum StackPosition {
    StackName,
    Entity,
    Section,
    SectionField,
    FieldPath,
    View,
    IdlInstruction,
    IdlTypeDocs,
}

impl StackPosition {
    const ALL: [Self; 8] = [
        Self::StackName,
        Self::Entity,
        Self::Section,
        Self::SectionField,
        Self::FieldPath,
        Self::View,
        Self::IdlInstruction,
        Self::IdlTypeDocs,
    ];

    fn kind(self) -> Kind {
        match self {
            Self::StackName => Kind::DisplayName,
            Self::IdlTypeDocs => Kind::FreeText,
            _ => Kind::Identifier,
        }
    }

    /// Put `hostile` into `spec` directly, past any upstream normalization,
    /// so the generators' own checks are what is tested.
    fn apply(self, spec: &mut SerializableStackSpec, hostile: &str) {
        let entity = &mut spec.entities[0];
        match self {
            Self::StackName => spec.stack_name = hostile.to_string(),
            Self::Entity => entity.state_name = hostile.to_string(),
            Self::Section => entity.sections[0].name = hostile.to_string(),
            Self::SectionField => entity.sections[0].fields[0].field_name = hostile.to_string(),
            Self::FieldPath => {
                let (_, info) = entity.field_mappings.pop_first().unwrap();
                entity.field_mappings.insert(hostile.to_string(), info);
            }
            Self::View => entity.views[0].id = hostile.to_string(),
            Self::IdlInstruction => spec.idls[0].instructions[0].name = hostile.to_string(),
            Self::IdlTypeDocs => {
                for idl in &mut spec.idls {
                    for type_def in &mut idl.types {
                        type_def.docs = vec![hostile.to_string()];
                    }
                    for account in &mut idl.accounts {
                        account.docs = vec![hostile.to_string()];
                    }
                    for instruction in &mut idl.instructions {
                        instruction.docs = vec![hostile.to_string()];
                    }
                }
            }
        }
    }
}

fn ore_stack_spec() -> SerializableStackSpec {
    let manifest = load_stack_manifest_v2(include_bytes!(
        "../../stacks/ore/.arete/OreStream.stack-manifest.json"
    ))
    .unwrap()
    .artifact;
    let live = load_live_spec_v2(include_bytes!(
        "../../stacks/ore/.arete/OreStream.live-spec.json"
    ))
    .unwrap()
    .artifact;
    let programs = vec![
        load_program_spec(include_bytes!(
            "../../stacks/ore/.arete/ore.program-spec.json"
        ))
        .unwrap()
        .artifact,
        load_program_spec(include_bytes!(
            "../../stacks/ore/.arete/entropy.program-spec.json"
        ))
        .unwrap()
        .artifact,
    ];
    stack_spec_from_artifacts_v2(&programs, &live, &manifest).unwrap()
}

/// Generated files by language.
#[derive(Default)]
struct Generated {
    typescript: Vec<(String, String)>,
    rust: Vec<(String, String)>,
    python: Vec<(String, String)>,
}

/// Outcome of one generator on one case: `None` when it refused the input.
type Outcome = Option<Vec<(String, String)>>;

fn program_sdks(spec: &SerializableStackSpec) -> [Outcome; 3] {
    let typescript = typescript::compile_program_modules(spec.clone(), None)
        .ok()
        .map(|output| vec![("program.ts".to_string(), output.full_file())]);
    let rust = rust::compile_program_modules(spec.clone(), None)
        .ok()
        .map(|output| {
            vec![
                ("lib.rs".to_string(), output.lib_rs),
                ("types.rs".to_string(), output.types_rs),
                ("programs.rs".to_string(), output.programs_rs),
            ]
        });
    let python = python::compile_program_modules(spec.clone(), None)
        .ok()
        .map(|output| {
            vec![
                ("__init__.py".to_string(), output.init_py),
                ("models.py".to_string(), output.models_py),
                ("programs.py".to_string(), output.programs_py),
            ]
        });
    [typescript, rust, python]
}

fn stack_sdks(spec: &SerializableStackSpec) -> [Outcome; 3] {
    let typescript = typescript::compile_stack_spec(spec.clone(), None)
        .ok()
        .map(|output| vec![("stack.ts".to_string(), output.full_file())]);
    let rust = rust::compile_stack_spec(spec.clone(), None)
        .ok()
        .map(|output| {
            let mut files = vec![
                ("lib.rs".to_string(), output.lib_rs),
                ("types.rs".to_string(), output.types_rs),
                ("entity.rs".to_string(), output.entity_rs),
            ];
            files.extend(
                output
                    .programs_rs
                    .map(|programs| ("programs.rs".to_string(), programs)),
            );
            files
        });
    let python = python::compile_stack_spec(spec.clone(), None)
        .ok()
        .map(|output| {
            let mut files = vec![
                ("__init__.py".to_string(), output.init_py),
                ("models.py".to_string(), output.models_py),
                ("views.py".to_string(), output.views_py),
            ];
            files.extend(
                output
                    .programs_py
                    .map(|programs| ("programs.py".to_string(), programs)),
            );
            files
        });
    [typescript, rust, python]
}

/// Record the outcomes of one case and check them against its kind.
fn record(case: &str, kind: Kind, outcomes: [Outcome; 3], generated: &mut Generated) {
    for (language, outcome) in ["TypeScript", "Rust", "Python"].iter().zip(&outcomes) {
        match (kind, outcome) {
            (Kind::FreeText, None) => {
                panic!("{case}: the {language} generator refused free text it must escape")
            }
            (Kind::Identifier, Some(_)) => {
                panic!("{case}: the {language} generator accepted a name that is not an identifier")
            }
            _ => {}
        }
    }
    let [typescript, rust, python] = outcomes;
    let label = |files: Vec<(String, String)>| {
        files
            .into_iter()
            .map(|(file, source)| (format!("{case} / {file}"), source))
            .collect::<Vec<_>>()
    };
    generated
        .typescript
        .extend(typescript.map(label).unwrap_or_default());
    generated.rust.extend(rust.map(label).unwrap_or_default());
    generated
        .python
        .extend(python.map(label).unwrap_or_default());
}

#[test]
fn hostile_idl_and_stack_text_never_escapes_generated_code() {
    let mut rng = Rng(0x00A4_0499_5EED_1DEA);
    let mut generated = Generated::default();

    for position in IdlPosition::ALL {
        for _ in 0..IDL_CASES_PER_POSITION {
            let hostile = rng.hostile();
            let case = format!("IDL {position:?} = {hostile:?}");
            let outcomes = match build_program_only_stack_spec_from_idl_bytes(
                &idl(position, &hostile),
                None,
                "Demo",
            ) {
                Ok(spec) => program_sdks(&spec),
                // Refused before generation: every generator refused.
                Err(_) => [None, None, None],
            };
            record(&case, position.kind(), outcomes, &mut generated);
        }
    }

    let ore = ore_stack_spec();
    for position in StackPosition::ALL {
        for _ in 0..STACK_CASES_PER_POSITION {
            let hostile = rng.hostile();
            let case = format!("stack {position:?} = {hostile:?}");
            let mut spec = ore.clone();
            position.apply(&mut spec, &hostile);
            record(&case, position.kind(), stack_sdks(&spec), &mut generated);
        }
    }

    assert!(!generated.typescript.is_empty() && !generated.rust.is_empty());
    for (label, source) in &generated.rust {
        check_rust(label, source);
    }
    for (label, source) in &generated.typescript {
        check_lexed(label, typescript_code(source), source);
    }
    for (label, source) in &generated.python {
        check_lexed(label, python_code(source), source);
    }
    parse_python(&generated.python);
}

/// The free-text renderers escape rather than drop text: the payload is
/// still visible in the SDK, inside its comment or literal.
#[test]
fn hostile_docs_and_messages_are_kept_as_text() {
    let spec = build_program_only_stack_spec_from_idl_bytes(
        &idl(
            IdlPosition::ErrorMessage,
            "x\"); PWNED(0x1337); */ '''\"\"\"",
        ),
        None,
        "Demo",
    )
    .unwrap();
    let [typescript, rust, python] = program_sdks(&spec);
    for files in [typescript, rust, python] {
        let files = files.expect("free text must generate");
        assert!(
            files.iter().any(|(_, source)| source.contains("PWNED")),
            "the error message is kept"
        );
    }
}

fn excerpt(source: &str, needle: &str) -> String {
    let at = source.find(needle).unwrap_or(0);
    let start = source[..at]
        .char_indices()
        .rev()
        .nth(300)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let end = source[at..]
        .char_indices()
        .nth(300)
        .map(|(index, _)| at + index)
        .unwrap_or(source.len());
    source[start..end].to_string()
}

fn check_rust(label: &str, source: &str) {
    if let Err(error) = syn::parse_file(source) {
        panic!("{label}: generated Rust does not parse: {error}\n{source}");
    }
    let tokens: proc_macro2::TokenStream = source.parse().unwrap();
    fn walk(tokens: proc_macro2::TokenStream, label: &str, source: &str) {
        for token in tokens {
            if let proc_macro2::TokenTree::Group(group) = token {
                if group.delimiter() == proc_macro2::Delimiter::Parenthesis
                    && group.stream().to_string().trim() == "0x1337"
                {
                    panic!(
                        "{label}: the payload call reached Rust code:\n{}",
                        excerpt(source, "PWNED")
                    );
                }
                walk(group.stream(), label, source);
            }
        }
    }
    walk(tokens, label, source);
}

fn check_lexed(label: &str, code: Result<String, String>, source: &str) {
    let code =
        code.unwrap_or_else(|error| panic!("{label}: {error}:\n{}", excerpt(source, "PWNED")));
    assert!(
        !code.contains("(0x1337)"),
        "{label}: the payload call reached code:\n{}",
        excerpt(source, "PWNED")
    );
    let mut open = Vec::new();
    for character in code.chars() {
        match character {
            '(' | '[' | '{' => open.push(character),
            ')' | ']' | '}' => {
                let expected = match character {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                assert_eq!(
                    open.pop(),
                    Some(expected),
                    "{label}: unbalanced `{character}` in code:\n{}",
                    excerpt(source, "PWNED")
                );
            }
            _ => {}
        }
    }
    assert!(open.is_empty(), "{label}: unclosed {open:?}");
}

/// Parse (never run) the generated Python with `ast.parse`, when a
/// `python3` is available.
fn parse_python(files: &[(String, String)]) {
    let dir = std::env::temp_dir().join(format!("arete-hostile-codegen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut labels = BTreeMap::new();
    let paths: Vec<PathBuf> = files
        .iter()
        .enumerate()
        .map(|(index, (label, source))| {
            let path = dir.join(format!("case_{index}.py"));
            std::fs::write(&path, source).unwrap();
            labels.insert(path.display().to_string(), label.clone());
            path
        })
        .collect();
    let script = "import ast, sys\nfailed = False\nfor path in sys.argv[1:]:\n    try:\n        ast.parse(open(path, 'rb').read(), path)\n    except SyntaxError as error:\n        failed = True\n        print(path, error)\nsys.exit(1 if failed else 0)\n";
    let result = std::process::Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(script)
        .args(&paths)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Err(error) => {
            eprintln!("python3 unavailable ({error}); skipped ast.parse of generated Python")
        }
        Ok(output) => assert!(
            output.status.success(),
            "generated Python does not parse:\n{}\n{:?}",
            String::from_utf8_lossy(&output.stdout),
            labels
        ),
    }
}

/// Code of a TypeScript module with every comment and string literal blanked
/// out: what remains is what would run. Template literals keep their
/// `${...}` substitutions, which are code. Errors on a literal or comment
/// that is left open, or a string literal that spans a line.
fn typescript_code(source: &str) -> Result<String, String> {
    let chars: Vec<char> = source.chars().collect();
    let line_terminator =
        |character: char| matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let mut code = String::with_capacity(source.len());
    // Brace depth at which each open template substitution started.
    let mut substitutions: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    // Scan template text from `i` to its closing backtick (false) or the next
    // `${` (true).
    let scan_template = |i: &mut usize| -> Result<bool, String> {
        while *i < chars.len() {
            match chars[*i] {
                '\\' => *i += 2,
                '`' => {
                    *i += 1;
                    return Ok(false);
                }
                '$' if chars.get(*i + 1) == Some(&'{') => {
                    *i += 2;
                    return Ok(true);
                }
                _ => *i += 1,
            }
        }
        Err("unterminated template literal".to_string())
    };
    while i < chars.len() {
        let character = chars[i];
        let next = chars.get(i + 1).copied();
        if character == '/' && next == Some('/') {
            while i < chars.len() && !line_terminator(chars[i]) {
                i += 1;
            }
            code.push(' ');
        } else if character == '/' && next == Some('*') {
            i += 2;
            while !(chars.get(i) == Some(&'*') && chars.get(i + 1) == Some(&'/')) {
                if i >= chars.len() {
                    return Err("unterminated block comment".to_string());
                }
                i += 1;
            }
            i += 2;
            code.push(' ');
        } else if character == '\'' || character == '"' {
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("unterminated string literal".to_string()),
                    Some('\\') => i += 2,
                    Some(&quote) if quote == character => {
                        i += 1;
                        break;
                    }
                    Some(&other) if line_terminator(other) => {
                        return Err("line break inside a string literal".to_string())
                    }
                    Some(_) => i += 1,
                }
            }
            code.push_str("''");
        } else if character == '`' {
            i += 1;
            code.push_str("``");
            if scan_template(&mut i)? {
                depth += 1;
                substitutions.push(depth);
                code.push('(');
            }
        } else if character == '{' {
            depth += 1;
            code.push(character);
            i += 1;
        } else if character == '}' && substitutions.last() == Some(&depth) {
            substitutions.pop();
            depth -= 1;
            i += 1;
            code.push(')');
            if scan_template(&mut i)? {
                depth += 1;
                substitutions.push(depth);
                code.push('(');
            }
        } else {
            if character == '}' {
                depth = depth.saturating_sub(1);
            }
            code.push(character);
            i += 1;
        }
    }
    Ok(code)
}

/// Code of a Python module with comments and string literals blanked out.
/// f-strings are kept whole, since their replacement fields are code.
fn python_code(source: &str) -> Result<String, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut code = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let character = chars[i];
        if character == '#' {
            while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            code.push(' ');
            continue;
        }
        // A string prefix: up to two of r/b/u/f right before a quote, not
        // the end of a longer identifier.
        let starts_token = i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_');
        let mut start = i;
        while starts_token
            && start < chars.len()
            && start - i < 2
            && matches!(chars[start].to_ascii_lowercase(), 'r' | 'b' | 'u' | 'f')
        {
            start += 1;
        }
        if starts_token && start < chars.len() && matches!(chars[start], '\'' | '"') {
            let prefix: String = chars[i..start]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            let quote = chars[start];
            let triple =
                chars.get(start + 1) == Some(&quote) && chars.get(start + 2) == Some(&quote);
            let mut end = start + if triple { 3 } else { 1 };
            loop {
                match chars.get(end) {
                    None => return Err("unterminated string literal".to_string()),
                    Some('\\') => end += 2,
                    Some(&other) if other == quote => {
                        if !triple {
                            end += 1;
                            break;
                        }
                        if chars.get(end + 1) == Some(&quote) && chars.get(end + 2) == Some(&quote)
                        {
                            end += 3;
                            break;
                        }
                        end += 1;
                    }
                    Some(&other) if !triple && (other == '\n' || other == '\r') => {
                        return Err("line break inside a string literal".to_string())
                    }
                    Some(_) => end += 1,
                }
            }
            if prefix.contains('f') {
                code.extend(&chars[i..end]);
            } else {
                code.push_str("''");
            }
            i = end;
            continue;
        }
        code.push(character);
        i += 1;
    }
    Ok(code)
}
