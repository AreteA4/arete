//! Typed models of the values program account reads decode.
//!
//! A program SDK reads each IDL account into a model derived from the
//! program's IDL ([`ProgramModels`]). Its fields are typed the way the
//! Program Read HTTP API encodes them, including the IDL's defined types:
//! every struct, tuple struct and enum reachable from an account gets a
//! model of its own, and fields reference it by name ([`WireType::Model`]).
//!
//! The public Program Read wire shapes are:
//!
//! - integers up to 32 bits are JSON numbers; `u64` a number up to 2^53 and a
//!   decimal string above; `u128`/`i128` always decimal strings;
//! - a struct is an object keyed by the IDL field names; a tuple struct an
//!   object keyed `field_0`, `field_1`, ...; an inline tuple an array;
//! - an enum's unit variant is its name (`"Active"`); a data variant a
//!   one-key object from its name to its payload (`{"Stable": {"amp": 5}}`),
//!   with tuple payloads represented as arrays;
//! - options are the value or `null`; vectors and fixed arrays are arrays;
//!   maps are objects with string keys.
//!
//! A field the flat convention of the stack generators can already express
//! (a scalar, or an option or array of scalars) keeps exactly that rendering
//! ([`ModelField::typed`] is `None`), so an account without nested types
//! renders as it always has, and shares the model an entity maps it to.
//!
//! [`bind_idl_models`] names every model in an SDK's shared types module.
//! Each also has a *stable name*, the name the program's standalone SDK
//! declares it under, which the `generated` re-exports (Rust) and the
//! `program_sdks/<program>/models.py` aliases (Python) map to the declared
//! name inside a stack.

