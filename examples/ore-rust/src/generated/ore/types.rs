use serde::{Deserialize, Serialize};
use arete_sdk::serde_utils;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundId {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub round_id: Option<u64>,
    #[serde(default)]
    pub round_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundState {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub expires_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub estimated_expires_at_unix: Option<Option<i64>>,
    #[serde(default)]
    pub motherlode: Option<Option<f64>>,
    #[serde(default)]
    pub total_deployed: Option<Option<f64>>,
    #[serde(default)]
    pub total_vaulted: Option<Option<f64>>,
    #[serde(default)]
    pub total_winnings: Option<Option<f64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub total_miners: Option<Option<u64>>,
    #[serde(default)]
    pub deployed_per_square: Option<Option<Vec<serde_json::Value>>>,
    #[serde(default)]
    pub deployed_per_square_ui: Option<Option<Vec<serde_json::Value>>>,
    #[serde(default)]
    pub count_per_square: Option<Option<Vec<serde_json::Value>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundResults {
    #[serde(default)]
    pub top_miner: Option<Option<String>>,
    #[serde(default)]
    pub top_miner_reward: Option<Option<f64>>,
    #[serde(default)]
    pub rent_payer: Option<Option<String>>,
    #[serde(default)]
    pub slot_hash: Option<Option<String>>,
    #[serde(default)]
    pub expires_at_slot_hash: Option<Option<Vec<serde_json::Value>>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub rng: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub winning_square: Option<Option<u64>>,
    #[serde(default)]
    pub did_hit_motherlode: Option<Option<bool>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub pre_reveal_rng: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub pre_reveal_winning_square: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundMetrics {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub deploy_count: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub checkpoint_count: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundTreasury {
    #[serde(default)]
    pub motherlode: Option<Option<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRoundEntropy {
    #[serde(default)]
    pub entropy_value: Option<Option<String>>,
    #[serde(default)]
    pub entropy_seed: Option<Option<String>>,
    #[serde(default)]
    pub entropy_slot_hash: Option<Option<String>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub entropy_start_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub entropy_end_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub entropy_samples: Option<Option<u64>>,
    #[serde(default)]
    pub entropy_var_address: Option<Option<String>>,
    #[serde(default)]
    pub resolved_seed: Option<Option<Vec<serde_json::Value>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreRound {
    #[serde(default)]
    pub id: OreRoundId,
    #[serde(default)]
    pub state: OreRoundState,
    #[serde(default)]
    pub results: OreRoundResults,
    #[serde(default)]
    pub metrics: OreRoundMetrics,
    #[serde(default)]
    pub treasury: OreRoundTreasury,
    #[serde(default)]
    pub entropy: OreRoundEntropy,
    #[serde(default)]
    pub ore_metadata: Option<Option<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreTreasuryId {
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreTreasuryState {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub balance: Option<Option<u64>>,
    #[serde(default)]
    pub motherlode: Option<Option<f64>>,
    #[serde(default)]
    pub total_refined: Option<Option<f64>>,
    #[serde(default)]
    pub total_staked: Option<Option<f64>>,
    #[serde(default)]
    pub total_unclaimed: Option<Option<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreTreasury {
    #[serde(default)]
    pub id: OreTreasuryId,
    #[serde(default)]
    pub state: OreTreasuryState,
    #[serde(default)]
    pub treasury_snapshot: Option<Option<serde_json::Value>>,
}



#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Treasury {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub balance: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub buffer_a: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub motherlode: Option<u64>,
    #[serde(default)]
    pub miner_rewards_factor: Option<serde_json::Value>,
    #[serde(default)]
    pub stake_rewards_factor: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub buffer_b: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub total_refined: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub total_staked: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub total_unclaimed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreMinerId {
    #[serde(default)]
    pub authority: Option<String>,
    #[serde(default)]
    pub miner_address: Option<String>,
    #[serde(default)]
    pub automation_address: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreMinerRewards {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub rewards_sol: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub rewards_ore: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub refined_ore: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub lifetime_rewards_sol: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub lifetime_rewards_ore: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub lifetime_deployed: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreMinerState {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub round_id: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub checkpoint_id: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub checkpoint_fee: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub last_claim_ore_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_i64")]
    pub last_claim_sol_at: Option<Option<i64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreMinerAutomation {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub amount: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub balance: Option<Option<u64>>,
    #[serde(default)]
    pub executor: Option<Option<String>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub fee: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub strategy: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub mask: Option<Option<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_option_u64")]
    pub reload: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OreMiner {
    #[serde(default)]
    pub id: OreMinerId,
    #[serde(default)]
    pub rewards: OreMinerRewards,
    #[serde(default)]
    pub state: OreMinerState,
    #[serde(default)]
    pub automation: OreMinerAutomation,
    #[serde(default)]
    pub miner_snapshot: Option<Option<serde_json::Value>>,
    #[serde(default)]
    pub automation_snapshot: Option<Option<serde_json::Value>>,
}



#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Miner {
    #[serde(default)]
    pub authority: Option<String>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_vec_u64")]
    pub deployed: Option<Vec<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_vec_u64")]
    pub cumulative: Option<Vec<u64>>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub checkpoint_fee: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub checkpoint_id: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_i64")]
    pub last_claim_ore_at: Option<i64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_i64")]
    pub last_claim_sol_at: Option<i64>,
    #[serde(default)]
    pub rewards_factor: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub rewards_sol: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub rewards_ore: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub refined_ore: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub round_id: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub lifetime_rewards_sol: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub lifetime_rewards_ore: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub lifetime_deployed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Automation {
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub amount: Option<u64>,
    #[serde(default)]
    pub authority: Option<String>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub balance: Option<u64>,
    #[serde(default)]
    pub executor: Option<String>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub fee: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub strategy: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub mask: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::deserialize_option_u64")]
    pub reload: Option<u64>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWrapper<T> {
    #[serde(default, deserialize_with = "serde_utils::deserialize_i64")]
    pub timestamp: i64,
    pub data: T,
    #[serde(default)]
    pub slot: Option<f64>,
    #[serde(default)]
    pub signature: Option<String>,
}

impl<T: Default> Default for EventWrapper<T> {
    fn default() -> Self {
        Self {
            timestamp: 0,
            data: T::default(),
            slot: None,
            signature: None,
        }
    }
}
