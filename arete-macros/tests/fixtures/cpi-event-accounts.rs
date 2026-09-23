//! PumpSwap trades through the generated `#[arete]` runtime.
//!
//! `idl/pump_amm.json` is `buy`, `sell`, `BuyEvent`, `SellEvent`, `Pool` and
//! their types, copied unchanged from the published PumpSwap IDL (see the
//! driver in `tests/cpi_event_accounts.rs` for the source revision).
use arete::interpreter::{VmDebugEvent, VmDebugger};
use arete::prelude::*;
use arete::runtime::yellowstone_grpc_proto::{
    geyser::*,
    prelude::{
        CompiledInstruction, InnerInstruction, InnerInstructions, Message, MessageHeader,
        Transaction, TransactionStatusMeta,
    },
};
use arete::runtime::{
    base64::Engine as _,
    serde_json::{self, json, Value},
    shipstern, tokio,
};
use std::sync::{Arc, Mutex};

#[arete(idl = "idl/pump_amm.json")]
mod pumpswap {
    use arete::macros::Stream;
    use serde::{Deserialize, Serialize};

    #[entity(name = "PumpSwapPool")]
    pub struct PumpSwapPool {
        pub id: PoolId,
        pub trade: PoolTrade,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PoolId {
        // Keyed by the event payload's own `pool`: a trade handed another
        // instruction's accounts shows up here as the wrong mints.
        #[map([pump_amm_sdk::events::BuyEvent::pool, pump_amm_sdk::events::SellEvent::pool],
              primary_key, strategy = SetOnce)]
        pub pool: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PoolTrade {
        #[map([pump_amm_sdk::events::BuyEvent::accounts::pool,
               pump_amm_sdk::events::SellEvent::accounts::pool], strategy = LastWrite)]
        pub pool_account: Option<String>,

        #[map([pump_amm_sdk::events::BuyEvent::accounts::base_mint,
               pump_amm_sdk::events::SellEvent::accounts::base_mint], strategy = LastWrite)]
        pub base_mint: Option<String>,

        #[map([pump_amm_sdk::events::BuyEvent::accounts::quote_mint,
               pump_amm_sdk::events::SellEvent::accounts::quote_mint], strategy = LastWrite)]
        pub quote_mint: Option<String>,

        #[map([pump_amm_sdk::events::BuyEvent::accounts::user,
               pump_amm_sdk::events::SellEvent::accounts::user], strategy = LastWrite)]
        pub trader: Option<String>,

        #[map(pump_amm_sdk::events::BuyEvent::base_amount_out, strategy = LastWrite)]
        pub base_amount_out: Option<u64>,

        #[map(pump_amm_sdk::events::SellEvent::base_amount_in, strategy = LastWrite)]
        pub base_amount_in: Option<u64>,

        #[aggregate(from = [pump_amm_sdk::events::BuyEvent, pump_amm_sdk::events::SellEvent],
                    field = accounts::user, strategy = UniqueCount, lookup_by = pool)]
        pub unique_traders: Option<u64>,

        #[derive_from(from = [pump_amm_sdk::events::BuyEvent, pump_amm_sdk::events::SellEvent],
                      field = accounts::base_mint, lookup_by = pool, strategy = LastWrite)]
        pub derived_base_mint: Option<String>,

        #[event(from = pump_amm_sdk::events::BuyEvent,
                fields = [base_amount_out, quote_amount_in, accounts::base_mint,
                          accounts::quote_mint, accounts::user],
                lookup_by = pool, strategy = LastWrite)]
        pub last_buy: Option<serde_json::Value>,

        #[event(from = pump_amm_sdk::events::SellEvent,
                fields = [base_amount_in, quote_amount_out, accounts::base_mint,
                          accounts::quote_mint, accounts::user],
                lookup_by = pool, strategy = LastWrite)]
        pub last_sell: Option<serde_json::Value>,
    }

    #[entity(name = "PumpSwapPoolAccount")]
    pub struct PumpSwapPoolAccount {
        pub id: PoolAccountId,
        pub trade: PoolAccountTrade,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PoolAccountId {
        // Keyed by the emitting instruction's own `pool` account.
        #[map([pump_amm_sdk::events::BuyEvent::accounts::pool,
               pump_amm_sdk::events::SellEvent::accounts::pool], primary_key, strategy = SetOnce)]
        pub pool: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PoolAccountTrade {
        #[map([pump_amm_sdk::events::BuyEvent::accounts::base_mint,
               pump_amm_sdk::events::SellEvent::accounts::base_mint], strategy = LastWrite)]
        pub base_mint: Option<String>,

        #[aggregate(from = [pump_amm_sdk::events::BuyEvent, pump_amm_sdk::events::SellEvent],
                    strategy = Count, lookup_by = accounts::pool)]
        pub trade_count: Option<u64>,
    }
}

