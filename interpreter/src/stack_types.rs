//! One declaration per IDL-resolved type in a stack SDK.
//!
//! The SDK generators name each entity's resolved types (the IDL structs and
//! enums its fields map, e.g. `Header` or `PlanTerms`) entity by entity, but a
//! stack SDK declares every entity in one module (TypeScript `-core.ts`, Rust
//! `types.rs`, Python `models.py`). When several entities map the same IDL
//! type, or two programs' IDLs define types with the same name, the module
//! would declare that name more than once, or silently type one program's
//! fields with the other program's definition.
//!
//! [`StackResolvedTypes`] records every resolved type the stack has declared,
//! with its normalized definition ([`resolved_type_shape`]), and settles each
//! later entity's choice of name ([`StackResolvedTypes::claim`]):
//!
//! - a name no earlier entity declared stays as the entity chose it;
//! - a name an earlier entity declared with the same definition is shared:
//!   the entity references that declaration instead of repeating it;
//! - a name an earlier entity declared with a different definition is
//!   disambiguated deterministically with the entity's program name, then its
//!   entity name: `<Program><Type>`, `<Entity><Type>`, `<Entity><Type>2`, ...
//!   (an identical declaration under one of those names is shared too).
//!
//! Stacks whose entities never declare the same name twice generate exactly
//! what they did before.

use crate::ast::{
    BaseType, IdlAccountSnapshot, IdlArrayElementSnapshot, IdlDefinedInnerSnapshot,
    IdlEnumVariantFieldSnapshot, IdlFieldSnapshot, IdlSnapshot, IdlTypeDefKindSnapshot,
    IdlTypeDefSnapshot, IdlTypeSnapshot, IntegerKind, ResolvedField, ResolvedStructType,
    SerializableStreamSpec,
};
use crate::identifiers::{typescript as ts_ident, IdentifierCase};
use crate::idl_models::{DeclaredModel, IdlModel};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// How an entity refers to one of its resolved types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedTypeClaim {
    /// The entity declares the type under this name.
    Declare(String),
    /// An earlier entity already declared this identical type under this
    /// name; reference it without declaring it again.
    Shared(String),
}

impl ResolvedTypeClaim {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Declare(name) | Self::Shared(name) => name,
        }
    }

    pub(crate) fn into_name(self) -> String {
        match self {
            Self::Declare(name) | Self::Shared(name) => name,
        }
    }

    pub(crate) fn is_shared(&self) -> bool {
        matches!(self, Self::Shared(_))
    }
}

/// The resolved types declared so far in one stack module.
#[derive(Debug, Clone, Default)]
pub(crate) struct StackResolvedTypes {
    /// Declared name -> normalized definition.
    declared: BTreeMap<String, String>,
    /// Names the module declares for something else (entities, sections,
    /// runtime envelopes). Only consulted when a colliding type needs a new
    /// name, so they never change a name an entity chose on its own.
    reserved: HashSet<String>,
}

impl StackResolvedTypes {
    pub(crate) fn reserve(&mut self, names: impl IntoIterator<Item = String>) {
        self.reserved.extend(names);
    }

    pub(crate) fn is_declared(&self, name: &str) -> bool {
        self.declared.contains_key(name)
    }

    /// Whether the module already declares `name`, for a resolved type or
    /// anything else.
    pub(crate) fn is_taken(&self, name: &str) -> bool {
        self.declared.contains_key(name) || self.reserved.contains(name)
    }

    /// Record that the module declares `resolved` as `name`. The first
    /// declaration of a name wins.
    pub(crate) fn declare(&mut self, name: &str, resolved: &ResolvedStructType) {
        self.declared
            .entry(name.to_string())
            .or_insert_with(|| resolved_type_shape(resolved));
    }

    /// Settle the name for `resolved`, which the entity's own naming chose as
    /// `chosen` (already recorded in `taken`, the names the entity uses).
    ///
    /// `base_name` is the type's own name in the target language and
    /// `namespaces` the disambiguating prefixes, most preferred first
    /// (the program name, then the entity name).
    pub(crate) fn claim(
        &self,
        resolved: &ResolvedStructType,
        chosen: String,
        base_name: &str,
        namespaces: &[String],
        taken: &mut HashSet<String>,
    ) -> ResolvedTypeClaim {
        let Some(declared) = self.declared.get(&chosen) else {
            return ResolvedTypeClaim::Declare(chosen);
        };
        let shape = resolved_type_shape(resolved);
        if *declared == shape {
            return ResolvedTypeClaim::Shared(chosen);
        }

        let mut namespaces = namespaces
            .iter()
            .filter(|namespace| !namespace.is_empty())
            .collect::<Vec<_>>();
        namespaces.dedup();
        let numbered_stem = namespaces
            .last()
            .map(|namespace| format!("{namespace}{base_name}"))
            .unwrap_or_else(|| base_name.to_string());
        let candidates = namespaces
            .iter()
            .map(|namespace| format!("{namespace}{base_name}"))
            .chain((2usize..).map(|index| format!("{numbered_stem}{index}")));

        for candidate in candidates {
            if taken.contains(&candidate) || self.reserved.contains(&candidate) {
                continue;
            }
            match self.declared.get(&candidate) {
                Some(declared) if *declared == shape => {
                    taken.insert(candidate.clone());
                    return ResolvedTypeClaim::Shared(candidate);
                }
                Some(_) => continue,
                None => {
                    taken.insert(candidate.clone());
                    return ResolvedTypeClaim::Declare(candidate);
                }
            }
        }
        unreachable!("the numbered candidates are unbounded")
    }
}

