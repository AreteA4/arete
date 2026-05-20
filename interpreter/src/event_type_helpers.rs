#![allow(dead_code)]

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

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

/// Canonicalize an instruction event type while preserving any program scope.
///
/// Examples:
/// - `pump::buyIxState` -> `pump::BuyIxState`
/// - `buy_exact_sol_inIxState` -> `BuyExactSolInIxState`
pub fn canonicalize_instruction_event_type(event_type: &str) -> String {
    let (program_name, base_name) = match event_type.rsplit_once("::") {
        Some((program_name, base_name)) => (Some(program_name), base_name),
        None => (None, event_type),
    };

    let instruction_name = base_name.strip_suffix("IxState").unwrap_or(base_name);
    let canonical_name = to_pascal_case(instruction_name);

    match program_name {
        Some(program_name) => format!("{}::{}IxState", program_name, canonical_name),
        None => format!("{}IxState", canonical_name),
    }
}

/// Normalize legacy event type names emitted by older AST generators.
///
/// Account state names are already generated from IDL account identifiers and are
/// left unchanged here. Only instruction event types are canonicalized.
pub fn canonicalize_event_type_name(event_type: &str) -> String {
    if event_type.ends_with("IxState") {
        canonicalize_instruction_event_type(event_type)
    } else {
        event_type.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_event_type_name, canonicalize_instruction_event_type};

    #[test]
    fn canonicalize_instruction_event_type_normalizes_scoped_snake_case() {
        assert_eq!(
            canonicalize_instruction_event_type("pump::buy_exact_sol_inIxState"),
            "pump::BuyExactSolInIxState"
        );
    }

    #[test]
    fn canonicalize_instruction_event_type_preserves_pascal_case() {
        assert_eq!(
            canonicalize_instruction_event_type("ore::DeployIxState"),
            "ore::DeployIxState"
        );
    }

    #[test]
    fn canonicalize_event_type_name_leaves_accounts_unchanged() {
        assert_eq!(
            canonicalize_event_type_name("pump::BondingCurveState"),
            "pump::BondingCurveState"
        );
    }
}
