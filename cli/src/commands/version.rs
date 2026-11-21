//! `a4 version`: this CLI's version and the build of the Arete API it talks
//! to, so a report names both.

use anyhow::Result;
use arete_mcp::api_build::{api_line, api_report};
use colored::Colorize;
use serde_json::json;

use crate::api_client::ApiClient;

pub fn version(json: bool) -> Result<()> {
    let api_url = ApiClient::configured_api_url();
    let build = ApiClient::api_build(&api_url);
    if json {
        let report = json!({
            "cli": { "name": "a4", "version": env!("CARGO_PKG_VERSION") },
            "api": api_report(&api_url, &build),
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{} {}", "a4".bold(), env!("CARGO_PKG_VERSION"));
        println!("{} {}", "API".bold(), api_line(&api_url, &build));
    }
    Ok(())
}