const IDL: &str = include_str!("../idl/pump_amm.json");
const ANCHOR_EVENT_TAG: [u8; 8] = [228, 69, 165, 46, 81, 203, 154, 29];

fn discriminator(section: &str, name: &str) -> Vec<u8> {
    let idl: Value = serde_json::from_str(IDL).unwrap();
    serde_json::from_value(
        idl[section]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap()["discriminator"]
            .clone(),
    )
    .unwrap()
}

fn key(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn b58(key: [u8; 32]) -> String {
    arete::runtime::bs58::encode(key).into_string()
}

fn pump_amm() -> [u8; 32] {
    arete::runtime::bs58::decode("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA")
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap()
}

#[derive(Clone, Copy)]
struct Trade {
    pool: [u8; 32],
    user: [u8; 32],
    /// The payload's `user`. PumpSwap writes the signer here too; it differs
    /// in this fixture so reading the payload instead of `accounts::user`
    /// cannot pass.
    payload_user: [u8; 32],
    base_mint: [u8; 32],
    quote_mint: [u8; 32],
    base_amount: u64,
    quote_amount: u64,
}

impl Trade {
    fn new(tag: u8, base_amount: u64) -> Self {
        Self {
            pool: key(tag),
            user: key(tag + 1),
            payload_user: key(tag + 4),
            base_mint: key(tag + 2),
            quote_mint: key(tag + 3),
            base_amount,
            quote_amount: base_amount + 1_000,
        }
    }

    /// The instruction's accounts in IDL order: pool, user, global_config,
    /// base_mint, quote_mint, then token accounts and programs shared by all
    /// trades.
    fn accounts(&self, count: usize) -> Vec<[u8; 32]> {
        let mut accounts = vec![
            self.pool,
            self.user,
            key(1),
            self.base_mint,
            self.quote_mint,
        ];
        accounts.extend((accounts.len()..count).map(|index| key(100 + index as u8)));
        accounts
    }

    /// Borsh payload of `BuyEvent` (buy = true) or `SellEvent`, field for field.
    fn event(&self, buy: bool) -> Vec<u8> {
        let mut bytes = discriminator("events", if buy { "BuyEvent" } else { "SellEvent" });
        let u64s = |bytes: &mut Vec<u8>, values: &[u64]| {
            for value in values {
                bytes.extend(value.to_le_bytes());
            }
        };
        bytes.extend(1_700_000_000i64.to_le_bytes()); // timestamp
                                                      // base_amount_{out,in} .. user_quote_amount_{in,out}: the quote amount is 7th.
        u64s(
            &mut bytes,
            &[self.base_amount, 0, 0, 0, 0, 0, self.quote_amount],
        );
        u64s(&mut bytes, &[0; 6]);
        bytes.extend(self.pool);
        bytes.extend(self.payload_user);
        for _ in 0..5 {
            bytes.extend([0u8; 32]);
        }
        u64s(&mut bytes, &[0; 2]); // coin_creator_fee_basis_points, coin_creator_fee
        if buy {
            bytes.push(0); // track_volume
            u64s(&mut bytes, &[0; 3]); // total_unclaimed, total_claimed, current_sol_volume
            bytes.extend(0i64.to_le_bytes()); // last_update_timestamp
            u64s(&mut bytes, &[0]); // min_base_amount_out
            bytes.extend(3u32.to_le_bytes()); // ix_name
            bytes.extend(b"buy");
        }
        u64s(&mut bytes, &[0; 4]); // cashback_fee_basis_points .. buyback_fee
        bytes.extend(0i128.to_le_bytes()); // virtual_quote_reserves
        bytes.push(0); // can_boost
        u64s(&mut bytes, &[0; 3]); // base_supply, holder_rewards_bps, holder_rewards
        bytes
    }

    fn instruction_data(&self, buy: bool) -> Vec<u8> {
        let mut bytes = discriminator("instructions", if buy { "buy" } else { "sell" });
        bytes.extend(self.base_amount.to_le_bytes());
        bytes.extend(self.quote_amount.to_le_bytes());
        if buy {
            bytes.push(0); // track_volume: OptionBool
        }
        bytes
    }
}

#[derive(Default)]
struct TransactionBuilder {
    keys: Vec<[u8; 32]>,
    outer: Vec<CompiledInstruction>,
    inner: Vec<InnerInstructions>,
    logs: Vec<String>,
}

impl TransactionBuilder {
    fn index(&mut self, key: [u8; 32]) -> u32 {
        match self.keys.iter().position(|known| *known == key) {
            Some(index) => index as u32,
            None => {
                self.keys.push(key);
                (self.keys.len() - 1) as u32
            }
        }
    }

    fn indices(&mut self, accounts: &[[u8; 32]]) -> Vec<u8> {
        accounts
            .iter()
            .map(|account| self.index(*account) as u8)
            .collect()
    }

    fn outer(&mut self, program: [u8; 32], accounts: &[[u8; 32]], data: Vec<u8>) {
        let program_id_index = self.index(program);
        let accounts = self.indices(accounts);
        self.outer.push(CompiledInstruction {
            program_id_index,
            accounts,
            data,
        });
    }

    /// An inner instruction of the last outer instruction at `stack_height`.
    fn inner(&mut self, height: u32, program: [u8; 32], accounts: &[[u8; 32]], data: Vec<u8>) {
        let program_id_index = self.index(program);
        let accounts = self.indices(accounts);
        let index = self.outer.len() as u32 - 1;
        if self.inner.last().map(|group| group.index) != Some(index) {
            self.inner.push(InnerInstructions {
                index,
                instructions: Vec::new(),
            });
        }
        self.inner
            .last_mut()
            .unwrap()
            .instructions
            .push(InnerInstruction {
                program_id_index,
                accounts,
                data,
                stack_height: Some(height),
            });
    }

    fn cpi_event(&mut self, height: u32, trade: &Trade, buy: bool) {
        let mut data = ANCHOR_EVENT_TAG.to_vec();
        data.extend(trade.event(buy));
        self.inner(height, pump_amm(), &[key(2)], data);
    }

    fn build(self, signature: u8) -> SubscribeUpdateTransaction {
        SubscribeUpdateTransaction {
            slot: 100,
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![signature; 64],
                index: signature as u64,
                is_vote: false,
                transaction: Some(Transaction {
                    signatures: vec![vec![signature; 64]],
                    message: Some(Message {
                        header: Some(MessageHeader::default()),
                        account_keys: self.keys.iter().map(|key| key.to_vec()).collect(),
                        recent_blockhash: vec![5; 32],
                        instructions: self.outer,
                        versioned: false,
                        config: None,
                        address_table_lookups: vec![],
                    }),
                }),
                meta: Some(TransactionStatusMeta {
                    inner_instructions: self.inner,
                    log_messages: self.logs,
                    ..Default::default()
                }),
            }),
        }
    }
}

