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
        #[map(ore_sdk::accounts::Round::id, primary_key, strategy = SetOnce)]
        pub round_id: u64,

        #[map(ore_sdk::accounts::Round::__account_address, lookup_index, strategy = SetOnce)]
        pub round_address: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct RoundState {
        #[map(ore_sdk::accounts::Round::expires_at, strategy = LastWrite)]
        pub closes_at: Option<u64>,

        // Entropy deadline for the active round. This remains `expires_at` in the
        // public schema because existing clients use it for the round countdown.
        #[map(entropy_sdk::accounts::Var::end_at,
              lookup_index(register_from = [
                  (ore_sdk::instructions::Deploy, accounts::entropyVar, accounts::round),
                  (ore_sdk::instructions::Reset, accounts::entropyVar, accounts::round)
              ]),
              strategy = SetOnce)]
        pub expires_at: Option<u64>,

        #[computed({
            let expires_at_slot = state.expires_at.unwrap_or(0) as u64;
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
        #[computed(state.expires_at.slot_hash())]
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

        #[computed(results.rng.map(|r| r.reverse_bits() % 625 == 0))]
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
    // Entropy — Cross-program randomness state from the Entropy program
    // Linked to OreRound via Deploy/Reset instructions that reference both
    // accounts::round and accounts::entropyVar in the same transaction.
    // ========================================================================

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct EntropyState {
        #[map(entropy_sdk::accounts::Var::value,
              lookup_index(register_from = [
                  (ore_sdk::instructions::Deploy, accounts::entropyVar, accounts::round),
                  (ore_sdk::instructions::Reset, accounts::entropyVar, accounts::round)
              ]),
              when = entropy_sdk::instructions::Reveal,
              condition = "value != ZERO_32",
              strategy = LastWrite,
              transform = Base58Encode)]
        pub entropy_value: Option<String>,

        #[map(entropy_sdk::accounts::Var::seed, strategy = LastWrite, transform = Base58Encode)]
        pub entropy_seed: Option<String>,

        #[map(entropy_sdk::accounts::Var::slot_hash, strategy = LastWrite, transform = Base58Encode)]
        pub entropy_slot_hash: Option<String>,

        #[map(entropy_sdk::accounts::Var::start_at, strategy = LastWrite)]
        pub entropy_start_at: Option<u64>,

        #[map(entropy_sdk::accounts::Var::end_at, strategy = LastWrite)]
        pub entropy_end_at: Option<u64>,

        #[map(entropy_sdk::accounts::Var::samples, strategy = LastWrite)]
        pub entropy_samples: Option<u64>,

        #[map(entropy_sdk::accounts::Var::__account_address, strategy = SetOnce)]
        pub entropy_var_address: Option<String>,

        #[resolve(
            url = "https://entropy-api.onrender.com/var/{entropy.entropy_var_address}/seed?samples={entropy.entropy_samples}",
            extract = "seed",
            schedule_at = state.expires_at,
            condition = "entropy.entropy_value == null",
            strategy = SetOnce
        )]
        pub resolved_seed: Option<Vec<u8>>,
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
            "state": { "expires_at": slot.to_string() },
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
    }

    #[test]
    fn round_account_update_computes_total_deployed() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();

        vm.process_event(
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
    }

    #[test]
    fn string_samples_compute_pre_reveal_rng() {
        let slot = 123_456_790;
        record_slot_hash(slot, "11111111111111111111111111111111".to_string());

        let mut state = json!({
            "state": { "expires_at": slot.to_string() },
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
    fn queued_entropy_update_schedules_seed_resolution_after_deploy_mapping() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let round_address = "11111111111111111111111111111111";
        let entropy_address = "SysvarRent111111111111111111111111111111111";

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": round_address,
                "id": 42,
                "top_miner": vec![0_u8; 32],
                "rent_payer": vec![0_u8; 32],
                "slot_hash": vec![0_u8; 32],
            }),
            "ore::RoundState",
            Some(&UpdateContext::new_account(100, "round".to_string(), 1)),
            None,
        )
        .unwrap();
        let _ = vm.take_resolver_requests();

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": entropy_address,
                "end_at": "200",
                "samples": 1,
                "value": vec![0_u8; 32],
                "seed": vec![0_u8; 32],
                "slot_hash": vec![0_u8; 32],
            }),
            "entropy::VarState",
            Some(&UpdateContext::new_account(101, "entropy".to_string(), 2)),
            None,
        )
        .unwrap();

        vm.process_event(
            &bytecode,
            json!({
                "accounts": {
                    "round": round_address,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::DeployIxState",
            Some(&UpdateContext::new_instruction(
                102,
                "deploy".to_string(),
                1,
            )),
            None,
        )
        .unwrap();

        let scheduled = vm.take_scheduled_callbacks();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].0, 200);
        assert_eq!(scheduled[0].1.primary_key, json!(42));
    }

    #[test]
    fn remapped_entropy_update_can_schedule_future_seed_resolution() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let round_address = "11111111111111111111111111111111";

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": round_address,
                "id": 42,
                "top_miner": vec![0_u8; 32],
                "rent_payer": vec![0_u8; 32],
                "slot_hash": vec![0_u8; 32],
            }),
            "ore::RoundState",
            Some(&UpdateContext::new_account(100, "round".to_string(), 1)),
            None,
        )
        .unwrap();
        let _ = vm.take_resolver_requests();

        vm.process_event(
            &bytecode,
            json!({
                "__account_address": "SysvarRent111111111111111111111111111111111",
                "__resolved_primary_key": round_address,
                "end_at": "200",
                "samples": 1,
                "value": vec![0_u8; 32],
                "seed": vec![0_u8; 32],
                "slot_hash": vec![0_u8; 32],
            }),
            "entropy::VarState",
            Some(&UpdateContext::new_reprocessed(101, 2)),
            None,
        )
        .unwrap();

        let scheduled = vm.take_scheduled_callbacks();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].0, 200);
        assert_eq!(scheduled[0].1.primary_key, json!(42));
        assert!(vm.take_resolver_requests().is_empty());
    }

    #[test]
    fn reset_keeps_current_deadline_until_next_round_deploy() {
        let bytecode = ore_stream::create_multi_entity_bytecode();
        let mut vm = VmContext::new();
        let current_round = "11111111111111111111111111111111";
        let next_round = "SysvarC1ock11111111111111111111111111111111";
        let following_round = "SysvarS1otHashes111111111111111111111111111";
        let entropy_address = "SysvarRent111111111111111111111111111111111";

        for (slot, id, address) in [
            (100, 41, current_round),
            (101, 42, next_round),
            (102, 43, following_round),
        ] {
            vm.process_event(
                &bytecode,
                json!({
                    "__account_address": address,
                    "id": id,
                    "top_miner": vec![0_u8; 32],
                    "rent_payer": vec![0_u8; 32],
                    "slot_hash": vec![0_u8; 32],
                }),
                "ore::RoundState",
                Some(&UpdateContext::new_account(slot, format!("round-{id}"), id)),
                None,
            )
            .unwrap();
        }

        let entropy_update = |end_at: u64| {
            json!({
                "__account_address": entropy_address,
                "end_at": end_at.to_string(),
                "samples": 1,
                "value": vec![0_u8; 32],
                "seed": vec![0_u8; 32],
                "slot_hash": vec![0_u8; 32],
            })
        };

        vm.process_event(
            &bytecode,
            entropy_update(200),
            "entropy::VarState",
            Some(&UpdateContext::new_account(103, "entropy-1".to_string(), 1)),
            None,
        )
        .unwrap();

        vm.process_event(
            &bytecode,
            json!({
                "accounts": {
                    "round": current_round,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::DeployIxState",
            Some(&UpdateContext::new_instruction(
                104,
                "deploy-1".to_string(),
                1,
            )),
            None,
        )
        .unwrap();
        let _ = vm.take_scheduled_callbacks();

        vm.process_event(
            &bytecode,
            json!({
                "__resolved_primary_key": 41,
                "accounts": {
                    "round": current_round,
                    "roundNext": next_round,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::ResetIxState",
            Some(&UpdateContext::new_instruction(105, "reset".to_string(), 2)),
            None,
        )
        .unwrap();
        assert_eq!(
            vm.get_entity_state(0, &json!(41))
                .and_then(|state| state.pointer("/state/expires_at").cloned()),
            Some(json!("200"))
        );
        let _ = vm.take_scheduled_callbacks();
        assert!(vm
            .get_entity_state(0, &json!(42))
            .and_then(|state| state.pointer("/state/expires_at").cloned())
            .is_none());

        vm.process_event(
            &bytecode,
            entropy_update(300),
            "entropy::VarState",
            Some(&UpdateContext::new_account(106, "entropy-2".to_string(), 2)),
            None,
        )
        .unwrap();
        assert_eq!(
            vm.get_entity_state(0, &json!(41))
                .and_then(|state| state.pointer("/state/expires_at").cloned()),
            Some(json!("200"))
        );
        let _ = vm.take_scheduled_callbacks();

        vm.process_event(
            &bytecode,
            json!({
                "accounts": {
                    "round": next_round,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::DeployIxState",
            Some(&UpdateContext::new_instruction(
                107,
                "deploy-2".to_string(),
                3,
            )),
            None,
        )
        .unwrap();

        assert!(vm
            .get_entity_state(0, &json!(42))
            .and_then(|state| state.pointer("/state/expires_at").cloned())
            .is_none());

        vm.process_event(
            &bytecode,
            entropy_update(300),
            "entropy::VarState",
            Some(&UpdateContext::new_account(
                108,
                "entropy-2b".to_string(),
                3,
            )),
            None,
        )
        .unwrap();

        assert_eq!(
            vm.get_entity_state(0, &json!(42))
                .and_then(|state| state.pointer("/state/expires_at").cloned()),
            Some(json!("300"))
        );
        let scheduled = vm.take_scheduled_callbacks();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].0, 300);
        assert_eq!(scheduled[0].1.primary_key, json!(42));
        assert!(vm
            .get_entity_state(0, &json!(43))
            .and_then(|state| state.pointer("/state/expires_at").cloned())
            .is_none());

        vm.process_event(
            &bytecode,
            json!({
                "__resolved_primary_key": 42,
                "accounts": {
                    "round": next_round,
                    "roundNext": following_round,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::ResetIxState",
            Some(&UpdateContext::new_instruction(
                109,
                "reset-2".to_string(),
                4,
            )),
            None,
        )
        .unwrap();

        vm.process_event(
            &bytecode,
            entropy_update(400),
            "entropy::VarState",
            Some(&UpdateContext::new_account(110, "entropy-3".to_string(), 4)),
            None,
        )
        .unwrap();

        assert_eq!(
            vm.get_entity_state(0, &json!(42))
                .and_then(|state| state.pointer("/state/expires_at").cloned()),
            Some(json!("300"))
        );
        let _ = vm.take_scheduled_callbacks();

        vm.process_event(
            &bytecode,
            json!({
                "accounts": {
                    "round": following_round,
                    "entropyVar": entropy_address,
                },
                "data": {},
            }),
            "ore::DeployIxState",
            Some(&UpdateContext::new_instruction(
                111,
                "deploy-3".to_string(),
                5,
            )),
            None,
        )
        .unwrap();

        vm.process_event(
            &bytecode,
            entropy_update(400),
            "entropy::VarState",
            Some(&UpdateContext::new_account(
                112,
                "entropy-3b".to_string(),
                5,
            )),
            None,
        )
        .unwrap();

        assert_eq!(
            vm.get_entity_state(0, &json!(43))
                .and_then(|state| state.pointer("/state/expires_at").cloned()),
            Some(json!("400"))
        );
    }
}