/// The name of the program an entity's data comes from: the IDL whose
/// program id is the entity's, else the IDL the entity embeds.
pub(crate) fn entity_program_name<'a>(
    entity: &'a SerializableStreamSpec,
    idls: &'a [IdlSnapshot],
) -> Option<&'a str> {
    entity
        .program_id
        .as_deref()
        .and_then(|program_id| {
            idls.iter()
                .find(|idl| idl.program_id.as_deref() == Some(program_id))
        })
        .or(entity.idl.as_ref())
        .map(|idl| idl.name.as_str())
}

/// The IDL an entity's types resolve against: the one whose program id is the
/// entity's, else the snapshot the entity embeds, else the stack's only IDL.
///
/// A multi-IDL stack never falls back, because `idls.first()` types every
/// entity against whichever program happens to be declared first. A
/// single-program stack is unambiguous, so an entity that names no program
/// still resolves against it.
pub(crate) fn entity_idl<'a>(
    entity: &'a SerializableStreamSpec,
    idls: &'a [IdlSnapshot],
) -> Option<&'a IdlSnapshot> {
    entity
        .program_id
        .as_deref()
        .and_then(|program_id| {
            idls.iter()
                .find(|idl| idl.program_id.as_deref() == Some(program_id))
        })
        .or(entity.idl.as_ref())
        .or(match idls {
            [only] => Some(only),
            _ => None,
        })
}

/// The generated models of IDL account types, for typed account readers.
///
/// A program's account type is read into the model the program's own
/// entities map it to; that may be a renamed model when another program
/// defines a different type with the same name.
///
/// Every account of a program's IDL gets a model
/// ([`crate::idl_models::bind_idl_models`]): the one its entities already
/// map when that declares the same fields, else a model derived from the
/// IDL. So does every defined type those models reach. Each also has a
/// *stable name*, the name the program's standalone SDK gives it
/// ([`stable_account_model_names`]), which a program package extension
/// bundle uses in either SDK.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccountModels {
    /// `(program, account type)` -> model the program's entities use.
    by_program: BTreeMap<(String, String), String>,
    /// Declared model name -> its definition (first declaration).
    models: BTreeMap<String, DeclaredModel>,
    /// Names declared models occupy besides their own (the variant classes
    /// of a Python enum model).
    occupied: BTreeSet<String>,
    /// Program -> `(account, model, stable name)` for every IDL account, in
    /// IDL order.
    programs: BTreeMap<String, Vec<ProgramAccountModel>>,
    /// Program -> `(defined type, model, stable name)` for every defined
    /// type its account models reach, in declaration order.
    types: BTreeMap<String, Vec<ProgramTypeModel>>,
}

/// One IDL account of a program and the model it reads into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramAccountModel {
    /// The account's IDL name.
    pub(crate) account: String,
    /// The model declared in the SDK's shared types module (`types.rs` /
    /// `models.py`); `None` for an account read as raw JSON (enum layouts).
    pub(crate) model: Option<String>,
    /// The name the program's standalone SDK declares the model under.
    pub(crate) stable: String,
}

/// One IDL defined type a program's account models reach, and its model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramTypeModel {
    /// The type's IDL name.
    pub(crate) type_name: String,
    /// The model declared in the SDK's shared types module.
    pub(crate) model: String,
    /// The name the program's standalone SDK declares the model under.
    pub(crate) stable: String,
}

impl AccountModels {
    /// Record that the SDK declares `resolved` as `name` (first wins).
    pub(crate) fn declare_model(&mut self, name: &str, resolved: &ResolvedStructType) {
        self.models
            .entry(name.to_string())
            .or_insert_with(|| DeclaredModel::Resolved(resolved.clone()));
    }

    /// Record that the SDK declares the IDL-derived `model` as `name`,
    /// occupying the `extra` names too (first wins).
    pub(crate) fn declare_idl_model(&mut self, name: &str, model: &IdlModel, extra: Vec<String>) {
        self.models
            .entry(name.to_string())
            .or_insert_with(|| DeclaredModel::Idl(model.clone()));
        self.occupied.extend(extra);
    }

    /// The definition declared as `name`.
    pub(crate) fn model(&self, name: &str) -> Option<&DeclaredModel> {
        self.models.get(name)
    }

