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

    let state_enum_variants = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);
        quote! { #variant_name(accounts::#variant_name) }
    });

    let unpack_arms = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);

        quote! {
            d if d == accounts::#variant_name::DISCRIMINATOR => {
                Ok(#state_enum_name::#variant_name(
                    accounts::#variant_name::try_from_bytes(data)?
                ))
            }
        }
    });

    let convert_to_json_arms = idl.accounts.iter().map(|acc| {
        let variant_name = format_ident!("{}", acc.name);
        let type_name = format!("{}::{}State", program_name, acc.name);

        quote! {
            #state_enum_name::#variant_name(data) => {
                hyperstack::runtime::serde_json::json!({
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

        static PROGRAM_ID: std::sync::OnceLock<hyperstack::runtime::yellowstone_vixen_core::Pubkey> = std::sync::OnceLock::new();

        pub fn program_id() -> hyperstack::runtime::yellowstone_vixen_core::Pubkey {
            *PROGRAM_ID.get_or_init(|| {
                let decoded = hyperstack::runtime::bs58::decode(PROGRAM_ID_STR)
                    .into_vec()
                    .expect("Invalid program ID");
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&decoded);
                hyperstack::runtime::yellowstone_vixen_core::Pubkey::new(bytes)
            })
        }

        #[derive(Debug)]
        pub enum #state_enum_name {
            #(#state_enum_variants),*
        }

        impl #state_enum_name {
            pub fn try_unpack(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                if data.len() < 8 {
                    return Err("Data too short for discriminator".into());
                }

                let discriminator = &data[0..8];
                match discriminator {
                    #(#unpack_arms),*
                    _ => Err(format!("Unknown discriminator: {:?}", discriminator).into())
                }
            }

            pub fn to_json(&self) -> hyperstack::runtime::serde_json::Value {
                match self {
                    #(#convert_to_json_arms),*
                }
            }

            pub fn event_type(&self) -> &'static str {
                match self {
                    #(#type_name_arms),*
                }
            }

            pub fn to_value(&self) -> hyperstack::runtime::serde_json::Value {
                match self {
                    #(#to_value_arms),*
                }
            }
        }

        #[derive(Debug, Copy, Clone)]
        pub struct AccountParser;

        impl hyperstack::runtime::yellowstone_vixen_core::Parser for AccountParser {
            type Input = hyperstack::runtime::yellowstone_vixen_core::AccountUpdate;
            type Output = #state_enum_name;

            fn id(&self) -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(concat!(#program_name, "::AccountParser"))
            }

            fn prefilter(&self) -> hyperstack::runtime::yellowstone_vixen_core::Prefilter {
                hyperstack::runtime::yellowstone_vixen_core::Prefilter::builder()
                    .account_owners([program_id()])
                    .build()
                    .unwrap()
            }

            async fn parse(
                &self,
                acct: &hyperstack::runtime::yellowstone_vixen_core::AccountUpdate,
            ) -> hyperstack::runtime::yellowstone_vixen_core::ParseResult<Self::Output> {
                let inner = acct
                    .account
                    .as_ref()
                    .ok_or(hyperstack::runtime::yellowstone_vixen_core::ParseError::from("No account data"))?;

                #state_enum_name::try_unpack(&inner.data)
                    .map_err(|e| {
                        let msg = e.to_string();
                        if msg.contains("Unknown discriminator") || msg.contains("too short") {
                            hyperstack::runtime::yellowstone_vixen_core::ParseError::Filtered
                        } else {
                            hyperstack::runtime::yellowstone_vixen_core::ParseError::from(msg)
                        }
                    })
            }
        }

        impl hyperstack::runtime::yellowstone_vixen_core::ProgramParser for AccountParser {
            #[inline]
            fn program_id(&self) -> hyperstack::runtime::yellowstone_vixen_core::Pubkey {
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

    let uses_steel_discriminant = idl
        .instructions
        .iter()
        .any(|ix| ix.discriminant.is_some() && ix.discriminator.is_empty());

    let discriminator_size: usize = if uses_steel_discriminant { 1 } else { 8 };

    let unpack_arms = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let discriminator = ix.get_discriminator();
        let disc_size = discriminator_size;

        let has_args = !ix.args.is_empty();

        if uses_steel_discriminant {
            let discriminant_value = discriminator.first().copied().unwrap_or(0u8);
            if has_args {
                quote! {
                    [#discriminant_value, ..] => {
                        let data = instructions::#variant_name::try_from_bytes(&data[#disc_size..])?;
                        Ok(#ix_enum_name::#variant_name(data))
                    }
                }
            } else {
                quote! {
                    [#discriminant_value, ..] => {
                        Ok(#ix_enum_name::#variant_name(instructions::#variant_name {}))
                    }
                }
            }
        } else {
            let disc_bytes: Vec<u8> = discriminator.iter().take(8).copied().collect();
            let d0 = disc_bytes.first().copied().unwrap_or(0);
            let d1 = disc_bytes.get(1).copied().unwrap_or(0);
            let d2 = disc_bytes.get(2).copied().unwrap_or(0);
            let d3 = disc_bytes.get(3).copied().unwrap_or(0);
            let d4 = disc_bytes.get(4).copied().unwrap_or(0);
            let d5 = disc_bytes.get(5).copied().unwrap_or(0);
            let d6 = disc_bytes.get(6).copied().unwrap_or(0);
            let d7 = disc_bytes.get(7).copied().unwrap_or(0);

            if has_args {
                quote! {
                    [#d0, #d1, #d2, #d3, #d4, #d5, #d6, #d7, ..] => {
                        let data = instructions::#variant_name::try_from_bytes(&data[#disc_size..])?;
                        Ok(#ix_enum_name::#variant_name(data))
                    }
                }
            } else {
                quote! {
                    [#d0, #d1, #d2, #d3, #d4, #d5, #d6, #d7, ..] => {
                        Ok(#ix_enum_name::#variant_name(instructions::#variant_name {}))
                    }
                }
            }
        }
    });

    // Anchor CPI event wire format:
    //   bytes  0- 7: SHA256("anchor:event")[..8] = [29, 154, 203, 81, 46, 165, 69, 228]
    //   bytes  8-15: SHA256("event:<Name>")[..8]  = the event-specific discriminator
    //   bytes 16+  : Borsh-encoded event payload
    //
    // So we match a 16-byte prefix (anchor tag + event disc) and pass &data[8..] to
    // try_from_bytes(), which skips its own 8-byte discriminator and reads from byte 16.
    // SHA256("anchor:event")[..8] — the fixed tag prepended to all Anchor CPI event instructions
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
        let disc_bytes: Vec<u8> = discriminator.iter().take(8).copied().collect();
        let d0 = disc_bytes.first().copied().unwrap_or(0);
        let d1 = disc_bytes.get(1).copied().unwrap_or(0);
        let d2 = disc_bytes.get(2).copied().unwrap_or(0);
        let d3 = disc_bytes.get(3).copied().unwrap_or(0);
        let d4 = disc_bytes.get(4).copied().unwrap_or(0);
        let d5 = disc_bytes.get(5).copied().unwrap_or(0);
        let d6 = disc_bytes.get(6).copied().unwrap_or(0);
        let d7 = disc_bytes.get(7).copied().unwrap_or(0);

        // Match the full 16-byte prefix: anchor:event tag + event-specific discriminator.
        // Pass &data[8..] to try_from_bytes so it can skip the 8-byte event discriminator
        // and deserialize the payload starting at byte 16.
        quote! {
            [#at0, #at1, #at2, #at3, #at4, #at5, #at6, #at7,
             #d0, #d1, #d2, #d3, #d4, #d5, #d6, #d7, ..] => {
                let event_data = events::#event_type::try_from_bytes(&data[8..])?;
                Ok(#ix_enum_name::#variant_name(event_data))
            }
        }
    });

    let convert_to_json_arms_ix = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let type_name = format!("{}::{}", program_name, to_pascal_case(&ix.name));

        quote! {
            #ix_enum_name::#variant_name(data) => {
                hyperstack::runtime::serde_json::json!({
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
                hyperstack::runtime::serde_json::json!({
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
                hyperstack::runtime::serde_json::json!({
                    "data": data.to_json_value()
                })
            }
        }
    });

    let to_value_arms_ev = idl.events.iter().map(|ev| {
        let variant_name = format_ident!("Event_{}", ev.name);

        quote! {
            #ix_enum_name::#variant_name(data) => {
                hyperstack::runtime::serde_json::json!({
                    "data": data.to_json_value()
                })
            }
        }
    });

    let to_value_with_accounts_arms = idl.instructions.iter().map(|ix| {
        let variant_name = format_ident!("{}", to_pascal_case(&ix.name));
        let ix_name = &ix.name;
        let account_names: Vec<_> = ix.accounts.iter().map(|acc| &acc.name).collect();
        let expected_count = account_names.len();

        quote! {
            #ix_enum_name::#variant_name(data) => {
                let mut value = hyperstack::runtime::serde_json::json!({
                    "data": data.to_json_value()
                });

                if let Some(obj) = value.as_object_mut() {
                    let account_names: Vec<&str> = vec![#(#account_names),*];
                    let expected_count = #expected_count;
                    let actual_count = accounts.len();

                    // Warn if account count doesn't match IDL expectation
                    if actual_count != expected_count {
                        hyperstack::runtime::tracing::warn!(
                            instruction = #ix_name,
                            expected = expected_count,
                            actual = actual_count,
                            "Account count mismatch - IDL may be out of sync with program. Update your IDL to match the current program version."
                        );
                    }

                    let mut accounts_obj = hyperstack::runtime::serde_json::Map::new();
                    for (i, name) in account_names.iter().enumerate() {
                        if i < accounts.len() {
                            accounts_obj.insert(
                                name.to_string(),
                                hyperstack::runtime::serde_json::Value::String(hyperstack::runtime::bs58::encode(&accounts[i].0).into_string())
                            );
                        }
                    }
                    obj.insert("accounts".to_string(), hyperstack::runtime::serde_json::Value::Object(accounts_obj));
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
                hyperstack::runtime::serde_json::json!({
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
            pub fn try_unpack(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
                if data.is_empty() {
                    return Err("Empty instruction data".into());
                }

                match data {
                    #(#unpack_arms,)*
                    #(#event_unpack_arms,)*
                    _ => {
                        let disc_preview: Vec<u8> = data.iter().take(8).copied().collect();
                        Err(format!("Unknown instruction discriminator: {:?}", disc_preview).into())
                    }
                }
            }

            pub fn to_json(&self) -> hyperstack::runtime::serde_json::Value {
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

            pub fn to_value(&self) -> hyperstack::runtime::serde_json::Value {
                match self {
                    #(#to_value_arms_ix,)*
                    #(#to_value_arms_ev),*
                }
            }

            pub fn to_value_with_accounts(&self, accounts: &[hyperstack::runtime::yellowstone_vixen_core::KeyBytes<32>]) -> hyperstack::runtime::serde_json::Value {
                match self {
                    #(#to_value_with_accounts_arms,)*
                    #(#to_value_with_accounts_arms_ev),*
                }
            }
        }

        #[derive(Debug, Copy, Clone)]
        pub struct InstructionParser;

        impl hyperstack::runtime::yellowstone_vixen_core::Parser for InstructionParser {
            type Input = hyperstack::runtime::yellowstone_vixen_core::instruction::InstructionUpdate;
            type Output = #ix_enum_name;

            fn id(&self) -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(concat!(#program_name, "::InstructionParser"))
            }

            fn prefilter(&self) -> hyperstack::runtime::yellowstone_vixen_core::Prefilter {
                hyperstack::runtime::yellowstone_vixen_core::Prefilter::builder()
                    .transaction_accounts_include([program_id()])
                    .build()
                    .unwrap()
            }

            async fn parse(
                &self,
                ix_update: &hyperstack::runtime::yellowstone_vixen_core::instruction::InstructionUpdate,
            ) -> hyperstack::runtime::yellowstone_vixen_core::ParseResult<Self::Output> {
                if ix_update.program.equals_ref(program_id()) {
                    let parsed = #ix_enum_name::try_unpack(&ix_update.data)
                        .map_err(|e| {
                            let err_str = e.to_string();
                            if err_str.contains("Unknown instruction discriminator") {
                                hyperstack::runtime::yellowstone_vixen_core::ParseError::Filtered
                            } else {
                                let disc_preview: Vec<u8> = ix_update.data.iter().take(8).copied().collect();
                                hyperstack::runtime::tracing::warn!(
                                    discriminator = ?disc_preview,
                                    data_len = ix_update.data.len(),
                                    error = %err_str,
                                    "Failed to parse instruction"
                                );
                                hyperstack::runtime::yellowstone_vixen_core::ParseError::from(err_str)
                            }
                        })?;

                    Ok(parsed)
                } else {
                    Err(hyperstack::runtime::yellowstone_vixen_core::ParseError::Filtered)
                }
            }
        }

        impl hyperstack::runtime::yellowstone_vixen_core::ProgramParser for InstructionParser {
            #[inline]
            fn program_id(&self) -> hyperstack::runtime::yellowstone_vixen_core::Pubkey {
                program_id()
            }
        }
    }
}
