//! Error types for IDL search and lookup operations

use crate::search::Suggestion;

/// Structured error type for IDL lookup operations.
///
/// Named `IdlSearchError` to avoid conflict with `types::IdlError`
/// which represents error definitions within an IDL spec.
#[derive(Debug, Clone)]
pub enum IdlSearchError {
    NotFound {
        input: String,
        section: String,
        suggestions: Vec<Suggestion>,
        available: Vec<String>,
    },
    ParseError {
        path: String,
        source: String,
    },
    InvalidPath {
        path: String,
    },
}

impl std::fmt::Display for IdlSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdlSearchError::NotFound {
                input,
                section,
                suggestions,
                available,
            } => {
                write!(f, "Not found: '{}' in {}", input, section)?;
                if !suggestions.is_empty() {
                    write!(f, ". Did you mean: {}?", suggestions[0].candidate)?;
                } else if !available.is_empty() {
                    let preview = available
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, ". {}: {}", available_label(section), preview)?;
                }
                Ok(())
            }
            IdlSearchError::ParseError { path, source } => {
                write!(f, "Failed to parse '{}': {}", path, source)
            }
            IdlSearchError::InvalidPath { path } => {
                write!(
                    f,
                    "Invalid path '{}'. Expected a Rust path like foo::bar::Baz",
                    path
                )
            }
        }
    }
}

impl std::error::Error for IdlSearchError {}

fn available_label(section: &str) -> String {
    if section.starts_with("instruction fields") {
        "Available instruction fields".to_string()
    } else if section.starts_with("account fields") {
        "Available account fields".to_string()
    } else {
        format!("Available {}", section)
    }
}