    /// Whether a declared model occupies `name` besides its own.
    pub(crate) fn is_occupied(&self, name: &str) -> bool {
        self.occupied.contains(name)
    }

    /// The model `program`'s own entities map `account` to, if any (no
    /// cross-program fallback).
    pub(crate) fn own(&self, program: &str, account: &str) -> Option<&String> {
        self.by_program
            .get(&(program.to_string(), account.to_string()))
            .or_else(|| {
                self.by_program
                    .iter()
                    .find(|((owner, name), _)| {
                        owner == program && name.eq_ignore_ascii_case(account)
                    })
                    .map(|(_, model)| model)
            })
    }

    /// Bind `program`'s `account` to `model` under its stable name.
    pub(crate) fn bind(&mut self, program: &str, account: &str, model: Option<&str>, stable: &str) {
        if let Some(model) = model {
            self.by_program.insert(
                (program.to_string(), account.to_string()),
                model.to_string(),
            );
        }
        self.programs
            .entry(program.to_string())
            .or_default()
            .push(ProgramAccountModel {
                account: account.to_string(),
                model: model.map(str::to_string),
                stable: stable.to_string(),
            });
    }

    /// Bind `program`'s defined type `type_name` to `model` under its stable
    /// name.
    pub(crate) fn bind_type(&mut self, program: &str, type_name: &str, model: &str, stable: &str) {
        self.types
            .entry(program.to_string())
            .or_default()
            .push(ProgramTypeModel {
                type_name: type_name.to_string(),
                model: model.to_string(),
                stable: stable.to_string(),
            });
    }

