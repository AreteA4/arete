use arete::prelude::*;

#[arete(idl = "ore.json")]
mod ingestion {
    #[entity(name = "Round")]
    struct Round {
        #[map(ore_sdk::accounts::Board::round_id, primary_key, strategy = SetOnce)]
        id: u64,
    }
}

fn main() {
    let spec = ingestion::spec();
    assert_eq!(spec.program_runtime_definitions.len(), 1);
    let shared = arete::runtime::shipstern_core::instruction::InstructionShared {
        transaction_config: Some(Default::default()),
        ..Default::default()
    };
    let context = arete::transaction_metadata::instruction_update_context(&shared);
    assert_eq!(
        context.get_metadata("solana_transaction"),
        Some(&arete::runtime::serde_json::json!({"version": 1, "config": {}}))
    );
    let unknown = arete::transaction_metadata::instruction_update_context(&Default::default());
    assert!(unknown.solana_transaction().unwrap().is_none());
}
