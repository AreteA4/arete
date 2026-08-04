mod generated;

use arete_sdk::instruction::BuiltInstruction;
use arete_sdk::prelude::*;
use generated::ore::programs::entropy as entropy_program;
use generated::ore::programs::ore as ore_program;
use generated::ore::{OreDevex, OreRound, OreStreamStack, OreTreasury};

// Use your own API key in production (can be secret or publishable)
const API_KEY: &str = "hspk_alt8MN3BmJebxARE3IlOnnaAEibCrqqXfdG5VoGW";

// Demo wallet for instruction building (no transaction is sent).
const AUTHORITY: &str = "HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T";

fn demo_deploy_params() -> ore_program::DeployParams {
    ore_program::DeployParams {
        amount: 1_000_000,
        squares: 3,
        signer: Some(AUTHORITY.to_string()),
        authority: AUTHORITY.to_string(),
        round: "11111111111111111111111111111111".to_string(),
        entropy_var: Some("11111111111111111111111111111111".to_string()),
        entropy_program: Some(entropy_program::PROGRAM_ID.to_string()),
    }
}

fn print_instruction(label: &str, instruction: &BuiltInstruction) {
    println!("Built `deploy` instruction ({label}):");
    println!("  Program: {}", instruction.program_id);
    println!("  Accounts: {}", instruction.accounts.len());
    println!("  Data length: {} bytes", instruction.data.len());
}

fn print_round(round: &OreRound) {
    println!("\n=== Round #{} ===", round.id.round_id.unwrap_or(0));
    println!("Address: {:?}", round.id.round_address);
    println!("Motherlode: {:?}", round.state.motherlode);
    println!("Total Deployed: {:?}", round.state.total_deployed);
    println!("Expires At: {:?}", round.state.expires_at);
    println!("Deploy Count: {:?}", round.metrics.deploy_count);
    println!();
}

fn print_treasury(treasury: &OreTreasury) {
    println!("\n=== Treasury ===");
    println!("Address: {:?}", treasury.id.address);
    println!("Motherlode: {:?}", treasury.state.motherlode);
    println!("Total Refined: {:?}", treasury.state.total_refined);
    println!("Total Unclaimed: {:?}", treasury.state.total_unclaimed);
    println!();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- Program SDK demo: instruction building is pure (no network) ---
    println!("--- Building an ore `deploy` instruction offline ---\n");
    let (miner_pda, bump) = ore_program::pdas::miner(AUTHORITY)?;
    println!("Derived miner PDA: {miner_pda} (bump {bump})");
    // Devex address helpers are pure too (staged extension, root re-export).
    println!(
        "Treasury PDA (via devex helper): {}",
        generated::ore::treasury_address()?
    );
    let instruction = ore_program::deploy(demo_deploy_params())?;
    print_instruction("standalone module", &instruction);
    println!();

    let a4 = Arete::<OreStreamStack>::builder()
        .api_key(API_KEY)
        .connect()
        .await?;

    // The same typed builders are exposed on the connected client.
    let instruction = a4.programs.ore.deploy(demo_deploy_params())?;
    print_instruction("via a4.programs.ore", &instruction);

    // --- Chain reads over the stack HTTP endpoint (best-effort) ---
    println!("\n--- Reading the cluster clock ---\n");
    match a4.chain().clock().await {
        Ok(clock) => println!("Cluster clock: slot {}", clock.slot),
        Err(error) => eprintln!("warning: chain clock read failed: {error}"),
    }

    // --- Release-addressed program account reads (best-effort) ---
    println!("\n--- Fetching the ore Board account ---\n");
    let (board_pda, _) = ore_program::pdas::board()?;
    match a4.programs.ore.board_accounts() {
        Ok(reader) => match reader.fetch(&board_pda.to_string()).await {
            Ok(Some(board)) => {
                println!("Board account at {board_pda}:");
                println!("  Round ID: {:?}", board.round_id);
                println!("  Start Slot: {:?}", board.start_slot);
                println!("  End Slot: {:?}", board.end_slot);
            }
            Ok(None) => println!("Board account not found at {board_pda}"),
            Err(error) => eprintln!("warning: board account read failed: {error}"),
        },
        Err(error) => eprintln!("warning: board account reader unavailable: {error}"),
    }

    // --- Devex extensions: the staged OreDevex trait attaches to the client
    // via the stack module's root re-export (best-effort). ---
    println!("\n--- Reading the current round via the OreDevex extension ---\n");
    match a4.current_round().await {
        Some(round) => println!(
            "Current round via OreDevex: #{} (deploy count {:?})",
            round.id.round_id.unwrap_or(0),
            round.metrics.deploy_count
        ),
        None => eprintln!("warning: current round unavailable (no board/round state yet)"),
    }

    println!("\n--- Preparing deployWithCheckpoint via the OreDevex extension ---\n");
    match a4
        .deploy_with_checkpoint(generated::ore::DeployWithCheckpointInput {
            authority: AUTHORITY.to_string(),
            amount: 1_000_000,
            squares: 3,
            ..Default::default()
        })
        .await
    {
        Ok(operation) => println!(
            "Prepared '{}' spanning {} transaction body(ies)",
            operation.name(),
            operation.plan().len()
        ),
        Err(error) => eprintln!("warning: deployWithCheckpoint preparation failed: {error}"),
    }

    println!("\n--- Streaming OreRound and OreTreasury updates ---\n");

    let round_view = a4.views.ore_round.latest();
    let treasury_view = a4.views.ore_treasury.list();

    let round_handle = tokio::spawn(async move {
        let mut stream = round_view.listen().take(1);
        while let Some(round) = stream.next().await {
            if round.id.round_id.is_some() {
                print_round(&round);
            }
        }
    });

    let treasury_handle = tokio::spawn(async move {
        let mut stream = treasury_view.listen().take(1);
        while let Some(treasury) = stream.next().await {
            if treasury.id.address.is_some() {
                print_treasury(&treasury);
            }
        }
    });

    let _ = tokio::join!(round_handle, treasury_handle);
    Ok(())
}
