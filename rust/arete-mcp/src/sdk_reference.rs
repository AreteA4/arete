//! The reference of one installed SDK: what it exposes that is specific to
//! it.
//!
//! `a4 install` builds an [`SdkReference`] for each SDK it generates, from the
//! metadata it generates the SDK from, and writes it into the SDK's folder as
//! [`REFERENCE_FILE`], beside the [`README_FILE`] that [`render_markdown`]
//! renders from it. `a4 sdk describe` and the `describe_sdk` MCP tool read
//! that file back and render it, or the part a [`Selection`] picks, with the
//! same functions, so the README, the CLI and the MCP tool always agree.
//!
//! A reference holds only what differs between SDKs: the module and export
//! to import, each entity's views and row fields (the TypeScript path, the
//! wire name, the type and the unit), the stack's `read.*` helpers, and a
//! summary of each program. How to use an Arete SDK in general (sessions,
//! auth, `get`/`getOne`/`use`/`watch`) is the same for every SDK, so the
//! reference points to the docs and agent skills for it instead.

use std::fmt::Write as _;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::field_amounts::AmountScale;

/// The reference's file in an SDK folder.
pub const REFERENCE_FILE: &str = "sdk-reference.json";

/// The README rendered from it, in the same folder.
pub const README_FILE: &str = "README.md";

/// The [`SdkReference::schema_version`] this crate writes and reads.
pub const SCHEMA_VERSION: u32 = 1;

/// The CLI command that prints a reference.
pub const DESCRIBE_COMMAND: &str = "a4 sdk describe";

/// The MCP tool that returns a reference.
pub const DESCRIBE_TOOL: &str = "describe_sdk";

/// What an SDK was generated from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SdkKind {
    Stack,
    Program,
}

impl SdkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SdkKind::Stack => "stack",
            SdkKind::Program => "program",
        }
    }
}

/// The language an SDK was generated in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SdkLanguage {
    Typescript,
    Rust,
    Python,
}

impl SdkLanguage {
    fn title(self) -> &'static str {
        match self {
            SdkLanguage::Typescript => "TypeScript",
            SdkLanguage::Rust => "Rust",
            SdkLanguage::Python => "Python",
        }
    }

    /// Where the docs explain how to use any SDK in this language.
    fn docs(self) -> &'static str {
        match self {
            SdkLanguage::Typescript => "https://docs.arete.run/sdks/typescript/",
            SdkLanguage::Rust => "https://docs.arete.run/sdks/rust/",
            SdkLanguage::Python => "https://docs.arete.run/sdks/python/",
        }
    }
}

/// Where the docs explain prepared operations and transactions.
const PROGRAM_DOCS: &str = "https://docs.arete.run/using-stacks/transactions/";

/// One installed SDK's reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SdkReference {
    pub schema_version: u32,
    pub kind: SdkKind,
    /// The dependency's alias in `arete.toml`.
    pub alias: String,
    pub language: SdkLanguage,
    /// The registry package it was installed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub import: SdkImport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityReference>,
    /// The stack extension's `read.*` helpers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<FunctionReference>,
    /// The stack extension's plain helpers: `defaults.limits`, or a
    /// namespace (`math`) it does not declare in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub helpers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<ProgramReference>,
}

/// What code imports, and where the definition is reached from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SdkImport {
    /// The entry module, relative to the SDK folder: `ore.ts`.
    pub module: String,
    /// The import specifier from the project root:
    /// `./generated/typescript/stacks/ore/ore.js`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specifier: Option<String>,
    /// The exported definition: `ORE_STREAM_STACK`.
    pub export: String,
    /// The module that declares the row types, relative to the SDK folder,
    /// when it is not the entry module (which re-exports them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types_module: Option<String>,
}

/// An entity: its views and the fields of its rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityReference {
    /// The entity name, which prefixes its view ids: `OreRound`.
    pub name: String,
    /// The row type: `OreRound`.
    pub type_name: String,
    pub views: Vec<ViewReference>,
    pub fields: Vec<FieldReference>,
}

/// One view of an entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewReference {
    /// `OreRound/latest`.
    pub id: String,
    pub kind: ViewKind,
    /// Its path on the stack: `views.OreRound.latest`.
    pub access: String,
    /// A state view's key: `{ roundId: bigint }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// How a view is read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewKind {
    /// One row per key.
    State,
    /// Rows in the view's order.
    List,
}

impl ViewKind {
    fn as_str(self) -> &'static str {
        match self {
            ViewKind::State => "state",
            ViewKind::List => "list",
        }
    }
}

/// One field of an entity's rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldReference {
    /// The field's path in the SDK's rows: `id.roundId`.
    pub path: String,
    /// Its path in raw frames and in CLI and MCP output: `id.round_id`.
    pub wire: String,
    /// Its type in the SDK, without null: `bigint`.
    #[serde(rename = "type")]
    pub ty: String,
    /// Whether it can be null.
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<AmountReference>,
}

/// The token-amount scale of a field, with the paths it refers to in the
/// SDK's naming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AmountReference {
    pub scale: AmountScale,
    /// The token's decimals, when the stack fixes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u64>,
    /// The field the decimals are read from, when the stack does not fix
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals_from: Option<String>,
    /// The field holding the same amount at the other scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<String>,
}

impl AmountReference {
    /// One phrase, e.g. `token amount in whole units (raw / 10^9)`.
    pub fn describe(&self) -> String {
        let divisor = self.divisor();
        let mut text = match self.scale {
            AmountScale::Ui => format!("token amount in whole units (raw / {divisor})"),
            AmountScale::Raw => {
                format!("token amount in raw base units (divide by {divisor} for whole units)")
            }
        };
        if let Some(counterpart) = &self.counterpart {
            let other = match self.scale {
                AmountScale::Ui => "raw",
                AmountScale::Raw => "whole units",
            };
            let _ = write!(text, "; {other}: {counterpart}");
        }
        text
    }