    /// Every IDL account of `program` with its model, in IDL order.
    pub(crate) fn program_accounts(&self, program: &str) -> &[ProgramAccountModel] {
        self.programs
            .get(program)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every defined type `program`'s account models reach, with its model.
    pub(crate) fn program_types(&self, program: &str) -> &[ProgramTypeModel] {
        self.types
            .get(program)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// `(stable name, declared model)` for every model of `program` (its
    /// accounts', then its defined types'), each stable name once. These are
    /// the names its standalone SDK declares.
    pub(crate) fn program_model_names(&self, program: &str) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        self.program_accounts(program)
            .iter()
            .filter_map(|account| Some((account.stable.clone(), account.model.clone()?)))
            .chain(
                self.program_types(program)
                    .iter()
                    .map(|model| (model.stable.clone(), model.model.clone())),
            )
            .filter(|(stable, _)| seen.insert(stable.clone()))
            .collect()
    }

    /// Record that an entity of `program` maps `account_type` to `model`.
    pub(crate) fn record(&mut self, program: Option<&str>, account_type: &str, model: &str) {
        if let Some(program) = program {
            self.by_program
                .entry((program.to_string(), account_type.to_string()))
                .or_insert_with(|| model.to_string());
        }
    }
}

/// The fields an IDL account's data decodes into: its own fields, else the
/// struct (or tuple struct) the IDL declares under the account's name, the
/// shape Anchor IDLs use. `None` for an account whose layout is an enum.
pub(crate) fn idl_account_field_snapshots(
    idl: &IdlSnapshot,
    account: &IdlAccountSnapshot,
) -> Option<Vec<IdlFieldSnapshot>> {
    if !account.fields.is_empty() {
        return Some(account.fields.clone());
    }
    let type_def = idl
        .types
        .iter()
        .find(|type_def| type_def.name == account.name)
        .or_else(|| {
            idl.types
                .iter()
                .find(|type_def| type_def.name.eq_ignore_ascii_case(&account.name))
        });
    match type_def.map(|type_def| &type_def.type_def) {
        None => Some(Vec::new()),
        Some(IdlTypeDefKindSnapshot::Struct { fields, .. }) => Some(fields.clone()),
        Some(IdlTypeDefKindSnapshot::TupleStruct { fields, .. }) => Some(
            fields
                .iter()
                .enumerate()
                .map(|(index, type_)| IdlFieldSnapshot {
                    name: format!("_{index}"),
                    type_: type_.clone(),
                    amount_hint: None,
                })
                .collect(),
        ),
        Some(IdlTypeDefKindSnapshot::Enum { .. }) => None,
    }
}

/// An IDL account in the flat convention the stack generators use for the
/// accounts entities capture (the resolved types `arete-macros` records):
/// scalars keep their integer kind, fixed arrays and vectors their element
/// type, and nested defined types, tuples and maps stay JSON values. Tests
/// use it as an entity's captured type; account readers decode into the
/// typed [`crate::idl_models::IdlModel`] instead. `None` for an enum layout.
#[cfg(test)]
pub(crate) fn idl_account_model(
    idl: &IdlSnapshot,
    account: &IdlAccountSnapshot,
) -> Option<ResolvedStructType> {
    Some(ResolvedStructType {
        type_name: account.name.clone(),
        fields: idl_account_field_snapshots(idl, account)?
            .iter()
            .map(resolved_idl_field)
            .collect(),
        is_instruction: false,
        is_account: true,
        is_event: false,
        is_enum: false,
        enum_variants: Vec::new(),
    })
}

pub(crate) fn resolved_idl_field(field: &IdlFieldSnapshot) -> ResolvedField {
    let shape = idl_field_shape(&field.type_);
    ResolvedField {
        field_name: field.name.clone(),
        raw_name: Some(field.name.clone()),
        canonical_name: Some(crate::ast::to_camel_case_owned(&field.name)),
        field_type: shape.field_type,
        base_type: shape.base_type,
        integer_kind: shape.integer_kind,
        is_optional: shape.is_optional,
        is_array: shape.is_array,
    }
}

struct IdlFieldShape {
    field_type: String,
    base_type: BaseType,
    integer_kind: Option<IntegerKind>,
    is_optional: bool,
    is_array: bool,
}

fn simple_idl_base_type(name: &str) -> (BaseType, Option<IntegerKind>) {
    if let Some(kind) = IntegerKind::from_rust_type(name) {
        return (BaseType::Integer, Some(kind));
    }
    let base_type = match name {
        "f32" | "f64" => BaseType::Float,
        "bool" => BaseType::Boolean,
        "string" | "String" => BaseType::String,
        "publicKey" | "pubkey" | "Pubkey" => BaseType::Pubkey,
        "bytes" => BaseType::Binary,
        _ => BaseType::Object,
    };
    (base_type, None)
}

/// Mirror of `arete-macros`' `analyze_idl_type_with_resolution`, except that
/// a map is a JSON value (the macro types it as its value type).
fn idl_field_shape(idl_type: &IdlTypeSnapshot) -> IdlFieldShape {
    match idl_type {
        IdlTypeSnapshot::Simple(name) => {
            let (base_type, integer_kind) = simple_idl_base_type(name);
            IdlFieldShape {
                field_type: name.clone(),
                base_type,
                integer_kind,
                is_optional: false,
                is_array: false,
            }
        }
        IdlTypeSnapshot::Option(option) => {
            let inner = idl_field_shape(&option.option);
            IdlFieldShape {
                field_type: format!("Option<{}>", inner.field_type),
                is_optional: true,
                ..inner
            }
        }
        IdlTypeSnapshot::Vec(vec) => {
            let inner = idl_field_shape(&vec.vec);
            IdlFieldShape {
                field_type: format!("Vec<{}>", inner.field_type),
                is_array: true,
                ..inner
            }
        }
        IdlTypeSnapshot::Array(array) if array.array.len() >= 2 => {
            let element = match &array.array[0] {
                IdlArrayElementSnapshot::Type(IdlTypeSnapshot::Simple(name))
                | IdlArrayElementSnapshot::TypeName(name) => {
                    let (base_type, integer_kind) = simple_idl_base_type(name);
                    Some(IdlFieldShape {
                        field_type: name.clone(),
                        base_type,
                        integer_kind,
                        is_optional: false,
                        is_array: false,
                    })
                }
                IdlArrayElementSnapshot::Type(nested) => Some(idl_field_shape(nested)),
                IdlArrayElementSnapshot::Size(_) => None,
            };
            match element {
                Some(element) => IdlFieldShape {
                    field_type: format!("[{}]", element.field_type),
                    is_array: true,
                    ..element
                },
                None => json_array_shape(),
            }
        }
        IdlTypeSnapshot::Array(_) => json_array_shape(),
        IdlTypeSnapshot::Defined(defined) => IdlFieldShape {
            field_type: match &defined.defined {
                IdlDefinedInnerSnapshot::Named { name } => name.clone(),
                IdlDefinedInnerSnapshot::Simple(name) => name.clone(),
            },
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: false,
            is_array: false,
        },
        IdlTypeSnapshot::HashMap(map) => IdlFieldShape {
            field_type: format!(
                "HashMap<{}, {}>",
                idl_field_shape(&map.hash_map.0).field_type,
                idl_field_shape(&map.hash_map.1).field_type
            ),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: false,
            is_array: false,
        },
        IdlTypeSnapshot::Tuple(tuple) => IdlFieldShape {
            field_type: format!(
                "({})",
                tuple
                    .tuple
                    .iter()
                    .map(|element| idl_field_shape(element).field_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: false,
            is_array: false,
        },
    }
}

fn json_array_shape() -> IdlFieldShape {
    IdlFieldShape {
        field_type: "Array".to_string(),
        base_type: BaseType::Array,
        integer_kind: None,
        is_optional: false,
        is_array: true,
    }
}

/// The name a program's standalone SDK gives each IDL account's model, by
/// account name: the account name in `PascalCase`, else with an `Account`
/// suffix (then numbered) when that is `reserved` or another account of the
/// program already has it. It depends on the program's IDL alone, so a stack
/// SDK can alias every embedded program's models under the same names.
pub(crate) fn stable_account_model_names(
    idl: &IdlSnapshot,
    pascal: impl Fn(&str) -> String,
    reserved: &[&str],
) -> BTreeMap<String, String> {
    let mut used = reserved
        .iter()
        .map(|name| name.to_string())
        .collect::<HashSet<_>>();
    let mut names = BTreeMap::new();
    for account in &idl.accounts {
        if names.contains_key(&account.name) {
            continue;
        }
        let base = pascal(&account.name);
        let name = std::iter::once(base.clone())
            .chain(std::iter::once(format!("{base}Account")))
            .chain((2usize..).map(|index| format!("{base}Account{index}")))
            .find(|candidate| used.insert(candidate.clone()))
            .expect("the numbered candidates are unbounded");
        names.insert(account.name.clone(), name);
    }
    names
}

/// The names the shared types module tries for a program's account model,
/// most preferred first: its stable name, then prefixed with the program
/// name, then with an `Account` role suffix, then numbered (the TypeScript
/// generator's order for IDL account types).
pub(crate) fn account_model_candidates(
    stable: &str,
    program_name: &str,
) -> impl Iterator<Item = String> {
    let prefixed = format!(
        "{}{stable}",
        ts_ident::identifier_stem(program_name, IdentifierCase::Pascal)
    );
    let role = format!("{prefixed}Account");
    [stable.to_string(), prefixed, role.clone()]
        .into_iter()
        .chain((2usize..).map(move |index| format!("{role}{index}")))
}

/// The programs (by IDL name) whose generated module gets typed account
/// readers: every program the SDK emits a module for (`include_idl_only`:
/// every IDL; else every program an instruction targets) that has a program
/// read layer.
pub(crate) fn account_reader_programs(
    idls: &[IdlSnapshot],
    program_ids: &[String],
    instructions: &[crate::ast::InstructionDef],
    include_idl_only: bool,
    has_read_layer: impl Fn(&str) -> bool,
) -> HashSet<String> {
    if instructions.is_empty() && !include_idl_only {
        return HashSet::new();
    }
    let default_program_id = program_ids.first().map(String::as_str);
    idls.iter()
        .enumerate()
        .filter_map(|(index, idl)| {
            let program_id = idl
                .program_id
                .as_deref()
                .or_else(|| program_ids.get(index).map(String::as_str))?;
            let emitted = include_idl_only
                || instructions.iter().any(|instruction| {
                    instruction.program_id.as_deref().or(default_program_id) == Some(program_id)
                });
            (emitted && has_read_layer(program_id)).then(|| idl.name.clone())
        })
        .collect()
}

/// The prefixes that disambiguate an entity's resolved type from a
/// different, same-named type another entity declared: the entity's program
/// name (`subscriptions` -> `Subscriptions`), then the entity name.
pub(crate) fn resolved_type_namespaces(
    program_name: Option<&str>,
    entity_name: &str,
) -> Vec<String> {
    program_name
        .map(|name| ts_ident::identifier_stem(name, IdentifierCase::Pascal))
        .into_iter()
        .chain(std::iter::once(ts_ident::identifier_stem(
            entity_name,
            IdentifierCase::Pascal,
        )))
        .collect()
}

/// The normalized definition of a resolved type: its fields in order (or its
/// enum variants). The type's name and its account/event/instruction flags
/// are left out; the flags only change how fields that reference the type are
/// wrapped, never the type's own declaration.
pub(crate) fn resolved_type_shape(resolved: &ResolvedStructType) -> String {
    serde_json::to_string(&(resolved.is_enum, &resolved.enum_variants, &resolved.fields))
        .expect("resolved types serialize")
}

/// The IDL type definitions of every program in a stack, looked up from one
/// program's point of view.
///
/// Instruction codegen resolves the defined types an instruction's arguments
/// reference by name, and a name may be spelled with a different case than
/// its definition. Across programs, the first program to define a name
/// (compared case-insensitively) provides its stack-wide definition, which
/// every program whose own definition is identical (including the types it
/// references) shares. A program that defines the name differently resolves
/// its own definition, so its instructions are never encoded with another
/// program's layout.
pub(crate) struct ProgramTypeDefs<'a> {
    /// Lowercase name -> the first definition across programs, and its program.
    first: BTreeMap<String, (&'a IdlTypeDefSnapshot, usize)>,
    /// Every spelling any program defines.
    names: BTreeSet<String>,
    programs: Vec<ProgramDefs<'a>>,
    program_names: Vec<String>,
    scope: Option<usize>,
}

/// One program's type definitions.
struct ProgramDefs<'a> {
    by_name: BTreeMap<&'a str, &'a IdlTypeDefSnapshot>,
    /// Lowercase name -> the program's first spelling of it.
    lower: BTreeMap<String, &'a str>,
    /// Spellings the program defines differently from their first definition.
    own: BTreeSet<&'a str>,
}

impl<'a> ProgramDefs<'a> {
    /// The program's definition of `name`: exact spelling, then any case.
    fn get(&self, name: &str) -> Option<&'a IdlTypeDefSnapshot> {
        self.by_name.get(name).copied().or_else(|| {
            self.lower
                .get(&name.to_lowercase())
                .map(|spelling| self.by_name[spelling])
        })
    }
}

/// A defined type as seen from the current program.
#[derive(Clone, Copy)]
pub(crate) struct ProgramTypeDef<'a> {
    /// The name as the defining IDL spells it.
    pub(crate) name: &'a str,
    pub(crate) def: &'a IdlTypeDefSnapshot,
    /// The program whose own, different definition this is; `None` for the
    /// stack-wide definition.
    pub(crate) program: Option<usize>,
}