use crate::ast::{
    BaseType, IdlArrayElementSnapshot, IdlDefinedInnerSnapshot, IdlEnumVariantFieldSnapshot,
    IdlFieldSnapshot, IdlSnapshot, IdlTypeDefKindSnapshot, IdlTypeDefSnapshot, IdlTypeSnapshot,
    IntegerKind, ResolvedField, ResolvedStructType,
};
use crate::identifiers::{typescript as ts_ident, IdentifierCase};
use crate::stack_types::{
    account_model_candidates, idl_account_field_snapshots, resolved_idl_field,
    stable_account_model_names, AccountModels,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Tuples longer than this decode as raw JSON (the Rust `serde_utils::wire`
/// tuple shapes stop here).
const MAX_TUPLE_LEN: usize = 8;

/// How one value is laid out on the Program Read wire.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WireType {
    /// A primitive, classified as the flat convention classifies it.
    Scalar {
        base_type: BaseType,
        integer_kind: Option<IntegerKind>,
    },
    /// An IDL `Option`: the value or `null`.
    Option(Box<WireType>),
    /// An IDL vector or fixed array.
    List(Box<WireType>),
    /// An inline IDL tuple: a JSON array.
    Tuple(Vec<WireType>),
    /// An IDL map: a JSON object with string keys.
    Map(Box<WireType>),
    /// A defined type's model: its IDL name while models are built, its
    /// declared name once [`IdlModel::with_names`] resolved it.
    Model(String),
    /// Raw JSON: a type the IDL does not define, or a recursive reference.
    Json,
}

impl WireType {
    /// Whether the value holds an integer (whose wire form may be a string).
    pub(crate) fn has_integer(&self) -> bool {
        match self {
            WireType::Scalar { base_type, .. } => matches!(base_type, BaseType::Integer),
            WireType::Option(inner) | WireType::List(inner) | WireType::Map(inner) => {
                inner.has_integer()
            }
            WireType::Tuple(elements) => elements.iter().any(WireType::has_integer),
            WireType::Model(_) | WireType::Json => false,
        }
    }

    fn resolve(&self, names: &BTreeMap<String, String>) -> WireType {
        match self {
            WireType::Option(inner) => WireType::Option(Box::new(inner.resolve(names))),
            WireType::List(inner) => WireType::List(Box::new(inner.resolve(names))),
            WireType::Map(inner) => WireType::Map(Box::new(inner.resolve(names))),
            WireType::Tuple(elements) => WireType::Tuple(
                elements
                    .iter()
                    .map(|element| element.resolve(names))
                    .collect(),
            ),
            WireType::Model(name) => {
                WireType::Model(names.get(name).cloned().unwrap_or_else(|| name.clone()))
            }
            other => other.clone(),
        }
    }
}

/// One field of a model (a struct field, or a field of an enum variant).
#[derive(Debug, Clone)]
pub(crate) struct ModelField {
    /// The field's name and its flat projection (the convention the stack
    /// generators use for the accounts entities capture).
    pub(crate) flat: ResolvedField,
    /// The field's typed shape, when the flat projection cannot express it
    /// (defined types, tuples, maps, nested containers); `None` renders the
    /// flat projection.
    pub(crate) typed: Option<WireType>,
}

impl ModelField {
    fn new(field: &IdlFieldSnapshot, typed: WireType) -> Self {
        Self {
            flat: resolved_idl_field(field),
            typed: (!is_flat(&field.type_)).then_some(typed),
        }
    }

    /// The key the field has on the wire: the IDL name in snake_case,
    /// keeping leading underscores (`_padding_0`), never keyword-escaped.
    pub(crate) fn wire_name(&self) -> String {
        idl_wire_name(self.flat.raw_field_name())
    }
}

/// One variant of an enum model.
#[derive(Debug, Clone)]
pub(crate) struct ModelVariant {
    /// The variant's IDL name, its wire tag.
    pub(crate) name: String,
    /// Whether the payload is positional rather than named.
    pub(crate) is_tuple: bool,
    /// Its fields: named ones by name, tuple ones as `field_<index>`.
    pub(crate) fields: Vec<ModelField>,
}

#[derive(Debug, Clone)]
pub(crate) enum IdlModelKind {
    Struct(Vec<ModelField>),
    Enum(Vec<ModelVariant>),
}

/// A model derived from a program's IDL.
#[derive(Debug, Clone)]
pub(crate) struct IdlModel {
    /// The IDL name of the account or defined type.
    pub(crate) type_name: String,
    /// An account's model (else a defined type's).
    pub(crate) is_account: bool,
    pub(crate) kind: IdlModelKind,
}

impl IdlModel {
    /// The model with every [`WireType::Model`] reference renamed to the
    /// name `names` declares it under.
    pub(crate) fn with_names(&self, names: &BTreeMap<String, String>) -> IdlModel {
        let fields = |fields: &[ModelField]| {
            fields
                .iter()
                .map(|field| ModelField {
                    flat: field.flat.clone(),
                    typed: field.typed.as_ref().map(|typed| typed.resolve(names)),
                })
                .collect::<Vec<_>>()
        };
        IdlModel {
            type_name: self.type_name.clone(),
            is_account: self.is_account,
            kind: match &self.kind {
                IdlModelKind::Struct(struct_fields) => IdlModelKind::Struct(fields(struct_fields)),
                IdlModelKind::Enum(variants) => IdlModelKind::Enum(
                    variants
                        .iter()
                        .map(|variant| ModelVariant {
                            name: variant.name.clone(),
                            is_tuple: variant.is_tuple,
                            fields: fields(&variant.fields),
                        })
                        .collect(),
                ),
            },
        }
    }

    /// The model's definition without its name and role, to tell whether
    /// two models are the same type.
    fn definition(&self) -> String {
        format!("{:?}", self.kind)
    }
}

/// A model an SDK's shared types module (`types.rs` / `models.py`) declares.
#[derive(Debug, Clone)]
pub(crate) enum DeclaredModel {
    /// A resolved type as an entity maps it (the flat convention).
    Resolved(ResolvedStructType),
    /// A model derived from a program's IDL.
    Idl(IdlModel),
}

/// The wire key of an IDL field name, as the TypeScript program SDK keys it
/// (`idl_field_wire_name`) and the account-read key normalization produces
/// it from the raw name the server sends: an `_` before each upper-case
/// letter, lower-cased (`numSignatures` -> `num_signatures`). Unlike the
/// language identifiers, keywords are not escaped: a field named `type` is
/// keyed `type`.
pub(crate) fn idl_wire_name(raw: &str) -> String {
    arete_idl::utils::to_snake_case(raw)
}

fn primitive(name: &str) -> Option<(BaseType, Option<IntegerKind>)> {
    if let Some(kind) = IntegerKind::from_rust_type(name) {
        return Some((BaseType::Integer, Some(kind)));
    }
    Some((
        match name {
            "f32" | "f64" => BaseType::Float,
            "bool" => BaseType::Boolean,
            "string" | "String" => BaseType::String,
            "publicKey" | "pubkey" | "Pubkey" => BaseType::Pubkey,
            "bytes" => BaseType::Binary,
            _ => return None,
        },
        None,
    ))
}

fn is_primitive_type(idl_type: &IdlTypeSnapshot) -> bool {
    matches!(idl_type, IdlTypeSnapshot::Simple(name) if primitive(name).is_some())
}

fn is_primitive_list(idl_type: &IdlTypeSnapshot) -> bool {
    match idl_type {
        IdlTypeSnapshot::Vec(vec) => is_primitive_type(&vec.vec),
        IdlTypeSnapshot::Array(array) => matches!(
            array.array.as_slice(),
            [IdlArrayElementSnapshot::Type(IdlTypeSnapshot::Simple(name))
                | IdlArrayElementSnapshot::TypeName(name), IdlArrayElementSnapshot::Size(_)]
                if primitive(name).is_some()
        ),
        _ => false,
    }
}

/// Whether the flat projection types `idl_type` exactly: a scalar, an array
/// of scalars, or an option of either.
fn is_flat(idl_type: &IdlTypeSnapshot) -> bool {
    match idl_type {
        IdlTypeSnapshot::Option(option) => {
            is_primitive_type(&option.option) || is_primitive_list(&option.option)
        }
        other => is_primitive_type(other) || is_primitive_list(other),
    }
}

/// The definition of a defined type name in `idl`: exact spelling, the last
/// path segment (`state::Header`), then any case.
fn lookup_type<'a>(idl: &'a IdlSnapshot, name: &str) -> Option<&'a IdlTypeDefSnapshot> {
    let last = name.rsplit("::").next().unwrap_or(name);
    idl.types
        .iter()
        .find(|def| def.name == name)
        .or_else(|| idl.types.iter().find(|def| def.name == last))
        .or_else(|| {
            idl.types
                .iter()
                .find(|def| def.name.eq_ignore_ascii_case(last))
        })
}