    /// The same, for a field table: `whole units (raw / 10^9)`.
    fn describe_short(&self) -> String {
        let divisor = self.divisor();
        let mut text = match self.scale {
            AmountScale::Ui => format!("whole units (raw / {divisor})"),
            AmountScale::Raw => format!("raw units (/ {divisor} for whole)"),
        };
        if let Some(counterpart) = &self.counterpart {
            let other = match self.scale {
                AmountScale::Ui => "raw",
                AmountScale::Raw => "whole",
            };
            let _ = write!(text, "; {other}: {counterpart}");
        }
        text
    }

    fn divisor(&self) -> String {
        match (self.decimals, &self.decimals_from) {
            (Some(decimals), _) => format!("10^{decimals}"),
            (None, Some(from)) => format!("10^{from}"),
            (None, None) => "10^decimals".to_string(),
        }
    }
}

/// A function an SDK exposes: a `read.*` helper or a program operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FunctionReference {
    /// Its path on the stack or program: `read.currentRound`,
    /// `instructions.mining.deploy`.
    pub path: String,
    /// Its parameters as declared: `roundId: bigint`.
    #[serde(default)]
    pub params: Vec<String>,
    /// What it resolves to, when the source says:
    /// `{ board, round, roundAddress, clock, phase } | null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl FunctionReference {
    /// `read.roundState(roundId: bigint)`.
    pub fn signature(&self) -> String {
        format!("{}({})", self.path, self.params.join(", "))
    }

    /// The last segment of its path: `currentRound`.
    pub fn name(&self) -> &str {
        self.path.rsplit('.').next().unwrap_or(&self.path)
    }
}

/// A program an SDK exposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReference {
    /// Its key under `programs`: `ore`.
    pub key: String,
    /// The program's IDL name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Its own SDK folder, relative to this SDK's: `programs/ore`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    /// The program extension's `read.*` helpers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<FunctionReference>,
    /// The program extension's operations: `instructions.*`,
    /// `transactions.*` and `flows.*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<FunctionReference>,
    /// Account types, fetched with `accounts.<Name>.fetch(address)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// IDL instructions, built with `raw.<name>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    /// PDAs, derived with `pdas.<name>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pdas: Vec<String>,
    /// The program extension's plain helpers: `addresses.var`,
    /// `math.finalValue`, or a namespace (`addresses`) it does not declare
    /// in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub helpers: Vec<String>,
}

/// The part of a reference to show. Everything when empty; otherwise the
/// union of what each filter matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// An entity (`OreRound`) or one of its views (`OreRound/latest`).
    pub view: Option<String>,
    /// A `read.*` helper or program operation, by name or path.
    pub read: Option<String>,
    /// A program, by key or name.
    pub program: Option<String>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.view.is_none() && self.read.is_none() && self.program.is_none()
    }
}

/// An `a4 sdk describe` or `describe_sdk` request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DescribeRequest {
    /// The dependency alias; `None` lists the installed SDKs.
    pub alias: Option<String>,
    /// The dependency kind, when an alias names both a stack and a program.
    pub kind: Option<SdkKind>,
    pub selection: Selection,
    /// JSON instead of Markdown.
    pub json: bool,
}

/// Where the references of installed SDKs come from: the project the
/// server runs in. `a4 mcp` provides the one `a4 sdk describe` uses, so the
/// tool and the command answer the same request the same way.
pub trait SdkReferenceSource: Send + Sync {
    fn describe(&self, request: &DescribeRequest) -> Result<String>;
}

