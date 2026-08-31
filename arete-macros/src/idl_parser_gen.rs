//! IDL parser generation.

#![allow(dead_code)]

use crate::parse::idl::*;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub fn generate_parsers(idl: &IdlSpec, program_id: &str, sdk_module_name: &str) -> TokenStream {
    generate_named_parsers(idl, program_id, sdk_module_name, "parsers")
}

pub fn generate_named_parsers(
    idl: &IdlSpec,
    program_id: &str,
    sdk_module_name: &str,
    parser_module_name: &str,
) -> TokenStream {
    let account_parser = generate_account_parser(idl, program_id);
    let instruction_parser = generate_instruction_parser(idl, program_id);
    let sdk_module_ident = format_ident!("{}", sdk_module_name);
    let parser_module_ident = format_ident!("{}", parser_module_name);

    quote! {
        pub mod #parser_module_ident {
            use super::#sdk_module_ident::*;

            #account_parser
            #instruction_parser
        }
    }
}

fn generate_account_parser(idl: &IdlSpec, program_id: &str) -> TokenStream {
    let program_name = idl.get_name();
    let state_enum_name = format_ident!("{}State", to_pascal_case(program_name));

    let mut sorted_accounts: Vec<&IdlAccount> = idl.accounts.iter().collect();
    sorted_accounts.sort_by(|a, b| {
        b.get_discriminator()
            .len()
            .cmp(&a.get_discriminator().len())
    });

    let state_enum_variants = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);
        quote! { #variant_name(accounts::#variant_name) }
    });

    let unpack_checks = sorted_accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);

        quote! {
            if data.starts_with(accounts::#variant_name::DISCRIMINATOR) {
                return Ok(#state_enum_name::#variant_name(
                    accounts::#variant_name::try_from_bytes(data)?
                ));
            }
        }
    });

    let convert_to_json_arms = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);
        let type_name = format!("{}::{}State", program_name, acc.name);

        quote! {
            #state_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "type": #type_name,
                    "data": data.to_json_value()
                })
            }
        }
    });

    let type_name_arms = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);
        let type_name = format!("{}::{}State", program_name, acc.name);

        quote! {
            #state_enum_name::#variant_name(_) => #type_name
        }
    });

    let to_value_arms = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);

        quote! {
            #state_enum_name::#variant_name(data) => {
                data.to_json_value()
            }
        }
    });

    quote! {
        pub const PROGRAM_ID_STR: &str = #program_id;

        static PROGRAM_ID: std::sync::OnceLock<arete::runtime::yellowstone_vixen_core::Pubkey> = std::sync::OnceLock::new();

        pub fn program_id() -> arete::runtime::yellowstone_vixen_core::Pubkey {
            *PROGRAM_ID.get_or_init(|| {
                let decoded = arete::runtime::bs58::decode(PROGRAM_ID_STR)
                    .into_vec()
                    .expect("Invalid program ID");
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&decoded);
                arete::runtime::yellowstone_vixen_core::Pubkey::new(bytes)
            })
        }

        #[derive(Debug)]
        pub enum #state_enum_name {
            #(#state_enum_variants),*
        }

        impl #state_enum_name {
            pub fn try_unpack(data: &[u8]) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                if data.is_empty() {
                    return Err("Data too short for discriminator".into());
                }

                #(#unpack_checks)*

                let disc_preview: Vec<u8> = data.iter().take(8).copied().collect();
                Err(format!("Unknown discriminator: {:?}", disc_preview).into())
            }

            pub fn to_json(&self) -> arete::runtime::serde_json::Value {
                match self {
                    #(#convert_to_json_arms),*
                }
            }

            pub fn event_type(&self) -> &'static str {
                match self {
                    #(#type_name_arms),*
                }
            }

            pub fn to_value(&self) -> arete::runtime::serde_json::Value {
                match self {
                    #(#to_value_arms),*
                }
            }
        }

        #[derive(Debug, Copy, Clone)]
        pub struct AccountParser;

        impl arete::runtime::yellowstone_vixen_core::Parser for AccountParser {
            type Input = arete::runtime::yellowstone_vixen_core::AccountUpdate;
            type Output = #state_enum_name;

            fn id(&self) -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(concat!(#program_name, "::AccountParser"))
            }

            fn prefilter(&self) -> arete::runtime::yellowstone_vixen_core::Prefilter {
                arete::runtime::yellowstone_vixen_core::Prefilter::builder()
                    .account_owners([program_id()])
                    .build()
                    .unwrap()
            }

            async fn parse(
                &self,
                acct: &arete::runtime::yellowstone_vixen_core::AccountUpdate,
            ) -> arete::runtime::yellowstone_vixen_core::ParseResult<Self::Output> {
                let inner = acct
                    .account
                    .as_ref()
                    .ok_or(arete::runtime::yellowstone_vixen_core::ParseError::from("No account data"))?;

                #state_enum_name::try_unpack(&inner.data)
                    .map_err(|e| {
                        let msg = e.to_string();
                        if msg.contains("Unknown discriminator") || msg.contains("too short") {
                            arete::runtime::yellowstone_vixen_core::ParseError::Filtered
                        } else {
                            arete::runtime::yellowstone_vixen_core::ParseError::from(msg)
                        }
                    })
            }
        }

        impl arete::runtime::yellowstone_vixen_core::ProgramParser for AccountParser {
            #[inline]
            fn program_id(&self) -> arete::runtime::yellowstone_vixen_core::Pubkey {
                program_id()
            }
        }
    }
}

