use arete_sdk::prelude::*;
use arete_stacks::ore::{OreRound, OreStreamStack, OreTreasury};

// Use your own API key in production (can be secret or publishable)
const API_KEY: &str = "hspk_alt8MN3BmJebxARE3IlOnnaAEibCrqqXfdG5VoGW";

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
    println!("Balance: {:?}", treasury.state.balance);
    println!("Motherlode: {:?}", treasury.state.motherlode);
    println!("Total Refined: {:?}", treasury.state.total_refined);
    println!("Total Staked: {:?}", treasury.state.total_staked);
    println!("Total Unclaimed: {:?}", treasury.state.total_unclaimed);
    println!();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a4 = Arete::<OreStreamStack>::builder()
        .api_key(API_KEY)
        .connect()
        .await?;

    println!("--- Streaming OreRound and OreTreasury updates ---\n");

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