/// Every emitted patch as `(entity, key, patch)`, in order: what clients see.
#[derive(Default)]
struct Patches(Mutex<Vec<(String, Value, Value)>>);

impl VmDebugger for Patches {
    fn record(&self, event: VmDebugEvent) {
        if let VmDebugEvent::EmitMutation {
            entity_name,
            key,
            emitted: true,
            patch: Some(patch),
            ..
        } = event
        {
            self.0.lock().unwrap().push((entity_name, key, patch));
        }
    }
}

/// The captured `#[event]` record holds the trade's amounts (u64s are
/// published as decimal strings) and both mints.
fn assert_trade_record(record: &Value, trade: &Trade, buy: bool) {
    let (base, quote) = if buy {
        ("base_amount_out", "quote_amount_in")
    } else {
        ("base_amount_in", "quote_amount_out")
    };
    for (field, expected) in [
        (base, json!(trade.base_amount.to_string())),
        (quote, json!(trade.quote_amount.to_string())),
        ("base_mint", json!(b58(trade.base_mint))),
        ("quote_mint", json!(b58(trade.quote_mint))),
        ("user", json!(b58(trade.user))),
    ] {
        assert_eq!(record["data"][field], expected, "{field} in {record}");
    }
}

async fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();

    let vm = Arc::new(Mutex::new(arete::interpreter::vm::VmContext::new()));
    let patches = Arc::new(Patches::default());
    vm.lock().unwrap().set_debugger(patches.clone());
    let bytecode = Arc::new(pumpswap::create_multi_entity_bytecode());
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let handler = pumpswap::VmHandler::new(
        vm.clone(),
        bytecode.clone(),
        tx,
        None,
        arete::server::SlotTracker::new(),
        None,
        arete::interpreter::runtime_resolvers_factory::build_resolver().unwrap(),
        Arc::new(Mutex::new(
            arete::interpreter::scheduler::SlotScheduler::new(),
        )),
        Arc::new(tokio::sync::Semaphore::new(1)),
    );
    let pipeline = shipstern::instruction::InstructionPipeline::new(vec![Box::new(
        shipstern::Pipeline::new(pumpswap::parsers::InstructionParser, [handler]),
    )])
    .unwrap();

    let router = key(200);
    let token_program = key(3);
    let buy = Trade::new(10, 1_001);
    let sell = Trade::new(20, 1_002);
    let orphan = Trade::new(30, 1_003);
    let logged_buy = Trade::new(40, 1_004);
    let logged_sell = Trade::new(50, 1_005);

    // Two `emit_cpi!` trades under different pump_amm parents, both routed
    // through a foreign program, then an event whose parent pump_amm
    // instruction is not in the IDL and so is never handled.
    let mut cpi = TransactionBuilder::default();
    cpi.outer(router, &[key(4)], vec![0]);
    cpi.inner(2, pump_amm(), &buy.accounts(23), buy.instruction_data(true));
    cpi.inner(3, token_program, &[buy.user], vec![3]);
    cpi.cpi_event(3, &buy, true);
    cpi.inner(
        2,
        pump_amm(),
        &sell.accounts(21),
        sell.instruction_data(false),
    );
    cpi.cpi_event(3, &sell, false);
    cpi.outer(pump_amm(), &orphan.accounts(23), vec![9; 8]);
    cpi.cpi_event(2, &orphan, true);
    pipeline.handle(&cpi.build(7)).await.unwrap();

    // `emit!` log events. The sell is an inner pump_amm instruction, so its
    // `Program data:` line also falls inside the outer buy's log range.
    let program = b58(pump_amm());
    let data = |trade: &Trade, buy: bool| {
        format!(
            "Program data: {}",
            arete::runtime::base64::engine::general_purpose::STANDARD.encode(trade.event(buy))
        )
    };
    let mut logs = TransactionBuilder::default();
    logs.outer(
        pump_amm(),
        &logged_buy.accounts(23),
        logged_buy.instruction_data(true),
    );
    logs.inner(
        2,
        pump_amm(),
        &logged_sell.accounts(21),
        logged_sell.instruction_data(false),
    );
    logs.logs = vec![
        format!("Program {program} invoke [1]"),
        data(&logged_buy, true),
        format!("Program {program} invoke [2]"),
        data(&logged_sell, false),
        format!("Program {program} success"),
        format!("Program {program} success"),
    ];
    pipeline.handle(&logs.build(8)).await.unwrap();

    // The same signer sells on the sell pool again, with another payload
    // `user`: it is one trader by `accounts::user`, two by the payload.
    let sell_again = Trade {
        payload_user: key(99),
        base_amount: 1_006,
        quote_amount: 2_006,
        ..sell
    };
    let mut again = TransactionBuilder::default();
    again.outer(
        pump_amm(),
        &sell_again.accounts(21),
        sell_again.instruction_data(false),
    );
    again.cpi_event(2, &sell_again, false);
    pipeline.handle(&again.build(9)).await.unwrap();

    let state_id = |entity: &str| bytecode.entities[entity].state_id;
    let pool = |trade: &Trade| {
        vm.lock()
            .unwrap()
            .get_entity_state(state_id("PumpSwapPool"), &json!(b58(trade.pool)))
            .unwrap_or_else(|| panic!("no PumpSwapPool for {}", b58(trade.pool)))["trade"]
            .clone()
    };

    // (latest trade on the pool, its first trade, buy?)
    for (trade, first, is_buy) in [
        (&buy, &buy, true),
        (&sell_again, &sell, false),
        (&logged_buy, &logged_buy, true),
        (&logged_sell, &logged_sell, false),
    ] {
        let state = pool(trade);
        for (field, expected) in [
            ("pool_account", trade.pool),
            ("base_mint", trade.base_mint),
            ("quote_mint", trade.quote_mint),
            ("trader", trade.user),
            ("derived_base_mint", trade.base_mint),
        ] {
            assert_eq!(state[field], json!(b58(expected)), "{field} in {state}");
        }
        let amount = if is_buy {
            "base_amount_out"
        } else {
            "base_amount_in"
        };
        assert_eq!(
            state[amount],
            json!(trade.base_amount.to_string()),
            "{state}"
        );
        assert_eq!(state["unique_traders"], json!(1), "{state}");
        let record = if is_buy {
            &state["last_buy"]
        } else {
            &state["last_sell"]
        };
        assert_trade_record(record, trade, is_buy);

        // The pool's first emitted patch is one record: amounts and both mints.
        let first_patch = patches
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(entity, key, _)| entity == "PumpSwapPool" && *key == json!(b58(first.pool)))
            .map(|(_, _, patch)| patch["trade"].clone())
            .unwrap();
        assert_eq!(
            first_patch[amount],
            json!(first.base_amount.to_string()),
            "{first_patch}"
        );
        assert_eq!(first_patch["base_mint"], json!(b58(first.base_mint)));
        assert_eq!(first_patch["quote_mint"], json!(b58(first.quote_mint)));
    }

    // The emitting instruction of a CPI event is its own inner occurrence.
    assert_eq!(pool(&buy)["last_buy"]["ix_path"], json!("0.0.1"));
    assert_eq!(pool(&sell)["last_sell"]["ix_path"], json!("0.0"));

    // The orphan still records its payload, with no accounts from anywhere else.
    let orphaned = pool(&orphan);
    assert_eq!(
        orphaned["base_amount_out"],
        json!(orphan.base_amount.to_string())
    );
    for field in [
        "pool_account",
        "base_mint",
        "quote_mint",
        "trader",
        "derived_base_mint",
    ] {
        assert!(
            orphaned.get(field).is_none_or(Value::is_null),
            "{field} must stay unset in {orphaned}"
        );
    }
    let record = &orphaned["last_buy"]["data"];
    assert_eq!(
        record["base_amount_out"],
        json!(orphan.base_amount.to_string())
    );
    for field in ["base_mint", "quote_mint", "user"] {
        assert!(record.get(field).is_none_or(Value::is_null), "{record}");
    }

    // Keyed by the emitting instruction's `pool` account: the orphan has none,
    // so clients never see a row for it.
    let mut emitted_keys = patches
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(entity, _, _)| entity == "PumpSwapPoolAccount")
        .map(|(_, key, _)| key.clone())
        .collect::<Vec<_>>();
    emitted_keys.dedup();
    let trades = [(&buy, 1), (&sell, 2), (&logged_buy, 1), (&logged_sell, 1)];
    assert_eq!(
        emitted_keys,
        [&buy, &sell, &logged_buy, &logged_sell, &sell]
            .map(|trade| json!(b58(trade.pool)))
            .to_vec()
    );
    for (trade, count) in trades {
        let state = vm
            .lock()
            .unwrap()
            .get_entity_state(state_id("PumpSwapPoolAccount"), &json!(b58(trade.pool)))
            .unwrap();
        assert_eq!(
            state["trade"]["base_mint"],
            json!(b58(trade.base_mint)),
            "{state}"
        );
        assert_eq!(state["trade"]["trade_count"], json!(count), "{state}");
    }

    sdk_types();
    println!("pumpswap trades verified");
}