impl SdkReference {
    /// Parse a [`REFERENCE_FILE`], refusing a schema this crate cannot read.
    pub fn from_json(contents: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(contents)?;
        let version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(SCHEMA_VERSION)) {
            bail!(
                "unsupported SDK reference schema {}; reinstall with this a4 (`a4 install`)",
                version.map_or_else(|| "(none)".to_string(), |version| version.to_string())
            );
        }
        Ok(serde_json::from_value(value)?)
    }

    /// The reference as written to [`REFERENCE_FILE`].
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("an SDK reference serializes")
        )
    }

    /// The program `key`, by key or IDL name.
    pub fn program(&self, key: &str) -> Option<&ProgramReference> {
        self.programs
            .iter()
            .find(|program| program.key == key)
            .or_else(|| {
                self.programs
                    .iter()
                    .find(|program| program.name.eq_ignore_ascii_case(key))
            })
    }

    /// The part of this reference `selection` picks: its import, and the
    /// entities, views, reads and programs it names. An error names what is
    /// available when a filter matches nothing.
    pub fn select(&self, selection: &Selection) -> Result<SdkReference> {
        if selection.is_empty() {
            return Ok(self.clone());
        }
        let mut selected = SdkReference {
            entities: Vec::new(),
            reads: Vec::new(),
            helpers: Vec::new(),
            programs: Vec::new(),
            ..self.clone()
        };
        if let Some(view) = &selection.view {
            selected.entities.push(self.select_view(view)?);
        }
        let program = selection
            .program
            .as_deref()
            .map(|key| {
                self.program(key).ok_or_else(|| {
                    anyhow::anyhow!(
                        "no program '{key}' in {} '{}'; programs: {}",
                        self.kind.as_str(),
                        self.alias,
                        list_or_none(self.programs.iter().map(|program| program.key.as_str()))
                    )
                })
            })
            .transpose()?;
        match (&selection.read, program) {
            (Some(read), Some(program)) => {
                selected
                    .programs
                    .push(select_program_function(program, read)?);
            }
            (Some(read), None) => self.select_read(read, &mut selected)?,
            (None, Some(program)) => selected.programs.push(program.clone()),
            (None, None) => {}
        }
        Ok(selected)
    }

    fn select_view(&self, view: &str) -> Result<EntityReference> {
        let (entity_name, view_name) = match view.split_once(['/', '.']) {
            Some((entity, view)) => (entity, Some(view)),
            None => (view, None),
        };
        let Some(entity) = self
            .entities
            .iter()
            .find(|entity| entity.name == entity_name)
            .or_else(|| {
                self.entities
                    .iter()
                    .find(|entity| entity.name.eq_ignore_ascii_case(entity_name))
            })
        else {
            bail!(
                "no entity '{entity_name}' in {} '{}'; views: {}",
                self.kind.as_str(),
                self.alias,
                list_or_none(
                    self.entities
                        .iter()
                        .flat_map(|entity| entity.views.iter().map(|view| view.id.as_str()))
                )
            );
        };
        let Some(view_name) = view_name else {
            return Ok(entity.clone());
        };
        let views = entity
            .views
            .iter()
            .filter(|view| {
                view.id
                    .split_once('/')
                    .is_some_and(|(_, name)| name.eq_ignore_ascii_case(view_name))
            })
            .cloned()
            .collect::<Vec<_>>();
        if views.is_empty() {
            bail!(
                "no view '{view}' in {} '{}'; {} views: {}",
                self.kind.as_str(),
                self.alias,
                entity.name,
                list_or_none(entity.views.iter().map(|view| view.id.as_str()))
            );
        }
        Ok(EntityReference {
            views,
            ..entity.clone()
        })
    }

    /// The stack read `read`, else the program read or operation of that
    /// name.
    fn select_read(&self, read: &str, selected: &mut SdkReference) -> Result<()> {
        if let Some(found) = find_function(&self.reads, read) {
            selected.reads.push(found.clone());
            return Ok(());
        }
        for program in &self.programs {
            if let Ok(program) = select_program_function(program, read) {
                selected.programs.push(program);
                return Ok(());
            }
        }
        let mut names = self
            .reads
            .iter()
            .map(FunctionReference::signature)
            .collect::<Vec<_>>();
        for program in &self.programs {
            names.extend(
                program
                    .reads
                    .iter()
                    .chain(&program.operations)
                    .map(|function| format!("programs.{}.{}", program.key, function.path)),
            );
        }
        bail!(
            "no read or operation '{read}' in {} '{}'; available: {}",
            self.kind.as_str(),
            self.alias,
            list_or_none(names.iter().map(String::as_str))
        )
    }
}

/// `program` with only the read or operation `name`.
fn select_program_function(program: &ProgramReference, name: &str) -> Result<ProgramReference> {
    let reads = find_function(&program.reads, name).cloned();
    let operations = if reads.is_none() {
        find_function(&program.operations, name).cloned()
    } else {
        None
    };
    if reads.is_none() && operations.is_none() {
        bail!(
            "no read or operation '{name}' in program '{}'; available: {}",
            program.key,
            list_or_none(
                program
                    .reads
                    .iter()
                    .chain(&program.operations)
                    .map(|function| function.path.as_str())
            )
        );
    }
    Ok(ProgramReference {
        reads: reads.into_iter().collect(),
        operations: operations.into_iter().collect(),
        accounts: Vec::new(),
        instructions: Vec::new(),
        pdas: Vec::new(),
        helpers: Vec::new(),
        ..program.clone()
    })
}

/// The function whose path or last segment is `name` (`read.` optional).
fn find_function<'a>(
    functions: &'a [FunctionReference],
    name: &str,
) -> Option<&'a FunctionReference> {
    let name = name.trim_end_matches("()");
    functions
        .iter()
        .find(|function| function.path == name || function.path == format!("read.{name}"))
        .or_else(|| functions.iter().find(|function| function.name() == name))
}

fn list_or_none<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names = names.collect::<Vec<_>>();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

/// The README of the SDK folder `reference` describes: everything in it.
pub fn render_markdown(reference: &SdkReference) -> String {
    let mut out = String::new();
    let title = match reference.kind {
        SdkKind::Stack => "SDK reference",
        SdkKind::Program => "program SDK reference",
    };
    let _ = writeln!(
        out,
        "# `{}` {} {title}\n",
        reference.alias,
        reference.language.title()
    );
    let _ = writeln!(out, "{}", provenance_line(reference));
    let _ = writeln!(
        out,
        "This file lists only what is specific to this SDK. For how to use any Arete SDK \
         ({}), see the {} or {}.",
        generic_topics(reference),
        skill_names(reference),
        generic_docs(reference)
    );
    let _ = writeln!(
        out,
        "Look parts up with `{DESCRIBE_COMMAND} {} [--view <Entity/view>] [--read <name>] \
         [--program <key>] [--json]` or the MCP tool `{DESCRIBE_TOOL}`.\n",
        reference.alias
    );
    render_body(reference, &mut out, true);
    out
}

/// What `a4 sdk describe` and `describe_sdk` print: the whole README, or
/// for a selection, the import and the parts selected.
pub fn render_selection(reference: &SdkReference, selection: &Selection) -> Result<String> {
    if selection.is_empty() {
        return Ok(render_markdown(reference));
    }
    let selected = reference.select(selection)?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# `{}` {} SDK reference (selection)\n",
        selected.alias,
        selected.language.title()
    );
    render_body(&selected, &mut out, false);
    Ok(out)
}

/// The README of the folder of program `key` inside a stack SDK.
pub fn render_program_markdown(reference: &SdkReference, key: &str) -> Option<String> {
    let program = reference
        .programs
        .iter()
        .find(|program| program.key == key)?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# `programs.{}` {} program SDK reference\n",
        program.key,
        reference.language.title()
    );
    let _ = writeln!(
        out,
        "{}Part of the `{}` stack SDK (`../../{README_FILE}`); `a4 install` regenerates it, so \
         do not edit it. Reach it as `{}`.",
        program_id_sentence(program),
        reference.alias,
        program_access(reference, program)
    );
    let _ = writeln!(
        out,
        "This file lists only what is specific to this program. For how to prepare, inspect and \
         execute operations, see the `arete-programs` agent skill or {PROGRAM_DOCS}.\n"
    );
    render_program_sections(program, &mut out, true, 2);
    Some(out)
}

