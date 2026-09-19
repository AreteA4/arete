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
    IdlArrayElementSnapshot, IdlDefinedInnerSnapshot, IdlEnumVariantFieldSnapshot, IdlSnapshot,
    IdlTypeDefKindSnapshot, IdlTypeDefSnapshot, IdlTypeSnapshot, ResolvedStructType,
    SerializableStreamSpec,
};
use crate::identifiers::{typescript as ts_ident, IdentifierCase};
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
/// reference by name. Across programs, the first program to define a name
/// provides its stack-wide definition, which every program whose own
/// definition is identical (including the types it references) shares. A
/// program that defines the name differently resolves its own definition,
/// so its instructions are never encoded with another program's layout.
pub(crate) struct ProgramTypeDefs<'a> {
    /// Each name's first definition across programs.
    first: BTreeMap<String, &'a IdlTypeDefSnapshot>,
    /// The program each name's first definition comes from.
    first_program: BTreeMap<String, usize>,
    /// Lowercase name -> first spelling, for case-insensitive lookups.
    lower: BTreeMap<String, String>,
    /// Program index -> names that program defines differently from their
    /// first definition, with its own definition.
    own: Vec<BTreeMap<String, &'a IdlTypeDefSnapshot>>,
    program_names: Vec<String>,
    scope: Option<usize>,
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
        let mut first: BTreeMap<String, &'a IdlTypeDefSnapshot> = BTreeMap::new();
        let mut first_program = BTreeMap::new();
        let mut lower = BTreeMap::new();
        for (program, idl) in idls.iter().enumerate() {
            for def in &idl.types {
                if !first.contains_key(&def.name) {
                    first.insert(def.name.clone(), def);
                    first_program.insert(def.name.clone(), program);
                    lower.insert(def.name.to_lowercase(), def.name.clone());
                }
            }
        }
        let own = idls
            .iter()
            .map(|idl| {
                let defs = idl
                    .types
                    .iter()
                    .map(|def| (def.name.as_str(), def))
                    .collect::<BTreeMap<_, _>>();
                let mut differs = BTreeMap::new();
                idl.types
                    .iter()
                    .filter(|def| {
                        definition_differs(&def.name, &defs, &first, &mut differs, &mut Vec::new())
                    })
                    .map(|def| (def.name.clone(), def))
                    .collect()
            })
            .collect();
        Self {
            first,
            first_program,
            lower,
            own,
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

    /// Every defined type name in the stack.
    pub(crate) fn names(&self) -> impl Iterator<Item = &String> {
        self.first.keys()
    }

    /// The definition of `name` (matched case-insensitively when no program
    /// spells it exactly so) for the current program.
    pub(crate) fn lookup(&self, name: &str) -> Option<ProgramTypeDef<'a>> {
        let (name, def) = match self.first.get_key_value(name) {
            Some((name, def)) => (name, *def),
            None => {
                let name = self.lower.get(&name.to_lowercase())?;
                (name, self.first[name])
            }
        };
        let own = self
            .scope
            .and_then(|program| Some((program, self.own.get(program)?.get(name)?)));
        Some(match own {
            Some((program, def)) => ProgramTypeDef {
                name: &def.name,
                def,
                program: Some(program),
            },
            None => ProgramTypeDef {
                name: &def.name,
                def,
                program: None,
            },
        })
    }

    /// `(type, first program, other program)` for every type a program
    /// defines differently from the program that defined it first.
    pub(crate) fn conflicts(&self) -> Vec<(String, String, String)> {
        self.own
            .iter()
            .enumerate()
            .flat_map(|(program, own)| {
                own.keys().map(move |name| {
                    (
                        name.clone(),
                        self.program_names[self.first_program[name]].clone(),
                        self.program_names[program].clone(),
                    )
                })
            })
            .collect()
    }
}

/// Whether `name`, as `defs` (one program's types) defines it, differs from
/// its first definition across programs, directly or through a type it
/// references. Memoized in `differs`; a type already being compared counts as
/// the same (recursive types only differ through their other parts).
fn definition_differs(
    name: &str,
    defs: &BTreeMap<&str, &IdlTypeDefSnapshot>,
    first: &BTreeMap<String, &IdlTypeDefSnapshot>,
    differs: &mut BTreeMap<String, bool>,
    visiting: &mut Vec<String>,
) -> bool {
    if let Some(known) = differs.get(name) {
        return *known;
    }
    let (Some(own), Some(first_def)) = (defs.get(name), first.get(name)) else {
        return false;
    };
    if visiting.iter().any(|visited| visited == name) {
        return false;
    }
    visiting.push(name.to_string());
    let result = definition_shape(own) != definition_shape(first_def)
        || referenced_type_names(own)
            .iter()
            .any(|referenced| definition_differs(referenced, defs, first, differs, visiting));
    visiting.pop();
    differs.insert(name.to_string(), result);
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
}
