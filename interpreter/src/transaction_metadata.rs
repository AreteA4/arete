//! Observed Solana transaction metadata, separate from program data and effective budgets.
//!
//! Missing metadata means the source did not retain the version. In particular,
//! a missing V1 config is insufficient evidence to classify legacy versus v0.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const SOLANA_TRANSACTION_METADATA_KEY: &str = "solana_transaction";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolanaTransactionVersion {
    Legacy,
    V0,
    V1,
}

impl Serialize for SolanaTransactionVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Legacy => serializer.serialize_str("legacy"),
            Self::V0 => serializer.serialize_u8(0),
            Self::V1 => serializer.serialize_u8(1),
        }
    }
}

impl<'de> Deserialize<'de> for SolanaTransactionVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Version {
            Name(String),
            Number(u8),
        }
        match Version::deserialize(deserializer)? {
            Version::Name(name) if name == "legacy" => Ok(Self::Legacy),
            Version::Number(0) => Ok(Self::V0),
            Version::Number(1) => Ok(Self::V1),
            _ => Err(serde::de::Error::custom("expected legacy, 0 or 1")),
        }
    }
}

/// The inline requests as observed on the wire, without inserting defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTransactionConfig {
    /// Serialized as a decimal string to preserve the full u64 range in JSON.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_u64_string"
    )]
    pub priority_fee_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_unit_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_accounts_data_size_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTransactionMetadata {
    pub version: SolanaTransactionVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<SolanaTransactionConfig>,
}

mod optional_u64_string {
    use super::*;

    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        value.map(|value| value.to_string()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(serde::de::Error::custom("expected a decimal u64 string"));
                }
                value.parse().map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UpdateContext;
    use serde_json::json;

    #[test]
    fn transaction_metadata_versions_and_unknown() {
        for (version, encoded) in [
            (SolanaTransactionVersion::Legacy, json!("legacy")),
            (SolanaTransactionVersion::V0, json!(0)),
            (SolanaTransactionVersion::V1, json!(1)),
        ] {
            let metadata = SolanaTransactionMetadata {
                version,
                config: None,
            };
            let context = UpdateContext::new_instruction(123, "signature".into(), 4)
                .with_solana_transaction(metadata.clone());
            assert_eq!(context.solana_transaction().unwrap(), Some(metadata));
            assert_eq!(
                context.get_metadata(SOLANA_TRANSACTION_METADATA_KEY),
                Some(&json!({"version": encoded}))
            );
        }
        for context in [
            UpdateContext::empty(),
            UpdateContext::new_account(123, "account".into(), 4),
        ] {
            assert_eq!(context.solana_transaction().unwrap(), None);
            assert!(context
                .to_value()
                .get(SOLANA_TRANSACTION_METADATA_KEY)
                .is_none());
        }
        for version in [json!("0"), json!("1"), json!(2), json!(-1), json!(true)] {
            assert!(serde_json::from_value::<SolanaTransactionVersion>(version).is_err());
        }
    }

    #[test]
    fn transaction_metadata_config_presence_zeros_and_precision() {
        for config in [
            json!({}),
            json!({"priority_fee_lamports": "0"}),
            json!({"compute_unit_limit": 0}),
            json!({"loaded_accounts_data_size_limit": 0}),
            json!({"heap_size": 0}),
            json!({"priority_fee_lamports": u64::MAX.to_string(), "compute_unit_limit": 0, "loaded_accounts_data_size_limit": 0, "heap_size": 0}),
        ] {
            let encoded = json!({"version": 1, "config": config});
            let metadata: SolanaTransactionMetadata =
                serde_json::from_value(encoded.clone()).unwrap();
            assert!(metadata.config.is_some());
            assert_eq!(serde_json::to_value(&metadata).unwrap(), encoded);
            let context = UpdateContext::empty().with_solana_transaction(metadata.clone());
            assert_eq!(context.solana_transaction().unwrap(), Some(metadata));
            assert_eq!(context.to_value()[SOLANA_TRANSACTION_METADATA_KEY], encoded);
        }
    }