fn provenance_line(reference: &SdkReference) -> String {
    let source = match (&reference.package, &reference.version) {
        (Some(package), Some(version)) => {
            format!("{} package `{package}` {version}", reference.kind.as_str())
        }
        (Some(package), None) => format!("{} package `{package}`", reference.kind.as_str()),
        _ => format!("local {}", reference.kind.as_str()),
    };
    format!(
        "Generated from {source}, installed as `{}`. `a4 install` regenerates this folder, so \
         do not edit it.",
        reference.alias
    )
}

fn skill_names(reference: &SdkReference) -> &'static str {
    match reference.kind {
        SdkKind::Stack if reference.programs.is_empty() => "`arete-streams` agent skill",
        SdkKind::Stack => "`arete-streams` and `arete-programs` agent skills",
        SdkKind::Program => "`arete-programs` agent skill",
    }
}

/// What the docs and skills cover that the reference leaves out.
fn generic_topics(reference: &SdkReference) -> &'static str {
    match reference.kind {
        SdkKind::Stack if reference.programs.is_empty() => {
            "`createSession`, auth, `get`/`getOne`/`use`/`watch`"
        }
        SdkKind::Stack => {
            "`createSession`, auth, `get`/`getOne`/`use`/`watch`, preparing and executing operations"
        }
        SdkKind::Program => "`createSession`, wallets, preparing and executing operations",
    }
}

fn generic_docs(reference: &SdkReference) -> &'static str {
    match (reference.kind, reference.language) {
        (SdkKind::Program, SdkLanguage::Typescript) => PROGRAM_DOCS,
        (_, language) => language.docs(),
    }
}

/// `Program `<id>`. `, when the program id is known.
fn program_id_sentence(program: &ProgramReference) -> String {
    program
        .program_id
        .as_deref()
        .map(|id| format!("Program `{id}`. "))
        .unwrap_or_default()
}

/// How code reaches the definition: `session.stacks.ore`.
fn root_access(reference: &SdkReference) -> String {
    let key = session_key(&reference.alias);
    match reference.kind {
        SdkKind::Stack => format!("session.stacks.{key}"),
        SdkKind::Program => format!("session.programs.{key}"),
    }
}

fn program_access(reference: &SdkReference, program: &ProgramReference) -> String {
    match reference.kind {
        SdkKind::Stack => format!("{}.programs.{}", root_access(reference), program.key),
        SdkKind::Program => root_access(reference),
    }
}

/// The key a session registers the SDK under: its alias, when that is an
/// identifier.
fn session_key(alias: &str) -> &str {
    let mut characters = alias.chars();
    let identifier = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'));
    if identifier {
        alias
    } else {
        "app"
    }
}

fn render_body(reference: &SdkReference, out: &mut String, full: bool) {
    render_import(reference, out, full);
    if !reference.entities.is_empty() {
        render_naming(reference, out);
        let _ = writeln!(out, "## Entities and views\n");
        for entity in &reference.entities {
            render_entity(reference, entity, out);
        }
    }
    if !reference.reads.is_empty() {
        let _ = writeln!(out, "## Reads\n");
        let _ = writeln!(
            out,
            "Derived values in one call, as `await {}.read.<name>(...)`.\n",
            root_access(reference)
        );
        let detail = if full { Detail::Summary } else { Detail::Full };
        for read in &reference.reads {
            render_function(read, out, detail);
        }
        out.push('\n');
    }
    if !reference.helpers.is_empty() {
        let _ = writeln!(
            out,
            "Helpers on `{}`: {}.\n",
            root_access(reference),
            code_list(&reference.helpers)
        );
    }
    if reference.kind == SdkKind::Program {
        for program in &reference.programs {
            let id = program_id_sentence(program);
            if !id.is_empty() {
                let _ = writeln!(out, "{}\n", id.trim_end());
            }
            render_program_sections(program, out, true, 2);
        }
        return;
    }
    if !reference.programs.is_empty() {
        let _ = writeln!(out, "## Programs\n");
        for program in &reference.programs {
            render_program(reference, program, out, !full, 3);
        }
    }
}

fn render_import(reference: &SdkReference, out: &mut String, full: bool) {
    let import = &reference.import;
    let _ = writeln!(out, "## Import\n");
    match reference.language {
        SdkLanguage::Typescript => {
            let specifier = import
                .specifier
                .clone()
                .unwrap_or_else(|| format!("./{}", typescript_specifier(&import.module)));
            let _ = writeln!(out, "```ts");
            let _ = writeln!(
                out,
                "import {{ {} }} from '{specifier}';{}",
                import.export,
                if import.specifier.is_some() {
                    " // from the project root"
                } else {
                    " // from this folder"
                }
            );
            let _ = writeln!(out, "```\n");
            let key = session_key(&reference.alias);
            let registration = match reference.kind {
                SdkKind::Stack => format!(
                    "`createSession({{ stacks: {{ {key}: {} }} }})` gives `{}`; in React, \
                     `useArete({})`",
                    import.export,
                    root_access(reference),
                    import.export
                ),
                SdkKind::Program => format!(
                    "`createSession({{ programs: {{ {key}: {} }} }})` gives `{}`",
                    import.export,
                    root_access(reference)
                ),
            };
            let _ = write!(
                out,
                "Register it as `{key}`: {registration}. Paths below start there."
            );
            if full && !reference.entities.is_empty() {
                let types = import.types_module.as_deref().unwrap_or(&import.module);
                let _ = write!(
                    out,
                    " Row types (`{}`) are exported from the same module (declared in `{types}`).",
                    reference
                        .entities
                        .iter()
                        .map(|entity| entity.type_name.as_str())
                        .collect::<Vec<_>>()
                        .join("`, `")
                );
            }
            out.push_str("\n\n");
        }
        SdkLanguage::Rust | SdkLanguage::Python => {
            let _ = writeln!(
                out,
                "Module `{}`, definition `{}`.\n",
                import.module, import.export
            );
        }
    }
}

