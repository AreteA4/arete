//! `a4 protocol webhook add|list|remove|test`: the webhook endpoints that
//! receive a deployment's emitted events.
//!
//! Every subcommand needs the owner's secret key and finds the deployment
//! the way `request-listing` does (`--deployment`, else the project's).
//! `add` subscribes an https endpoint to the events the project's build
//! declares (or `--events`) and prints the endpoint's signing secret, which
//! the server returns only then: store it where the endpoint runs as
//! `ARETE_WEBHOOK_SECRET`. `--secret-only` prints just the secret, to pipe
//! it into a host's secret store.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;

use super::listing::{require_secret_key, resolve_deployment, ListingArgs};
use crate::api_client::{ApiClient, ProtocolWebhook, ProtocolWebhookTest};

#[derive(Subcommand, Debug)]
pub enum WebhookCommands {
    /// Subscribe an https endpoint to the deployment's events and print its
    /// signing secret (shown only once)
    Add(AddArgs),
    /// List the deployment's webhook endpoints
    List(ListingArgs),
    /// Disable a webhook endpoint
    Remove(EndpointArgs),
    /// Send a signed test delivery to an endpoint now
    Test(EndpointArgs),
}

/// Options for `a4 protocol webhook add`.
#[derive(Args, Debug, Clone, Default)]
pub struct AddArgs {
    /// The endpoint's https URL (e.g. https://app.example/arete/webhook)
    #[arg(long)]
    pub url: String,

    /// Event names to subscribe to, comma-separated (default: every event
    /// the project's build declares)
    #[arg(long, value_delimiter = ',')]
    pub events: Vec<String>,

    /// Print only the signing secret, e.g. to pipe it into
    /// `wrangler secret put ARETE_WEBHOOK_SECRET`
    #[arg(long)]
    pub secret_only: bool,

    #[command(flatten)]
    pub listing: ListingArgs,
}

/// Options for `a4 protocol webhook remove|test`.
#[derive(Args, Debug, Clone, Default)]
pub struct EndpointArgs {
    /// The webhook endpoint's id (from `a4 protocol webhook list`)
    pub webhook_id: String,

    #[command(flatten)]
    pub listing: ListingArgs,
}

pub fn run(command: WebhookCommands, json: bool) -> Result<()> {
    match command {
        WebhookCommands::Add(args) => add(&args, json),
        WebhookCommands::List(args) => list(&args, json),
        WebhookCommands::Remove(args) => remove(&args, json),
        WebhookCommands::Test(args) => test(&args, json),
    }
}

/// `a4 protocol webhook add`.
pub fn add(args: &AddArgs, json: bool) -> Result<()> {
    let url = check_url(&args.url)?;
    let events = if args.events.is_empty() {
        declared_events(&args.listing.dir)?
    } else {
        clean_events(&args.events)?
    };
    let client = ApiClient::new()?;
    require_secret_key(&client, "webhook add")?;
    let deployment = resolve_deployment(&client, &args.listing)?;
    let webhook = client.create_protocol_webhook(&deployment, &url, &events)?;
    if args.secret_only {
        let secret = webhook
            .secret
            .as_deref()
            .context("The server did not return the endpoint's signing secret")?;
        println!("{secret}");
    } else if json {
        println!("{}", serde_json::to_string_pretty(&webhook)?);
    } else {
        print!("{}", render_created(&webhook));
    }
    Ok(())
}

/// `a4 protocol webhook list`.
pub fn list(args: &ListingArgs, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    require_secret_key(&client, "webhook list")?;
    let deployment = resolve_deployment(&client, args)?;
    let webhooks = client.list_protocol_webhooks(&deployment)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&webhooks)?);
    } else {
        print!("{}", render_list(&webhooks));
    }
    Ok(())
}

/// `a4 protocol webhook remove`.
pub fn remove(args: &EndpointArgs, json: bool) -> Result<()> {
    let webhook_id = check_id(&args.webhook_id)?;
    let client = ApiClient::new()?;
    require_secret_key(&client, "webhook remove")?;
    let deployment = resolve_deployment(&client, &args.listing)?;
    let webhook = client.disable_protocol_webhook(&deployment, &webhook_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&webhook)?);
    } else {
        println!(
            "{} {} ({}): {}",
            "Disabled".green().bold(),
            webhook.id,
            webhook.url,
            webhook.status
        );
    }
    Ok(())
}

/// `a4 protocol webhook test`.
pub fn test(args: &EndpointArgs, json: bool) -> Result<()> {
    let webhook_id = check_id(&args.webhook_id)?;
    let client = ApiClient::new()?;
    require_secret_key(&client, "webhook test")?;
    let deployment = resolve_deployment(&client, &args.listing)?;
    let result = client.test_protocol_webhook(&deployment, &webhook_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", render_test(&result));
    }
    if !result.delivered {
        bail!("The test delivery did not reach the endpoint");
    }
    Ok(())
}

