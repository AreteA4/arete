use arete_idl::error::IdlSearchError;
use arete_idl::search::suggest_similar;
use proc_macro2::{Span, TokenStream};
use syn::Item;

// Error message style guide:
// - start with a lowercase problem statement (`invalid`, `unknown`, `missing`)
// - name the DSL concept directly (`strategy`, `resolver`, `view field`)
// - include what was provided and what was expected
// - add a single best suggestion when possible, otherwise preview available values

pub fn combine_errors(errors: Vec<syn::Error>) -> syn::Error {
    let mut iter = errors.into_iter();
    let mut combined = iter.next().unwrap_or_else(|| {
        internal_codegen_error(Span::call_site(), "attempted to combine zero errors")
    });

    for error in iter {
        combined.combine(error);
    }

    combined
}

#[derive(Default)]
pub struct ErrorCollector {
    errors: Vec<syn::Error>,
}

impl ErrorCollector {
    pub fn push(&mut self, error: syn::Error) {
        self.errors.push(error);
    }

    pub fn finish(self) -> syn::Result<()> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(combine_errors(self.errors))
        }
    }
}

pub fn internal_codegen_error(span: Span, msg: impl Into<String>) -> syn::Error {
    syn::Error::new(
        span,
        format!("internal code generation error: {}", msg.into()),
    )
}

pub fn preview_values(values: &[String], limit: usize) -> String {
    values
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn suggestion_or_available_suffix(
    input: &str,
    available: &[String],
    available_label: &str,
) -> String {
    let candidate_refs: Vec<&str> = available.iter().map(String::as_str).collect();
    let suggestions = suggest_similar(input, &candidate_refs, 3);

    if let Some(suggestion) = suggestions.first() {
        format!(". Did you mean: {}?", suggestion.candidate)
    } else if !available.is_empty() {
        format!(". {}: {}", available_label, preview_values(available, 6))
    } else {
        String::new()
    }
}

pub fn invalid_choice_message(
    choice_kind: &str,
    actual: &str,
    context: &str,
    expected: &[&str],
) -> String {
    let available = expected
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let candidate_refs: Vec<&str> = available.iter().map(String::as_str).collect();
    let suggestion = suggest_similar(actual, &candidate_refs, 3)
        .first()
        .map(|suggestion| format!(". Did you mean: {}?", suggestion.candidate))
        .unwrap_or_default();

    format!(
        "invalid {} '{}' for {}. Expected one of: {}{}",
        choice_kind,
        actual,
        context,
        expected.join(", "),
        suggestion
    )
}

pub fn unknown_value_message(
    value_kind: &str,
    actual: &str,
    available_label: &str,
    available: &[String],
) -> String {
    format!(
        "unknown {} '{}'{}",
        value_kind,
        actual,
        suggestion_or_available_suffix(actual, available, available_label)
    )
}

pub fn idl_error_to_syn(span: Span, error: IdlSearchError) -> syn::Error {
    syn::Error::new(span, error.to_string())
}

pub fn parse_generated_items(
    tokens: TokenStream,
    span: Span,
    context: &str,
) -> syn::Result<Vec<Item>> {
    syn::parse2::<syn::File>(tokens)
        .map(|file| file.items)
        .map_err(|error| {
            internal_codegen_error(span, format!("{context} generated invalid Rust: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_choice_message_suggests_nearest_value() {
        let message =
            invalid_choice_message("strategy", "LastWrit", "#[map]", &["SetOnce", "LastWrite"]);

        assert!(message.contains("invalid strategy 'LastWrit' for #[map]"));
        assert!(message.contains("Expected one of: SetOnce, LastWrite"));
        assert!(message.contains("Did you mean: LastWrite?"));
    }

    #[test]
    fn unknown_value_message_falls_back_to_available_values() {
        let message = unknown_value_message(
            "resolver-backed type",
            "u64",
            "Available types",
            &["TokenMetadata".to_string()],
        );

        assert_eq!(
            message,
            "unknown resolver-backed type 'u64'. Available types: TokenMetadata"
        );
    }

    #[test]
    fn invalid_choice_message_does_not_repeat_available_values() {
        let message =
            invalid_choice_message("strategy", "foo", "#[map]", &["SetOnce", "LastWrite"]);

        assert_eq!(
            message,
            "invalid strategy 'foo' for #[map]. Expected one of: SetOnce, LastWrite"
        );
    }
}
