use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::Serialize;

use arete_idl::analysis::{
    build_account_index, classify_accounts, extract_pda_graph, extract_type_graph,
    find_account_usage, find_connections, find_links, AccountCategory, SeedKind,
};
use arete_idl::discriminator::compute_discriminator;
use arete_idl::parse::parse_idl_file;
use arete_idl::search::{search_idl, suggest_similar, IdlSection, MatchType, SearchResult};
use arete_idl::types::{
    IdlAccount, IdlField, IdlInstruction, IdlSpec, IdlType, IdlTypeArrayElement, IdlTypeDef,
    IdlTypeDefKind, IdlTypeDefinedInner,
};

/// Load and parse an IDL file, returning a clear error on failure.
fn load_idl(path: &Path) -> Result<IdlSpec> {
    parse_idl_file(path)
        .map_err(|e| anyhow::anyhow!("Failed to load IDL file '{}': {}", path.display(), e))
}

/// Print a value as pretty-printed JSON.
#[allow(dead_code)]
fn print_json<T: Serialize>(val: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(val)?);
    Ok(())
}

fn format_discriminator(bytes: &[u8]) -> String {
    let hex = bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", hex)
}

fn instruction_discriminator(ix: &IdlInstruction) -> Vec<u8> {
    if !ix.discriminator.is_empty() {
        return ix.discriminator.clone();
    }

    if let Some(disc) = &ix.discriminant {
        let value = disc.value as u8;
        return vec![value, 0, 0, 0, 0, 0, 0, 0];
    }

    compute_discriminator("global", &ix.name).to_vec()
}

fn account_discriminator(account: &IdlAccount) -> Vec<u8> {
    if !account.discriminator.is_empty() {
        return account.discriminator.clone();
    }

    compute_discriminator("account", &account.name).to_vec()
}

fn format_idl_type(ty: &IdlType) -> String {
    match ty {
        IdlType::Simple(s) => s.clone(),
        IdlType::Array(arr) => {
            if arr.array.len() == 2 {
                let item = match &arr.array[0] {
                    IdlTypeArrayElement::Nested(t) => format_idl_type(t),
                    IdlTypeArrayElement::Type(t) => t.clone(),
                    IdlTypeArrayElement::Size(n) => n.to_string(),
                };
                let size = match &arr.array[1] {
                    IdlTypeArrayElement::Size(n) => n.to_string(),
                    IdlTypeArrayElement::Nested(t) => format_idl_type(t),
                    IdlTypeArrayElement::Type(t) => t.clone(),
                };
                format!("[{}; {}]", item, size)
            } else {
                let parts = arr
                    .array
                    .iter()
                    .map(|el| match el {
                        IdlTypeArrayElement::Nested(t) => format_idl_type(t),
                        IdlTypeArrayElement::Type(t) => t.clone(),
                        IdlTypeArrayElement::Size(n) => n.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("array({})", parts)
            }
        }
        IdlType::Option(o) => format!("Option<{}>", format_idl_type(&o.option)),
        IdlType::Vec(v) => format!("Vec<{}>", format_idl_type(&v.vec)),
        IdlType::HashMap(m) => format!(
            "HashMap<{}, {}>",
            format_idl_type(&m.hash_map.0),
            format_idl_type(&m.hash_map.1)
        ),
        IdlType::Defined(d) => match &d.defined {
            IdlTypeDefinedInner::Named { name } => name.clone(),
            IdlTypeDefinedInner::Simple(name) => name.clone(),
        },
    }
}

fn type_kind_label(kind: &IdlTypeDefKind) -> &'static str {
    match kind {
        IdlTypeDefKind::Struct { .. } => "Struct",
        IdlTypeDefKind::TupleStruct { .. } => "TupleStruct",
        IdlTypeDefKind::Enum { .. } => "Enum",
    }
}

fn type_member_count(kind: &IdlTypeDefKind) -> usize {
    match kind {
        IdlTypeDefKind::Struct { fields, .. } => fields.len(),
        IdlTypeDefKind::TupleStruct { fields, .. } => fields.len(),
        IdlTypeDefKind::Enum { variants, .. } => variants.len(),
    }
}

fn account_field_count(account: &IdlAccount) -> usize {
    account
        .type_def
        .as_ref()
        .map(type_member_count)
        .unwrap_or(0)
}

fn find_instruction<'a>(idl: &'a IdlSpec, name: &str) -> Option<&'a IdlInstruction> {
    idl.instructions
        .iter()
        .find(|ix| ix.name.eq_ignore_ascii_case(name))
}

