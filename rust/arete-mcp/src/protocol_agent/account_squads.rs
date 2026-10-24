//! Account Squads tools: the Squads v4 multisigs the calling Arete account
//! has attached (`<api>/account/squads`).
//!
//! A protocol's admin steps are proposed to a multisig named in its
//! deployment config, and a production deployment is refused unless that
//! multisig is an `active` attachment of the account: Arete's proposer is
//! an Initiate-only member. The dashboard page sets this up with any browser
//! wallet, hardware wallets included; the Squads app's own "add member" flow
//! works too. The prepared transactions here are for hot wallets, and are
//! never signed by this server: a member signs them and
//! [`ProtocolAgent::account_squads_submit`] sends and confirms them.
//!
//! Dev environments keep their own proposer (`a4 dev squads`) and never use
//! these routes.

use anyhow::{anyhow, Result};
use arete_sdk::protocol::{
    squads_dashboard_url, ProtocolError, ProtocolTransactionFormat, SquadsClient,
    SquadsClientOptions, SquadsCreateMember, SquadsCreateRequest, SQUADS_DASHBOARD_URL,
};
use arete_sdk::AuthConfig;
use serde_json::{json, Value};

use super::{invalid, squads_multisig, ProtocolAgent};
use crate::protocol_api::SquadsRefused;

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

/// Map an account Squads failure: argument problems are [`super::InvalidArgs`],
/// refusals carry their code and what to do ([`SquadsRefused`]).
fn squads_error(error: ProtocolError) -> anyhow::Error {
    match error {
        ProtocolError::Request {
            what,
            status,
            wire_code,
            body,
            message,
        } => anyhow!(SquadsRefused {
            what,
            status,
            code: wire_code.unwrap_or_default(),
            body: body.unwrap_or_else(|| json!({ "error": message })),
        }),
        ProtocolError::InvalidInput(message) => invalid(message),
        other => anyhow!(other),
    }
}

fn transaction_format(format: Option<&str>) -> Result<Option<ProtocolTransactionFormat>> {
    match format.map(str::trim) {
        None | Some("") => Ok(None),
        Some("v0") => Ok(Some(ProtocolTransactionFormat::V0)),
        Some("v1") => Ok(Some(ProtocolTransactionFormat::V1)),
        Some(other) => Err(invalid(format!(
            "`transaction_format` must be `v0` or `v1`, got `{other}`"
        ))),
    }
}

