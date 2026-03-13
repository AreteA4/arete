#![allow(dead_code)]

/// Extract the base name from a potentially scoped event type.
/// "ore::RoundState" -> "RoundState"
/// "RoundState" -> "RoundState"  (backwards compat)
pub fn event_type_base_name(event_type: &str) -> &str {
    event_type.rsplit("::").next().unwrap_or(event_type)
}

/// Extract the program name from a scoped event type.
/// "ore::RoundState" -> Some("ore")
/// "RoundState" -> None
pub fn event_type_program(event_type: &str) -> Option<&str> {
    event_type.rsplit_once("::").map(|(prefix, _)| prefix)
}

/// Strip the "State" or "IxState" suffix from a potentially scoped event type,
/// returning just the base account/instruction name.
/// "ore::RoundState" -> "Round"
/// "ore::DeployIxState" -> "Deploy"
/// "RoundState" -> "Round"
pub fn strip_event_type_suffix(event_type: &str) -> &str {
    let base = event_type_base_name(event_type);
    base.strip_suffix("IxState")
        .or_else(|| base.strip_suffix("State"))
        .unwrap_or(base)
}

/// Create a scoped event type name.
/// ("ore", "Round", false) -> "ore::RoundState"
/// ("ore", "Deploy", true) -> "ore::DeployIxState"
pub fn scoped_event_type(program_name: &str, type_name: &str, is_instruction: bool) -> String {
    let suffix = if is_instruction { "IxState" } else { "State" };
    format!("{}::{}{}", program_name, type_name, suffix)
}

use crate::parse::idl::IdlSpec;

pub type IdlLookup<'a> = &'a [(String, &'a IdlSpec)];

pub fn find_idl_for_type<'a>(type_str: &str, idls: IdlLookup<'a>) -> Option<&'a IdlSpec> {
    if idls.is_empty() {
        return None;
    }
    let first_segment = type_str.split("::").next()?.trim();
    idls.iter()
        .find(|(sdk_name, _)| sdk_name == first_segment)
        .map(|(_, idl)| *idl)
        .or_else(|| Some(idls[0].1))
}

pub fn program_name_for_type<'a>(type_str: &str, idls: IdlLookup<'a>) -> Option<&'a str> {
    find_idl_for_type(type_str, idls).map(|idl| idl.get_name())
}

pub fn program_name_from_sdk_prefix(sdk_module: &str) -> &str {
    sdk_module.strip_suffix("_sdk").unwrap_or(sdk_module)
}

/// Convert a snake_case identifier to lowerCamelCase.
/// "bonding_curve" -> "bondingCurve"
/// "mint" -> "mint" (single word unchanged)
/// "associated_bonding_curve" -> "associatedBondingCurve"
pub fn snake_to_lower_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}
