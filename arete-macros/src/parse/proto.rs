//! Proto file parsing.
//!
//! Parses protobuf schema files for code generation.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ProtoAnalysis {
    pub package: String,
    pub messages: HashMap<String, ProtoMessage>,
    pub oneof_types: Vec<OneofType>,
}

#[derive(Debug, Clone)]
pub struct ProtoMessage {
    pub name: String,
    pub fields: Vec<ProtoField>,
}

#[derive(Debug, Clone)]
pub struct ProtoField {
    pub name: String,
    pub proto_type: String,
    pub field_number: u32,
}

#[derive(Debug, Clone)]
pub struct OneofType {
    pub message_name: String,
    pub oneof_field: String,
    pub variants: Vec<OneofVariant>,
}

#[derive(Debug, Clone)]
pub struct OneofVariant {
    pub type_name: String,
    pub field_name: String,
    pub number: u32,
}

pub fn parse_proto_file<P: AsRef<Path>>(path: P) -> Result<ProtoAnalysis, String> {
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read proto file {:?}: {}", path.as_ref(), e))?;

    parse_proto_content(&content)
}

pub fn parse_proto_content(content: &str) -> Result<ProtoAnalysis, String> {
    let mut analysis = ProtoAnalysis {
        package: String::new(),
        messages: HashMap::new(),
        oneof_types: Vec::new(),
    };

    analysis.package = extract_package(content)?;
    extract_messages(content, &mut analysis)?;
    extract_oneofs(content, &mut analysis)?;

    Ok(analysis)
}

fn extract_package(content: &str) -> Result<String, String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("package ") && trimmed.ends_with(';') {
            let package = trimmed
                .strip_prefix("package ")
                .and_then(|s| s.strip_suffix(';'))
                .map(|s| s.trim().to_string())
                .ok_or("Failed to parse package name")?;
            return Ok(package);
        }
    }
    Err("No package declaration found".to_string())
}

fn extract_messages(content: &str, analysis: &mut ProtoAnalysis) -> Result<(), String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("message ") {
            if let Some(msg_name) = extract_message_name(line) {
                let fields = extract_message_fields(&lines, &mut i)?;
                analysis.messages.insert(
                    msg_name.clone(),
                    ProtoMessage {
                        name: msg_name,
                        fields,
                    },
                );
            }
        }
        i += 1;
    }

    Ok(())
}

fn extract_message_name(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 && parts[0] == "message" {
        Some(parts[1].trim_end_matches('{').trim().to_string())
    } else {
        None
    }
}

fn extract_message_fields(
    lines: &[&str],
    start_idx: &mut usize,
) -> Result<Vec<ProtoField>, String> {
    let mut fields = Vec::new();
    let mut brace_count = 0;
    let mut found_opening = false;

    let start_line = lines[*start_idx].trim();
    if start_line.contains('{') {
        brace_count += 1;
        found_opening = true;
    }

    *start_idx += 1;

    while *start_idx < lines.len() {
        let line = lines[*start_idx].trim();

        if line.contains('{') {
            brace_count += 1;
            if !found_opening {
                found_opening = true;
            }
        }

        if line.contains('}') {
            brace_count -= 1;
            if brace_count == 0 {
                break;
            }
        }

        if found_opening
            && brace_count == 1
            && !line.starts_with("oneof")
            && !line.starts_with("//")
            && !line.is_empty()
        {
            if let Some(field) = parse_field_line(line) {
                fields.push(field);
            }
        }

        *start_idx += 1;
    }

    Ok(fields)
}

fn parse_field_line(line: &str) -> Option<ProtoField> {
    let line = line.trim();

    if line.starts_with("//") || line.is_empty() || line.starts_with("oneof") || line == "}" {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let mut field_type = parts[0].to_string();
    let mut field_name = parts[1].to_string();
    let mut number_part = parts[3];

    if parts[0] == "repeated" || parts[0] == "optional" {
        if parts.len() < 5 {
            return None;
        }
        field_type = format!("{} {}", parts[0], parts[1]);
        field_name = parts[2].to_string();
        number_part = parts[4];
    }

    let field_number = number_part.trim_end_matches(';').parse::<u32>().ok()?;

    Some(ProtoField {
        name: field_name,
        proto_type: field_type,
        field_number,
    })
}

fn extract_oneofs(content: &str, analysis: &mut ProtoAnalysis) -> Result<(), String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("message ") {
            if let Some(msg_name) = extract_message_name(line) {
                extract_oneofs_from_message(&lines, &mut i, &msg_name, analysis)?;
            }
        }
        i += 1;
    }

    Ok(())
}

