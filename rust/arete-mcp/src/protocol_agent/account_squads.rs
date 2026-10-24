//! Account Squads tools: the Squads v4 multisigs the calling Arete account
//! has attached (`<api>/account/squads`).
//!
//! A protocol's admin steps are proposed to a multisig named in its
//! deployment config, and a production deployment is refused unless that
//! multisig is an `active` attachment of the account: Arete's proposer is
//! an Initiate-only member. The dashboard page sets this up with any browser
//! wallet, hardware wallets included; the Squads app's own "add member" flow
//! works too. Arete builds no Squads transactions: code that creates a
//! multisig or adds the proposer uses the squads-v4 program SDK
//! (`a4 install program squads-v4`) and its Arete setup helpers.
//!
//! These calls only read the account's setup and attach a multisig by
//! address. Dev environments keep their own proposer (`a4 dev squads`) and
//! never use these routes.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use super::{squads_multisig, ProtocolAgent};
use crate::protocol_api::{path_segment, SquadsRefused};

/// The dashboard's Squads page.
pub const SQUADS_DASHBOARD_URL: &str = "https://app.arete.run/dashboard/squads";

/// The dashboard page for one multisig, or the whole account.
pub fn squads_dashboard_url(multisig: Option<&str>) -> String {
    match multisig.map(str::trim).filter(|m| !m.is_empty()) {
        Some(multisig) => format!("{SQUADS_DASHBOARD_URL}/{multisig}"),
        None => SQUADS_DASHBOARD_URL.to_string(),
    }
}

/// Where code that creates a multisig or adds the proposer goes.
pub const SQUADS_PROGRAM_SDK_HINT: &str = "For code or a hot-wallet script, install the squads-v4 program SDK (`a4 install program squads-v4`) and use its Arete setup helpers; the member's wallet signs.";

/// The Squads app steps that add the proposer without any transaction from
/// Arete: what teams whose members sign with hardware wallets can follow.
pub fn squads_app_steps(proposer: &str) -> Vec<String> {
    vec![
        "Open the multisig in the Squads app (app.squads.so).".into(),
        "Members → Add member.".into(),
        format!("Paste Arete's proposer: {proposer}."),
        "Permission: Initiate only (not Vote, not Execute); it does not count toward the threshold.".into(),
        "Create the proposal, then approve and execute it with enough members.".into(),
    ]
}

/// What a problem means, in words.
fn problem_text(problem: Option<&str>) -> &'static str {
    match problem {
        Some("not_member") => "Arete's proposer is not a member yet",
        Some("permissions_invalid") => {
            "Arete's proposer is a member with permissions other than Initiate only"
        }
        Some("squads_multisig_missing") => "no Squads v4 multisig exists at this address",
        Some("squads_multisig_invalid") => "this account is not a Squads v4 multisig",
        Some("create_unconfirmed") => "its create transaction has not confirmed",
        _ => "it is not active",
    }
}

/// What to do about one attachment, phrased for an agent.
pub fn attachment_next(attachment: &Value, proposer: Option<&str>) -> String {
    let address = attachment["address"].as_str().unwrap_or("<multisig>");
    let dashboard = attachment["dashboardUrl"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| squads_dashboard_url(Some(address)));
    match attachment["state"].as_str() {
        Some("active") => format!(
            "Active: Arete's proposer can propose to {address}. Declare the multisig as a resource of the protocol and put this address in its deployment config (`deploymentConfigSnippet`)."
        ),
        _ => {
            let problem = attachment["problem"].as_str();
            let why = attachment["problemDescription"]
                .as_str()
                .map(|text| text.trim_end_matches('.').to_string())
                .unwrap_or_else(|| problem_text(problem).to_string());
            let fix = match problem {
                Some("permissions_invalid") => format!(
                    "Remove the proposer in the Squads app, then add it back with Initiate only. Manage it at {dashboard}."
                ),
                Some("squads_multisig_missing") | Some("squads_multisig_invalid") => format!(
                    "Check the address is the multisig account, not one of its vaults; forget this attachment on {dashboard} if it is wrong."
                ),
                Some("create_unconfirmed") => format!(
                    "The creator signs and sends the create transaction again, or creates the multisig at {dashboard}."
                ),
                _ => format!(
                    "Ask the user how the multisig's members sign. Send them to {dashboard} (any browser wallet, Ledger included), or have them add {} in the Squads app: Members → Add member → paste the proposer → permission Initiate only → approve and execute. {SQUADS_PROGRAM_SDK_HINT}",
                    proposer.unwrap_or("Arete's proposer")
                ),
            };
            format!(
                "{} ({address}): {why}. {fix}",
                attachment["state"].as_str().unwrap_or("pending")
            )
        }
    }
}