/// `ore.ts` -> `ore.js`: TypeScript imports name the emitted file.
fn typescript_specifier(module: &str) -> String {
    match module.strip_suffix(".ts") {
        Some(stem) => format!("{stem}.js"),
        None => module.to_string(),
    }
}

fn render_naming(reference: &SdkReference, out: &mut String) {
    let _ = writeln!(out, "## Field names and types\n");
    let fields = || reference.entities.iter().flat_map(|entity| &entity.fields);
    // The example: a state view's key field, else an id, else the first
    // renamed field.
    let key_fields = reference
        .entities
        .iter()
        .flat_map(|entity| &entity.views)
        .filter_map(|view| view.key.as_deref())
        .filter_map(|key| key.trim_start_matches(['{', ' ']).split(':').next())
        .collect::<Vec<_>>();
    let example = fields()
        .find(|field| {
            field.path != field.wire
                && key_fields
                    .iter()
                    .any(|key| field.path.rsplit('.').next() == Some(*key))
        })
        .or_else(|| fields().find(|field| field.path != field.wire && field.path.ends_with("Id")))
        .or_else(|| fields().find(|field| field.path != field.wire));
    match (reference.language, example) {
        (SdkLanguage::Typescript, Some(example)) => {
            let _ = writeln!(
                out,
                "- Rows use the paths below (`{}`). `a4 get`, `a4 stream`, the MCP `read_view` \
                 tool and raw frames use the wire names, marked `←` (`{}`): a wire path in \
                 TypeScript reads `undefined`.",
                example.path, example.wire
            );
        }
        (SdkLanguage::Typescript, None) => {
            let _ = writeln!(
                out,
                "- Rows use the paths below, which match the wire names in `a4 get`, `a4 stream` \
                 and the MCP `read_view` tool."
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "- Fields use the wire names of `a4 get`, `a4 stream` and the MCP `read_view` tool."
            );
        }
    }
    if reference.language == SdkLanguage::Typescript
        && fields().any(|field| field.ty.contains("bigint"))
    {
        let _ = writeln!(
            out,
            "- `bigint` fields are 64-bit integers: compare them with `n` literals, and convert \
             them (`Number()`, `String()`) before mixing them with numbers or passing them to \
             `JSON.stringify`."
        );
    }
    if fields().any(|field| field.amount.is_some()) {
        let _ = writeln!(
            out,
            "- Token amounts say their scale: whole units (`1.5`) or raw integer base units \
             (`1500000000`), and the power of ten between them."
        );
    }
    let _ = writeln!(
        out,
        "- A nullable field is null until the stack has seen its data: check it rather than \
         defaulting it, so a wrong path does not pass silently.\n"
    );
}

fn render_entity(reference: &SdkReference, entity: &EntityReference, out: &mut String) {
    let _ = writeln!(out, "### {}\n", entity.name);
    let views = entity
        .views
        .iter()
        .map(|view| match &view.key {
            Some(key) => format!("`{}` ({}, key `{key}`)", view.access, view.kind.as_str()),
            None => format!("`{}` ({})", view.access, view.kind.as_str()),
        })
        .collect::<Vec<_>>();
    if !views.is_empty() {
        let _ = writeln!(out, "Views: {}.\n", views.join(", "));
    }
    if entity.fields.is_empty() {
        return;
    }
    let _ = writeln!(out, "Row `{}`:\n", entity.type_name);
    let types = entity
        .fields
        .iter()
        .map(|field| field_type(reference, field))
        .collect::<Vec<_>>();
    let path_width = entity
        .fields
        .iter()
        .map(|field| field.path.len())
        .max()
        .unwrap_or(0);
    let type_width = types.iter().map(String::len).max().unwrap_or(0);
    let _ = writeln!(out, "```text");
    for (field, ty) in entity.fields.iter().zip(&types) {
        let mut notes = Vec::new();
        if let Some(wire) = wire_note(field) {
            notes.push(format!("← {wire}"));
        }
        if let Some(amount) = &field.amount {
            notes.push(amount.describe_short());
        }
        let line = if notes.is_empty() {
            format!("{:path_width$}  {ty}", field.path)
        } else {
            format!(
                "{:path_width$}  {ty:type_width$}  {}",
                field.path,
                notes.join(" · ")
            )
        };
        let _ = writeln!(out, "{}", line.trim_end());
    }
    let _ = writeln!(out, "```\n");
}

/// A field's wire name, when it is not its path: only the last segment when
/// the rest is the same (`round_id` for `id.roundId`).
fn wire_note(field: &FieldReference) -> Option<&str> {
    if field.wire == field.path {
        return None;
    }
    match (field.wire.rsplit_once('.'), field.path.rsplit_once('.')) {
        (Some((wire_parent, wire_leaf)), Some((path_parent, _))) if wire_parent == path_parent => {
            Some(wire_leaf)
        }
        _ => Some(&field.wire),
    }
}

fn field_type(reference: &SdkReference, field: &FieldReference) -> String {
    match (reference.language, field.nullable) {
        (SdkLanguage::Typescript, true) => format!("{} | null", field.ty),
        (SdkLanguage::Rust, true) => format!("Option<{}>", field.ty),
        (SdkLanguage::Python, true) => format!("{} | None", field.ty),
        (_, false) => field.ty.clone(),
    }
}

/// How much of a function's documentation to show.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// Its title and return shape.
    Title,
    /// And the first sentence of its description.
    Summary,
    /// And all of its description.
    Full,
}

