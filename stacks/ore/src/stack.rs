use arete::prelude::*;

#[arete(idl = ["idl/ore.json", "idl/entropy.json"])]
pub mod ore_stream {
    use arete::macros::Stream;
    use arete::resolvers::{ResolvedSlotHash, TokenMetadata};

    use serde::{Deserialize, Serialize};

    // Program-derived address seeds for the ORE program. The Steel IDL does not
    // embed per-account PDA metadata, so these are declared explicitly and matched
    // against instruction account names by the compiler. Seeds are taken from the
    // ORE instruction docs.
    //
    // Note: `round = ["round", round_id]` is intentionally omitted because some
    // instructions derive it from a cross-account field (`board.round_id`), which
    // is not yet expressible here (follow-up: per-instruction seed binding).
    pdas! {
        ore {
            treasury = [literal("treasury")];
            config = [literal("config")];
            board = [literal("board")];
            miner = [literal("miner"), account("authority")];
            automation = [literal("automation"), account("authority")];
        }
    }

    #[entity(name = "OreRound")]
    #[view(name = "latest", sort_by = "id.round_id", order = "desc")]
    pub struct OreRound {
        pub id: RoundId,
        pub state: RoundState,
        pub results: RoundResults,
        pub metrics: RoundMetrics,
        pub treasury: RoundTreasury,
        pub entropy: EntropyState,
        #[resolve(address = "oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp")]
        pub ore_metadata: Option<TokenMetadata>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundId {
        // Board carries the authoritative active round id, so its updates can
        // populate the matching OreRound deadline before the round closes.
        #[map([
            ore_sdk::accounts::Round::id,
            ore_sdk::accounts::Board::round_id
        ], primary_key, strategy = SetOnce)]
        pub round_id: u64,

        #[map(ore_sdk::accounts::Round::__account_address, lookup_index, strategy = SetOnce)]
        pub round_address: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundState {
        #[map(ore_sdk::accounts::Round::expires_at, strategy = LastWrite)]
        pub closes_at: Option<u64>,

        // Board.end_slot and Entropy Var.end_at identify the same round. Indexing
        // the board value lets an early Var update wait for the matching board
        // update instead of depending on Deploy/Reset account-address remapping.
        #[map(ore_sdk::accounts::Board::end_slot,
              lookup_index,
              strategy = LastWrite,
              emit = false)]
        pub end_at: Option<u64>,

        // Preserve the public field name used by existing clients.
        #[computed(state.end_at)]
        pub expires_at: Option<u64>,

        #[computed({
            let expires_at_slot = state.end_at.unwrap_or(0) as u64;
            let current_slot = __slot;
            if current_slot > 0 && expires_at_slot > current_slot {
                Some(__timestamp + (((expires_at_slot - current_slot) * 400 / 1000) as i64))
            } else {
                None
            }
        })]
        pub estimated_expires_at_unix: Option<i64>,

        #[map(ore_sdk::accounts::Round::motherlode, strategy = LastWrite,
              transform = ui_amount(11))]
        pub motherlode: Option<f64>,

        #[computed(state.deployed_per_square.sum().ui_amount(9))]
        pub total_deployed: Option<f64>,

        #[map(ore_sdk::accounts::Round::total_vaulted, strategy = LastWrite,
              transform = ui_amount(9))]
        pub total_vaulted: Option<f64>,

        #[map(ore_sdk::accounts::Round::total_winnings, strategy = LastWrite,
              transform = ui_amount(9))]
        pub total_winnings: Option<f64>,

        #[map(ore_sdk::accounts::Round::total_miners, strategy = LastWrite)]
        pub total_miners: Option<u64>,

        // Per-square deployed SOL amounts (25 squares in 5x5 grid)
        #[map(ore_sdk::accounts::Round::deployed, strategy = LastWrite)]
        pub deployed_per_square: Option<Vec<u64>>,

        #[computed(state.deployed_per_square.map(|x| x.ui_amount(9)))]
        pub deployed_per_square_ui: Option<Vec<f64>>,

        // Per-square miner counts (25 squares in 5x5 grid)
        #[map(ore_sdk::accounts::Round::count, strategy = LastWrite)]
        pub count_per_square: Option<Vec<u64>>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundResults {
        #[map(ore_sdk::accounts::Round::top_miner, strategy = LastWrite, transform = Base58Encode)]
        pub top_miner: Option<String>,

        #[map(ore_sdk::accounts::Round::rewards, strategy = LastWrite, emit = false)]
        pub rewards_per_square: Option<Vec<u64>>,

        #[computed(results.rewards_per_square.sum().ui_amount(11))]
        pub top_miner_reward: Option<f64>,

        #[map(ore_sdk::accounts::Round::rent_payer, strategy = LastWrite, transform = Base58Encode)]
        pub rent_payer: Option<String>,

        #[map(ore_sdk::accounts::Round::slot_hash, strategy = LastWrite, transform = Base58Encode)]
        pub slot_hash: Option<String>,

        // Raw bytes of slot_hash for RNG calculation (from Round account, not entropy)
        #[map(ore_sdk::accounts::Round::slot_hash, strategy = LastWrite, emit = false)]
        pub slot_hash_bytes: Option<Vec<u8>>,

        // Computed field that fetches the slot hash at expires_at from our cache
        // This is populated from the SlotHashes sysvar via gRPC subscription
        #[computed(state.end_at.slot_hash())]
        pub expires_at_slot_hash: Option<ResolvedSlotHash>,

        #[computed(
            if results.slot_hash_bytes.is_none() {
                None
            } else {
                let hash = results.slot_hash_bytes.to_bytes();
                if (hash.len() as u64) != 32 {
                    None
                } else {
                    let all_zeros = hash == [0u8; 32];
                    let all_ff = hash == [0xFFu8; 32];
                    if all_zeros || all_ff {
                        None
                    } else {
                        let r1 = u64::from_le_bytes(hash[0..8]);
                        let r2 = u64::from_le_bytes(hash[8..16]);
                        let r3 = u64::from_le_bytes(hash[16..24]);
                        let r4 = u64::from_le_bytes(hash[24..32]);
                        Some(r1 ^ r2 ^ r3 ^ r4)
                    }
                }
            }
        )]
        pub rng: Option<u64>,

        #[computed(results.rng.map(|r| r % 25))]
        pub winning_square: Option<u64>,

        #[computed(if id.round_id.unwrap_or(0) as u64 >= 335000 {
            results.rng.map(|r| r.reverse_bits() % 500 == 0)
        } else {
            results.rng.map(|r| r.reverse_bits() % 625 == 0)
        })]
        pub did_hit_motherlode: Option<bool>,

        // Pre-reveal RNG calculation using resolved seed from API
        // keccak_rng resolver: keccak256(slot_hash || seed || samples_le_bytes) → XOR-folded u64
        #[computed(results.expires_at_slot_hash.keccak_rng(entropy.resolved_seed, entropy.entropy_samples))]
        pub pre_reveal_rng_candidate: Option<u64>,

        #[computed(results.rng.filter(|rng| !rng.is_null()).or(results.pre_reveal_rng_candidate))]
        pub pre_reveal_rng: Option<u64>,

        #[computed(results.pre_reveal_rng.map(|r| r % 25))]
        pub pre_reveal_winning_square: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundMetrics {
        // Count of deploy instructions for this round
        #[aggregate(from = ore_sdk::instructions::Deploy, strategy = Count, lookup_by = accounts::round)]
        pub deploy_count: Option<u64>,

        // Count of checkpoint instructions for this round
        #[aggregate(from = ore_sdk::instructions::Checkpoint, strategy = Count, lookup_by = accounts::round)]
        pub checkpoint_count: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundTreasury {
        #[map(ore_sdk::accounts::Treasury::motherlode,
              lookup_index(register_from = [
                  (ore_sdk::instructions::Reset, accounts::treasury, accounts::roundNext)
              ]),
              strategy = SetOnce,
              transform = ui_amount(11))]
        pub motherlode: Option<f64>,
    }

    // ========================================================================
    // Entropy — Cross-program randomness state from the Entropy program.
    // Var.end_at joins to the matching Board.end_slot lookup index. This remains
    // correct when the shared entropy account updates before the Deploy instruction.
    // ========================================================================

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct EntropyState {
        #[map(entropy_sdk::accounts::Var::value,
              join_on = end_at,
              when = entropy_sdk::instructions::Reveal,
              condition = "value != ZERO_32",
              strategy = LastWrite,
              transform = Base58Encode)]
        pub entropy_value: Option<String>,

        #[map(entropy_sdk::accounts::Var::seed,
              join_on = end_at,
              strategy = LastWrite,
              transform = Base58Encode)]
        pub entropy_seed: Option<String>,

        #[map(entropy_sdk::accounts::Var::slot_hash,
              join_on = end_at,
              strategy = LastWrite,
              transform = Base58Encode)]
        pub entropy_slot_hash: Option<String>,

        #[map(entropy_sdk::accounts::Var::start_at, join_on = end_at, strategy = LastWrite)]
        pub entropy_start_at: Option<u64>,

        #[map(entropy_sdk::accounts::Var::end_at, join_on = end_at, strategy = LastWrite)]
        pub entropy_end_at: Option<u64>,

        #[map(entropy_sdk::accounts::Var::samples, join_on = end_at, strategy = LastWrite)]
        pub entropy_samples: Option<u64>,

        #[map(entropy_sdk::accounts::Var::__account_address,
              join_on = end_at,
              strategy = SetOnce)]
        pub entropy_var_address: Option<String>,

        #[resolve(
            url = "https://entropy-api.onrender.com/var/{entropy.entropy_var_address}/seed?samples={entropy.entropy_samples}",
            extract = "seed",
            schedule_at = entropy.entropy_end_at,
            condition = "entropy.entropy_value == null",
            strategy = SetOnce
        )]
        pub resolved_seed: Option<Vec<u8>>,
    }

    // ========================================================================
    // Board Entity — Authoritative singleton current-round state
    // ========================================================================

    #[entity(name = "OreBoard")]
    pub struct OreBoard {
        pub id: BoardId,
        pub state: BoardState,

        #[snapshot(strategy = LastWrite)]
        pub board_snapshot: Option<ore_sdk::accounts::Board>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct BoardId {
        #[map(ore_sdk::accounts::Board::__account_address, primary_key, strategy = SetOnce)]
        pub address: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct BoardState {
        #[map(ore_sdk::accounts::Board::round_id, strategy = LastWrite)]
        pub round_id: u64,

        #[map(ore_sdk::accounts::Board::start_slot, strategy = LastWrite)]
        pub start_slot: u64,

        #[map(ore_sdk::accounts::Board::end_slot, strategy = LastWrite)]
        pub end_slot: u64,

        #[map(ore_sdk::accounts::Board::production_cost_ema, strategy = LastWrite)]
        pub production_cost_ema: u64,
    }

    // ========================================================================
    // Treasury Entity — Singleton protocol-wide state
    // ========================================================================

    #[entity(name = "OreTreasury")]
    pub struct OreTreasury {
        pub id: TreasuryId,
        pub state: TreasuryState,

        #[snapshot(strategy = LastWrite)]
        pub treasury_snapshot: Option<ore_sdk::accounts::Treasury>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct TreasuryId {
        #[map(ore_sdk::accounts::Treasury::__account_address, primary_key, strategy = SetOnce)]
        pub address: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct TreasuryState {
        #[map(ore_sdk::accounts::Treasury::motherlode, strategy = LastWrite,
              transform = ui_amount(11))]
        pub motherlode: Option<f64>,

        #[map(ore_sdk::accounts::Treasury::total_refined, strategy = LastWrite,
              transform = ui_amount(11))]
        pub total_refined: Option<f64>,

        #[map(ore_sdk::accounts::Treasury::total_unclaimed, strategy = LastWrite,
              transform = ui_amount(11))]
        pub total_unclaimed: Option<f64>,
    }

    // ========================================================================
    // Miner Entity — Per-user mining state across all rounds
    // ========================================================================

    #[entity(name = "OreMiner")]
    pub struct OreMiner {
        pub id: MinerId,
        pub rewards: MinerRewards,
        pub state: MinerState,
        pub automation: MinerAutomation,

        #[snapshot(strategy = LastWrite, transforms = [(authority, Base58Encode)])]
        pub miner_snapshot: Option<ore_sdk::accounts::Miner>,

        #[snapshot(strategy = LastWrite, transforms = [(authority, Base58Encode), (executor, Base58Encode)])]
        pub automation_snapshot: Option<ore_sdk::accounts::Automation>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct MinerId {
        // Both Miner and Automation accounts share authority as the identity key
        #[map([ore_sdk::accounts::Miner::authority, ore_sdk::accounts::Automation::authority], primary_key, strategy = SetOnce, transform = Base58Encode)]
        pub authority: String,

        #[map(ore_sdk::accounts::Miner::__account_address, lookup_index, strategy = SetOnce)]
        pub miner_address: String,

        #[map(ore_sdk::accounts::Automation::__account_address, strategy = SetOnce)]
        pub automation_address: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct MinerRewards {
        #[map(ore_sdk::accounts::Miner::rewards_sol, strategy = LastWrite)]
        pub rewards_sol: Option<u64>,

        #[map(ore_sdk::accounts::Miner::rewards_ore, strategy = LastWrite)]
        pub rewards_ore: Option<u64>,

        #[map(ore_sdk::accounts::Miner::refined_ore, strategy = LastWrite)]
        pub refined_ore: Option<u64>,

        #[map(ore_sdk::accounts::Miner::lifetime_rewards_sol, strategy = LastWrite)]
        pub lifetime_rewards_sol: Option<u64>,

        #[map(ore_sdk::accounts::Miner::lifetime_rewards_ore, strategy = LastWrite)]
        pub lifetime_rewards_ore: Option<u64>,

        #[map(ore_sdk::accounts::Miner::lifetime_deployed, strategy = LastWrite)]
        pub lifetime_deployed: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct MinerState {
        #[map(ore_sdk::accounts::Miner::round_id, strategy = LastWrite)]
        pub round_id: Option<u64>,

        #[map(ore_sdk::accounts::Miner::deployed, strategy = LastWrite)]
        pub deployed_per_square: Option<Vec<u64>>,

        #[computed(state.deployed_per_square.map(|x| x.ui_amount(9)))]
        pub deployed_per_square_ui: Option<Vec<f64>>,

        #[computed(state.deployed_per_square.sum().ui_amount(9))]
        pub total_deployed: Option<f64>,

        #[map(ore_sdk::accounts::Miner::checkpoint_id, strategy = LastWrite)]
        pub checkpoint_id: Option<u64>,

        #[map(ore_sdk::accounts::Miner::checkpoint_fee, strategy = LastWrite)]
        pub checkpoint_fee: Option<u64>,

        #[map(ore_sdk::accounts::Miner::last_claim_ore_at, strategy = LastWrite)]
        pub last_claim_ore_at: Option<i64>,

        #[map(ore_sdk::accounts::Miner::last_claim_sol_at, strategy = LastWrite)]
        pub last_claim_sol_at: Option<i64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct MinerAutomation {
        #[map(ore_sdk::accounts::Automation::amount, strategy = LastWrite)]
        pub amount: Option<u64>,

        #[map(ore_sdk::accounts::Automation::balance, strategy = LastWrite)]
        pub balance: Option<u64>,

        #[map(ore_sdk::accounts::Automation::executor, strategy = LastWrite, transform = Base58Encode)]
        pub executor: Option<String>,

        #[map(ore_sdk::accounts::Automation::fee, strategy = LastWrite)]
        pub fee: Option<u64>,

        #[map(ore_sdk::accounts::Automation::strategy, strategy = LastWrite)]
        pub strategy: Option<u64>,

        #[map(ore_sdk::accounts::Automation::mask, strategy = LastWrite)]
        pub mask: Option<u64>,

        #[map(ore_sdk::accounts::Automation::reload, strategy = LastWrite)]
        pub reload: Option<u64>,
    }
}

#[cfg(test)]
mod tests {
    use super::ore_stream;
    use arete::runtime::{
        arete_interpreter::{record_slot_hash, vm::VmContext, UpdateContext},
        serde_json::{from_value, json},
    };

    #[test]
    fn cached_expiry_slot_hash_populates_computed_round_result() {
        let slot = 123_456_789;
        record_slot_hash(slot, "11111111111111111111111111111111".to_string());

        let mut state = json!({
            "id": { "round_id": "0" },
            "state": { "end_at": slot.to_string() },
            "entropy": {},
            "results": {},
        });

        ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(slot), 0).unwrap();

        let results: ore_stream::RoundResults = from_value(state["results"].clone()).unwrap();
        assert_eq!(results.expires_at_slot_hash.unwrap().bytes, vec![0_u8; 32]);
    }

    #[test]
    fn deployed_per_square_computes_total_deployed() {
        let mut state = json!({
            "id": { "round_id": "0" },
            "state": {
                "deployed_per_square": ["1000000000", "500000000"],
            },
            "entropy": {},
            "results": {},
        });

        ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(1), 0).unwrap();

        assert_eq!(
            state
                .pointer("/state/total_deployed")
                .and_then(|value| value.as_f64()),
            Some(1.5)
        );
        assert_eq!(
            state.pointer("/state/deployed_per_square_ui"),
            Some(&json!([1.0, 0.5]))
        );
    }

    #[test]
    fn miner_deployment_computes_ui_values() {
        let mut state = json!({
            "id": { "authority": "authority" },
            "state": {
                "round_id": "42",
                "deployed_per_square": [1000000000, 250000000],
            },
            "rewards": {},
            "automation": {},
        });

        ore_stream::ore_miner::evaluate_computed_fields(&mut state, Some(1), 0).unwrap();

        assert_eq!(
            state
                .pointer("/state/deployed_per_square_ui/0")
                .and_then(|value| value.as_f64()),
            Some(1.0)
        );
        assert_eq!(
            state
                .pointer("/state/deployed_per_square_ui/1")
                .and_then(|value| value.as_f64()),
            Some(0.25)
        );
        assert_eq!(
            state
                .pointer("/state/total_deployed")
                .and_then(|value| value.as_f64()),
            Some(1.25)
        );
    }

    #[test]
    fn round_expiry_computes_unix_timestamp_from_update_context() {
        let mut state = json!({
            "id": { "round_id": "0" },
            "state": { "end_at": "1150" },
            "entropy": {},
            "results": {},
        });

        ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(1000), 1_000).unwrap();

        assert_eq!(
            state
                .pointer("/state/estimated_expires_at_unix")
                .and_then(|value| value.as_i64()),
            Some(1_060)
        );
    }

    #[test]
    fn board_deadline_populates_active_round_countdown() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let mut context = UpdateContext::new_account(1_000, "board".to_string(), 1);
        context.timestamp = Some(10_000);

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": "11111111111111111111111111111111",
                "round_id": 42,
                "start_slot": "900",
                "end_slot": "1150",
                "production_cost_ema": "1000",
            }),
            "ore::BoardState",
            Some(&context),
            None,
        )
        .unwrap();

        let round = vm.get_entity_state(0, &json!(42)).unwrap();
        assert_eq!(round.pointer("/state/end_at"), Some(&json!("1150")));
        assert_eq!(round.pointer("/state/expires_at"), Some(&json!("1150")));
        assert_eq!(
            round
                .pointer("/state/estimated_expires_at_unix")
                .and_then(|value| value.as_i64()),
            Some(10_060)
        );
    }

    #[test]
    fn motherlode_odds_change_at_round_335000() {
        fn did_hit(round_id: u64, reversed_rng: u64) -> bool {
            let mut slot_hash_bytes = reversed_rng.reverse_bits().to_le_bytes().to_vec();
            slot_hash_bytes.extend([0_u8; 24]);
            let mut state = json!({
                "id": { "round_id": round_id },
                "state": {},
                "entropy": {},
                "results": { "slot_hash_bytes": slot_hash_bytes },
            });

            ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(1), 0).unwrap();
            ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(1), 0).unwrap();
            state
                .pointer("/results/did_hit_motherlode")
                .and_then(|value| value.as_bool())
                .unwrap()
        }

        assert!(did_hit(334_999, 625));
        assert!(!did_hit(335_000, 625));
        assert!(did_hit(335_000, 500));
    }

    #[test]
    fn board_account_updates_authoritative_round_state() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let board_address = "11111111111111111111111111111111";

        for (slot, round_id, start_slot, end_slot, production_cost_ema) in [
            (100, 334_999, 90, 240, 1_000),
            (241, 335_000, 242, u64::MAX, 1_100),
        ] {
            vm.process_event(
                &bytecode,
                json!({
                    "__account_address": board_address,
                    "round_id": round_id,
                    "start_slot": start_slot,
                    "end_slot": end_slot.to_string(),
                    "production_cost_ema": production_cost_ema,
                }),
                "ore::BoardState",
                Some(&UpdateContext::new_account(
                    slot,
                    format!("board-{round_id}"),
                    round_id,
                )),
                None,
            )
            .unwrap();
        }

        let board = vm.get_entity_state(1, &json!(board_address)).unwrap();
        assert_eq!(
            board
                .pointer("/id/address")
                .and_then(|value| value.as_str()),
            Some(board_address)
        );
        assert_eq!(
            board
                .pointer("/state/round_id")
                .and_then(|value| value.as_u64()),
            Some(335_000)
        );
        assert_eq!(
            board
                .pointer("/state/start_slot")
                .and_then(|value| value.as_u64()),
            Some(242)
        );
        assert_eq!(
            board
                .pointer("/state/end_slot")
                .and_then(|value| value.as_str()),
            Some("18446744073709551615")
        );
        assert_eq!(
            board
                .pointer("/state/production_cost_ema")
                .and_then(|value| value.as_u64()),
            Some(1_100)
        );
        assert!(board
            .pointer("/board_snapshot")
            .and_then(|value| value.as_object())
            .is_some());
    }

    #[test]
    fn treasury_account_keeps_its_embedded_address_key() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let treasury_address = "45db2FSR4mcXdSVVZbKbwojU6uYDpMyhpEi7cC8nHaWG";

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": treasury_address,
                "__resolved_primary_key": "round-address-from-shared-resolver",
                "motherlode": 4_140_000_000_000_u64,
                "total_refined": 0,
                "total_unclaimed": 0,
            }),
            "ore::TreasuryState",
            Some(&UpdateContext::new_account(
                100,
                "treasury-update".to_string(),
                1,
            )),
            None,
        )
        .unwrap();

        let treasury = vm.get_entity_state(2, &json!(treasury_address)).unwrap();
        assert_eq!(
            treasury
                .pointer("/id/address")
                .and_then(|value| value.as_str()),
            Some(treasury_address)
        );
        assert!(vm
            .get_entity_state(2, &json!("round-address-from-shared-resolver"))
            .is_none());
    }

    #[test]
    fn round_account_update_computes_total_deployed() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();

        let mutations = vm
            .process_event(
                &bytecode,
                json!({
                    "__account_address": "11111111111111111111111111111111",
                    "id": 42,
                    "deployed": ["1000000000", "500000000"],
                    "top_miner": vec![0_u8; 32],
                    "rent_payer": vec![0_u8; 32],
                    "slot_hash": vec![0_u8; 32],
                }),
                "ore::RoundState",
                Some(&UpdateContext::new_account(100, "round".to_string(), 1)),
                None,
            )
            .unwrap();

        assert_eq!(
            vm.get_entity_state(0, &json!(42)).and_then(|state| {
                state
                    .pointer("/state/total_deployed")
                    .and_then(|value| value.as_f64())
            }),
            Some(1.5)
        );
        let round_state = vm.get_entity_state(0, &json!(42)).unwrap();
        assert_eq!(
            round_state.pointer("/state/deployed_per_square_ui"),
            Some(&json!([1.0, 0.5]))
        );
        assert_eq!(
            mutations[0].patch.pointer("/state/deployed_per_square_ui"),
            Some(&json!([1.0, 0.5]))
        );
        assert_eq!(
            mutations[0].patch.pointer("/state/total_deployed"),
            Some(&json!(1.5))
        );
    }

    #[test]
    fn miner_account_update_computes_and_emits_deployment_ui_values() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();

        let mutations = vm
            .process_event(
                &bytecode,
                json!({
                    "__account_address": "11111111111111111111111111111111",
                    "authority": vec![0_u8; 32],
                    "round_id": 42,
                    "deployed": ["1000000000", "500000000"],
                }),
                "ore::MinerState",
                Some(&UpdateContext::new_account(100, "miner".to_string(), 1)),
                None,
            )
            .unwrap();

        let miner_state = vm
            .get_entity_state(3, &json!("11111111111111111111111111111111"))
            .unwrap();
        assert_eq!(
            miner_state.pointer("/state/deployed_per_square"),
            Some(&json!(["1000000000", "500000000"]))
        );
        assert_eq!(
            miner_state.pointer("/state/deployed_per_square_ui"),
            Some(&json!([1.0, 0.5]))
        );
        assert_eq!(
            miner_state.pointer("/state/total_deployed"),
            Some(&json!(1.5))
        );
        assert_eq!(
            mutations[0].patch.pointer("/state/deployed_per_square"),
            Some(&json!(["1000000000", "500000000"]))
        );
        assert_eq!(
            mutations[0].patch.pointer("/state/deployed_per_square_ui"),
            Some(&json!([1.0, 0.5]))
        );
        assert_eq!(
            mutations[0].patch.pointer("/state/total_deployed"),
            Some(&json!(1.5))
        );
    }

    #[test]
    fn string_samples_compute_pre_reveal_rng() {
        let slot = 123_456_790;
        record_slot_hash(slot, "11111111111111111111111111111111".to_string());

        let mut state = json!({
            "id": { "round_id": "0" },
            "state": { "end_at": slot.to_string() },
            "entropy": {
                "resolved_seed": vec![0_u8; 32],
                "entropy_samples": "1",
            },
            "results": {},
        });

        ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(slot), 0).unwrap();

        let rng = state
            .pointer("/results/pre_reveal_rng")
            .and_then(|value| value.as_u64())
            .unwrap();
        assert_eq!(
            state
                .pointer("/results/pre_reveal_winning_square")
                .and_then(|value| value.as_u64()),
            Some(rng % 25)
        );

        state["results"]["slot_hash_bytes"] = json!(vec![1_u8; 32]);
        state["entropy"]["entropy_samples"] = json!("2");
        ore_stream::ore_round::evaluate_computed_fields(&mut state, Some(slot), 0).unwrap();

        let final_rng = state
            .pointer("/results/rng")
            .and_then(|value| value.as_u64())
            .unwrap();
        assert_eq!(
            state
                .pointer("/results/pre_reveal_rng")
                .and_then(|value| value.as_u64()),
            Some(final_rng)
        );
        assert_eq!(
            state
                .pointer("/results/pre_reveal_winning_square")
                .and_then(|value| value.as_u64()),
            Some(final_rng % 25)
        );
    }

    #[test]
    fn entropy_update_waits_for_matching_board_end_slot() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let entropy_address = "SysvarRent111111111111111111111111111111111";

        let mutations = vm
            .process_event(
                &bytecode,
                json!({
                    "__account_address": entropy_address,
                    "start_at": "150",
                    "end_at": "200",
                    "samples": 1,
                    "value": vec![0_u8; 32],
                    "seed": vec![0_u8; 32],
                    "slot_hash": vec![0_u8; 32],
                }),
                "entropy::VarState",
                Some(&UpdateContext::new_account(100, "entropy".to_string(), 1)),
                None,
            )
            .unwrap();

        assert!(mutations.is_empty());
        assert!(vm.get_entity_state(0, &json!(42)).is_none());
        assert!(vm.take_scheduled_callbacks().is_empty());

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": "11111111111111111111111111111111",
                "round_id": 42,
                "start_slot": "150",
                "end_slot": "200",
                "production_cost_ema": "1000",
            }),
            "ore::BoardState",
            Some(&UpdateContext::new_account(101, "board".to_string(), 2)),
            None,
        )
        .unwrap();

        let round = vm.get_entity_state(0, &json!(42)).unwrap();
        assert_eq!(round.pointer("/state/end_at"), Some(&json!("200")));
        assert_eq!(round.pointer("/state/expires_at"), Some(&json!("200")));
        assert_eq!(
            round.pointer("/entropy/entropy_end_at"),
            Some(&json!("200"))
        );
        assert_eq!(round.pointer("/entropy/entropy_samples"), Some(&json!(1)));
        assert_eq!(
            round.pointer("/entropy/entropy_var_address"),
            Some(&json!(entropy_address))
        );

        let scheduled = vm.take_scheduled_callbacks();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].0, 200);
        assert_eq!(scheduled[0].1.primary_key, json!(42));
    }

    #[test]
    fn shared_entropy_account_routes_consecutive_rounds_by_end_slot() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let entropy_address = "SysvarRent111111111111111111111111111111111";

        for (slot, round_id, end_slot) in [(100, 41, 200), (300, 42, 400)] {
            vm.process_event(
                &bytecode,
                json!({
                    "__account_address": entropy_address,
                    "start_at": (end_slot - 50).to_string(),
                    "end_at": end_slot.to_string(),
                    "samples": round_id,
                    "value": vec![0_u8; 32],
                    "seed": vec![0_u8; 32],
                    "slot_hash": vec![0_u8; 32],
                }),
                "entropy::VarState",
                Some(&UpdateContext::new_account(
                    slot,
                    format!("entropy-{round_id}"),
                    round_id,
                )),
                None,
            )
            .unwrap();

            vm.process_event(
                &bytecode,
                json!({
                    "__account_address": "11111111111111111111111111111111",
                    "round_id": round_id,
                    "start_slot": (end_slot - 50).to_string(),
                    "end_slot": end_slot.to_string(),
                    "production_cost_ema": "1000",
                }),
                "ore::BoardState",
                Some(&UpdateContext::new_account(
                    slot + 1,
                    format!("board-{round_id}"),
                    round_id + 100,
                )),
                None,
            )
            .unwrap();

            let scheduled = vm.take_scheduled_callbacks();
            assert_eq!(scheduled.len(), 1);
            assert_eq!(scheduled[0].0, end_slot);
            assert_eq!(scheduled[0].1.primary_key, json!(round_id));
        }

        let previous_round = vm.get_entity_state(0, &json!(41)).unwrap();
        assert_eq!(
            previous_round.pointer("/state/expires_at"),
            Some(&json!("200"))
        );
        assert_eq!(
            previous_round.pointer("/entropy/entropy_samples"),
            Some(&json!(41))
        );

        let current_round = vm.get_entity_state(0, &json!(42)).unwrap();
        assert_eq!(
            current_round.pointer("/state/expires_at"),
            Some(&json!("400"))
        );
        assert_eq!(
            current_round.pointer("/entropy/entropy_samples"),
            Some(&json!(42))
        );
    }
}