impl<'a> ProgramTypeDefs<'a> {
    pub(crate) fn new(idls: &'a [IdlSnapshot]) -> Self {
        let mut first = BTreeMap::new();
        let mut names = BTreeSet::new();
        for (program, idl) in idls.iter().enumerate() {
            for def in &idl.types {
                names.insert(def.name.clone());
                first
                    .entry(def.name.to_lowercase())
                    .or_insert((def, program));
            }
        }
        let mut programs = idls
            .iter()
            .map(|idl| {
                let mut lower = BTreeMap::new();
                for def in &idl.types {
                    lower
                        .entry(def.name.to_lowercase())
                        .or_insert(def.name.as_str());
                }
                ProgramDefs {
                    by_name: idl
                        .types
                        .iter()
                        .map(|def| (def.name.as_str(), def))
                        .collect(),
                    lower,
                    own: BTreeSet::new(),
                }
            })
            .collect::<Vec<_>>();
        for program in &mut programs {
            let mut differs = BTreeMap::new();
            program.own = program
                .by_name
                .values()
                .filter(|def| {
                    definition_differs(def, program, &first, &mut differs, &mut Vec::new())
                })
                .map(|def| def.name.as_str())
                .collect();
        }
        Self {
            first,
            names,
            programs,
            program_names: idls.iter().map(|idl| idl.name.clone()).collect(),
            scope: None,
        }
    }

