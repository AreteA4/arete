use arete::interpreter::{self as vm, SolanaTransactionMetadata, UpdateContext};
use arete::runtime::yellowstone_grpc_proto::{
    geyser::*,
    prelude::{
        CompiledInstruction, InnerInstruction, InnerInstructions, Message, MessageHeader,
        Transaction, TransactionConfig, TransactionStatusMeta,
    },
    prost::Message as _,
};
use arete::runtime::{
    serde_json::{self, json, Value},
    shipstern, shipstern_core, tokio,
};
use shipstern::Handler;
use shipstern_core::instruction::InstructionUpdate;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct Observations {
    hooks: Mutex<Vec<Option<SolanaTransactionMetadata>>>,
    events: Mutex<Vec<(String, Option<Value>)>>,
    continuations: Mutex<Vec<Option<SolanaTransactionMetadata>>>,
}
static OBS: std::sync::OnceLock<Arc<Observations>> = std::sync::OnceLock::new();
impl vm::VmDebugger for Observations {
    fn record(&self, event: vm::VmDebugEvent) {
        if let vm::VmDebugEvent::ProcessEventStart {
            event_type,
            context,
        } = event
        {
            self.events.lock().unwrap().push((event_type, context));
        }
    }
}
impl vm::RuntimeResolver for Observations {
    fn resolve_batch<'a>(
        &'a self,
        _: &'a [vm::RuntimeResolverRequest],
    ) -> vm::ResolverBatchFuture<'a> {
        Box::pin(async { Ok(Default::default()) })
    }
    fn resolve_and_apply<'a>(
        &'a self,
        state: &'a Mutex<vm::vm::VmContext>,
        bytecode: &'a vm::compiler::MultiEntityBytecode,
        requests: Vec<vm::ResolverRequest>,
        context: Option<UpdateContext>,
    ) -> vm::ResolverApplyFuture<'a> {
        Box::pin(async move {
            self.continuations
                .lock()
                .unwrap()
                .push(context.unwrap().solana_transaction().unwrap());
            for request in requests {
                state
                    .lock()
                    .unwrap()
                    .apply_resolver_result(bytecode, &request.cache_key, Value::Null)
                    .unwrap();
            }
            Vec::new()
        })
    }
}
fn get_instruction_hooks(_: &str) -> Vec<fn(&mut vm::InstructionContext)> {
    vec![|context| {
        OBS.get()
            .unwrap()
            .hooks
            .lock()
            .unwrap()
            .push(context.solana_transaction().unwrap());
        context.register_pda_reverse_lookup("pending-account", "key");
    }]
}
fn get_resolver_for_account_type(
    _: &str,
) -> Option<fn(&str, &Value, &mut vm::ResolveContext) -> vm::KeyResolution> {
    None
}

mod parsers {
    use super::*;
    #[derive(Debug, Clone)]
    pub struct AccountValue;
    impl AccountValue {
        pub fn event_type(&self) -> &str {
            "AccountState"
        }
        pub fn to_value(&self) -> Value {
            json!({"data": {"value": 42}})
        }
    }
    #[derive(Debug, Clone)]
    pub struct InstructionValue(pub &'static str);
    impl InstructionValue {
        pub fn event_type(&self) -> &str {
            self.0
        }
        pub fn to_value_with_accounts(&self, accounts: &[shipstern_core::Pubkey]) -> Value {
            json!({"data": {"value": 42}, "accounts": {"owner": accounts.first().map(|p| arete::runtime::bs58::encode(p).into_string())}})
        }
        pub fn try_unpack_log_event(bytes: &[u8]) -> Result<Self, ()> {
            if bytes == [42] {
                Ok(Self("LogCpiEvent"))
            } else {
                Err(())
            }
        }
    }
}
#[derive(Debug)]
struct ProgramParser(u8);
impl shipstern_core::Parser for ProgramParser {
    type Input = InstructionUpdate;
    type Output = parsers::InstructionValue;
    fn id(&self) -> std::borrow::Cow<'static, str> {
        format!("program-{}", self.0).into()
    }
    fn prefilter(&self) -> shipstern_core::Prefilter {
        Default::default()
    }
    async fn parse(&self, ix: &InstructionUpdate) -> shipstern_core::ParseResult<Self::Output> {
        if ix.program != shipstern_core::Pubkey::new([self.0; 32]) {
            return Err(shipstern_core::ParseError::Filtered);
        }
        Ok(parsers::InstructionValue(if self.0 == 1 {
            "OuterIxState"
        } else {
            "InnerCpiEvent"
        }))
    }
}
mod single {
    use super::*;
    __single_handler!();
}
mod multi {
    use super::*;
    __multi_handler!();
}