fn extract_oneofs_from_message(
    lines: &[&str],
    start_idx: &mut usize,
    message_name: &str,
    analysis: &mut ProtoAnalysis,
) -> Result<(), String> {
    let mut brace_count = 0;
    let mut found_opening = false;

    let start_line = lines[*start_idx].trim();
    if start_line.contains('{') {
        brace_count += 1;
        found_opening = true;
    }

    *start_idx += 1;

    while *start_idx < lines.len() {
        let mut line = lines[*start_idx].trim();

        // Check for oneof BEFORE updating brace counts
        if found_opening && brace_count == 1 && line.starts_with("oneof ") {
            let oneof_field = extract_oneof_field_name(line);
            if let Some(field_name) = oneof_field {
                let variants = extract_oneof_variants(lines, start_idx)?;

                analysis.oneof_types.push(OneofType {
                    message_name: message_name.to_string(),
                    oneof_field: field_name,
                    variants,
                });

                // Re-read the current line after extract_oneof_variants
                line = lines[*start_idx].trim();
            }
        }

        if line.contains('{') {
            brace_count += 1;
        }

        if line.contains('}') {
            brace_count -= 1;
            if brace_count == 0 {
                break;
            }
        }

        *start_idx += 1;
    }

    Ok(())
}

fn extract_oneof_field_name(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 && parts[0] == "oneof" {
        Some(parts[1].trim_end_matches('{').trim().to_string())
    } else {
        None
    }
}

fn extract_oneof_variants(
    lines: &[&str],
    start_idx: &mut usize,
) -> Result<Vec<OneofVariant>, String> {
    let mut variants = Vec::new();
    let mut brace_count = 1;

    *start_idx += 1;

    while *start_idx < lines.len() {
        let line = lines[*start_idx].trim();

        if line.contains('{') {
            brace_count += 1;
        }

        if line.contains('}') {
            brace_count -= 1;
            if brace_count == 0 {
                break;
            }
        }

        if brace_count == 1 && !line.is_empty() && !line.starts_with("//") {
            if let Some(variant) = parse_oneof_variant(line) {
                variants.push(variant);
            }
        }

        *start_idx += 1;
    }

    Ok(variants)
}

fn parse_oneof_variant(line: &str) -> Option<OneofVariant> {
    let line = line.trim();

    if line.is_empty() || line.starts_with("//") {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let type_name = parts[0].to_string();
    let field_name = parts[1].to_string();
    let number = parts[3].trim_end_matches(';').parse::<u32>().ok()?;

    Some(OneofVariant {
        type_name,
        field_name,
        number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_package() {
        let content = "package vixen.parser.settlement;";
        let result = extract_package(content).unwrap();
        assert_eq!(result, "vixen.parser.settlement");
    }

    #[test]
    fn test_parse_simple_message() {
        let content = r#"
message GameRegistry {
    uint64 game_id = 1;
    string game_program = 2;
    int64 created_at = 3;
}
"#;
        let mut analysis = ProtoAnalysis {
            package: String::new(),
            messages: HashMap::new(),
            oneof_types: Vec::new(),
        };
        extract_messages(content, &mut analysis).unwrap();

        assert!(analysis.messages.contains_key("GameRegistry"));
        let msg = &analysis.messages["GameRegistry"];
        assert_eq!(msg.fields.len(), 3);
        assert_eq!(msg.fields[0].name, "game_id");
        assert_eq!(msg.fields[0].proto_type, "uint64");
    }

    #[test]
    fn test_parse_oneof() {
        let content = r#"
package test;

message ProgramIxs {
    oneof ix_oneof {
        PlaceBetIx place_bet = 1;
        CreateGameIx create_game = 2;
    }
}
"#;
        let analysis = parse_proto_content(content).unwrap();

        assert_eq!(analysis.oneof_types.len(), 1);
        assert_eq!(analysis.oneof_types[0].message_name, "ProgramIxs");
        assert_eq!(analysis.oneof_types[0].oneof_field, "ix_oneof");
        assert_eq!(analysis.oneof_types[0].variants.len(), 2);
        assert_eq!(analysis.oneof_types[0].variants[0].type_name, "PlaceBetIx");
        assert_eq!(analysis.oneof_types[0].variants[0].field_name, "place_bet");
        assert_eq!(
            analysis.oneof_types[0].variants[1].type_name,
            "CreateGameIx"
        );
        assert_eq!(
            analysis.oneof_types[0].variants[1].field_name,
            "create_game"
        );
    }
}
