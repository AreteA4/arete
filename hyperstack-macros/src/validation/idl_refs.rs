use crate::event_type_helpers::{find_idl_for_type, IdlLookup};
use crate::parse;
use crate::utils::path_to_string;
use hyperstack_idl::error::IdlSearchError;
use hyperstack_idl::search::{
    lookup_account, lookup_instruction, lookup_instruction_field, lookup_type, suggest_similar,
    InstructionFieldKind,
};
use hyperstack_idl::types::{IdlSpec, IdlTypeDefKind};

fn not_found_with_suggestions(
    input: &str,
    section: String,
    available: Vec<String>,
) -> IdlSearchError {
    let candidates: Vec<&str> = available.iter().map(String::as_str).collect();
    let suggestions = suggest_similar(input, &candidates, 3);
    IdlSearchError::NotFound {
        input: input.to_string(),
        section,
        suggestions,
        available,
    }
}

fn find_idl_for_program_name<'a>(program_name: &str, idls: IdlLookup<'a>) -> Option<&'a IdlSpec> {
    idls.iter()
        .find(|(_, idl)| idl.get_name() == program_name)
        .map(|(_, idl)| *idl)
        .or_else(|| {
            let sdk_name = format!("{}_sdk", program_name);
            idls.iter()
                .find(|(name, _)| name == &sdk_name)
                .map(|(_, idl)| *idl)
        })
}

pub fn resolve_instruction_lookup<'a>(
    event_attr: &parse::EventAttribute,
    fallback_instruction_key: &str,
    idls: IdlLookup<'a>,
) -> Result<(&'a IdlSpec, String), IdlSearchError> {
    if let Some(path) = event_attr
        .from_instruction
        .as_ref()
        .or(event_attr.inferred_instruction.as_ref())
    {
        return resolve_instruction_lookup_from_path(path, idls);
    }

    if !event_attr.instruction.is_empty() {
        return resolve_instruction_lookup_from_string(&event_attr.instruction, idls);
    }

    resolve_instruction_lookup_from_string(fallback_instruction_key, idls)
}

pub fn resolve_instruction_lookup_from_path<'a>(
    instruction_path: &syn::Path,
    idls: IdlLookup<'a>,
) -> Result<(&'a IdlSpec, String), IdlSearchError> {
    let type_str = path_to_string(instruction_path);
    let idl = find_idl_for_type(&type_str, idls).ok_or_else(|| IdlSearchError::InvalidPath {
        path: type_str.clone(),
    })?;
    let instruction_name = instruction_path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .ok_or_else(|| IdlSearchError::InvalidPath {
            path: type_str.clone(),
        })?;
    lookup_instruction(idl, &instruction_name)?;
    Ok((idl, instruction_name))
}

pub fn resolve_instruction_lookup_from_string<'a>(
    instruction: &str,
    idls: IdlLookup<'a>,
) -> Result<(&'a IdlSpec, String), IdlSearchError> {
    let (program_name, instruction_name) =
        instruction
            .rsplit_once("::")
            .ok_or_else(|| IdlSearchError::InvalidPath {
                path: instruction.to_string(),
            })?;

    let idl = find_idl_for_program_name(program_name, idls).ok_or_else(|| {
        IdlSearchError::InvalidPath {
            path: instruction.to_string(),
        }
    })?;

    lookup_instruction(idl, instruction_name)?;
    Ok((idl, instruction_name.to_string()))
}

pub fn validate_instruction_field_spec(
    idl: &IdlSpec,
    instruction_name: &str,
    field_spec: &parse::FieldSpec,
) -> Result<(), IdlSearchError> {
    let lookup = lookup_instruction_field(idl, instruction_name, &field_spec.ident.to_string())?;
    if let Some(location) = &field_spec.explicit_location {
        match (location, lookup.kind) {
            (parse::FieldLocation::Account, InstructionFieldKind::Account)
            | (parse::FieldLocation::InstructionArg, InstructionFieldKind::Arg) => {}
            (parse::FieldLocation::Account, InstructionFieldKind::Arg) => {
                return Err(IdlSearchError::InvalidPath {
                    path: format!(
                        "accounts::{} is not valid for instruction '{}'",
                        field_spec.ident, instruction_name
                    ),
                });
            }
            (parse::FieldLocation::InstructionArg, InstructionFieldKind::Account) => {
                return Err(IdlSearchError::InvalidPath {
                    path: format!(
                        "args::{} is not valid for instruction '{}'",
                        field_spec.ident, instruction_name
                    ),
                });
            }
        }
    }
    Ok(())
}

fn fields_from_type_def(type_def: &IdlTypeDefKind) -> Vec<String> {
    match type_def {
        IdlTypeDefKind::Struct { fields, .. } => {
            fields.iter().map(|field| field.name.clone()).collect()
        }
        _ => Vec::new(),
    }
}

fn account_fields(idl: &IdlSpec, account_name: &str) -> Result<Vec<String>, IdlSearchError> {
    let account = lookup_account(idl, account_name)?;
    if let Some(type_def) = &account.type_def {
        return Ok(fields_from_type_def(type_def));
    }

    match lookup_type(idl, account_name) {
        Ok(type_def) => Ok(fields_from_type_def(&type_def.type_def)),
        Err(_) => Ok(Vec::new()),
    }
}

pub fn validate_account_field(
    idl: &IdlSpec,
    account_name: &str,
    field_name: &str,
) -> Result<(), IdlSearchError> {
    let fields = account_fields(idl, account_name)?;
    // Some IDLs omit struct field metadata for accounts; keep that case non-fatal
    // so validation does not reject otherwise valid mappings on incomplete schemas.
    if fields.is_empty() || fields.iter().any(|field| field == field_name) {
        return Ok(());
    }

    Err(not_found_with_suggestions(
        field_name,
        format!("account fields for '{}'", account_name),
        fields,
    ))
}

pub fn validate_mapping_source(
    source_type: &str,
    mapping: &parse::MapAttribute,
    idls: IdlLookup,
) -> Result<(), IdlSearchError> {
    if mapping.source_field_name.is_empty() || mapping.source_field_name.starts_with("__") {
        return Ok(());
    }

    if source_type.contains("::instructions::") || mapping.is_instruction {
        let path =
            syn::parse_str::<syn::Path>(source_type).map_err(|_| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let (idl, instruction_name) = resolve_instruction_lookup_from_path(&path, idls)?;
        let temp_field = parse::FieldSpec {
            ident: syn::Ident::new(&mapping.source_field_name, mapping.source_field_span),
            explicit_location: None,
        };
        validate_instruction_field_spec(idl, &instruction_name, &temp_field)
    } else if source_type.contains("::accounts::") {
        let path =
            syn::parse_str::<syn::Path>(source_type).map_err(|_| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let idl =
            find_idl_for_type(source_type, idls).ok_or_else(|| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        let account_name = path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .ok_or_else(|| IdlSearchError::InvalidPath {
                path: source_type.to_string(),
            })?;
        validate_account_field(idl, &account_name, &mapping.source_field_name)
    } else {
        Ok(())
    }
}