fn fixture(config: Option<TransactionConfig>, versioned: bool) -> SubscribeUpdateTransaction {
    let outer_program = arete::runtime::bs58::encode([1u8; 32]).into_string();
    let inner_program = arete::runtime::bs58::encode([2u8; 32]).into_string();
    SubscribeUpdateTransaction {
        slot: 123,
        transaction: Some(SubscribeUpdateTransactionInfo {
            signature: vec![3; 64],
            index: 7,
            is_vote: false,
            transaction: Some(Transaction {
                signatures: vec![vec![3; 64]],
                message: Some(Message {
                    header: Some(MessageHeader::default()),
                    account_keys: vec![vec![1; 32], vec![2; 32], vec![4; 32]],
                    recent_blockhash: vec![5; 32],
                    instructions: vec![CompiledInstruction {
                        program_id_index: 0,
                        accounts: vec![2],
                        data: vec![42],
                    }],
                    versioned,
                    config,
                    address_table_lookups: vec![],
                }),
            }),
            meta: Some(TransactionStatusMeta {
                inner_instructions: vec![InnerInstructions {
                    index: 0,
                    instructions: vec![InnerInstruction {
                        program_id_index: 1,
                        accounts: vec![2],
                        data: vec![43],
                        stack_height: Some(2),
                    }],
                }],
                log_messages: vec![
                    format!("Program {outer_program} invoke [1]"),
                    "Program data: Kg==".into(),
                    format!("Program {inner_program} invoke [2]"),
                    "Program data: Kg==".into(),
                    format!("Program {inner_program} success"),
                    format!("Program {outer_program} success"),
                ],
                ..Default::default()
            }),
        }),
    }
}
fn bytecode() -> vm::compiler::MultiEntityBytecode {
    use vm::compiler::{EntityBytecode, MultiEntityBytecode, OpCode};
    let handlers = [
        "OuterIxState",
        "InnerCpiEvent",
        "LogCpiEvent",
        "AccountState",
    ]
    .into_iter()
    .map(|event| {
        (
            event.to_string(),
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
                    path: vm::ast::FieldPath::new(&["data", "value"]),
                    dest: 3,
                    default: None,
                },
                OpCode::SetField {
                    object: 2,
                    path: "value".into(),
                    value: 3,
                },
                OpCode::QueueResolver {
                    state_id: 0,
                    entity_name: "Test".into(),
                    resolver: vm::ast::ResolverType::Token,
                    input_path: None,
                    input_value: Some(json!(event)),
                    url_template: None,
                    strategy: vm::ast::ResolveStrategy::LastWrite,
                    extracts: vec![],
                    condition: None,
                    schedule_at: None,
                    state: 2,
                    key: 0,
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
        )
    })
    .collect::<std::collections::HashMap<_, _>>();
    MultiEntityBytecode {
        event_routing: handlers
            .keys()
            .map(|event| (event.clone(), vec!["Test".into()]))
            .collect(),
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
        when_events: Default::default(),
        proto_router: Default::default(),
    }
}
async fn run() {
    let observations = Arc::new(Observations::default());
    OBS.set(observations.clone()).unwrap();
    for variant in [false, true] {
        for (config, versioned) in [
            (None, false),
            (None, true),
            (Some(TransactionConfig::default()), true),
            (
                Some(TransactionConfig {
                    priority_fee: Some(0),
                    compute_unit_limit: Some(0),
                    loaded_accounts_data_size_limit: Some(0),
                    heap_size: Some(0),
                }),
                true,
            ),
            (
                Some(TransactionConfig {
                    priority_fee: Some(u64::MAX),
                    compute_unit_limit: Some(1_400_000),
                    loaded_accounts_data_size_limit: None,
                    heap_size: Some(32_768),
                }),
                true,
            ),
        ] {
            let transaction = fixture(config.clone(), versioned);
            // Protobuf field 7 is exercised on the wire, including present-empty (3a 00).
            let wire = transaction.encode_to_vec();
            let decoded = SubscribeUpdateTransaction::decode(wire.as_slice()).unwrap();
            assert_eq!(decoded, transaction);
            if config == Some(TransactionConfig::default()) {
                assert!(decoded
                    .transaction
                    .as_ref()
                    .unwrap()
                    .transaction
                    .as_ref()
                    .unwrap()
                    .message
                    .as_ref()
                    .unwrap()
                    .encode_to_vec()
                    .windows(2)
                    .any(|bytes| bytes == [0x3a, 0]));
            }
            let instructions = InstructionUpdate::build_from_txn(&decoded).unwrap();
            let all = instructions
                .iter()
                .flat_map(|ix| ix.visit_all())
                .collect::<Vec<_>>();
            assert_eq!(all.len(), 2);
            assert_eq!(
                all.iter().map(|ix| ix.data.clone()).collect::<Vec<_>>(),
                [vec![42], vec![43]]
            );
            assert!(Arc::ptr_eq(&all[0].shared, &all[1].shared));
            assert_eq!(all[0].accounts, all[1].accounts);
            assert_eq!(
                all[0]
                    .log_messages()
                    .iter()
                    .filter(|line| line.starts_with("Program data:"))
                    .count(),
                2
            );
            assert_eq!(
                all[1]
                    .log_messages()
                    .iter()
                    .filter(|line| line.starts_with("Program data:"))
                    .count(),
                1
            );
            let expected =
                arete::transaction_metadata::observed_transaction_metadata(&all[0].shared);
            assert_eq!(expected.is_some(), config.is_some());
            let state = Arc::new(Mutex::new(vm::vm::VmContext::new()));
            state.lock().unwrap().set_debugger(observations.clone());
            state
                .lock()
                .unwrap()
                .queue_account_update(
                    0,
                    vm::QueuedAccountUpdate {
                        pda_address: "pending-account".into(),
                        account_type: "AccountState".into(),
                        account_data: json!({"data": {"value": 42}}),
                        slot: 50,
                        write_version: 1,
                        signature: "account-origin".into(),
                    },
                )
                .unwrap();
            let bytecode = Arc::new(bytecode());
            let (tx, mut rx) = tokio::sync::mpsc::channel(16);
            let tracker = arete::server::SlotTracker::new();
            macro_rules! check_handler {
                ($handler:expr) => {{
                    let handler = $handler;
                    for (index, ix) in all.iter().enumerate() {
                        let value = parsers::InstructionValue(if index == 0 {
                            "OuterIxState"
                        } else {
                            "InnerCpiEvent"
                        });
                        handler.handle(&value, ix).await.unwrap();
                        let context = state.lock().unwrap().current_context().cloned().unwrap();
                        assert_eq!(context.slot, Some(123));
                        assert_eq!(context.txn_index, Some(7));
                        assert_eq!(context.solana_transaction().unwrap(), expected);
                        assert_eq!(observations.hooks.lock().unwrap().last(), Some(&expected));
                        assert_eq!(
                            observations.continuations.lock().unwrap().last(),
                            Some(&expected)
                        );
                        let _ = rx.try_recv();
                    }
                    // The bundled Shipstern dispatcher self-filters two programs and
                    // walks the CPI tree in the same order on duplicate replay.
                    let pipeline = shipstern::instruction::InstructionPipeline::new(vec![
                        Box::new(shipstern::Pipeline::new(
                            ProgramParser(1),
                            [handler.clone()],
                        )),
                        Box::new(shipstern::Pipeline::new(
                            ProgramParser(2),
                            [handler.clone()],
                        )),
                    ])
                    .unwrap();
                    pipeline.handle(&decoded).await.unwrap();
                    while rx.try_recv().is_ok() {}
                    // An account update following V1 must not inherit transaction metadata.
                    let account = SubscribeUpdateAccount {
                        slot: 124,
                        account: Some(SubscribeUpdateAccountInfo {
                            pubkey: vec![4; 32],
                            txn_signature: Some(vec![6; 64]),
                            write_version: 1,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    handler
                        .handle(&parsers::AccountValue, &account)
                        .await
                        .unwrap();
                    assert!(state
                        .lock()
                        .unwrap()
                        .current_context()
                        .unwrap()
                        .solana_transaction()
                        .unwrap()
                        .is_none());
                }};
            }
            let scheduler = Arc::new(Mutex::new(vm::scheduler::SlotScheduler::new()));
            let semaphore = Arc::new(tokio::sync::Semaphore::new(2));
            if variant {
                check_handler!(multi::VmHandler::new(
                    state.clone(),
                    bytecode,
                    tx,
                    None,
                    tracker,
                    None,
                    observations.clone(),
                    scheduler,
                    semaphore
                ));
            } else {
                check_handler!(single::VmHandler::new(
                    state.clone(),
                    bytecode,
                    tx,
                    None,
                    tracker,
                    None,
                    observations.clone(),
                    scheduler,
                    semaphore
                ));
            }
            assert_eq!(
                state.lock().unwrap().get_entity_state(0, &json!("key")),
                Some(json!({"value": 42}))
            );
            let events = std::mem::take(&mut *observations.events.lock().unwrap());
            let names = events
                .iter()
                .map(|(event, _)| event.as_str())
                .collect::<Vec<_>>();
            let mut expected_names = if variant {
                vec![
                    "OuterIxState",
                    "LogCpiEvent",
                    "LogCpiEvent",
                    "InnerCpiEvent",
                    "LogCpiEvent",
                ]
            } else {
                vec!["OuterIxState", "InnerCpiEvent"]
            };
            let replay_names = expected_names.clone();
            expected_names.insert(1, "AccountState");
            expected_names.extend(replay_names);
            expected_names.push("AccountState");
            assert_eq!(names, expected_names);
            for (event, context) in events {
                let metadata = context.unwrap().get("solana_transaction").cloned();
                assert_eq!(
                    metadata,
                    if event == "AccountState" {
                        None
                    } else {
                        expected.as_ref().map(|m| serde_json::to_value(m).unwrap())
                    }
                );
            }
        }
    }
}
fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(run());
}