fn find_account<'a>(idl: &'a IdlSpec, name: &str) -> Option<&'a IdlAccount> {
    idl.accounts
        .iter()
        .find(|account| account.name.eq_ignore_ascii_case(name))
}

fn find_type<'a>(idl: &'a IdlSpec, name: &str) -> Option<&'a IdlTypeDef> {
    idl.types
        .iter()
        .find(|type_def| type_def.name.eq_ignore_ascii_case(name))
}

fn not_found_error(section: &str, name: &str, candidates: &[String]) -> anyhow::Error {
    let candidate_refs = candidates.iter().map(String::as_str).collect::<Vec<_>>();
    if let Some(suggestion) = suggest_similar(name, &candidate_refs, 3).first() {
        anyhow::anyhow!(
            "{} '{}' not found, did you mean: {}?",
            section,
            name,
            suggestion.candidate
        )
    } else {
        anyhow::anyhow!("{} '{}' not found", section, name)
    }
}

fn print_fields(fields: &[IdlField]) {
    if fields.is_empty() {
        println!("  {}", "(none)".dimmed());
        return;
    }

    for field in fields {
        println!(
            "  {:<30} {}",
            field.name.green(),
            format_idl_type(&field.type_).cyan()
        );
    }
}

#[derive(Serialize)]
struct SearchResultJson {
    name: String,
    section: String,
    match_type: String,
}

#[derive(Serialize)]
struct AccountRelationJson {
    account_name: String,
    matched_type: Option<String>,
    instruction_count: usize,
    category: String,
}

#[derive(Serialize)]
struct InstructionUsageJson {
    instruction_name: String,
    writable: bool,
    signer: bool,
    readonly: bool,
    pda: bool,
}

#[derive(Serialize)]
struct InstructionLinkJson {
    instruction_name: String,
    account_a_writable: bool,
    account_b_writable: bool,
}

#[derive(Serialize)]
struct PdaSeedInfoJson {
    kind: String,
    value: String,
}

#[derive(Serialize)]
struct PdaNodeJson {
    account_name: String,
    instruction_name: String,
    seeds: Vec<PdaSeedInfoJson>,
}

#[derive(Serialize)]
struct PubkeyFieldRefJson {
    field_name: String,
    likely_target: Option<String>,
}

#[derive(Serialize)]
struct TypeNodeJson {
    type_name: String,
    pubkey_fields: Vec<PubkeyFieldRefJson>,
}

#[derive(Serialize)]
struct AccountRoleJson {
    writable: bool,
    signer: bool,
    pda: bool,
}

#[derive(Serialize)]
struct InstructionContextJson {
    instruction_name: String,
    from_role: AccountRoleJson,
    to_role: AccountRoleJson,
    all_accounts: Vec<String>,
}

#[derive(Serialize)]
struct DirectConnectionJson {
    from: String,
    to: String,
    instructions: Vec<InstructionContextJson>,
}

#[derive(Serialize)]
struct TransitiveConnectionJson {
    from: String,
    intermediary: String,
    to: String,
    hop1_instruction: String,
    hop2_instruction: String,
}

#[derive(Serialize)]
struct InvalidExistingJson {
    account: String,
    suggestions: Vec<String>,
}

#[derive(Serialize)]
struct ConnectionReportJson {
    new_account: String,
    direct: Vec<DirectConnectionJson>,
    transitive: Vec<TransitiveConnectionJson>,
    invalid_existing: Vec<InvalidExistingJson>,
}

fn format_section(section: &IdlSection) -> String {
    match section {
        IdlSection::Instruction => "instruction".to_string(),
        IdlSection::Account => "account".to_string(),
        IdlSection::Type => "type".to_string(),
        IdlSection::Error => "error".to_string(),
        IdlSection::Event => "event".to_string(),
        IdlSection::Constant => "constant".to_string(),
    }
}

fn format_match_type(mt: &MatchType) -> String {
    match mt {
        MatchType::Exact => "exact".to_string(),
        MatchType::CaseInsensitive => "case-insensitive".to_string(),
        MatchType::Contains => "contains".to_string(),
        MatchType::Fuzzy(d) => format!("fuzzy({})", d),
    }
}

fn format_account_category(category: &AccountCategory) -> &'static str {
    match category {
        AccountCategory::Entity => "Entity",
        AccountCategory::Infrastructure => "Infrastructure",
        AccountCategory::Role => "Role",
        AccountCategory::Other => "Other",
    }
}

fn format_seed_kind(kind: &SeedKind) -> &'static str {
    match kind {
        SeedKind::Const => "Const",
        SeedKind::Account => "Account",
        SeedKind::Arg => "Arg",
    }
}