fn render_function(function: &FunctionReference, out: &mut String, detail: Detail) {
    let _ = write!(out, "- `{}`", function.signature());
    if let Some(title) = &function.title {
        let _ = write!(out, " — {title}.");
    }
    if let Some(returns) = &function.returns {
        let _ = write!(out, " Returns `{returns}`.");
    }
    if let Some(description) = &function.description {
        match detail {
            Detail::Title => {}
            Detail::Summary => {
                let _ = write!(out, " {}", first_sentence(description));
            }
            Detail::Full => {
                let _ = write!(out, " {description}");
            }
        }
    }
    out.push('\n');
}

/// Text up to the end of its first sentence.
fn code_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn first_sentence(text: &str) -> &str {
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices(". ") {
        // Not an abbreviation such as `e.g.`: the next word starts a sentence.
        if bytes
            .get(at + 2)
            .is_some_and(|next| next.is_ascii_uppercase() || *next == b'`')
            && !text[..at].ends_with("e.g")
            && !text[..at].ends_with("i.e")
        {
            return &text[..=at];
        }
    }
    text
}

/// A program of a stack SDK, under its own heading: in full (`detailed`:
/// every read and operation described), or as the summary the stack README
/// lists.
fn render_program(
    reference: &SdkReference,
    program: &ProgramReference,
    out: &mut String,
    detailed: bool,
    level: usize,
) {
    let heading = "#".repeat(level);
    match &program.program_id {
        Some(id) => {
            let _ = writeln!(
                out,
                "{heading} `programs.{}` (program `{id}`)\n",
                program.key
            );
        }
        None => {
            let _ = writeln!(out, "{heading} `programs.{}`\n", program.key);
        }
    }
    let _ = write!(out, "Reach it as `{}`.", program_access(reference, program));
    if !detailed {
        if let Some(directory) = &program.directory {
            let _ = write!(
                out,
                " Descriptions: `{directory}/{README_FILE}`, or `{DESCRIBE_COMMAND} {} --program {}`.",
                reference.alias, program.key
            );
        }
    }
    out.push_str("\n\n");
    render_program_sections(program, out, detailed, level + 1);
}