/// The text of `source` from `start` up to the next `end`.
fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let from = source
        .find(start)
        .unwrap_or_else(|| panic!("no {start:?} in:\n{source}"));
    let rest = &source[from..];
    let len = rest[start.len()..]
        .find(end)
        .map_or(rest.len(), |at| at + start.len());
    &rest[..len]
}

/// The generated clients type the account fields as pubkey strings.
fn sdk_types() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".arete");
    let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
    let program = arete_artifacts::load_program_spec(&read("pump_amm.program-spec.json"))
        .unwrap()
        .artifact;
    let live = arete_artifacts::load_live_spec_v2(&read("Pumpswap.live-spec.json"))
        .unwrap()
        .artifact;
    let manifest = arete_artifacts::load_stack_manifest_v2(&read("Pumpswap.stack-manifest.json"))
        .unwrap()
        .artifact;
    let spec = arete::interpreter::public_artifacts::stack_spec_from_artifacts_v2(
        &[program],
        &live,
        &manifest,
    )
    .unwrap();
    let ts = arete::interpreter::typescript::compile_stack_spec(spec.clone(), None).unwrap();
    let py = arete::interpreter::python::compile_stack_spec(spec.clone(), None).unwrap();
    let rs = arete::interpreter::rust::compile_stack_spec(spec, None).unwrap();

    let ts = section(
        &ts.interfaces,
        "export interface PumpSwapPoolTrade {",
        "\n}",
    );
    let py = section(&py.models_py, "class PumpSwapPoolTrade", "last_sell");
    let rs = section(&rs.types_rs, "pub struct PumpSwapPoolTrade {", "\n}");
    for (camel, snake) in [
        ("poolAccount", "pool_account"),
        ("baseMint", "base_mint"),
        ("quoteMint", "quote_mint"),
        ("trader", "trader"),
        ("derivedBaseMint", "derived_base_mint"),
    ] {
        assert!(ts.contains(&format!("{camel}: string | null;")), "{ts}");
        assert!(
            py.contains(&format!("{snake}: Optional[str] = None")),
            "{py}"
        );
        assert!(
            rs.contains(&format!("pub {snake}: Option<Option<String>>,")),
            "{rs}"
        );
    }
}

fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(run());
}