fn collect_account_names(idl: &IdlSpec) -> Vec<String> {
    let mut names = build_account_index(idl).into_keys().collect::<Vec<_>>();
    names.sort();
    names
}

fn resolve_account_name<'a>(name: &str, candidates: &'a [String]) -> Result<&'a str> {
    candidates
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .map(String::as_str)
        .ok_or_else(|| not_found_error("account", name, candidates))
}

fn collect_instruction_usage(idl: &IdlSpec, account_name: &str) -> Vec<InstructionUsageJson> {
    let mut usage = Vec::new();
    for ix in &idl.instructions {
        if let Some(account) = ix
            .accounts
            .iter()
            .find(|account| account.name == account_name)
        {
            usage.push(InstructionUsageJson {
                instruction_name: ix.name.clone(),
                writable: account.is_mut,
                signer: account.is_signer,
                readonly: !account.is_mut && !account.is_signer,
                pda: account.pda.is_some(),
            });
        }
    }
    usage
}

fn format_account_role_flags(writable: bool, signer: bool, pda: bool) -> String {
    format!(
        "writable={}, signer={}, pda={}",
        writable.to_string().cyan(),
        signer.to_string().cyan(),
        pda.to_string().cyan()
    )
}

#[derive(Parser)]
#[command(about = "Inspect and analyze Anchor/Shank IDL files")]
pub struct IdlArgs {
    #[command(subcommand)]
    pub command: IdlCommands,
}

#[derive(Subcommand)]
pub enum IdlCommands {
    /// Show a high-level summary of the IDL (instruction count, accounts, types, etc.)
    Summary {
        /// Path to the IDL JSON file
        path: PathBuf,
    },