/// A program's reads, operations, accounts, raw instructions and PDAs,
/// under headings of `level`.
fn render_program_sections(
    program: &ProgramReference,
    out: &mut String,
    detailed: bool,
    level: usize,
) {
    let heading = "#".repeat(level);
    let detail = if detailed {
        Detail::Full
    } else {
        Detail::Title
    };
    if !program.reads.is_empty() {
        let _ = writeln!(out, "{heading} Reads\n");
        for read in &program.reads {
            render_function(read, out, detail);
        }
        out.push('\n');
    }
    if !program.operations.is_empty() {
        let _ = writeln!(
            out,
            "{heading} Operations\n\nEach returns a prepared operation to inspect or execute.\n"
        );
        for operation in &program.operations {
            render_function(operation, out, detail);
        }
        out.push('\n');
    }
    let lists = [
        (
            "Accounts",
            "accounts.<Name>.fetch(address)",
            &program.accounts,
        ),
        ("Raw instructions", "raw.<name>", &program.instructions),
        ("PDAs", "pdas.<name>", &program.pdas),
        ("Helpers", "", &program.helpers),
    ];
    if lists.iter().all(|(_, _, names)| names.is_empty()) {
        return;
    }
    let _ = writeln!(out, "{heading} Accounts and instructions\n");
    for (label, usage, names) in lists {
        if names.is_empty() {
            continue;
        }
        if usage.is_empty() {
            let _ = writeln!(out, "- {label}: {}", code_list(names));
        } else {
            let _ = writeln!(out, "- {label} (`{usage}`): {}", code_list(names));
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(path: &str, wire: &str, ty: &str) -> FieldReference {
        FieldReference {
            path: path.into(),
            wire: wire.into(),
            ty: ty.into(),
            nullable: true,
            amount: None,
        }
    }

    fn read(path: &str, params: &[&str], title: &str) -> FunctionReference {
        FunctionReference {
            path: path.into(),
            params: params.iter().map(|param| param.to_string()).collect(),
            returns: None,
            title: Some(title.into()),
            description: None,
        }
    }

    /// The shape `a4 install stack ore --ts` records for ORE.
    fn ore() -> SdkReference {
        let mut total_deployed = field("state.totalDeployed", "state.total_deployed", "number");
        total_deployed.amount = Some(AmountReference {
            scale: AmountScale::Ui,
            decimals: Some(9),
            decimals_from: None,
            counterpart: None,
        });
        let mut deployed = field(
            "state.deployedPerSquare",
            "state.deployed_per_square",
            "bigint[]",
        );
        deployed.amount = Some(AmountReference {
            scale: AmountScale::Raw,
            decimals: Some(9),
            decimals_from: None,
            counterpart: Some("state.deployedPerSquareUi".into()),
        });
        let mut current_round = read("read.currentRound", &[], "Current round");
        current_round.returns = Some("{ board, round, roundAddress, clock, phase } | null".into());
        current_round.description = Some(
            "Reads the Board entity and the chain clock together. Returns null when the Board \
             is missing."
                .into(),
        );
        SdkReference {
            schema_version: SCHEMA_VERSION,
            kind: SdkKind::Stack,
            alias: "ore".into(),
            language: SdkLanguage::Typescript,
            package: Some("ore".into()),
            version: Some("1.1.9".into()),
            import: SdkImport {
                module: "ore.ts".into(),
                specifier: Some("./generated/typescript/stacks/ore/ore.js".into()),
                export: "ORE_STREAM_STACK".into(),
                types_module: Some("ore-core.ts".into()),
            },
            entities: vec![
                EntityReference {
                    name: "OreRound".into(),
                    type_name: "OreRound".into(),
                    views: vec![
                        ViewReference {
                            id: "OreRound/state".into(),
                            kind: ViewKind::State,
                            access: "views.OreRound.state".into(),
                            key: Some("{ roundId: bigint }".into()),
                        },
                        ViewReference {
                            id: "OreRound/list".into(),
                            kind: ViewKind::List,
                            access: "views.OreRound.list".into(),
                            key: None,
                        },
                        ViewReference {
                            id: "OreRound/latest".into(),
                            kind: ViewKind::List,
                            access: "views.OreRound.latest".into(),
                            key: None,
                        },
                    ],
                    fields: vec![
                        field("id.roundAddress", "id.round_address", "string"),
                        field("id.roundId", "id.round_id", "bigint"),
                        deployed,
                        total_deployed,
                        field("state.motherlode", "state.motherlode", "number"),
                    ],
                },
                EntityReference {
                    name: "OreBoard".into(),
                    type_name: "OreBoard".into(),
                    views: vec![ViewReference {
                        id: "OreBoard/state".into(),
                        kind: ViewKind::State,
                        access: "views.OreBoard.state".into(),
                        key: Some("{ address: string }".into()),
                    }],
                    fields: vec![field("state.roundId", "state.round_id", "bigint")],
                },
            ],
            reads: vec![
                read("read.boardState", &[], "Board state"),
                read("read.roundState", &["roundId: bigint"], "Round state"),
                current_round,
            ],
            helpers: Vec::new(),
            programs: vec![ProgramReference {
                key: "ore".into(),
                name: "ore".into(),
                program_id: Some("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".into()),
                directory: Some("programs/ore".into()),
                reads: vec![read("read.board", &[], "Read board")],
                operations: vec![FunctionReference {
                    description: Some("Deploys SOL to the selected squares.".into()),
                    ..read(
                        "instructions.mining.deploy",
                        &["input: DeploySemanticInput"],
                        "Deploy to squares",
                    )
                }],
                accounts: vec!["Board".into(), "Round".into()],
                instructions: vec!["deploy".into(), "checkpoint".into()],
                pdas: vec!["board".into(), "round".into()],
                helpers: vec!["addresses".into(), "math.phase".into()],
            }],
        }
    }

    #[test]
    fn the_readme_lists_import_fields_with_wire_names_and_units_reads_and_programs() {
        let readme = render_markdown(&ore());

        assert!(readme.starts_with("# `ore` TypeScript SDK reference\n"));
        assert!(readme.contains("stack package `ore` 1.1.9, installed as `ore`"));
        assert!(readme.contains(
            "import { ORE_STREAM_STACK } from './generated/typescript/stacks/ore/ore.js'; // from the project root"
        ));
        assert!(readme.contains(
            "`createSession({ stacks: { ore: ORE_STREAM_STACK } })` gives `session.stacks.ore`"
        ));
        assert!(readme.contains("`arete-streams` and `arete-programs` agent skills"));
        assert!(readme.contains("a4 sdk describe ore"));
        assert!(readme.contains("`describe_sdk`"));
        // The naming rule, with this SDK's own state key as the example.
        assert!(readme.contains("Rows use the paths below (`id.roundId`)"));
        assert!(readme.contains("marked `←` (`id.round_id`)"));
        assert!(readme.contains("- Token amounts say their scale"));
        assert!(readme.contains("`bigint` fields are 64-bit integers"));
        assert!(readme.contains(
            "Views: `views.OreRound.state` (state, key `{ roundId: bigint }`), `views.OreRound.list` (list), `views.OreRound.latest` (list)."
        ));
        assert!(readme.contains("id.roundId               bigint | null    ← round_id\n"));
        assert!(readme.contains(
            "state.totalDeployed      number | null    ← total_deployed · whole units (raw / 10^9)\n"
        ));
        assert!(readme.contains(
            "state.deployedPerSquare  bigint[] | null  ← deployed_per_square · raw units (/ 10^9 for whole); whole: state.deployedPerSquareUi\n"
        ));
        // A field named the same on the wire has no note.
        assert!(readme.contains("\nstate.motherlode         number | null\n"));
        assert!(readme.contains(
            "Derived values in one call, as `await session.stacks.ore.read.<name>(...)`."
        ));
        // The README summarises a read's description in its first sentence.
        assert!(readme.contains(
            "- `read.currentRound()` — Current round. Returns `{ board, round, roundAddress, clock, phase } | null`. Reads the Board entity and the chain clock together.\n"
        ));
        assert!(readme.contains("- `read.roundState(roundId: bigint)` — Round state.\n"));
        assert!(readme.contains(
            "### `programs.ore` (program `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`)"
        ));
        assert!(readme.contains("Reach it as `session.stacks.ore.programs.ore`. Descriptions: `programs/ore/README.md`, or `a4 sdk describe ore --program ore`."));
        // The stack README summarises program operations without their descriptions.
        assert!(readme.contains(
            "- `instructions.mining.deploy(input: DeploySemanticInput)` — Deploy to squares.\n"
        ));
        assert!(!readme.contains("Deploys SOL"));
        assert!(readme.contains("- Accounts (`accounts.<Name>.fetch(address)`): `Board`, `Round`"));
        assert!(readme.contains("- Helpers: `addresses`, `math.phase`"));
        // Generic usage is left to the docs.
        assert!(!readme.contains("getOne("));
        assert!(!readme.contains(".watch("));
    }

    #[test]
    fn the_program_readme_describes_each_operation() {
        let readme = render_program_markdown(&ore(), "ore").unwrap();

        assert!(readme.starts_with("# `programs.ore` TypeScript program SDK reference\n"));
        assert!(readme.contains(
            "Program `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`. Part of the `ore` stack SDK"
        ));
        assert!(readme.contains("Reach it as `session.stacks.ore.programs.ore`."));
        assert!(readme.contains("\n## Operations\n"));
        assert!(readme.contains(
            "- `instructions.mining.deploy(input: DeploySemanticInput)` — Deploy to squares. Deploys SOL to the selected squares.\n"
        ));
        assert!(readme.contains("- PDAs (`pdas.<name>`): `board`, `round`"));
        assert!(render_program_markdown(&ore(), "entropy").is_none());
    }

    #[test]
    fn a_view_selection_keeps_its_entity_fields_and_only_that_view() {
        let selection = Selection {
            view: Some("OreRound/latest".into()),
            ..Selection::default()
        };
        let selected = ore().select(&selection).unwrap();

        assert_eq!(selected.entities.len(), 1);
        assert_eq!(selected.entities[0].views.len(), 1);
        assert_eq!(selected.entities[0].views[0].id, "OreRound/latest");
        assert_eq!(selected.entities[0].fields.len(), 5);
        assert!(selected.reads.is_empty() && selected.programs.is_empty());

        let text = render_selection(&ore(), &selection).unwrap();
        assert!(text.contains("Views: `views.OreRound.latest` (list)."));
        assert!(text.contains("← round_id"));
        assert!(!text.contains("## Reads"));
        // An entity alone selects all its views; names are matched loosely.
        let entity = ore()
            .select(&Selection {
                view: Some("oreround".into()),
                ..Selection::default()
            })
            .unwrap();
        assert_eq!(entity.entities[0].views.len(), 3);
    }

    #[test]
    fn a_read_selection_finds_stack_reads_then_program_functions() {
        let selection = Selection {
            read: Some("currentRound".into()),
            ..Selection::default()
        };
        let read = ore().select(&selection).unwrap();
        assert_eq!(read.reads.len(), 1);
        assert!(read.entities.is_empty() && read.programs.is_empty());
        // A selected read is described in full.
        assert!(render_selection(&ore(), &selection)
            .unwrap()
            .contains("together. Returns null when the Board is missing."));

        let operation = ore()
            .select(&Selection {
                read: Some("deploy".into()),
                ..Selection::default()
            })
            .unwrap();
        assert!(operation.reads.is_empty());
        assert_eq!(
            operation.programs[0].operations[0].path,
            "instructions.mining.deploy"
        );
        assert!(operation.programs[0].accounts.is_empty());

        let program = ore()
            .select(&Selection {
                program: Some("ore".into()),
                ..Selection::default()
            })
            .unwrap();
        let text = render_selection(
            &ore(),
            &Selection {
                program: Some("ore".into()),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(program.programs[0].accounts.len(), 2);
        assert!(text.contains("Deploys SOL to the selected squares."));
    }

    #[test]
    fn unknown_names_list_what_is_available() {
        let error = ore()
            .select(&Selection {
                view: Some("OreRound/oldest".into()),
                ..Selection::default()
            })
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("OreRound/state, OreRound/list, OreRound/latest"),
            "{error}"
        );

        let error = ore()
            .select(&Selection {
                read: Some("nextRound".into()),
                ..Selection::default()
            })
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("read.roundState(roundId: bigint)"),
            "{error}"
        );
        assert!(
            error.contains("programs.ore.instructions.mining.deploy"),
            "{error}"
        );

        let error = ore()
            .select(&Selection {
                program: Some("entropy".into()),
                ..Selection::default()
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("programs: ore"), "{error}");
    }

    #[test]
    fn a_wire_name_is_shortened_only_when_its_section_is_the_same() {
        assert_eq!(
            wire_note(&field("id.roundId", "id.round_id", "bigint")),
            Some("round_id")
        );
        assert_eq!(
            wire_note(&field("tokenInfo.mint", "token_info.mint", "string")),
            Some("token_info.mint")
        );
        assert_eq!(
            wire_note(&field("oreMetadata", "ore_metadata", "T")),
            Some("ore_metadata")
        );
        assert_eq!(
            wire_note(&field("state.motherlode", "state.motherlode", "number")),
            None
        );
        assert_eq!(
            first_sentence("Fetches the Board, e.g. `board`. Then more."),
            "Fetches the Board, e.g. `board`."
        );
    }

    #[test]
    fn the_json_file_round_trips_and_refuses_other_schemas() {
        let reference = ore();
        let json = reference.to_json();
        assert!(json.contains("\"wire\": \"id.round_id\""));
        assert!(json.contains("\"type\": \"bigint\""));
        assert_eq!(SdkReference::from_json(&json).unwrap(), reference);

        let newer = json.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
        assert!(SdkReference::from_json(&newer)
            .unwrap_err()
            .to_string()
            .contains("unsupported SDK reference schema 2"));
    }

    #[test]
    fn a_program_sdk_renders_its_program_at_the_top_level() {
        let stack = ore();
        let program = SdkReference {
            kind: SdkKind::Program,
            alias: "ore-program".into(),
            import: SdkImport {
                module: "ore-program.ts".into(),
                specifier: None,
                export: "ORE_PROGRAM".into(),
                types_module: None,
            },
            entities: Vec::new(),
            reads: Vec::new(),
            ..stack.clone()
        };
        let readme = render_markdown(&program);
        assert!(readme.starts_with("# `ore-program` TypeScript program SDK reference\n"));
        assert!(
            readme.contains("import { ORE_PROGRAM } from './ore-program.js'; // from this folder")
        );
        // Not an identifier, so registered under a placeholder key.
        assert!(readme.contains(
            "`createSession({ programs: { app: ORE_PROGRAM } })` gives `session.programs.app`"
        ));
        assert!(readme.contains("\nProgram `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`.\n"));
        assert!(readme.contains("\n## Operations\n"));
        assert!(readme.contains("Deploys SOL to the selected squares."));
        assert!(readme.contains(
            "`arete-programs` agent skill or https://docs.arete.run/using-stacks/transactions/"
        ));
        assert!(!readme.contains("getOne"));
        assert!(!readme.contains("## Field names"));
    }
}