/// Deliveries go to public https endpoints only: they carry a signature,
/// but the body should not cross the network in the clear.
fn check_url(raw: &str) -> Result<String> {
    let url = raw.trim();
    let parsed = reqwest::Url::parse(url).with_context(|| format!("--url '{url}' is not a URL"))?;
    if parsed.scheme() != "https" {
        bail!(
            "--url must be an https URL on a public host (webhooks are delivered over https only). To receive events locally, run `a4 dev up`, which delivers the dev environment's events to your automations"
        );
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        bail!("--url '{url}' has no host");
    }
    Ok(url.to_string())
}

fn check_id(raw: &str) -> Result<String> {
    let id = raw.trim();
    uuid::Uuid::parse_str(id).with_context(|| {
        format!("'{id}' is not a webhook id; `a4 protocol webhook list` shows them")
    })?;
    Ok(id.to_string())
}

fn clean_events(events: &[String]) -> Result<Vec<String>> {
    let mut cleaned: Vec<String> = events
        .iter()
        .map(|event| event.trim().to_string())
        .filter(|event| !event.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    if cleaned.is_empty() {
        bail!("--events names no event");
    }
    Ok(cleaned)
}

/// Every event the project's build declares.
fn declared_events(dir: &std::path::Path) -> Result<Vec<String>> {
    let release = super::load_local_release(dir).context(
        "Pass --events <name,...>, or run this in a built protocol project (`npx arete-protocol build`) to subscribe to every event it declares",
    )?;
    let events: Vec<String> = release.spec.events.keys().cloned().collect();
    if events.is_empty() {
        bail!(
            "The protocol declares no events, so a webhook would receive nothing. Add `events` and an `emit` reaction to the protocol, or pass --events"
        );
    }
    Ok(events)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub fn render_created(webhook: &ProtocolWebhook) -> String {
    let mut out = format!(
        "{} {} → {}\n  events: {}\n",
        "Created webhook".green().bold(),
        webhook.id,
        webhook.url,
        webhook.events.join(", ")
    );
    match webhook.secret.as_deref() {
        Some(secret) => {
            out.push_str(&format!(
                "\n  {}  {}\n\n",
                "Signing secret:".bold(),
                secret.yellow().bold()
            ));
            out.push_str(
                "  It is shown only now. Store it where the endpoint runs as ARETE_WEBHOOK_SECRET:\n",
            );
            out.push_str("    wrangler secret put ARETE_WEBHOOK_SECRET    # Cloudflare Workers\n");
            out.push_str("    vercel env add ARETE_WEBHOOK_SECRET         # Vercel\n");
            out.push_str("    fly secrets set ARETE_WEBHOOK_SECRET=...    # Fly.io\n");
            out.push_str(
                "  Lost it? `a4 protocol webhook remove <id>` and add the endpoint again.\n",
            );
        }
        None => out.push_str(
            "\n  The server returned no signing secret; remove the endpoint and add it again.\n",
        ),
    }
    out.push_str(&format!(
        "\nCheck it: a4 protocol webhook test {}\n",
        webhook.id
    ));
    out
}

pub fn render_list(webhooks: &[ProtocolWebhook]) -> String {
    if webhooks.is_empty() {
        return "No webhook endpoints. Add one: a4 protocol webhook add --url https://…\n"
            .to_string();
    }
    let mut out = format!(
        "{:<36}  {:<9}  {:<8}  {:<40}  {}\n",
        "ID".bold(),
        "STATUS".bold(),
        "SECRET".bold(),
        "URL".bold(),
        "EVENTS".bold()
    );
    for webhook in webhooks {
        out.push_str(&format!(
            "{:<36}  {:<9}  {:<8}  {:<40}  {}\n",
            webhook.id,
            webhook.status,
            webhook
                .secret_hint
                .as_deref()
                .map(|hint| format!("…{hint}"))
                .unwrap_or_default(),
            webhook.url,
            webhook.events.join(",")
        ));
    }
    out
}

pub fn render_test(result: &ProtocolWebhookTest) -> String {
    let code = result
        .response_code
        .map(|code| format!(" (HTTP {code})"))
        .unwrap_or_default();
    if result.delivered {
        format!(
            "{} delivery {}{code}\n",
            "Delivered".green().bold(),
            result.delivery_id
        )
    } else {
        format!(
            "{} delivery {}{code}: {}\n",
            "Not delivered".red().bold(),
            result.delivery_id,
            result.error.as_deref().unwrap_or("no response")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use serde_json::{json, Value};

    const DEPLOYMENT: &str = "6f1c0e2a-0000-4000-8000-000000000000";
    const WEBHOOK: &str = "0b5d6a3e-0000-4000-8000-000000000001";

    fn webhook_json(secret: Option<&str>) -> Value {
        let mut value = json!({
            "id": WEBHOOK,
            "deploymentId": DEPLOYMENT,
            "url": "https://app.example/arete/webhook",
            "role": "publisher",
            "scope": "all",
            "events": ["tipJar.closed", "tipJar.tipped"],
            "status": "active",
            "secretHint": "a1b2",
            "createdAt": "2026-10-23T00:00:00Z"
        });
        if let Some(secret) = secret {
            value["secret"] = json!(secret);
        }
        value
    }

    #[test]
    fn add_posts_the_url_and_events_and_returns_the_secret_once() {
        let server = MockServer::json(201, &webhook_json(Some("whsec_abc")).to_string());
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_sk_test".into());
        let webhook = client
            .create_protocol_webhook(
                DEPLOYMENT,
                "https://app.example/arete/webhook",
                &["tipJar.closed".into(), "tipJar.tipped".into()],
            )
            .unwrap();
        assert_eq!(webhook.secret.as_deref(), Some("whsec_abc"));
        let request = server.request();
        assert_eq!(
            request.request_line,
            format!("POST /api/protocol-deployments/{DEPLOYMENT}/webhooks HTTP/1.1")
        );
        assert_eq!(request.header("authorization"), Some("Bearer a4_sk_test"));
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({
                "url": "https://app.example/arete/webhook",
                "events": ["tipJar.closed", "tipJar.tipped"]
            })
        );
        colored::control::set_override(false);
        let rendered = render_created(&webhook);
        assert!(rendered.contains("whsec_abc"), "{rendered}");
        assert!(rendered.contains("wrangler secret put ARETE_WEBHOOK_SECRET"));
        assert!(rendered.contains(&format!("a4 protocol webhook test {WEBHOOK}")));
    }

    #[test]
    fn list_remove_and_test_use_the_endpoint_routes() {
        let server = MockServer::json_sequence(vec![
            (200, json!([webhook_json(None)]).to_string()),
            (200, {
                let mut disabled = webhook_json(None);
                disabled["status"] = json!("disabled");
                disabled.to_string()
            }),
            (
                200,
                json!({ "deliveryId": "d-1", "delivered": false, "responseCode": 500, "error": "HTTP 500" })
                    .to_string(),
            ),
        ]);
        let client = ApiClient::with_base_url(server.base_url()).with_api_key("a4_sk_test".into());

        let webhooks = client.list_protocol_webhooks(DEPLOYMENT).unwrap();
        assert_eq!(webhooks.len(), 1);
        assert_eq!(webhooks[0].secret, None);
        assert_eq!(
            server.request().request_line,
            format!("GET /api/protocol-deployments/{DEPLOYMENT}/webhooks HTTP/1.1")
        );

        let disabled = client
            .disable_protocol_webhook(DEPLOYMENT, WEBHOOK)
            .unwrap();
        assert_eq!(disabled.status, "disabled");
        let request = server.request();
        assert_eq!(
            request.request_line,
            format!("DELETE /api/protocol-deployments/{DEPLOYMENT}/webhooks/{WEBHOOK} HTTP/1.1")
        );
        assert_eq!(request.header("authorization"), Some("Bearer a4_sk_test"));

        let result = client.test_protocol_webhook(DEPLOYMENT, WEBHOOK).unwrap();
        assert!(!result.delivered);
        assert_eq!(
            server.request().request_line,
            format!("POST /api/protocol-deployments/{DEPLOYMENT}/webhooks/{WEBHOOK}/test HTTP/1.1")
        );

        colored::control::set_override(false);
        let table = render_list(&webhooks);
        assert!(table.contains("…a1b2") && table.contains("tipJar.closed,tipJar.tipped"));
        assert!(render_test(&result).contains("Not delivered delivery d-1 (HTTP 500): HTTP 500"));
        assert!(render_list(&[]).contains("a4 protocol webhook add"));
    }

    #[test]
    fn urls_must_be_https_and_ids_uuids() {
        assert!(check_url("https://app.example/hook").is_ok());
        let error = check_url("http://app.example/hook")
            .unwrap_err()
            .to_string();
        assert!(error.contains("https"), "{error}");
        assert!(check_url("not a url").is_err());
        assert!(check_id(WEBHOOK).is_ok());
        assert!(check_id("abc").is_err());
        assert_eq!(
            clean_events(&[" b".into(), "a".into(), "b".into(), "".into()]).unwrap(),
            ["a", "b"]
        );
        assert!(clean_events(&["".into()]).is_err());
    }

    #[test]
    fn default_events_need_a_build() {
        let dir = tempfile::tempdir().unwrap();
        let error = format!("{:#}", declared_events(dir.path()).unwrap_err());
        assert!(error.contains("--events"), "{error}");
    }
}