    /// Look names up from `program`'s point of view (`None`: stack-wide).
    pub(crate) fn set_scope(&mut self, program: Option<usize>) {
        self.scope = program;
    }

    pub(crate) fn program_name(&self, program: usize) -> &str {
        &self.program_names[program]
    }

    /// Every defined type name in the stack, as the IDLs spell them.
    pub(crate) fn names(&self) -> impl Iterator<Item = &String> {
        self.names.iter()
    }

    /// The definition of `name` (in any case) for the current program: the
    /// program's own when it defines the name differently, else the first.
    pub(crate) fn lookup(&self, name: &str) -> Option<ProgramTypeDef<'a>> {
        let scoped = self.scope.and_then(|program| {
            let def = self.programs.get(program)?.get(name)?;
            Some((program, def))
        });
        if let Some((program, def)) = scoped {
            if self.programs[program].own.contains(def.name.as_str()) {
                return Some(ProgramTypeDef {
                    name: &def.name,
                    def,
                    program: Some(program),
                });
            }
        }
        let key = scoped.map_or(name, |(_, def)| def.name.as_str());
        let (def, _) = self.first.get(&key.to_lowercase())?;
        Some(ProgramTypeDef {
            name: &def.name,
            def,
            program: None,
        })
    }

    /// `(type, first program, other program)` for every type a program
    /// defines differently from the program that defined it first.
    pub(crate) fn conflicts(&self) -> Vec<(String, String, String)> {
        self.programs
            .iter()
            .enumerate()
            .flat_map(|(program, defs)| {
                defs.own.iter().map(move |name| {
                    let (_, first_program) = self.first[&name.to_lowercase()];
                    (
                        name.to_string(),
                        self.program_names[first_program].clone(),
                        self.program_names[program].clone(),
                    )
                })
            })
            .collect()
    }
}

/// Whether `own`, one of `program`'s definitions, differs from the first
/// definition of its name across programs, directly or through a type it
/// references (as the program resolves that name). Memoized by spelling in
/// `differs`; a type already being compared counts as the same (recursive
/// types only differ through their other parts).
fn definition_differs<'a>(
    own: &'a IdlTypeDefSnapshot,
    program: &ProgramDefs<'a>,
    first: &BTreeMap<String, (&'a IdlTypeDefSnapshot, usize)>,
    differs: &mut BTreeMap<String, bool>,
    visiting: &mut Vec<String>,
) -> bool {
    if let Some(known) = differs.get(&own.name) {
        return *known;
    }
    if visiting.contains(&own.name) {
        return false;
    }
    let Some((first_def, _)) = first.get(&own.name.to_lowercase()) else {
        return false;
    };
    visiting.push(own.name.clone());
    let result = definition_shape(own) != definition_shape(first_def)
        || referenced_type_names(own).iter().any(|referenced| {
            program.get(referenced).is_some_and(|referenced| {
                definition_differs(referenced, program, first, differs, visiting)
            })
        });
    visiting.pop();
    differs.insert(own.name.clone(), result);
    result
}