fn generate_instruction_parser(idl: &IdlSpec, _program_id: &str) -> TokenStream {
    let program_name = idl.get_name();
    let ix_enum_name = format_ident!("{}Instruction", to_pascal_case(program_name));

    // Instruction variants
    let ix_enum_variants = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        quote! { #variant_name(instructions::#variant_name) }
    });

    // Event variants: prefixed with "Event_" to avoid name collisions with instruction variants
    let event_enum_variants = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);
        let event_type = format_ident!("{}", ev.name);
        quote! { #variant_name(events::#event_type) }
    });

    let discriminator_size = idl.instruction_discriminator_size();

    let unpack_arms = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let disc_size = discriminator_size;

        // Match the whole tag. A pattern narrower than the discriminator would also match a
        // different instruction that happens to share the leading byte.
        let disc_bytes: Vec<u8> = ix
            .get_discriminator()
            .into_iter()
            .chain(std::iter::repeat(0))
            .take(discriminator_size)
            .collect();

        let body = if ix.args.is_empty() {
            quote! { Ok(#ix_enum_name::#variant_name(instructions::#variant_name {})) }
        } else {
            quote! {
                let data = instructions::#variant_name::try_from_bytes(&data[#disc_size..])?;
                Ok(#ix_enum_name::#variant_name(data))
            }
        };

        quote! {
            [#(#disc_bytes),*, ..] => { #body }
        }
    });

    // Anchor-compatible CPI event wire format:
    //   bytes 0-7:  SHA256("anchor:event")[..8] as little-endian u64
    //   bytes 8..: event-specific discriminator followed by event payload
    //
    // Standard Anchor events use an 8-byte event discriminator, but some
    // Codama/Pinocchio programs use shorter discriminators while retaining the
    // same Anchor event tag. Match the discriminator length declared in the IDL.
    // We pass &data[8..] to try_from_bytes(), which validates and strips the
    // event-specific discriminator before decoding the payload.
    let anchor_event_tag: [u8; 8] = [228u8, 69, 165, 46, 81, 203, 154, 29];
    let at0 = anchor_event_tag[0];
    let at1 = anchor_event_tag[1];
    let at2 = anchor_event_tag[2];
    let at3 = anchor_event_tag[3];
    let at4 = anchor_event_tag[4];
    let at5 = anchor_event_tag[5];
    let at6 = anchor_event_tag[6];
    let at7 = anchor_event_tag[7];

    let event_unpack_arms = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);
        let event_type = format_ident!("{}", ev.name);
        let discriminator = ev.get_discriminator();

        quote! {
            data if data.starts_with(&[#at0, #at1, #at2, #at3, #at4, #at5, #at6, #at7])
                && data[8..].starts_with(&[#(#discriminator),*]) => {
                let event_data = events::#event_type::try_from_bytes(&data[8..])?;
                Ok(#ix_enum_name::#variant_name(event_data))
            }
        }
    });

    let event_log_unpack_arms = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);
        let event_type = format_ident!("{}", ev.name);
        let discriminator = ev.get_discriminator();

        quote! {
            data if data.starts_with(&[#(#discriminator),*]) => {
                let event_data = events::#event_type::try_from_bytes(data)?;
                Ok(#ix_enum_name::#variant_name(event_data))
            }
        }
    });

    let convert_to_json_arms_ix = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let type_name = format!("{}::{}", program_name, to_pascal_case(&ix.name));

        quote! {
            #ix_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "type": #type_name,
                    "data": data.to_json_value()
                })
            }
        }
    });

    let convert_to_json_arms_ev = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);
        // Use CpiEvent suffix to distinguish from instructions
        let type_name = format!("{}::{}CpiEvent", program_name, ev.name);

        quote! {
            #ix_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "type": #type_name,
                    "data": data.to_json_value()
                })
            }
        }
    });

    let type_name_arms_ix = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let type_name = format!("{}::{}IxState", program_name, to_pascal_case(&ix.name));

        quote! {
            #ix_enum_name::#variant_name(_) => #type_name
        }
    });

    let type_name_arms_ev = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);
        // Use CpiEvent suffix to distinguish from instructions (IxState suffix).
        // The AST writer must generate the same suffix when it sees a type from
        // the `events` submodule (e.g., generated_sdk::events::Swap).
        let type_name = format!("{}::{}CpiEvent", program_name, ev.name);

        quote! {
            #ix_enum_name::#variant_name(_) => #type_name
        }
    });

    let to_value_arms_ix = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));

        quote! {
            #ix_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "data": data.to_json_value()
                })
            }
        }
    });

    let to_value_arms_ev = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);

        quote! {
            #ix_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "data": data.to_json_value()
                })
            }
        }
    });

    let to_value_with_accounts_arms = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let ix_name = &ix.name;
        let account_names: Vec<_> = ix.accounts.iter().map(|acc| &acc.name).collect();
        let declared_count = account_names.len();
        // Declaration order matters, not just the count: which accounts were omitted decides
        // whether the positional mapping below still lines up.
        let optional_flags: Vec<_> = ix.accounts.iter().map(|acc| acc.optional).collect();

        quote! {
            #ix_enum_name::#variant_name(data) => {
                let mut value = arete::runtime::serde_json::json!({
                    "data": data.to_json_value()
                });

                if let Some(obj) = value.as_object_mut() {
                    let account_names: Vec<&str> = vec![#(#account_names),*];
                    let optional_flags: Vec<bool> = vec![#(#optional_flags),*];
                    let declared_count = #declared_count;
                    let actual_count = accounts.len();

                    // Fewer accounts than declared is benign only when every omitted declaration
                    // is optional — that is, when they form an optional suffix. Counting required
                    // accounts instead would accept a non-trailing optional being omitted, which
                    // shifts every later key onto the wrong name: with
                    // `[authority, payer?, system_program]` and two accounts supplied, the system
                    // program gets labelled `payer` and nothing says so.
                    //
                    // More accounts than declared stays silent: Anchor's `remaining_accounts` are
                    // never declared in an IDL, so programs using them exceed the count on every
                    // call, and warning would tell the reader to "update" a correct IDL.
                    let omitted_are_all_optional = optional_flags
                        .iter()
                        .skip(actual_count)
                        .all(|optional| *optional);

                    if actual_count < declared_count && !omitted_are_all_optional {
                        arete::runtime::tracing::warn!(
                            instruction = #ix_name,
                            declared = declared_count,
                            actual = actual_count,
                            "Accounts missing where the IDL declares a required one — decoded account names after the gap may be wrong"
                        );
                    }

                    let mut accounts_obj = arete::runtime::serde_json::Map::new();
                    for (i, name) in account_names.iter().enumerate() {
                        if i < accounts.len() {
                            accounts_obj.insert(
                                name.to_string(),
                                arete::runtime::serde_json::Value::String(arete::runtime::bs58::encode(&accounts[i].0).into_string())
                            );
                        }
                    }
                    obj.insert("accounts".to_string(), arete::runtime::serde_json::Value::Object(accounts_obj));
                }

                value
            }
        }
    });

    // CPI events have no accounts — expose event fields directly under "data"
    let to_value_with_accounts_arms_ev = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);

        quote! {
            #ix_enum_name::#variant_name(data) => {
                arete::runtime::serde_json::json!({
                    "data": data.to_json_value()
                })
            }
        }
    });

    quote! {
        #[derive(Debug)]
        pub enum #ix_enum_name {
            #(#ix_enum_variants,)*
            #(#event_enum_variants),*
        }

        impl #ix_enum_name {
            pub fn try_unpack(data: &[u8]) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                if data.is_empty() {
                    return Err("Empty instruction data".into());
                }

                match data {
                    #(#event_unpack_arms,)*
                    #(#unpack_arms,)*
                    _ => {
                        let disc_preview: Vec<u8> = data.iter().take(8).copied().collect();
                        Err(format!("Unknown instruction discriminator: {:?}", disc_preview).into())
                    }
                }
            }

            pub fn to_json(&self) -> arete::runtime::serde_json::Value {
                match self {
                    #(#convert_to_json_arms_ix,)*
                    #(#convert_to_json_arms_ev),*
                }
            }

            pub fn event_type(&self) -> &'static str {
                match self {
                    #(#type_name_arms_ix,)*
                    #(#type_name_arms_ev),*
                }
            }

            pub fn to_value(&self) -> arete::runtime::serde_json::Value {
                match self {
                    #(#to_value_arms_ix,)*
                    #(#to_value_arms_ev),*
                }
            }

            pub fn to_value_with_accounts(&self, accounts: &[arete::runtime::yellowstone_vixen_core::KeyBytes<32>]) -> arete::runtime::serde_json::Value {
                match self {
                    #(#to_value_with_accounts_arms,)*
                    #(#to_value_with_accounts_arms_ev),*
                }
            }

            pub fn try_unpack_log_event(data: &[u8]) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                if data.is_empty() {
                    return Err("Empty event data".into());
                }

                match data {
                    #(#event_log_unpack_arms,)*
                    _ => {
                        let disc_preview: Vec<u8> = data.iter().take(8).copied().collect();
                        Err(format!("Unknown event discriminator: {:?}", disc_preview).into())
                    }
                }
            }
        }

        #[derive(Debug, Copy, Clone)]
        pub struct InstructionParser;

        impl arete::runtime::yellowstone_vixen_core::Parser for InstructionParser {
            type Input = arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate;
            type Output = #ix_enum_name;

            fn id(&self) -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(concat!(#program_name, "::InstructionParser"))
            }

            fn prefilter(&self) -> arete::runtime::yellowstone_vixen_core::Prefilter {
                arete::runtime::yellowstone_vixen_core::Prefilter::builder()
                    .transaction_accounts_include([program_id()])
                    .build()
                    .unwrap()
            }

            async fn parse(
                &self,
                ix_update: &arete::runtime::yellowstone_vixen_core::instruction::InstructionUpdate,
            ) -> arete::runtime::yellowstone_vixen_core::ParseResult<Self::Output> {
                if ix_update.program.equals_ref(program_id()) {
                    let parsed = #ix_enum_name::try_unpack(&ix_update.data)
                        .map_err(|e| {
                            let err_str = e.to_string();
                            if err_str.contains("Unknown instruction discriminator") {
                                arete::runtime::yellowstone_vixen_core::ParseError::Filtered
                            } else {
                                let disc_preview: Vec<u8> = ix_update.data.iter().take(8).copied().collect();
                                arete::runtime::tracing::warn!(
                                    discriminator = ?disc_preview,
                                    data_len = ix_update.data.len(),
                                    error = %err_str,
                                    "Failed to parse instruction"
                                );
                                arete::runtime::yellowstone_vixen_core::ParseError::from(err_str)
                            }
                        })?;

                    Ok(parsed)
                } else {
                    Err(arete::runtime::yellowstone_vixen_core::ParseError::Filtered)
                }
            }
        }

        impl arete::runtime::yellowstone_vixen_core::ProgramParser for InstructionParser {
            #[inline]
            fn program_id(&self) -> arete::runtime::yellowstone_vixen_core::Pubkey {
                program_id()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_parser_uses_variable_length_discriminators() {
        let json = r#"{
            "name": "steelish",
            "instructions": [],
            "accounts": [
                {
                    "name": "Counter",
                    "discriminator": [7]
                }
            ],
            "types": [
                {
                    "name": "Counter",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "value", "type": "u64" }
                        ]
                    }
                }
            ],
            "events": [],
            "errors": []
        }"#;

        let idl = parse_idl_content(json).expect("test IDL should parse");
        let code = generate_named_parsers(
            &idl,
            "11111111111111111111111111111111",
            "generated_sdk",
            "parsers",
        )
        .to_string();

        assert!(
            code.contains("starts_with"),
            "account parser should use slice prefix matching, got: {}",
            code
        );
        assert!(
            !code.contains("let discriminator = & data [0 .. 8]"),
            "account parser should not hard-code 8-byte discriminator slices, got: {}",
            code
        );
    }
}
