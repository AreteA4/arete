mod support;

use support::{arete_dir, cargo_toml, escape_path, macro_manifest_dir, workspace_root, TempCrate};

/// The published PumpSwap IDL (`pump_amm`, `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`) from
/// <https://github.com/pump-fun/pump-public-docs/blob/e0687ae9b7e064a0f54efc7297c65eecfbba3a8f/idl/pump_amm.json>,
/// cut down to `buy`, `sell`, `BuyEvent`, `SellEvent` and the types they use. Kept entries are
/// verbatim, so account order, discriminators and payload layouts are the real program's.
const PUMP_AMM_IDL: &str = include_str!("fixtures/pump_amm.json");

#[test]
fn pumpswap_trade_events_carry_their_emitting_instruction_accounts() {
    let temp = TempCrate::new(
        "cpi-event-accounts",
        "pumpswap-trades",
        cargo_toml(
            "pumpswap-trades",
            &[
                format!("arete = {{ path = \"{}\" }}", escape_path(&arete_dir())),
                format!(
                    "arete-artifacts = {{ path = \"{}\" }}",
                    escape_path(&workspace_root().join("arete-artifacts"))
                ),
                "serde = { version = \"1\", features = [\"derive\"] }".into(),
                "serde_json = \"1\"".into(),
                "borsh = { version = \"1.5\", features = [\"derive\"] }".into(),
                "solana-pubkey = { version = \"2.2\", features = [\"serde\", \"borsh\"] }".into(),
                "tracing-subscriber = { version = \"0.3\", features = [\"env-filter\"] }".into(),
            ],
        ),
        include_str!("fixtures/cpi-event-accounts.rs"),
        &[("idl/pump_amm.json", PUMP_AMM_IDL)],
    );
    let output = temp.cargo_run();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "generated runtime failed:\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("pumpswap trades verified"), "{stdout}");
    // The event under the unhandled parent says why its accounts are missing.
    assert!(
        stderr.lines().any(
            |line| line.contains("CPI event has no handled emitting instruction")
                && line.contains("ix_path=1.0")
        ),
        "{stderr}"
    );
}

/// `accounts::` on an event is checked against the IDL's instruction accounts,
/// at every selector site, and cannot collide with a payload field in a capture.
#[test]
fn event_account_selectors_are_validated() {
    let source = r#"use arete_macros::arete;

#[arete(idl = "idl/pump_amm.json")]
mod invalid {
    #[entity(name = "Pool")]
    struct Pool {
        #[map(pump_amm_sdk::events::BuyEvent::pool, primary_key, strategy = SetOnce)]
        pool: String,

        #[map(pump_amm_sdk::events::BuyEvent::accounts::base_mnt, strategy = LastWrite)]
        base_mint: Option<String>,

        #[aggregate(from = pump_amm_sdk::events::SellEvent, field = accounts::usr,
                    strategy = UniqueCount, lookup_by = pool)]
        traders: Option<u64>,

        #[aggregate(from = pump_amm_sdk::events::SellEvent, field = base_amount_in, strategy = Sum,
                    condition = "accounts.qoute_mint != 0", lookup_by = pool)]
        volume: Option<u64>,

        #[event(from = pump_amm_sdk::events::BuyEvent, fields = [pool, accounts::pool],
                lookup_by = pool)]
        last_buy: Option<serde_json::Value>,
    }
}

fn main() {}
"#;
    let temp = TempCrate::new(
        "cpi-event-accounts",
        "invalid-selectors",
        cargo_toml(
            "invalid-selectors",
            &[format!(
                "arete-macros = {{ path = \"{}\" }}",
                escape_path(&macro_manifest_dir())
            )],
        ),
        source,
        &[("idl/pump_amm.json", PUMP_AMM_IDL)],
    );
    let output = temp.cargo_check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    for expected in [
        "Not found: 'base_mnt' in instruction accounts that can emit event 'BuyEvent'",
        "Not found: 'usr' in instruction accounts that can emit event 'SellEvent'",
        "Not found: 'qoute_mint' in instruction accounts that can emit event 'SellEvent'",
        "`pool` is captured twice",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in:\n{stderr}"
        );
    }
    assert!(!stderr.contains("is not valid for event"), "{stderr}");
}
