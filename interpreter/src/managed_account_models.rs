//! JSON account models for IDL-only Rust programs as well as stack programs.
use crate::ast::*;
use crate::rust::{to_pascal_case, to_snake_case};
use crate::stack_types::AccountModels;
use std::collections::{BTreeMap, HashSet};

fn integer(ty: &IdlTypeSnapshot) -> bool {
    matches!(ty, IdlTypeSnapshot::Simple(name) if matches!(name.as_str(), "u64" | "i64" | "u128" | "i128"))
}

fn type_name(ty: &IdlTypeSnapshot, names: &BTreeMap<String, String>) -> String {
    match ty {
        IdlTypeSnapshot::Simple(name) => match name.as_str() {
            "pubkey" | "publicKey" | "string" => "String".into(),
            "bytes" => "Vec<u8>".into(),
            "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128"
            | "f32" | "f64" | "bool" => name.clone(),
            _ => names
                .get(name)
                .cloned()
                .unwrap_or_else(|| "serde_json::Value".into()),
        },
        IdlTypeSnapshot::Defined(defined) => {
            let name = match &defined.defined {
                IdlDefinedInnerSnapshot::Simple(name) | IdlDefinedInnerSnapshot::Named { name } => {
                    name
                }
            };
            names
                .get(name)
                .cloned()
                .unwrap_or_else(|| "serde_json::Value".into())
        }
        IdlTypeSnapshot::Option(option) => format!("Option<{}>", type_name(&option.option, names)),
        IdlTypeSnapshot::Vec(vector) => format!("Vec<{}>", type_name(&vector.vec, names)),
        IdlTypeSnapshot::Array(array) => format!(
            "Vec<{}>",
            array
                .array
                .first()
                .map(|element| match element {
                    IdlArrayElementSnapshot::Type(ty) => type_name(ty, names),
                    IdlArrayElementSnapshot::TypeName(name) =>
                        type_name(&IdlTypeSnapshot::Simple(name.clone()), names),
                    _ => "serde_json::Value".into(),
                })
                .unwrap_or_else(|| "serde_json::Value".into())
        ),
        IdlTypeSnapshot::Tuple(tuple) => format!(
            "({},)",
            tuple
                .tuple
                .iter()
                .map(|ty| type_name(ty, names))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        // JSON account transport cannot faithfully represent arbitrary non-string map keys.
        IdlTypeSnapshot::HashMap(_) => "serde_json::Value".into(),
    }
}

fn integer_attribute(ty: &IdlTypeSnapshot) -> &'static str {
    if integer(ty) {
        "#[serde(deserialize_with = \"serde_utils::deserialize_integer\")]\n"
    } else if matches!(ty, IdlTypeSnapshot::Option(option) if integer(&option.option)) {
        "#[serde(deserialize_with = \"serde_utils::deserialize_optional_integer\")]\n"
    } else if matches!(ty, IdlTypeSnapshot::Vec(vector) if integer(&vector.vec))
        || matches!(ty, IdlTypeSnapshot::Array(array) if array.array.first().is_some_and(|element| match element { IdlArrayElementSnapshot::Type(ty) => integer(ty), IdlArrayElementSnapshot::TypeName(name) => integer(&IdlTypeSnapshot::Simple(name.clone())), _ => false }))
    {
        "#[serde(deserialize_with = \"serde_utils::deserialize_integer_vec\")]\n"
    } else {
        ""
    }
}

fn field(field: &IdlFieldSnapshot, names: &BTreeMap<String, String>, public: bool) -> String {
    let name = to_snake_case(&field.name);
    let alias = if name != field.name {
        format!("#[serde(alias = {:?})]\n", field.name)
    } else {
        String::new()
    };
    format!(
        "{}{}{}{}: {},",
        alias,
        integer_attribute(&field.type_),
        if public { "pub " } else { "" },
        crate::identifiers::rust::identifier(&name, crate::identifiers::IdentifierCase::Preserve),
        type_name(&field.type_, names)
    )
}