    /// List all instructions
    Instructions {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show details for a single instruction
    Instruction {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Instruction name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all account types
    Accounts {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show details for a single account type
    Account {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Account name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all type definitions
    Types {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show details for a single type definition
    Type {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Type name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all error codes
    Errors {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all events
    Events {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all constants
    Constants {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Fuzzy-search instructions, accounts, types, and errors
    Search {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Search query
        query: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compute the Anchor discriminator for an instruction or account
    Discriminator {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Instruction or account name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show cross-instruction account relationships
    Relations {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show which instructions use a given account
    AccountUsage {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Account name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show instructions that link two accounts together
    Links {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// First account name
        a: String,

        /// Second account name
        b: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Extract PDA seed graph from instructions
    PdaGraph {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Extract type-level pubkey reference graph
    TypeGraph {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Find how a new account connects to existing accounts
    Connect {
        /// Path to the IDL JSON file
        path: PathBuf,

        /// New account name to connect
        new_account: String,

        /// Existing account names (comma-separated)
        #[arg(long, value_delimiter = ',')]
        existing: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Suggest Arete integration points
        #[arg(long)]
        suggest_hs: bool,
    },
}

pub fn run(args: IdlArgs) -> Result<()> {
    match args.command {
        IdlCommands::Summary { ref path } => {
            let idl = load_idl(path)?;
            let format = if idl.address.is_some() {
                "modern"
            } else {
                "legacy"
            };
            let address = idl
                .address
                .as_deref()
                .or_else(|| idl.metadata.as_ref().and_then(|m| m.address.as_deref()))
                .unwrap_or("-");

            println!("{}", "IDL Summary".bold());
            println!("  {} {}", "Name:".bold(), idl.get_name().green());
            println!("  {} {}", "Format:".bold(), format.cyan());
            println!("  {} {}", "Address:".bold(), address.yellow());
            println!("  {} {}", "Version:".bold(), idl.get_version().cyan());
            println!();
            println!("{}", "Counts".bold());
            println!("  {:<14} {}", "Instructions".bold(), idl.instructions.len());
            println!("  {:<14} {}", "Accounts".bold(), idl.accounts.len());
            println!("  {:<14} {}", "Types".bold(), idl.types.len());
            println!("  {:<14} {}", "Events".bold(), idl.events.len());
            println!("  {:<14} {}", "Errors".bold(), idl.errors.len());
            println!("  {:<14} {}", "Constants".bold(), idl.constants.len());
        }
        IdlCommands::Instructions { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.instructions);
            }

            println!("{}", "Instructions".bold());
            println!(
                "  {:<32} {:>8} {:>6}  {}",
                "Name".bold(),
                "Accounts".bold(),
                "Args".bold(),
                "Discriminator".bold()
            );
            println!("  {}", "-".repeat(76).dimmed());
            for ix in &idl.instructions {
                let disc = format_discriminator(&instruction_discriminator(ix));
                println!(
                    "  {:<32} {:>8} {:>6}  {}",
                    ix.name.green(),
                    ix.accounts.len().to_string().cyan(),
                    ix.args.len().to_string().cyan(),
                    disc.yellow()
                );
            }
        }
        IdlCommands::Instruction {
            ref path,
            ref name,
            json,
        } => {
            let idl = load_idl(path)?;
            let ix = find_instruction(&idl, name).ok_or_else(|| {
                let candidates = idl
                    .instructions
                    .iter()
                    .map(|it| it.name.clone())
                    .collect::<Vec<_>>();
                not_found_error("instruction", name, &candidates)
            })?;

            if json {
                return print_json(ix);
            }

            println!("{} {}", "Instruction:".bold(), ix.name.green().bold());
            println!(
                "  {} {}",
                "Discriminator:".bold(),
                format_discriminator(&instruction_discriminator(ix)).yellow()
            );
            println!();
            println!("{}", "Accounts".bold());
            if ix.accounts.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                println!(
                    "  {:<26} {:<8} {:<8} {}",
                    "Name".bold(),
                    "Writable".bold(),
                    "Signer".bold(),
                    "PDA".bold()
                );
                println!("  {}", "-".repeat(70).dimmed());
                for account in &ix.accounts {
                    let pda = if account.pda.is_some() { "yes" } else { "no" };
                    println!(
                        "  {:<26} {:<8} {:<8} {}",
                        account.name.green(),
                        account.is_mut.to_string().cyan(),
                        account.is_signer.to_string().cyan(),
                        pda.yellow()
                    );
                }
            }

            println!();
            println!("{}", "Args".bold());
            print_fields(&ix.args);
        }
        IdlCommands::Accounts { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.accounts);
            }

            println!("{}", "Accounts".bold());
            println!(
                "  {:<32} {:>8}  {}",
                "Name".bold(),
                "Fields".bold(),
                "Discriminator".bold()
            );
            println!("  {}", "-".repeat(70).dimmed());
            for account in &idl.accounts {
                println!(
                    "  {:<32} {:>8}  {}",
                    account.name.green(),
                    account_field_count(account).to_string().cyan(),
                    format_discriminator(&account_discriminator(account)).yellow()
                );
            }
        }
        IdlCommands::Account {
            ref path,
            ref name,
            json,
        } => {
            let idl = load_idl(path)?;
            let account = find_account(&idl, name).ok_or_else(|| {
                let candidates = idl
                    .accounts
                    .iter()
                    .map(|it| it.name.clone())
                    .collect::<Vec<_>>();
                not_found_error("account", name, &candidates)
            })?;

            if json {
                return print_json(account);
            }

            println!("{} {}", "Account:".bold(), account.name.green().bold());
            println!(
                "  {} {}",
                "Discriminator:".bold(),
                format_discriminator(&account_discriminator(account)).yellow()
            );
            println!("{}", "Fields".bold());
            match &account.type_def {
                Some(IdlTypeDefKind::Struct { fields, .. }) => print_fields(fields),
                Some(IdlTypeDefKind::TupleStruct { fields, .. }) => {
                    if fields.is_empty() {
                        println!("  {}", "(none)".dimmed());
                    } else {
                        for (idx, field_type) in fields.iter().enumerate() {
                            println!(
                                "  {:<30} {}",
                                format!("field_{}", idx).green(),
                                format_idl_type(field_type).cyan()
                            );
                        }
                    }
                }
                Some(IdlTypeDefKind::Enum { variants, .. }) => {
                    if variants.is_empty() {
                        println!("  {}", "(none)".dimmed());
                    } else {
                        for variant in variants {
                            println!("  {}", variant.name.green());
                        }
                    }
                }
                None => println!("  {}", "(no embedded fields in IDL account type)".dimmed()),
            }
        }
        IdlCommands::Types { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.types);
            }

            println!("{}", "Types".bold());
            println!(
                "  {:<32} {:<12} {}",
                "Name".bold(),
                "Kind".bold(),
                "Fields/Variants".bold()
            );
            println!("  {}", "-".repeat(60).dimmed());
            for type_def in &idl.types {
                println!(
                    "  {:<32} {:<12} {}",
                    type_def.name.green(),
                    type_kind_label(&type_def.type_def).cyan(),
                    type_member_count(&type_def.type_def).to_string().yellow()
                );
            }
        }
        IdlCommands::Type {
            ref path,
            ref name,
            json,
        } => {
            let idl = load_idl(path)?;
            let type_def = find_type(&idl, name).ok_or_else(|| {
                let candidates = idl
                    .types
                    .iter()
                    .map(|it| it.name.clone())
                    .collect::<Vec<_>>();
                not_found_error("type", name, &candidates)
            })?;

            if json {
                return print_json(type_def);
            }

            println!("{} {}", "Type:".bold(), type_def.name.green().bold());
            println!(
                "  {} {}",
                "Kind:".bold(),
                type_kind_label(&type_def.type_def).cyan()
            );
            match &type_def.type_def {
                IdlTypeDefKind::Struct { fields, .. } => {
                    println!("{}", "Fields".bold());
                    print_fields(fields);
                }
                IdlTypeDefKind::TupleStruct { fields, .. } => {
                    println!("{}", "Tuple Fields".bold());
                    if fields.is_empty() {
                        println!("  {}", "(none)".dimmed());
                    } else {
                        for (idx, field_type) in fields.iter().enumerate() {
                            println!(
                                "  {:<30} {}",
                                format!("item_{}", idx).green(),
                                format_idl_type(field_type).cyan()
                            );
                        }
                    }
                }
                IdlTypeDefKind::Enum { variants, .. } => {
                    println!("{}", "Variants".bold());
                    if variants.is_empty() {
                        println!("  {}", "(none)".dimmed());
                    } else {
                        for variant in variants {
                            println!("  {}", variant.name.green());
                        }
                    }
                }
            }
        }
        IdlCommands::Errors { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.errors);
            }

            println!("{}", "Errors".bold());
            if idl.errors.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                println!(
                    "  {:<8} {:<32} {}",
                    "Code".bold(),
                    "Name".bold(),
                    "Message".bold()
                );
                println!("  {}", "-".repeat(70).dimmed());
                for error in &idl.errors {
                    let msg = error.msg.as_deref().unwrap_or("-");
                    println!(
                        "  {:<8} {:<32} {}",
                        error.code.to_string().cyan(),
                        error.name.green(),
                        msg.yellow()
                    );
                }
            }
        }
        IdlCommands::Events { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.events);
            }

            println!("{}", "Events".bold());
            if idl.events.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                println!("  {:<32} {}", "Name".bold(), "Discriminator".bold());
                println!("  {}", "-".repeat(60).dimmed());
                for event in &idl.events {
                    let disc = format_discriminator(&event.get_discriminator());
                    println!("  {:<32} {}", event.name.green(), disc.yellow());
                }
            }
        }
        IdlCommands::Constants { ref path, json } => {
            let idl = load_idl(path)?;
            if json {
                return print_json(&idl.constants);
            }

            println!("{}", "Constants".bold());
            if idl.constants.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                println!(
                    "  {:<32} {:<16} {}",
                    "Name".bold(),
                    "Type".bold(),
                    "Value".bold()
                );
                println!("  {}", "-".repeat(70).dimmed());
                for constant in &idl.constants {
                    println!(
                        "  {:<32} {:<16} {}",
                        constant.name.green(),
                        format_idl_type(&constant.type_).cyan(),
                        constant.value.yellow()
                    );
                }
            }
        }
        IdlCommands::Search {
            ref path,
            ref query,
            json,
        } => {
            let idl = load_idl(path)?;
            let results = search_idl(&idl, query);

            if json {
                let json_results: Vec<SearchResultJson> = results
                    .iter()
                    .map(|r| SearchResultJson {
                        name: r.name.clone(),
                        section: format_section(&r.section),
                        match_type: format_match_type(&r.match_type),
                    })
                    .collect();
                return print_json(&json_results);
            }

            if results.is_empty() {
                println!("  {} '{}'", "No results found for".dimmed(), query);
            } else {
                // Group by section
                let sections = [
                    ("Instructions", IdlSection::Instruction),
                    ("Accounts", IdlSection::Account),
                    ("Types", IdlSection::Type),
                    ("Errors", IdlSection::Error),
                    ("Events", IdlSection::Event),
                    ("Constants", IdlSection::Constant),
                ];
                for (label, section) in &sections {
                    let section_results: Vec<&SearchResult> = results
                        .iter()
                        .filter(|r| {
                            std::mem::discriminant(&r.section) == std::mem::discriminant(section)
                        })
                        .collect();
                    if !section_results.is_empty() {
                        println!("{}", label.bold());
                        for r in &section_results {
                            println!(
                                "  {} {}",
                                r.name.green(),
                                format!("({})", format_match_type(&r.match_type)).dimmed()
                            );
                        }
                        println!();
                    }
                }
            }
        }
        IdlCommands::Discriminator {
            ref path,
            ref name,
            json,
        } => {
            let idl = load_idl(path)?;

            #[derive(Serialize)]
            struct DiscriminatorResult {
                name: String,
                namespace: String,
                hex: String,
                bytes: Vec<u8>,
            }

            let mut results: Vec<DiscriminatorResult> = Vec::new();

            // Check instructions
            if let Some(ix) = find_instruction(&idl, name) {
                let disc = instruction_discriminator(ix);
                results.push(DiscriminatorResult {
                    name: ix.name.clone(),
                    namespace: "global".to_string(),
                    hex: format_discriminator(&disc),
                    bytes: disc,
                });
            }

            // Check accounts
            if let Some(acc) = find_account(&idl, name) {
                let disc = account_discriminator(acc);
                results.push(DiscriminatorResult {
                    name: acc.name.clone(),
                    namespace: "account".to_string(),
                    hex: format_discriminator(&disc),
                    bytes: disc,
                });
            }

            if results.is_empty() {
                let mut candidates: Vec<String> =
                    idl.instructions.iter().map(|ix| ix.name.clone()).collect();
                candidates.extend(idl.accounts.iter().map(|a| a.name.clone()));
                return Err(not_found_error("instruction or account", name, &candidates));
            }

            if json {
                return print_json(&results);
            }

            for r in &results {
                println!("{} {}", "Name:".bold(), r.name.green());
                println!("{} {}", "Namespace:".bold(), r.namespace.cyan());
                println!("{} {}", "Discriminator:".bold(), r.hex.yellow());
                println!("{} {:?}", "Bytes:".bold(), r.bytes);
                println!();
            }
        }
        IdlCommands::Relations { ref path, json } => {
            let idl = load_idl(path)?;
            let mut relations = classify_accounts(&idl);
            relations.sort_by(|a, b| a.account_name.cmp(&b.account_name));

            if json {
                let out = relations
                    .iter()
                    .map(|relation| AccountRelationJson {
                        account_name: relation.account_name.clone(),
                        matched_type: relation.matched_type.clone(),
                        instruction_count: relation.instruction_count,
                        category: format_account_category(&relation.category).to_string(),
                    })
                    .collect::<Vec<_>>();
                return print_json(&out);
            }

            println!("{}", "Account Relations".bold());
            println!(
                "  {:<32} {:<16} {}",
                "Account".bold(),
                "Category".bold(),
                "Instruction Count".bold()
            );
            println!("  {}", "-".repeat(72).dimmed());
            for relation in &relations {
                println!(
                    "  {:<32} {:<16} {}",
                    relation.account_name.green(),
                    format_account_category(&relation.category).cyan(),
                    relation.instruction_count.to_string().yellow()
                );
            }
        }
        IdlCommands::AccountUsage {
            ref path,
            ref name,
            json,
        } => {
            let idl = load_idl(path)?;
            let candidates = collect_account_names(&idl);
            let account_name = resolve_account_name(name, &candidates)?;
            let _usage_summary = find_account_usage(&idl, account_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Account '{}' exists in index but no usage was found",
                    account_name
                )
            })?;
            let usage = collect_instruction_usage(&idl, account_name);

            if json {
                return print_json(&usage);
            }

            println!(
                "{} {}",
                "Account Usage:".bold(),
                account_name.green().bold()
            );
            println!(
                "  {} {}",
                "Total Instructions:".bold(),
                usage.len().to_string().cyan()
            );

            let writable = usage
                .iter()
                .filter(|entry| entry.writable)
                .map(|entry| entry.instruction_name.as_str())
                .collect::<Vec<_>>();
            let signer = usage
                .iter()
                .filter(|entry| entry.signer)
                .map(|entry| entry.instruction_name.as_str())
                .collect::<Vec<_>>();
            let readonly = usage
                .iter()
                .filter(|entry| entry.readonly)
                .map(|entry| entry.instruction_name.as_str())
                .collect::<Vec<_>>();

            println!();
            println!(
                "{} ({})",
                "Writable".bold(),
                writable.len().to_string().cyan()
            );
            if writable.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for instruction_name in &writable {
                    println!("  {}", instruction_name.green());
                }
            }

            println!();
            println!("{} ({})", "Signer".bold(), signer.len().to_string().cyan());
            if signer.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for instruction_name in &signer {
                    println!("  {}", instruction_name.green());
                }
            }