/// Every account model of one program and the defined types they reach.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProgramModels {
    /// Each IDL account (first declaration of a name), in IDL order: its
    /// model (`None` for an enum layout, which reads as raw JSON) and the
    /// defined types first reached from it, dependencies first.
    pub(crate) accounts: Vec<(String, Option<IdlModel>, Vec<String>)>,
    /// Every defined type reachable from an account, by IDL name.
    pub(crate) types: BTreeMap<String, IdlModel>,
}

impl ProgramModels {
    pub(crate) fn build(idl: &IdlSnapshot) -> Self {
        let mut builder = ModelBuilder {
            idl,
            types: BTreeMap::new(),
            order: Vec::new(),
            visiting: Vec::new(),
        };
        let mut accounts = Vec::new();
        let mut seen = HashSet::new();
        for account in &idl.accounts {
            if !seen.insert(account.name.as_str()) {
                continue;
            }
            let first = builder.order.len();
            let model = idl_account_field_snapshots(idl, account).map(|fields| IdlModel {
                type_name: account.name.clone(),
                is_account: true,
                kind: IdlModelKind::Struct(builder.fields(&fields)),
            });
            accounts.push((account.name.clone(), model, builder.order[first..].to_vec()));
        }
        ProgramModels {
            accounts,
            types: builder.types,
        }
    }
}

struct ModelBuilder<'a> {
    idl: &'a IdlSnapshot,
    types: BTreeMap<String, IdlModel>,
    /// Defined types in the order their models were completed.
    order: Vec<String>,
    /// Defined types whose model is being built (a reference back to one is
    /// recursive, and reads as raw JSON).
    visiting: Vec<String>,
}

impl ModelBuilder<'_> {
    fn fields(&mut self, fields: &[IdlFieldSnapshot]) -> Vec<ModelField> {
        fields
            .iter()
            .map(|field| {
                let typed = self.wire_type(&field.type_);
                ModelField::new(field, typed)
            })
            .collect()
    }

    fn wire_type(&mut self, idl_type: &IdlTypeSnapshot) -> WireType {
        match idl_type {
            IdlTypeSnapshot::Simple(name) => match primitive(name) {
                Some((base_type, integer_kind)) => WireType::Scalar {
                    base_type,
                    integer_kind,
                },
                None => self.defined(name),
            },
            IdlTypeSnapshot::Option(option) => {
                WireType::Option(Box::new(self.wire_type(&option.option)))
            }
            IdlTypeSnapshot::Vec(vec) => WireType::List(Box::new(self.wire_type(&vec.vec))),
            IdlTypeSnapshot::Array(array) => match array.array.as_slice() {
                [IdlArrayElementSnapshot::Type(element), IdlArrayElementSnapshot::Size(_)] => {
                    WireType::List(Box::new(self.wire_type(element)))
                }
                [IdlArrayElementSnapshot::TypeName(name), IdlArrayElementSnapshot::Size(_)] => {
                    WireType::List(Box::new(
                        self.wire_type(&IdlTypeSnapshot::Simple(name.clone())),
                    ))
                }
                _ => WireType::Json,
            },
            IdlTypeSnapshot::HashMap(map) => {
                WireType::Map(Box::new(self.wire_type(&map.hash_map.1)))
            }
            IdlTypeSnapshot::Tuple(tuple) => {
                let elements = tuple
                    .tuple
                    .iter()
                    .map(|element| self.wire_type(element))
                    .collect::<Vec<_>>();
                if elements.is_empty() || elements.len() > MAX_TUPLE_LEN {
                    WireType::Json
                } else {
                    WireType::Tuple(elements)
                }
            }
            IdlTypeSnapshot::Defined(defined) => match &defined.defined {
                IdlDefinedInnerSnapshot::Named { name } | IdlDefinedInnerSnapshot::Simple(name) => {
                    self.defined(name)
                }
            },
        }
    }

    fn defined(&mut self, name: &str) -> WireType {
        let Some(def) = lookup_type(self.idl, name) else {
            return WireType::Json;
        };
        let key = def.name.clone();
        if self.visiting.contains(&key) {
            return WireType::Json;
        }
        if !self.types.contains_key(&key) {
            self.visiting.push(key.clone());
            let kind = match &def.type_def {
                IdlTypeDefKindSnapshot::Struct { fields, .. } => {
                    IdlModelKind::Struct(self.fields(fields))
                }
                IdlTypeDefKindSnapshot::TupleStruct { fields, .. } => {
                    IdlModelKind::Struct(self.fields(&indexed_fields(fields.iter())))
                }
                IdlTypeDefKindSnapshot::Enum { variants, .. } => IdlModelKind::Enum(
                    variants
                        .iter()
                        .map(|variant| ModelVariant {
                            name: variant.name.clone(),
                            is_tuple: variant.fields.iter().all(|field| {
                                matches!(field, IdlEnumVariantFieldSnapshot::Tuple(_))
                            }),
                            fields: {
                                let fields = variant
                                    .fields
                                    .iter()
                                    .enumerate()
                                    .map(|(index, field)| match field {
                                        IdlEnumVariantFieldSnapshot::Named(named) => named.clone(),
                                        IdlEnumVariantFieldSnapshot::Tuple(type_) => {
                                            indexed_field(index, type_)
                                        }
                                    })
                                    .collect::<Vec<_>>();
                                self.fields(&fields)
                            },
                        })
                        .collect(),
                ),
            };
            self.visiting.pop();
            self.types.insert(
                key.clone(),
                IdlModel {
                    type_name: key.clone(),
                    is_account: false,
                    kind,
                },
            );
            self.order.push(key.clone());
        }
        WireType::Model(key)
    }
}