fn member_pubkey(name: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if super::is_pubkey(value) {
        Ok(value.to_string())
    } else {
        Err(invalid(format!(
            "`{name}` '{value}' is not a base58 public key"
        )))
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
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
                    "Remove the proposer in the Squads app (or with remove-proposer), then add it back with Initiate only. Manage it at {dashboard}."
                ),
                Some("squads_multisig_missing") | Some("squads_multisig_invalid") => format!(
                    "Check the address is the multisig account, not one of its vaults; forget this attachment if it is wrong ({dashboard})."
                ),
                Some("create_unconfirmed") => format!(
                    "The creator signs and submits the create transaction again, or prepares a new one at {dashboard}."
                ),
                _ => format!(
                    "Ask the user how the multisig's members sign. With a hardware or browser wallet, send them to {dashboard} (it builds and signs there), or have them add {} in the Squads app: Members → Add member → paste the proposer → permission Initiate only → approve and execute. Only for a hot-wallet keypair, prepare the add-proposer transaction here.",
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
            "No multisig is set up. Ask the user whether they already have a Squads multisig and how its members sign (hardware wallet, browser wallet or a hot-wallet keypair). Either way the easiest path is {dashboard}: it attaches an existing multisig or creates one, and signs with any browser wallet, Ledger included. An existing multisig can also be attached by address here (squads_attach) and the proposer added in the Squads app."
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
    /// The account Squads client for the hosted API and key.
    fn account_squads(&self) -> Result<SquadsClient> {
        let (origin, key) = self.hosted_origin()?;
        let key = key.ok_or_else(|| {
            anyhow!(
                "No Arete API key found in ARETE_API_KEY or the a4 login. Run `a4 auth login`, or set ARETE_API_KEY to a secret key."
            )
        })?;
        SquadsClient::new(
            SquadsClientOptions::default()
                .with_api_url(format!("{origin}/api"))
                .with_auth(AuthConfig::default().with_secret_key(key))
                .with_http_client(self.http.clone()),
        )
        .map_err(squads_error)
    }

    /// `squads_status`: the account's setup and attachments, or one
    /// attachment in detail with `multisig`.
    pub async fn account_squads_status(&self, multisig: Option<&str>) -> Result<Value> {
        let client = self.account_squads()?;
        match multisig.map(str::trim).filter(|m| !m.is_empty()) {
            None => {
                let status = client.status().await.map_err(squads_error)?;
                let mut body = to_value(&status);
                // Attachments archived after being active: their
                // deployments' proposals wait until the proposer is back.
                if let Ok(notifications) = client.notifications().await {
                    if !notifications.is_empty() {
                        body["notifications"] = to_value(&notifications);
                    }
                }
                body["next"] = json!(status_next(&body));
                Ok(body)
            }
            Some(multisig) => {
                let multisig = squads_multisig(multisig)?;
                let detail = client.get(&multisig).await.map_err(squads_error)?;
                let mut body = to_value(&detail);
                let proposer = Some(detail.proposer.clone()).filter(|p| !p.is_empty());
                if let Some(proposer) = &proposer {
                    if detail.attachment.state != arete_sdk::protocol::SquadsAttachmentState::Active
                    {
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
        let client = self.account_squads()?;
        let attachment = client
            .attach(&address, label.map(str::trim).filter(|l| !l.is_empty()))
            .await
            .map_err(squads_error)?;
        let mut body = to_value(&attachment);
        let proposer = client
            .status()
            .await
            .ok()
            .map(|status| status.proposer.pubkey);
        if let Some(proposer) = &proposer {
            body["proposer"] = json!(proposer);
            if attachment.state != arete_sdk::protocol::SquadsAttachmentState::Active {
                body["squadsAppSteps"] = json!(squads_app_steps(proposer));
            }
        }
        body["next"] = json!(attachment_next(&body, proposer.as_deref()));
        Ok(body)
    }

    /// `squads_add_proposer`: the unsigned config proposal adding the
    /// proposer (Initiate only), for `member` (who can initiate) to sign.
    /// The dashboard link and the Squads-app steps come first: they need no
    /// transaction from here.
    pub async fn account_squads_add_proposer(
        &self,
        multisig: &str,
        member: &str,
        format: Option<&str>,
    ) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        let member = member_pubkey("member", member)?;
        let format = transaction_format(format)?;
        let client = self.account_squads()?;
        let prepared = client
            .add_proposer(&multisig, &member, format)
            .await
            .map_err(squads_error)?;
        let proposer = client
            .status()
            .await
            .ok()
            .map(|status| status.proposer.pubkey);
        let mut body = to_value(&prepared);
        body["dashboardUrl"] = json!(squads_dashboard_url(Some(&multisig)));
        if let Some(proposer) = &proposer {
            body["proposer"] = json!(proposer);
            body["squadsAppSteps"] = json!(squads_app_steps(proposer));
        }
        body["next"] = json!(format!(
            "Prefer {} (any browser wallet, Ledger included) or the Squads-app steps; they need nothing from here. Only a hot wallet signs this transaction: {member} signs it and submits it with squads_submit (`transactionId`), or `a4 squads add-proposer {multisig} --member {member} --keypair <file>` does both. Then the members approve and execute config proposal {} in Squads (or on the dashboard), and squads_status reports the multisig `active`.",
            squads_dashboard_url(Some(&multisig)),
            prepared
                .transaction_index
                .map(|index| index.to_string())
                .unwrap_or_else(|| "?".into())
        ));
        Ok(body)
    }

    /// An unsigned `multisigCreateV2` with the proposer already an
    /// Initiate-only member, for `creator` to sign and pay.
    pub async fn account_squads_create(
        &self,
        members: Vec<(String, Vec<String>)>,
        threshold: u16,
        creator: &str,
        label: Option<&str>,
        format: Option<&str>,
    ) -> Result<Value> {
        if members.is_empty() {
            return Err(invalid("create needs at least one member"));
        }
        let creator = member_pubkey("creator", creator)?;
        let mut create_members = Vec::with_capacity(members.len());
        for (pubkey, permissions) in members {
            let pubkey = member_pubkey("member", &pubkey)?;
            for permission in &permissions {
                if !matches!(permission.as_str(), "initiate" | "vote" | "execute") {
                    return Err(invalid(format!(
                        "member permission `{permission}` must be `initiate`, `vote` or `execute`"
                    )));
                }
            }
            create_members.push(SquadsCreateMember::new(pubkey, permissions));
        }
        let request = SquadsCreateRequest {
            members: create_members,
            threshold,
            creator: creator.clone(),
            label: label
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string),
            transaction_format: transaction_format(format)?,
        };
        let client = self.account_squads()?;
        let prepared = client.create(&request).await.map_err(squads_error)?;
        let mut body = to_value(&prepared);
        body["dashboardUrl"] = json!(SQUADS_DASHBOARD_URL);
        body["next"] = json!(format!(
            "{creator} signs and pays for this transaction (creation fee and rent) and submits it (squads_submit). Arete's proposer is a member from the start, so the multisig {} becomes active once it confirms. With a hardware wallet, create it on {SQUADS_DASHBOARD_URL} instead.",
            prepared.multisig
        ));
        Ok(body)
    }

    /// An unsigned approval of config proposal `index`, for `member`.
    pub async fn account_squads_approve(
        &self,
        multisig: &str,
        index: u64,
        member: &str,
        format: Option<&str>,
    ) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        let member = member_pubkey("member", member)?;
        let prepared = self
            .account_squads()?
            .approve(&multisig, index, &member, transaction_format(format)?)
            .await
            .map_err(squads_error)?;
        Ok(to_value(&prepared))
    }

    /// An unsigned execution of approved config proposal `index`.
    pub async fn account_squads_execute(
        &self,
        multisig: &str,
        index: u64,
        member: &str,
        format: Option<&str>,
    ) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        let member = member_pubkey("member", member)?;
        let prepared = self
            .account_squads()?
            .execute(&multisig, index, &member, transaction_format(format)?)
            .await
            .map_err(squads_error)?;
        Ok(to_value(&prepared))
    }

    /// An unsigned config proposal removing the proposer.
    pub async fn account_squads_remove_proposer(
        &self,
        multisig: &str,
        member: &str,
        format: Option<&str>,
    ) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        let member = member_pubkey("member", member)?;
        let prepared = self
            .account_squads()?
            .remove_proposer(&multisig, &member, transaction_format(format)?)
            .await
            .map_err(squads_error)?;
        Ok(to_value(&prepared))
    }

    /// Forget a `pending` or `archived` attachment.
    pub async fn account_squads_forget(&self, multisig: &str) -> Result<Value> {
        let multisig = squads_multisig(multisig)?;
        self.account_squads()?
            .forget(&multisig)
            .await
            .map_err(squads_error)?;
        Ok(json!({ "multisig": multisig, "forgotten": true }))
    }

    /// `squads_submit`: send a prepared transaction a member signed, and
    /// wait for it to confirm.
    pub async fn account_squads_submit(
        &self,
        transaction_id: &str,
        signed_transaction: &[u8],
    ) -> Result<Value> {
        let result = self
            .account_squads()?
            .submit(transaction_id, signed_transaction)
            .await
            .map_err(squads_error)?;
        let mut body = to_value(&result);
        body["next"] = json!(match body["status"].as_str() {
            Some("confirmed") => match &body["multisig"] {
                Value::Null => "Confirmed.".to_string(),
                multisig => format!("Confirmed. {}", attachment_next(multisig, None)),
            },
            Some("pending") => "Sent, not confirmed yet. Submit the same transactionId and signed transaction again to re-check; it is never sent twice.".into(),
            Some("expired") => "The blockhash expired before it confirmed. Prepare a new transaction and sign it again.".into(),
            _ => "The transaction failed on chain; see `error`.".into(),
        });
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
        let raw = next.find("hot-wallet keypair").unwrap();
        assert!(dashboard < raw && app < raw, "{next}");
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
    fn refusals_keep_their_code() {
        let error = squads_error(ProtocolError::Request {
            what: "Squads forget".into(),
            status: 409,
            wire_code: Some("protocol-squads-multisig-active".into()),
            message: "active".into(),
            body: Some(json!({ "error": "active", "code": "protocol-squads-multisig-active" })),
        });
        let refused = error.downcast_ref::<SquadsRefused>().unwrap();
        assert_eq!(refused.normalized_code(), "protocol_squads_multisig_active");
        assert!(error.to_string().contains("Remove the proposer"), "{error}");
        assert!(squads_error(ProtocolError::InvalidInput("x".into()))
            .downcast_ref::<super::super::InvalidArgs>()
            .is_some());
    }
}