            println!();
            println!(
                "{} ({})",
                "Readonly".bold(),
                readonly.len().to_string().cyan()
            );
            if readonly.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for instruction_name in &readonly {
                    println!("  {}", instruction_name.green());
                }
            }
        }
        IdlCommands::Links {
            ref path,
            ref a,
            ref b,
            json,
        } => {
            let idl = load_idl(path)?;
            let candidates = collect_account_names(&idl);
            let account_a = resolve_account_name(a, &candidates)?;
            let account_b = resolve_account_name(b, &candidates)?;
            let links = find_links(&idl, account_a, account_b);

            if json {
                let out = links
                    .iter()
                    .map(|link| InstructionLinkJson {
                        instruction_name: link.instruction_name.clone(),
                        account_a_writable: link.account_a_writable,
                        account_b_writable: link.account_b_writable,
                    })
                    .collect::<Vec<_>>();
                return print_json(&out);
            }

            println!(
                "{} {} {} {}",
                "Links between".bold(),
                account_a.green().bold(),
                "and".bold(),
                account_b.green().bold()
            );
            if links.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                println!(
                    "  {:<32} {:<14} {}",
                    "Instruction".bold(),
                    format!("{} Writable", account_a).bold(),
                    format!("{} Writable", account_b).bold()
                );
                println!("  {}", "-".repeat(72).dimmed());
                for link in &links {
                    println!(
                        "  {:<32} {:<14} {}",
                        link.instruction_name.green(),
                        link.account_a_writable.to_string().cyan(),
                        link.account_b_writable.to_string().cyan()
                    );
                }
            }
        }
        IdlCommands::PdaGraph { ref path, json } => {
            let idl = load_idl(path)?;
            let graph = extract_pda_graph(&idl);

            if json {
                let out = graph
                    .iter()
                    .map(|node| PdaNodeJson {
                        account_name: node.account_name.clone(),
                        instruction_name: node.instruction_name.clone(),
                        seeds: node
                            .seeds
                            .iter()
                            .map(|seed| PdaSeedInfoJson {
                                kind: format_seed_kind(&seed.kind).to_string(),
                                value: seed.value.clone(),
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                return print_json(&out);
            }

            println!("{}", "PDA Graph".bold());
            if graph.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for node in &graph {
                    println!(
                        "  {} {} {}",
                        node.account_name.green().bold(),
                        "in".dimmed(),
                        node.instruction_name.cyan()
                    );
                    if node.seeds.is_empty() {
                        println!("    {}", "(no seeds)".dimmed());
                    } else {
                        for seed in &node.seeds {
                            println!(
                                "    - {:<8} {}",
                                format_seed_kind(&seed.kind).yellow(),
                                seed.value
                            );
                        }
                    }
                }
            }
        }
        IdlCommands::TypeGraph { ref path, json } => {
            let idl = load_idl(path)?;
            let graph = extract_type_graph(&idl);

            if json {
                let out = graph
                    .iter()
                    .map(|node| TypeNodeJson {
                        type_name: node.type_name.clone(),
                        pubkey_fields: node
                            .pubkey_fields
                            .iter()
                            .map(|field| PubkeyFieldRefJson {
                                field_name: field.field_name.clone(),
                                likely_target: field.likely_target.clone(),
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                return print_json(&out);
            }

            println!("{}", "Type Graph".bold());
            if graph.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for node in &graph {
                    println!("  {}", node.type_name.green().bold());
                    for field in &node.pubkey_fields {
                        let target = field.likely_target.as_deref().unwrap_or("?");
                        println!(
                            "    - {:<24} {} {}",
                            field.field_name.cyan(),
                            "->".dimmed(),
                            target.yellow()
                        );
                    }
                }
            }
        }
        IdlCommands::Connect {
            ref path,
            ref new_account,
            ref existing,
            json,
            suggest_hs,
        } => {
            let idl = load_idl(path)?;
            let candidates = collect_account_names(&idl);
            let resolved_new = resolve_account_name(new_account, &candidates)?;

            let mut valid_existing = Vec::new();
            let mut invalid_existing: Vec<(String, Vec<String>)> = Vec::new();
            for name in existing {
                if let Some(canonical) = candidates
                    .iter()
                    .find(|candidate| candidate.eq_ignore_ascii_case(name))
                {
                    if !valid_existing.iter().any(|entry| entry == canonical) {
                        valid_existing.push(canonical.clone());
                    }
                } else {
                    eprintln!("Warning: account '{}' not found in IDL, skipping", name);
                    let candidate_refs = candidates.iter().map(String::as_str).collect::<Vec<_>>();
                    let suggestions = suggest_similar(name, &candidate_refs, 3)
                        .iter()
                        .map(|s| s.candidate.clone())
                        .collect::<Vec<_>>();
                    invalid_existing.push((name.clone(), suggestions));
                }
            }

            let existing_refs = valid_existing
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let report = find_connections(&idl, resolved_new, &existing_refs);

            let mut invalid_all = invalid_existing;
            for (name, suggestions) in &report.invalid_existing {
                if !invalid_all
                    .iter()
                    .any(|(existing_name, _)| existing_name == name)
                {
                    invalid_all.push((name.clone(), suggestions.clone()));
                }
            }

            if json {
                let out = ConnectionReportJson {
                    new_account: report.new_account.clone(),
                    direct: report
                        .direct
                        .iter()
                        .map(|connection| DirectConnectionJson {
                            from: connection.from.clone(),
                            to: connection.to.clone(),
                            instructions: connection
                                .instructions
                                .iter()
                                .map(|instruction| InstructionContextJson {
                                    instruction_name: instruction.instruction_name.clone(),
                                    from_role: AccountRoleJson {
                                        writable: instruction.from_role.writable,
                                        signer: instruction.from_role.signer,
                                        pda: instruction.from_role.pda,
                                    },
                                    to_role: AccountRoleJson {
                                        writable: instruction.to_role.writable,
                                        signer: instruction.to_role.signer,
                                        pda: instruction.to_role.pda,
                                    },
                                    all_accounts: instruction.all_accounts.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                    transitive: report
                        .transitive
                        .iter()
                        .map(|connection| TransitiveConnectionJson {
                            from: connection.from.clone(),
                            intermediary: connection.intermediary.clone(),
                            to: connection.to.clone(),
                            hop1_instruction: connection.hop1_instruction.clone(),
                            hop2_instruction: connection.hop2_instruction.clone(),
                        })
                        .collect(),
                    invalid_existing: invalid_all
                        .iter()
                        .map(|(account, suggestions)| InvalidExistingJson {
                            account: account.clone(),
                            suggestions: suggestions.clone(),
                        })
                        .collect(),
                };
                return print_json(&out);
            }

            println!(
                "{} {}",
                "Connect: new account".bold(),
                report.new_account.green().bold()
            );
            println!(
                "  {} {}",
                "Valid existing inputs:".bold(),
                valid_existing.len().to_string().cyan()
            );
            if !valid_existing.is_empty() {
                println!("  {}", valid_existing.join(", ").green());
            }

            println!();
            println!("{}", "Direct Connections".bold());
            if report.direct.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for connection in &report.direct {
                    println!("  {} {}", "Existing: ".bold(), connection.to.green().bold());
                    for instruction in &connection.instructions {
                        println!(
                            "    - {} {}",
                            instruction.instruction_name.cyan(),
                            format_account_role_flags(
                                instruction.from_role.writable,
                                instruction.from_role.signer,
                                instruction.from_role.pda
                            )
                        );
                    }
                }
            }

            println!();
            println!("{}", "Transitive Connections".bold());
            if report.transitive.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for connection in &report.transitive {
                    println!(
                        "  {} {} {} {} {}",
                        connection.from.green(),
                        format!("--({})-->", connection.hop1_instruction).dimmed(),
                        connection.intermediary.yellow(),
                        format!("--({})-->", connection.hop2_instruction).dimmed(),
                        connection.to.green()
                    );
                }
            }

            if !invalid_all.is_empty() {
                println!();
                println!("{}", "Invalid Existing Accounts".bold());
                for (name, suggestions) in &invalid_all {
                    if suggestions.is_empty() {
                        println!("  - {} {}", name.red(), "(no suggestions)".dimmed());
                    } else {
                        println!(
                            "  - {} {} {}",
                            name.red(),
                            "->".dimmed(),
                            suggestions.join(", ").yellow()
                        );
                    }
                }
            }

            if suggest_hs {
                let mut register_from = Vec::new();
                let mut aggregate = Vec::new();

                for connection in &report.direct {
                    for instruction in &connection.instructions {
                        if instruction.from_role.writable
                            && !register_from
                                .iter()
                                .any(|name: &String| name == &instruction.instruction_name)
                        {
                            register_from.push(instruction.instruction_name.clone());
                        }
                        if instruction.from_role.signer
                            && !aggregate
                                .iter()
                                .any(|name: &String| name == &instruction.instruction_name)
                        {
                            aggregate.push(instruction.instruction_name.clone());
                        }
                    }
                }

                register_from.sort();
                aggregate.sort();

                println!();
                println!("{}", "Arete Suggestions".bold());
                if register_from.is_empty() && aggregate.is_empty() {
                    println!("  {}", "(none)".dimmed());
                } else {
                    for instruction_name in &register_from {
                        println!(
                            "  {} {}",
                            "register_from:".green().bold(),
                            instruction_name.cyan()
                        );
                    }
                    for instruction_name in &aggregate {
                        println!(
                            "  {} {}",
                            "aggregate:".green().bold(),
                            instruction_name.cyan()
                        );
                    }
                }
                println!(
                    "  {}",
                    "These are Arete-specific integration suggestions. Use `--suggest-a4` to see them.".dimmed()
                );
            }
        }
    }

    Ok(())
}