/// What to do after an account status read.
pub fn status_next(status: &Value) -> String {
    let dashboard = status["dashboardUrl"]
        .as_str()
        .unwrap_or(SQUADS_DASHBOARD_URL);
    let proposer = status["proposer"]["pubkey"].as_str();
    match status["setup"].as_str() {
        Some("notStarted") => format!(
            "No multisig is set up. Ask the user whether they already have a Squads multisig and how its members sign (hardware wallet, browser wallet or a hot-wallet keypair). The easiest path is {dashboard}: it attaches an existing multisig or creates one, and signs with any browser wallet, Ledger included. An existing multisig can also be attached by address here (squads_attach) and the proposer added in the Squads app. {SQUADS_PROGRAM_SDK_HINT}"
        ),
        Some("ready") => "Ready. Declare the multisig as a resource of each protocol with Squads steps and put an active multisig's address in its deployment config (`deploymentConfigSnippet`).".into(),
        _ => {
            let pending: Vec<String> = status["multisigs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|attachment| attachment_next(attachment, proposer))
                .collect();
            if pending.is_empty() {
                format!("No multisig is active. Open {dashboard}.")
            } else {
                pending.join(" ")
            }
        }
    }
}

impl ProtocolAgent {
    /// One call to `<api>/account/squads<route>` with the account's secret
    /// key; a non-2xx answer is a [`SquadsRefused`].
    async fn account_squads_call(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
        what: &str,
    ) -> Result<Value> {
        let (origin, key) = self.hosted_origin()?;
        let key = key.ok_or_else(|| {
            anyhow!(
                "No Arete API key found in ARETE_API_KEY or the a4 login. Run `a4 auth login`, or set ARETE_API_KEY to a secret key."
            )
        })?;
        let url = format!("{origin}/api/account/squads{route}");
        let mut request = self.http.request(method, &url).bearer_auth(&key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("The Arete API is unreachable at {url}"))?;
        let status = response.status();
        let header_code = response
            .headers()
            .get("x-error-code")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = response.text().await.unwrap_or_default();
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        if !status.is_success() {
            let code = header_code
                .filter(|code| !code.trim().is_empty())
                .or_else(|| body.get("code").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            return Err(anyhow!(SquadsRefused {
                what: what.to_string(),
                status: status.as_u16(),
                code,
                body,
            }));
        }
        Ok(body)
    }

    /// `squads_status`: the account's setup and attachments, or one
    /// attachment in detail with `multisig`.
    pub async fn account_squads_status(&self, multisig: Option<&str>) -> Result<Value> {
        match multisig.map(str::trim).filter(|m| !m.is_empty()) {
            None => {
                let mut body = self
                    .account_squads_call(
                        reqwest::Method::GET,
                        "",
                        None,
                        "Reading the account's Squads setup",
                    )
                    .await?;
                // Attachments archived after being active: their
                // deployments' proposals wait until the proposer is back.
                if let Ok(Value::Array(notifications)) = self
                    .account_squads_call(
                        reqwest::Method::GET,
                        "/notifications",
                        None,
                        "Reading the account's Squads notifications",
                    )
                    .await
                {
                    if !notifications.is_empty() {
                        body["notifications"] = Value::Array(notifications);
                    }
                }
                body["next"] = json!(status_next(&body));
                Ok(body)
            }
            Some(multisig) => {
                let multisig = squads_multisig(multisig)?;
                let mut body = self
                    .account_squads_call(
                        reqwest::Method::GET,
                        &format!("/multisigs/{}", path_segment(&multisig)?),
                        None,
                        "Reading the Squads multisig",
                    )
                    .await?;
                let proposer = body["proposer"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);
                if let Some(proposer) = &proposer {
                    if body["state"] != "active" {
                        body["squadsAppSteps"] = json!(squads_app_steps(proposer));
                    }
                }
                body["next"] = json!(attachment_next(&body, proposer.as_deref()));
                Ok(body)
            }
        }
    }

    /// `squads_attach`: attach an existing multisig by address.
    pub async fn account_squads_attach(&self, address: &str, label: Option<&str>) -> Result<Value> {
        let address = squads_multisig(address)?;
        let mut request = json!({ "address": address });
        if let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) {
            request["label"] = json!(label);
        }
        let mut body = self
            .account_squads_call(
                reqwest::Method::POST,
                "/multisigs",
                Some(request),
                "Attaching the Squads multisig",
            )
            .await?;
        let proposer = self
            .account_squads_call(
                reqwest::Method::GET,
                "",
                None,
                "Reading the account's Squads setup",
            )
            .await
            .ok()
            .and_then(|status| status["proposer"]["pubkey"].as_str().map(str::to_string));
        if let Some(proposer) = &proposer {
            body["proposer"] = json!(proposer);
            if body["state"] != "active" {
                body["squadsAppSteps"] = json!(squads_app_steps(proposer));
            }
        }
        body["next"] = json!(attachment_next(&body, proposer.as_deref()));
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_steps_lead_with_the_dashboard_and_the_squads_app() {
        let pending = json!({
            "address": "Ms1", "state": "pending", "problem": "not_member",
            "dashboardUrl": "https://app.arete.run/dashboard/squads/Ms1"
        });
        let next = attachment_next(&pending, Some("Prop1"));
        let dashboard = next
            .find("https://app.arete.run/dashboard/squads/Ms1")
            .unwrap();
        let app = next.find("Members → Add member").unwrap();
        let sdk = next.find("squads-v4 program SDK").unwrap();
        assert!(dashboard < app && app < sdk, "{next}");
        assert!(next.contains("Initiate only"));

        let active = json!({ "address": "Ms1", "state": "active" });
        assert!(attachment_next(&active, None).contains("deployment config"));

        let none = json!({ "setup": "notStarted", "dashboardUrl": SQUADS_DASHBOARD_URL });
        let next = status_next(&none);
        assert!(next.contains("Ask the user whether they already have a Squads multisig"));
        assert!(next.contains(SQUADS_DASHBOARD_URL));
    }

    #[test]
    fn squads_app_steps_give_initiate_only() {
        let steps = squads_app_steps("Prop1").join(" ");
        assert!(steps.contains("Prop1"));
        assert!(steps.contains("Initiate only"));
        assert!(steps.contains("approve and execute"));
    }

    #[test]
    fn dashboard_urls() {
        assert_eq!(squads_dashboard_url(None), SQUADS_DASHBOARD_URL);
        assert_eq!(
            squads_dashboard_url(Some("Ms1")),
            "https://app.arete.run/dashboard/squads/Ms1"
        );
    }
}