/// A tuple element as the wire keys it: `field_<index>`.
fn indexed_field(index: usize, type_: &IdlTypeSnapshot) -> IdlFieldSnapshot {
    IdlFieldSnapshot {
        name: format!("field_{index}"),
        type_: type_.clone(),
        amount_hint: None,
    }
}

fn indexed_fields<'a>(types: impl Iterator<Item = &'a IdlTypeSnapshot>) -> Vec<IdlFieldSnapshot> {
    types
        .enumerate()
        .map(|(index, type_)| indexed_field(index, type_))
        .collect()
}

/// How a language renders models, for [`bind_idl_models`].
pub(crate) struct ModelLanguage<'a> {
    /// The language's type-name case (`fee_config` -> `FeeConfig`).
    pub(crate) pascal: &'a dyn Fn(&str) -> String,
    /// A model rendered under a name, to tell whether two declarations are
    /// the same.
    pub(crate) render: &'a dyn Fn(&DeclaredModel, &str) -> String,
    /// The names other than its own a model declared under a name occupies
    /// (the variant classes of a Python enum).
    pub(crate) extra_names: &'a dyn Fn(&DeclaredModel, &str) -> Vec<String>,
    /// Names no model of any SDK takes (runtime envelopes, builtin structs).
    pub(crate) reserved: &'a [&'a str],
}

/// A model [`bind_idl_models`] declared, with its references resolved to
/// declared names.
pub(crate) type BoundModel = (String, IdlModel);

const PROBE: &str = "AccountModelProbe";

/// One namespace models are declared into: an SDK's shared types module, or
/// (for stable names) a standalone program SDK's.
struct Namespace<'n> {
    models: &'n mut AccountModels,
    taken: &'n dyn Fn(&str) -> bool,
    declared: Vec<BoundModel>,
}

enum Claim {
    /// Declared under this name.
    Declared(String),
    /// An identical model is already declared under this name.
    Shared(String),
}

impl Claim {
    fn name(&self) -> &str {
        match self {
            Claim::Declared(name) | Claim::Shared(name) => name,
        }
    }
}

impl Namespace<'_> {
    fn is_free(&self, name: &str) -> bool {
        self.models.model(name).is_none() && !self.models.is_occupied(name) && !(self.taken)(name)
    }

    /// Declare `model` under the first candidate that is free (or shares an
    /// identical declaration). `skip` rejects a candidate for this model.
    fn claim(
        &mut self,
        model: &IdlModel,
        candidates: impl Iterator<Item = String>,
        language: &ModelLanguage<'_>,
        skip: &dyn Fn(&str, &[String]) -> bool,
    ) -> Claim {
        let declared = DeclaredModel::Idl(model.clone());
        let rendered = (language.render)(&declared, PROBE);
        for candidate in candidates {
            if let Some(existing) = self.models.model(&candidate) {
                if (language.render)(existing, PROBE) == rendered {
                    return Claim::Shared(candidate);
                }
                continue;
            }
            let extra = (language.extra_names)(&declared, &candidate);
            if !self.is_free(&candidate)
                || extra.iter().any(|name| !self.is_free(name))
                || skip(&candidate, &extra)
            {
                continue;
            }
            self.models.declare_idl_model(&candidate, model, extra);
            self.declared.push((candidate.clone(), model.clone()));
            return Claim::Declared(candidate);
        }
        unreachable!("the numbered candidates are unbounded")
    }
}

/// The candidates for a defined type's model, most preferred first: its
/// stable name, prefixed with the program name, then with a `Type` role
/// suffix, then numbered (the TypeScript generator's order for IDL types).
fn type_model_candidates(stable: &str, program: Option<&str>) -> Box<dyn Iterator<Item = String>> {
    match program {
        Some(program) => {
            let prefixed = format!(
                "{}{stable}",
                ts_ident::identifier_stem(program, IdentifierCase::Pascal)
            );
            let role = format!("{prefixed}Type");
            Box::new(
                [stable.to_string(), prefixed, role.clone()]
                    .into_iter()
                    .chain((2usize..).map(move |index| format!("{role}{index}"))),
            )
        }
        None => {
            let role = format!("{stable}Type");
            Box::new(
                [stable.to_string(), role.clone()]
                    .into_iter()
                    .chain((2usize..).map(move |index| format!("{role}{index}"))),
            )
        }
    }
}