    #[test]
    fn transaction_metadata_invalid_fee_is_not_silently_unknown() {
        for fee in [
            json!(42),
            json!("18446744073709551616"),
            json!("-1"),
            json!("+1"),
            json!(""),
        ] {
            let context = UpdateContext::empty().with_metadata(
                SOLANA_TRANSACTION_METADATA_KEY.into(),
                json!({"version": 1, "config": {"priority_fee_lamports": fee}}),
            );
            assert!(context.solana_transaction().is_err());
        }
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use crate::{
        compiler::{EntityBytecode, MultiEntityBytecode, OpCode},
        vm::{QueuedInstructionEvent, VmContext},
        UpdateContext,
    };
    use serde_json::json;

    #[test]
    fn transaction_metadata_queued_event_uses_origin_and_restores_trigger() {
        let origin = UpdateContext::new_instruction(100, "origin".into(), 4)
            .with_solana_transaction(SolanaTransactionMetadata {
                version: SolanaTransactionVersion::V1,
                config: Some(SolanaTransactionConfig {
                    priority_fee_lamports: Some(u64::MAX),
                    ..Default::default()
                }),
            });
        let trigger = UpdateContext::new_instruction(200, "trigger".into(), 9);
        let mut vm = VmContext::new();
        vm.set_current_context(Some(origin.clone()));
        vm.queue_instruction_event(
            0,
            QueuedInstructionEvent {
                pda_address: "pda".into(),
                event_type: "QueuedIxState".into(),
                event_data: json!({"data": {"value": 42}}),
                slot: 100,
                signature: "origin".into(),
            },
        )
        .unwrap();
        let queued = vm
            .get_state_table_mut(0)
            .unwrap()
            .pending_instruction_events
            .get("pda")
            .unwrap()[0]
            .clone();
        assert_eq!(
            queued.context.solana_transaction().unwrap(),
            origin.solana_transaction().unwrap()
        );
        assert_eq!(queued.context.txn_index, Some(4));

        let handlers = [
            (
                "RegisterIxState".into(),
                vec![
                    OpCode::LoadConstant {
                        value: json!("pda"),
                        dest: 0,
                    },
                    OpCode::LoadConstant {
                        value: json!("key"),
                        dest: 1,
                    },
                    OpCode::UpdatePdaReverseLookup {
                        state_id: 0,
                        lookup_name: "default_pda_lookup".into(),
                        pda_address: 0,
                        primary_key: 1,
                    },
                ],
            ),
            (
                "QueuedIxState".into(),
                vec![
                    OpCode::LoadConstant {
                        value: json!("key"),
                        dest: 0,
                    },
                    OpCode::ReadOrInitState {
                        state_id: 0,
                        key: 0,
                        default: json!({}),
                        dest: 2,
                    },
                    OpCode::LoadEventField {
                        path: crate::ast::FieldPath::new(&["data"]),
                        dest: 3,
                        default: None,
                    },
                    OpCode::CreateEvent {
                        dest: 4,
                        event_value: 3,
                    },
                    OpCode::SetField {
                        object: 2,
                        path: "event".into(),
                        value: 4,
                    },
                    OpCode::UpdateState {
                        state_id: 0,
                        key: 0,
                        value: 2,
                    },
                    OpCode::EmitMutation {
                        entity_name: "Test".into(),
                        key: 0,
                        state: 2,
                    },
                ],
            ),
        ]
        .into();
        let bytecode = MultiEntityBytecode {
            entities: [(
                "Test".into(),
                EntityBytecode {
                    state_id: 0,
                    handlers,
                    entity_name: "Test".into(),
                    when_events: Default::default(),
                    non_emitted_fields: Default::default(),
                    computed_paths: vec![],
                    computed_fields_evaluator: None,
                },
            )]
            .into(),
            event_routing: [("RegisterIxState".into(), vec!["Test".into()])].into(),
            when_events: Default::default(),
            proto_router: Default::default(),
        };
        let mutations = vm
            .process_event(
                &bytecode,
                json!({}),
                "RegisterIxState",
                Some(&trigger),
                None,
            )
            .unwrap();
        assert_eq!(mutations.len(), 1);
        let state = vm.get_entity_state(0, &json!("key")).unwrap();
        assert_eq!(state["event"]["slot"], json!(100));
        assert_eq!(state["event"]["signature"], json!("origin"));
        assert_eq!(state["event"]["data"], json!({"value": 42}));
        assert!(
            state["event"]
                .get(SOLANA_TRANSACTION_METADATA_KEY)
                .is_none(),
            "ordinary event schemas remain unchanged"
        );
        let restored = vm.current_context().unwrap();
        assert_eq!(restored.signature, trigger.signature);
        assert_eq!(restored.txn_index, Some(9));
        assert_eq!(restored.solana_transaction().unwrap(), None);
    }
}