/// A type definition without its name and docs.
fn definition_shape(def: &IdlTypeDefSnapshot) -> serde_json::Value {
    serde_json::json!({
        "serialization": def.serialization,
        "type": def.type_def,
    })
}

/// Every name a definition refers to that could be another defined type.
fn referenced_type_names(def: &IdlTypeDefSnapshot) -> BTreeSet<String> {
    fn words(text: &str, names: &mut BTreeSet<String>) {
        names.extend(
            text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
                .filter(|word| !word.is_empty())
                .map(str::to_string),
        );
    }
    fn walk(ty: &IdlTypeSnapshot, names: &mut BTreeSet<String>) {
        match ty {
            IdlTypeSnapshot::Simple(simple) => words(simple, names),
            IdlTypeSnapshot::Array(array) => {
                for element in &array.array {
                    match element {
                        IdlArrayElementSnapshot::Type(inner) => walk(inner, names),
                        IdlArrayElementSnapshot::TypeName(name) => words(name, names),
                        IdlArrayElementSnapshot::Size(_) => {}
                    }
                }
            }
            IdlTypeSnapshot::Option(option) => walk(&option.option, names),
            IdlTypeSnapshot::Vec(vec) => walk(&vec.vec, names),
            IdlTypeSnapshot::HashMap(map) => {
                walk(&map.hash_map.0, names);
                walk(&map.hash_map.1, names);
            }
            IdlTypeSnapshot::Tuple(tuple) => {
                for element in &tuple.tuple {
                    walk(element, names);
                }
            }
            IdlTypeSnapshot::Defined(defined) => match &defined.defined {
                IdlDefinedInnerSnapshot::Named { name } => {
                    names.insert(name.clone());
                }
                IdlDefinedInnerSnapshot::Simple(name) => {
                    names.insert(name.clone());
                }
            },
        }
    }
    let mut names = BTreeSet::new();
    match &def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => {
            for field in fields {
                walk(&field.type_, &mut names);
            }
        }
        IdlTypeDefKindSnapshot::TupleStruct { fields, .. } => {
            for field in fields {
                walk(field, &mut names);
            }
        }
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            for field in variants.iter().flat_map(|variant| &variant.fields) {
                match field {
                    IdlEnumVariantFieldSnapshot::Named(named) => walk(&named.type_, &mut names),
                    IdlEnumVariantFieldSnapshot::Tuple(tuple) => walk(tuple, &mut names),
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BaseType, ResolvedField};

    fn resolved(name: &str, fields: &[(&str, BaseType)]) -> ResolvedStructType {
        ResolvedStructType {
            type_name: name.to_string(),
            fields: fields
                .iter()
                .map(|(field_name, base_type)| ResolvedField {
                    field_name: field_name.to_string(),
                    raw_name: None,
                    canonical_name: None,
                    field_type: format!("{base_type:?}"),
                    base_type: base_type.clone(),
                    integer_kind: None,
                    is_optional: false,
                    is_array: false,
                })
                .collect(),
            is_instruction: false,
            is_account: false,
            is_event: false,
            is_enum: false,
            enum_variants: Vec::new(),
        }
    }

    fn namespaces() -> Vec<String> {
        vec!["Beta".to_string(), "Vault".to_string()]
    }

    #[test]
    fn undeclared_names_are_kept() {
        let types = StackResolvedTypes::default();
        let header = resolved("header", &[("version", BaseType::Integer)]);
        let mut taken = HashSet::from(["Header".to_string()]);
        assert_eq!(
            types.claim(
                &header,
                "Header".into(),
                "Header",
                &namespaces(),
                &mut taken
            ),
            ResolvedTypeClaim::Declare("Header".into())
        );
    }

    #[test]
    fn identical_definitions_are_shared_whatever_their_flags() {
        let mut types = StackResolvedTypes::default();
        types.declare(
            "Header",
            &resolved("header", &[("version", BaseType::Integer)]),
        );
        let mut account = resolved("Header", &[("version", BaseType::Integer)]);
        account.is_account = true;
        let mut taken = HashSet::from(["Header".to_string()]);
        assert_eq!(
            types.claim(
                &account,
                "Header".into(),
                "Header",
                &namespaces(),
                &mut taken
            ),
            ResolvedTypeClaim::Shared("Header".into())
        );
    }

    #[test]
    fn different_definitions_are_namespaced_then_numbered() {
        let mut types = StackResolvedTypes::default();
        types.declare(
            "Header",
            &resolved("header", &[("version", BaseType::Integer)]),
        );
        let other = resolved("header", &[("owner", BaseType::Pubkey)]);
        let mut taken = HashSet::from(["Header".to_string()]);
        let claim = types.claim(&other, "Header".into(), "Header", &namespaces(), &mut taken);
        assert_eq!(claim, ResolvedTypeClaim::Declare("BetaHeader".into()));
        assert!(taken.contains("BetaHeader"));

        // An identical type already declared under the namespaced name is shared.
        types.declare("BetaHeader", &other);
        let mut taken = HashSet::from(["Header".to_string()]);
        assert_eq!(
            types.claim(&other, "Header".into(), "Header", &namespaces(), &mut taken),
            ResolvedTypeClaim::Shared("BetaHeader".into())
        );

        // A third definition skips taken, reserved and differently declared names.
        let third = resolved("header", &[("flag", BaseType::Boolean)]);
        types.reserve(["VaultHeader".to_string()]);
        let mut taken = HashSet::from(["Header".to_string()]);
        assert_eq!(
            types.claim(&third, "Header".into(), "Header", &namespaces(), &mut taken),
            ResolvedTypeClaim::Declare("VaultHeader2".into())
        );
    }

    fn idl(name: &str, types: serde_json::Value) -> IdlSnapshot {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "version": "0.1.0",
            "accounts": [],
            "instructions": [],
            "types": types,
            "discriminant_size": 8
        }))
        .expect("test IDL")
    }

    fn program_of(defs: &ProgramTypeDefs<'_>, name: &str) -> Option<Option<usize>> {
        defs.lookup(name).map(|found| found.program)
    }

    #[test]
    fn program_type_defs_resolve_each_programs_own_different_definition() {
        let struct_of =
            |fields: serde_json::Value| serde_json::json!({ "kind": "struct", "fields": fields });
        let inner_a = struct_of(serde_json::json!([{ "name": "a", "type": "u8" }]));
        let inner_b = struct_of(serde_json::json!([{ "name": "a", "type": "u16" }]));
        // `Outer` reads the same in both IDLs but refers to their different `Inner`.
        let outer =
            struct_of(serde_json::json!([{ "name": "inner", "type": { "defined": "Inner" } }]));
        let same = struct_of(serde_json::json!([{ "name": "x", "type": "bool" }]));
        let idls = vec![
            idl(
                "alpha",
                serde_json::json!([
                    { "name": "Inner", "type": inner_a },
                    { "name": "Outer", "type": outer },
                    { "name": "Same", "type": same }
                ]),
            ),
            idl(
                "beta",
                serde_json::json!([
                    { "name": "Inner", "docs": ["documented differently"], "type": inner_b },
                    { "name": "Outer", "type": outer },
                    { "name": "Same", "docs": ["documented differently"], "type": same }
                ]),
            ),
        ];
        let mut defs = ProgramTypeDefs::new(&idls);

        // Stack-wide and from the first program: the first definitions.
        for scope in [None, Some(0)] {
            defs.set_scope(scope);
            assert_eq!(program_of(&defs, "Inner"), Some(None));
            assert_eq!(program_of(&defs, "Outer"), Some(None));
        }

        // From `beta`: its own `Inner`, and its own `Outer` through it; the
        // identical `Same` (docs aside) stays shared.
        defs.set_scope(Some(1));
        assert_eq!(program_of(&defs, "Inner"), Some(Some(1)));
        assert_eq!(program_of(&defs, "inner"), Some(Some(1)));
        assert_eq!(program_of(&defs, "Outer"), Some(Some(1)));
        assert_eq!(program_of(&defs, "Same"), Some(None));
        assert_eq!(program_of(&defs, "Missing"), None);
        assert_eq!(
            defs.conflicts(),
            vec![
                ("Inner".to_string(), "alpha".to_string(), "beta".to_string()),
                ("Outer".to_string(), "alpha".to_string(), "beta".to_string()),
            ]
        );
    }

    #[test]
    fn program_type_defs_match_names_case_insensitively_across_programs() {
        let struct_of = |ty: &str| serde_json::json!({ "kind": "struct", "fields": [{ "name": "a", "type": ty }] });
        let idls = vec![
            idl(
                "alpha",
                serde_json::json!([{ "name": "Header", "type": struct_of("u8") }]),
            ),
            idl(
                "beta",
                serde_json::json!([{ "name": "header", "type": struct_of("u16") }]),
            ),
            idl(
                "gamma",
                serde_json::json!([{ "name": "HEADER", "type": struct_of("u8") }]),
            ),
        ];
        let mut defs = ProgramTypeDefs::new(&idls);

        // `beta`'s differently shaped `header` is its own, whatever the spelling.
        defs.set_scope(Some(1));
        for spelling in ["Header", "header", "HEADER"] {
            let found = defs.lookup(spelling).unwrap();
            assert_eq!((found.name, found.program), ("header", Some(1)));
        }
        // `gamma`'s identical `HEADER` shares `alpha`'s first definition.
        defs.set_scope(Some(2));
        let found = defs.lookup("header").unwrap();
        assert_eq!((found.name, found.program), ("Header", None));
        assert_eq!(
            defs.conflicts(),
            vec![(
                "header".to_string(),
                "alpha".to_string(),
                "beta".to_string()
            )]
        );
    }
}