pub(crate) fn definition(
    def: &IdlTypeDefSnapshot,
    name: &str,
    names: &BTreeMap<String, String>,
) -> String {
    let body = match &def.type_def {
        IdlTypeDefKindSnapshot::Struct { fields, .. } => format!(
            "pub struct {name} {{\n{}\n}}",
            fields
                .iter()
                .map(|f| field(f, names, true))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        IdlTypeDefKindSnapshot::TupleStruct { fields, .. } => format!(
            "pub struct {name}({});",
            fields
                .iter()
                .map(|ty| format!("{}pub {}", integer_attribute(ty), type_name(ty, names)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        IdlTypeDefKindSnapshot::Enum { variants, .. } => {
            let variants = variants
                .iter()
                .map(|variant| {
                    let label = to_pascal_case(&variant.name);
                    let payload = if variant.fields.is_empty() {
                        String::new()
                    } else if variant
                        .fields
                        .iter()
                        .all(|f| matches!(f, IdlEnumVariantFieldSnapshot::Named(_)))
                    {
                        format!(
                            " {{ {} }}",
                            variant
                                .fields
                                .iter()
                                .map(|f| match f {
                                    IdlEnumVariantFieldSnapshot::Named(f) => field(f, names, false),
                                    _ => unreachable!(),
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    } else if variant
                        .fields
                        .iter()
                        .all(|f| matches!(f, IdlEnumVariantFieldSnapshot::Tuple(_)))
                    {
                        format!(
                                "({})",
                                variant
                                    .fields
                                    .iter()
                                    .map(|f| match f {
                                        IdlEnumVariantFieldSnapshot::Tuple(ty) => format!(
                                            "{}{}",
                                            integer_attribute(ty),
                                            type_name(ty, names)
                                        ),
                                        _ => unreachable!(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                    } else {
                        return "compile_error!(\"Unsupported mixed enum payload\");".into();
                    };
                    format!("#[serde(rename = {:?})]\n{label}{payload},", variant.name)
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("pub enum {name} {{\n{variants}\n}}")
        }
    };
    format!("#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n{body}\n\n")
}

pub(crate) fn append_models(
    output: &mut String,
    idls: &[IdlSnapshot],
    generated: &mut HashSet<String>,
    accounts: &mut AccountModels,
) {
    for idl in idls {
        let mut reserved = generated.clone();
        let mut names = BTreeMap::new();
        for def in &idl.types {
            let name = accounts
                .get(Some(&idl.name), &def.name)
                .cloned()
                .unwrap_or_else(|| claim_name(&def.name, &idl.name, &mut reserved));
            reserved.insert(name.clone());
            names.insert(def.name.clone(), name);
        }
        for account in &idl.accounts {
            let name = names
                .entry(account.name.clone())
                .or_insert_with(|| {
                    accounts
                        .get(Some(&idl.name), &account.name)
                        .cloned()
                        .unwrap_or_else(|| claim_name(&account.name, &idl.name, &mut reserved))
                })
                .clone();
            accounts.record(Some(&idl.name), &account.name, &name);
            if !account.fields.is_empty()
                && !idl.types.iter().any(|def| def.name == account.name)
                && generated.insert(name.clone())
            {
                output.push_str(&definition(
                    &IdlTypeDefSnapshot {
                        name: account.name.clone(),
                        docs: vec![],
                        serialization: None,
                        type_def: IdlTypeDefKindSnapshot::Struct {
                            kind: "struct".into(),
                            fields: account.fields.clone(),
                        },
                    },
                    &name,
                    &names,
                ));
            }
        }
        for def in &idl.types {
            let name = &names[&def.name];
            if generated.insert(name.clone()) {
                output.push_str(&definition(def, name, &names));
            }
        }
    }
}

fn claim_name(raw: &str, program: &str, reserved: &mut HashSet<String>) -> String {
    let base = to_pascal_case(raw);
    if reserved.insert(base.clone()) {
        return base;
    }
    let prefixed = format!("{}{}", to_pascal_case(program), base);
    if reserved.insert(prefixed.clone()) {
        return prefixed;
    }
    let mut index = 2;
    loop {
        let candidate = format!("{prefixed}{index}");
        if reserved.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}