/// How one program's models are named in a namespace.
struct ProgramNaming<'p> {
    idl: &'p IdlSnapshot,
    models: &'p ProgramModels,
    /// Account name -> stable name.
    account_stable: BTreeMap<String, String>,
    /// Defined type name -> stable name; `None` while computing them.
    type_stable: Option<&'p BTreeMap<String, String>>,
}

impl ProgramNaming<'_> {
    /// Declare every model of the program: each account after the defined
    /// types first reached from it, dependencies first. Returns the name each
    /// defined type is declared under, by IDL name.
    fn bind(
        &self,
        namespace: &mut Namespace<'_>,
        language: &ModelLanguage<'_>,
    ) -> BTreeMap<String, String> {
        let mut names = BTreeMap::new();
        let account_stable_names = self
            .account_stable
            .values()
            .cloned()
            .collect::<BTreeSet<_>>();
        let account_definitions = self
            .models
            .accounts
            .iter()
            .filter_map(|(account, model, _)| {
                Some((
                    self.account_stable.get(account)?.clone(),
                    model.as_ref()?.definition(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        for (account, model, types) in &self.models.accounts {
            for type_name in types {
                let Some(type_model) = self.models.types.get(type_name) else {
                    continue;
                };
                let resolved = type_model.with_names(&names);
                let definition = type_model.definition();
                // A defined type never takes an account's stable name (or
                // hides one behind a variant class) unless it is that
                // account's own type.
                let skip = |candidate: &str, extra: &[String]| {
                    let own_account = account_definitions
                        .get(candidate)
                        .is_some_and(|account| *account == definition);
                    (account_stable_names.contains(candidate) && !own_account)
                        || extra.iter().any(|name| account_stable_names.contains(name))
                };
                let claim = match self.type_stable {
                    Some(stable) => {
                        let stable = stable
                            .get(type_name)
                            .cloned()
                            .unwrap_or_else(|| (language.pascal)(type_name));
                        let candidates = type_model_candidates(&stable, Some(&self.idl.name));
                        let claim = namespace.claim(&resolved, candidates, language, &skip);
                        namespace.models.bind_type(
                            &self.idl.name,
                            type_name,
                            claim.name(),
                            &stable,
                        );
                        claim
                    }
                    None => namespace.claim(
                        &resolved,
                        type_model_candidates(&(language.pascal)(type_name), None),
                        language,
                        &skip,
                    ),
                };
                names.insert(type_name.clone(), claim.name().to_string());
            }

            let stable = &self.account_stable[account];
            let Some(model) = model else {
                if self.type_stable.is_some() {
                    namespace.models.bind(&self.idl.name, account, None, stable);
                }
                continue;
            };
            let resolved = model.with_names(&names);
            let rendered = (language.render)(&DeclaredModel::Idl(resolved.clone()), PROBE);
            // The model the program's own entities map the account to, when
            // it declares the same fields.
            let own = namespace
                .models
                .own(&self.idl.name, account)
                .cloned()
                .filter(|own| {
                    namespace
                        .models
                        .model(own)
                        .is_some_and(|model| (language.render)(model, PROBE) == rendered)
                });
            let name = match own {
                Some(own) => own,
                None => namespace
                    .claim(
                        &resolved,
                        account_model_candidates(stable, &self.idl.name),
                        language,
                        &|_, _| false,
                    )
                    .name()
                    .to_string(),
            };
            if self.type_stable.is_some() {
                namespace
                    .models
                    .bind(&self.idl.name, account, Some(&name), stable);
            }
        }
        names
    }
}

/// The names a program's standalone SDK declares its defined types' models
/// under, by IDL name. They depend on the program's IDL alone.
fn stable_type_model_names(
    idl: &IdlSnapshot,
    models: &ProgramModels,
    account_stable: &BTreeMap<String, String>,
    language: &ModelLanguage<'_>,
) -> BTreeMap<String, String> {
    let mut standalone = AccountModels::default();
    let reserved = language
        .reserved
        .iter()
        .map(|name| name.to_string())
        .collect::<HashSet<_>>();
    let taken = |name: &str| reserved.contains(name);
    let mut namespace = Namespace {
        models: &mut standalone,
        taken: &taken,
        declared: Vec::new(),
    };
    ProgramNaming {
        idl,
        models,
        account_stable: account_stable.clone(),
        type_stable: None,
    }
    .bind(&mut namespace, language)
}

/// Bind every IDL account of the `programs` (by IDL name) to the model its
/// reader decodes into, and every defined type those reach to a model of its
/// own, returning the models the shared types module must declare for them,
/// in order (a model after the models it references).
///
/// An account keeps the model its program's entities already map when that
/// renders the same (so a stack's captured-account types and its account
/// readers stay one type); otherwise its model is declared under the first
/// free candidate ([`account_model_candidates`]; a defined type's are its
/// stable name, then prefixed with the program name, then with a `Type`
/// suffix), sharing any declared model that renders the same. `taken`
/// reports names the module declares for something else.
pub(crate) fn bind_idl_models(
    idls: &[IdlSnapshot],
    programs: &HashSet<String>,
    account_models: &mut AccountModels,
    language: &ModelLanguage<'_>,
    taken: &dyn Fn(&str) -> bool,
) -> Vec<BoundModel> {
    let taken = |name: &str| taken(name) || language.reserved.contains(&name);
    let mut namespace = Namespace {
        models: account_models,
        taken: &taken,
        declared: Vec::new(),
    };
    for idl in idls.iter().filter(|idl| programs.contains(&idl.name)) {
        let models = ProgramModels::build(idl);
        let account_stable = stable_account_model_names(idl, language.pascal, language.reserved);
        let type_stable = stable_type_model_names(idl, &models, &account_stable, language);
        ProgramNaming {
            idl,
            models: &models,
            account_stable,
            type_stable: Some(&type_stable),
        }
        .bind(&mut namespace, language);
    }
    namespace.declared
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The IDL of a checked-in ProgramSpec artifact, as generation sees it.
    pub(crate) fn program_spec_stack(
        bytes: &[u8],
        name: &str,
    ) -> crate::ast::SerializableStackSpec {
        let artifact = arete_artifacts::load_program_spec(bytes)
            .expect("fixture ProgramSpec loads")
            .artifact;
        crate::public_artifacts::stack_spec_from_program_artifacts(name, &[artifact])
            .expect("fixture ProgramSpec builds a program-only stack spec")
    }

    /// ORE: `Config` nests two structs, `Miner`/`Treasury` a `Numeric`.
    pub(crate) fn ore_stack() -> crate::ast::SerializableStackSpec {
        program_spec_stack(
            include_bytes!("../../stacks/ore/.arete/ore.program-spec.json"),
            "Ore",
        )
    }

    /// Pyth receiver: a data-carrying enum (`VerificationLevel`) with a
    /// camelCase field, nested structs, a vector of structs.
    pub(crate) fn pyth_rec_stack() -> crate::ast::SerializableStackSpec {
        program_spec_stack(
            include_bytes!("../tests/fixtures/program-specs/pyth-rec.program-spec.json"),
            "PythRec",
        )
    }

    /// Metaplex Core: tuple variants (`UpdateAuthority::Address(Pubkey)`),
    /// struct variants, vectors of structs holding enums, and an
    /// `Option<Vec<(enum, struct)>>`.
    pub(crate) fn mpl_core_stack() -> crate::ast::SerializableStackSpec {
        program_spec_stack(
            include_bytes!("../tests/fixtures/program-specs/mpl-core.program-spec.json"),
            "MplCore",
        )
    }

    fn account<'m>(models: &'m ProgramModels, name: &str) -> (&'m IdlModel, &'m [String]) {
        let (_, model, types) = models
            .accounts
            .iter()
            .find(|(account, _, _)| account == name)
            .unwrap_or_else(|| panic!("no account {name}"));
        (model.as_ref().expect("a struct account"), types)
    }

    fn field<'m>(fields: &'m [ModelField], name: &str) -> &'m ModelField {
        fields
            .iter()
            .find(|field| field.flat.raw_field_name() == name)
            .unwrap_or_else(|| panic!("no field {name}"))
    }

    fn fields(model: &IdlModel) -> &[ModelField] {
        match &model.kind {
            IdlModelKind::Struct(fields) => fields,
            IdlModelKind::Enum(_) => panic!("{} is an enum", model.type_name),
        }
    }

    fn variants(model: &IdlModel) -> &[ModelVariant] {
        match &model.kind {
            IdlModelKind::Enum(variants) => variants,
            IdlModelKind::Struct(_) => panic!("{} is a struct", model.type_name),
        }
    }

    fn model(name: &str) -> WireType {
        WireType::Model(name.to_string())
    }

    #[test]
    fn wire_names_are_snake_case_and_never_keyword_escaped() {
        assert_eq!(idl_wire_name("fee_rate"), "fee_rate");
        assert_eq!(idl_wire_name("numSignatures"), "num_signatures");
        assert_eq!(idl_wire_name("_padding_0"), "_padding_0");
        assert_eq!(idl_wire_name("type"), "type");
    }

    #[test]
    fn ore_account_models_reach_their_nested_defined_types() {
        let spec = ore_stack();
        let models = ProgramModels::build(&spec.idls[0]);

        // `Config` reaches its two structs, declared before it.
        let (config, reached) = account(&models, "Config");
        assert_eq!(reached, ["AdminConfig", "ProtocolConfig"]);
        assert_eq!(
            field(fields(config), "admin").typed,
            Some(model("AdminConfig"))
        );
        assert_eq!(
            field(fields(config), "protocol").typed,
            Some(model("ProtocolConfig"))
        );
        let admin = &models.types["AdminConfig"];
        assert!(!admin.is_account);
        // Its `u64` stays a flat field (decimal strings decode to integers).
        let fee_rate = field(fields(admin), "fee_rate");
        assert_eq!(fee_rate.typed, None);
        assert_eq!(fee_rate.flat.integer_kind, Some(IntegerKind::U64));

        // `Numeric` is reached once, from the first account using it.
        let (_, reached) = account(&models, "Miner");
        assert_eq!(reached, ["Numeric"]);
        let (treasury, reached) = account(&models, "Treasury");
        assert!(reached.is_empty());
        assert_eq!(
            field(fields(treasury), "miner_rewards_factor").typed,
            Some(model("Numeric"))
        );
        // A flat account types nothing.
        let (board, reached) = account(&models, "Board");
        assert!(reached.is_empty());
        assert!(fields(board).iter().all(|field| field.typed.is_none()));
    }

    #[test]
    fn data_enums_follow_the_program_read_wire_shape() {
        let spec = pyth_rec_stack();
        let models = ProgramModels::build(&spec.idls[0]);
        let (update, reached) = account(&models, "priceUpdateV2");
        assert_eq!(reached, ["VerificationLevel", "PriceFeedMessage"]);
        assert_eq!(
            field(fields(update), "verificationLevel").typed,
            Some(model("VerificationLevel"))
        );
        let level = variants(&models.types["VerificationLevel"]);
        assert_eq!(
            level
                .iter()
                .map(|variant| (variant.name.as_str(), variant.fields.len()))
                .collect::<Vec<_>>(),
            [("Partial", 1), ("Full", 0)]
        );
        // A named variant field keeps its name; its wire key is snake_case.
        assert_eq!(level[0].fields[0].flat.raw_field_name(), "numSignatures");
        assert_eq!(level[0].fields[0].wire_name(), "num_signatures");

        // Tuple variant fields keep their positional wire shape. The
        // synthetic names are only used while binding their field types.
        let spec = mpl_core_stack();
        let models = ProgramModels::build(&spec.idls[0]);
        let authority = variants(&models.types["UpdateAuthority"]);
        assert_eq!(authority[1].name, "Address");
        assert!(authority[1].is_tuple);
        assert_eq!(authority[1].fields[0].flat.raw_field_name(), "field_0");
        assert_eq!(authority[1].fields[0].flat.base_type, BaseType::Pubkey);
        // A struct variant keeps its field names.
        let owner = variants(&models.types["Authority"]);
        assert_eq!(owner[3].fields[0].flat.raw_field_name(), "address");

        // An `Option<Vec<(enum, struct)>>` is typed through every layer.
        let record = &models.types["ExternalRegistryRecord"];
        assert_eq!(
            field(fields(record), "lifecycleChecks").typed,
            Some(WireType::Option(Box::new(WireType::List(Box::new(
                WireType::Tuple(vec![
                    model("HookableLifecycleEvent"),
                    model("ExternalCheckResult"),
                ])
            )))))
        );
        // A vector of structs holding enums reaches both.
        let (_, reached) = account(&models, "PluginRegistryV1");
        assert!(reached.contains(&"RegistryRecord".to_string()));
        assert!(reached.contains(&"PluginType".to_string()));
        assert!(reached.contains(&"Authority".to_string()));
    }

    fn synthetic_idl(
        name: &str,
        accounts: serde_json::Value,
        types: serde_json::Value,
    ) -> IdlSnapshot {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "version": "0.1.0",
            "accounts": accounts,
            "instructions": [],
            "types": types,
            "discriminant_size": 8
        }))
        .expect("test IDL")
    }

    #[test]
    fn nested_containers_are_typed_and_recursion_reads_as_json() {
        let idl = synthetic_idl(
            "nested",
            serde_json::json!([{
                "name": "Holder",
                "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
                "fields": [
                    { "name": "amounts", "type": { "vec": { "option": "u64" } } },
                    { "name": "pair", "type": { "tuple": ["u64", "publicKey"] } },
                    { "name": "balances", "type": { "hashMap": ["string", "u128"] } },
                    { "name": "grid", "type": { "array": [{ "vec": "i64" }, 2] } },
                    { "name": "maybe", "type": { "option": "u64" } },
                    { "name": "node", "type": { "defined": "Node" } },
                    { "name": "unknown", "type": { "defined": "Missing" } }
                ]
            }]),
            serde_json::json!([{
                "name": "Node",
                "type": { "kind": "struct", "fields": [
                    { "name": "next", "type": { "option": { "defined": "Node" } } }
                ] }
            }]),
        );
        let models = ProgramModels::build(&idl);
        let (holder, reached) = account(&models, "Holder");
        let fields = fields(holder);
        let int = |kind| WireType::Scalar {
            base_type: BaseType::Integer,
            integer_kind: Some(kind),
        };
        assert_eq!(
            field(fields, "amounts").typed,
            Some(WireType::List(Box::new(WireType::Option(Box::new(int(
                IntegerKind::U64
            ))))))
        );
        assert_eq!(
            field(fields, "pair").typed,
            Some(WireType::Tuple(vec![
                int(IntegerKind::U64),
                WireType::Scalar {
                    base_type: BaseType::Pubkey,
                    integer_kind: None
                }
            ]))
        );
        assert_eq!(
            field(fields, "balances").typed,
            Some(WireType::Map(Box::new(int(IntegerKind::U128))))
        );
        assert_eq!(
            field(fields, "grid").typed,
            Some(WireType::List(Box::new(WireType::List(Box::new(int(
                IntegerKind::I64
            ))))))
        );
        // Flat shapes keep the flat projection.
        assert_eq!(field(fields, "maybe").typed, None);
        assert_eq!(field(fields, "unknown").typed, Some(WireType::Json));
        // `Node` references itself: the back reference reads as raw JSON.
        assert_eq!(reached, ["Node"]);
        assert_eq!(
            super::tests::fields(&models.types["Node"])[0].typed,
            Some(WireType::Option(Box::new(WireType::Json)))
        );
    }

    fn debug_language<'a>(reserved: &'a [&'a str]) -> ModelLanguage<'a> {
        ModelLanguage {
            pascal: &|name: &str| {
                let mut chars = name.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            },
            render: &|model, _| match model {
                DeclaredModel::Idl(model) => model.definition(),
                DeclaredModel::Resolved(resolved) => format!("{:?}", resolved.fields),
            },
            extra_names: &|_, _| Vec::new(),
            reserved,
        }
    }

    fn pool_program(name: &str, fee_type: &str) -> IdlSnapshot {
        synthetic_idl(
            name,
            serde_json::json!([{
                "name": "Pool",
                "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
                "fields": [
                    { "name": "fee", "type": { "defined": "Fee" } },
                    { "name": "curve", "type": { "defined": "Curve" } }
                ]
            }]),
            serde_json::json!([
                { "name": "Fee", "type": { "kind": "struct", "fields": [
                    { "name": "bps", "type": fee_type }
                ] } },
                { "name": "Curve", "type": { "kind": "enum", "variants": [
                    { "name": "Flat" },
                    { "name": "Stable", "fields": [{ "name": "fee", "type": { "defined": "Fee" } }] }
                ] } }
            ]),
        )
    }

    #[test]
    fn stack_models_share_identical_types_and_rename_different_ones() {
        // `beta`'s `Fee` differs from `alpha`'s; `gamma`'s is identical.
        let idls = vec![
            pool_program("alpha", "u16"),
            pool_program("beta", "u64"),
            pool_program("gamma", "u16"),
        ];
        let programs = idls.iter().map(|idl| idl.name.clone()).collect();
        let mut models = AccountModels::default();
        let declared =
            bind_idl_models(&idls, &programs, &mut models, &debug_language(&[]), &|_| {
                false
            });
        assert_eq!(
            declared
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            // `beta`'s `Curve` references its own `Fee`, so it differs too.
            ["Fee", "Curve", "Pool", "BetaFee", "BetaCurve", "BetaPool"]
        );
        // Every program keeps its standalone (stable) names.
        for program in ["alpha", "gamma"] {
            assert_eq!(
                models.program_model_names(program),
                [
                    ("Pool".to_string(), "Pool".to_string()),
                    ("Fee".to_string(), "Fee".to_string()),
                    ("Curve".to_string(), "Curve".to_string()),
                ]
            );
        }
        assert_eq!(
            models.program_model_names("beta"),
            [
                ("Pool".to_string(), "BetaPool".to_string()),
                ("Fee".to_string(), "BetaFee".to_string()),
                ("Curve".to_string(), "BetaCurve".to_string()),
            ]
        );
        // `beta`'s `Pool` references its renamed types.
        let (_, beta_pool) = &declared[5];
        assert_eq!(fields(beta_pool)[0].typed, Some(model("BetaFee")));
    }

    #[test]
    fn a_type_never_takes_an_accounts_stable_name_unless_it_is_that_account() {
        // The type `Pool` differs from the account `Pool`, whose `fee` field
        // reaches it (declared first): the type takes `PoolType`.
        let idl = synthetic_idl(
            "clash",
            serde_json::json!([{
                "name": "Pool",
                "discriminator": [1, 0, 0, 0, 0, 0, 0, 0],
                "fields": [{ "name": "inner", "type": { "defined": "pool" } }]
            }]),
            serde_json::json!([{ "name": "pool", "type": { "kind": "struct", "fields": [
                { "name": "bps", "type": "u16" }
            ] } }]),
        );
        let programs = HashSet::from([idl.name.clone()]);
        let mut models = AccountModels::default();
        bind_idl_models(
            &[idl],
            &programs,
            &mut models,
            &debug_language(&["Option"]),
            &|_| false,
        );
        assert_eq!(
            models.program_model_names("clash"),
            [
                ("Pool".to_string(), "Pool".to_string()),
                ("PoolType".to_string(), "PoolType".to_string()),
            ]
        );
    }
}
