//! Registry discovery for the MCP server.
//!
//! The streaming tools (`connect`, `subscribe`, `query_entities`) can only be
//! driven by an agent that already knows a stack's WebSocket URL and view ids.
//! Before this module existed, the only way to learn those was to shell out to
//! `a4 explore` — which meant an agent driving the MCP server over stdio had no
//! in-loop path from "what exists?" to "subscribe to it".
//!
//! These tools close that gap by reading the same public registry endpoints the
//! CLI reads. Everything here is a **GET against `/api/registry/*`**, which is
//! unauthenticated: no signup required to discover public stacks and programs.
//!
//! An api key is attached when one happens to be resolvable (env var or the
//! file `a4 auth login` writes), because `GET /api/registry` widens its result
//! set to include global stacks for an authenticated caller. Absence of a key
//! is never an error here — it just means you see the public set.
//!
//! Responses are proxied through as raw JSON rather than being reshaped into
//! local structs. The registry's payloads are the contract the CLI and docs
//! already describe, and re-modelling them here would add a second place to
//! update every time the platform grows a field.

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::credentials;

const DEFAULT_API_URL: &str = "https://api.arete.run";
const ENV_VAR_API_URL: &str = "ARETE_API_URL";

/// Hard ceiling on a single registry response. ASTs and stack manifests are
/// unbounded in principle, and an oversized body would blow up the agent's
/// context window rather than fail cleanly. Refusing with a pointer to the CLI
/// is a better outcome than silently truncating JSON into something unparseable.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Artifact kinds accepted by `resolve_artifact`, mirroring the three
/// `/api/registry/artifacts/{kind}/{hash}` routes.
const ARTIFACT_KINDS: [&str; 3] = ["program-spec", "live-spec", "stack-manifest"];

#[derive(Clone)]
pub struct RegistryClient {
    base_url: String,
    http: reqwest::Client,
}

impl Default for RegistryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryClient {
    pub fn new() -> Self {
        let base_url = std::env::var(ENV_VAR_API_URL)
            .ok()
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());

        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// List stacks. Public stacks always; global stacks too when a key resolves.
    pub async fn list_stacks(&self) -> Result<Value> {
        self.get("/api/registry").await
    }

    /// The pinned install descriptor for one stack — the exact identities
    /// `a4 install` would consume.
    pub async fn stack_install(&self, stack: &str) -> Result<Value> {
        let stack = path_segment(stack, "stack")?;
        self.get(&format!("/api/registry/stacks/{stack}/install"))
            .await
    }

    /// Entity and view schema for one stack. This is where an agent gets the
    /// `<EntityName>/<view>` ids that `subscribe` expects.
    pub async fn stack_schema(&self, stack: &str) -> Result<Value> {
        let stack = path_segment(stack, "stack")?;
        self.get(&format!("/api/registry/{stack}/schema")).await
    }

    /// List installable standalone programs.
    pub async fn list_programs(&self) -> Result<Value> {
        self.get("/api/registry/programs").await
    }

    /// The pinned install descriptor for one standalone program.
    pub async fn program_install(&self, program: &str) -> Result<Value> {
        let program = path_segment(program, "program")?;
        self.get(&format!("/api/registry/programs/{program}/install"))
            .await
    }

    /// Fetch a content-addressed artifact by kind and hash.
    pub async fn artifact(&self, kind: &str, hash: &str) -> Result<Value> {
        let kind = kind.trim();
        if !ARTIFACT_KINDS.contains(&kind) {
            return Err(anyhow!(
                "unknown artifact kind `{kind}`. Expected one of: {}",
                ARTIFACT_KINDS.join(", ")
            ));
        }
        let hash = path_segment(hash, "hash")?;
        self.get(&format!("/api/registry/artifacts/{kind}/{hash}"))
            .await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self.http.get(&url);

        // Best-effort auth. A hosted-stack URL is what makes `credentials::resolve`
        // treat a missing key as fatal; passing an empty target keeps it optional,
        // which is what we want — the registry answers unauthenticated callers.
        if let Ok(resolved) = credentials::resolve(None, "") {
            if let Some(key) = resolved.key {
                request = request.bearer_auth(key);
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| anyhow!("registry request to {url} failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| anyhow!("could not read registry response from {url}: {e}"))?;

        if !status.is_success() {
            // Surface the platform's structured `code` when there is one — those
            // are stable, the English messages are not.
            let code = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("code").and_then(Value::as_str).map(str::to_string));
            return Err(match code {
                Some(code) => anyhow!("registry returned {status} ({code}) for {path}"),
                None => anyhow!(
                    "registry returned {status} for {path}: {}",
                    truncate_for_error(&body)
                ),
            });
        }

        if body.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!(
                "registry response for {path} is {} bytes, over the {MAX_RESPONSE_BYTES} byte \
                 limit for a single tool result. Use `a4 explore` or `a4 install` on the \
                 command line for payloads this large.",
                body.len()
            ));
        }

        serde_json::from_str(&body)
            .map_err(|e| anyhow!("registry returned invalid JSON for {path}: {e}"))
    }
}

/// Validate a value destined for a URL path segment.
///
/// Rejecting rather than percent-encoding is deliberate: a `/` or `?` in a
/// stack reference means the agent has confused a reference with a path or a
/// URL, and silently encoding it would produce a confusing 404 instead of an
/// error that says what went wrong.
fn path_segment(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("`{field}` must not be empty"));
    }
    if let Some(bad) = trimmed
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || "/?#%&".contains(*c))
    {
        return Err(anyhow!(
            "`{field}` contains an invalid character {bad:?}. Pass a bare reference \
             (e.g. `ore`), not a URL or path."
        ));
    }
    Ok(trimmed.to_string())
}

fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 300;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let cut = body
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= LIMIT)
        .last()
        .unwrap_or(0);
    format!("{}…", &body[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_references() {
        assert_eq!(path_segment("ore", "stack").unwrap(), "ore");
        assert_eq!(path_segment("  ore  ", "stack").unwrap(), "ore");
        assert_eq!(path_segment("spl-token", "program").unwrap(), "spl-token");
    }

    #[test]
    fn rejects_empty() {
        assert!(path_segment("", "stack").is_err());
        assert!(path_segment("   ", "stack").is_err());
    }

    #[test]
    fn rejects_path_and_url_shapes() {
        // The failure mode this guards against: an agent passing a URL or a
        // nested path where a bare reference belongs.
        assert!(path_segment("https://arete.run/ore", "stack").is_err());
        assert!(path_segment("stacks/ore", "stack").is_err());
        assert!(path_segment("ore?json=1", "stack").is_err());
        assert!(path_segment("ore#frag", "stack").is_err());
        assert!(path_segment("o re", "stack").is_err());
        assert!(path_segment("ore\n", "stack").is_ok()); // trimmed
        assert!(path_segment("or\ne", "stack").is_err()); // interior
    }

    #[tokio::test]
    async fn rejects_unknown_artifact_kind() {
        let client = RegistryClient::new();
        let err = client.artifact("not-a-kind", "abc").await.unwrap_err();
        assert!(err.to_string().contains("unknown artifact kind"));
    }

    #[test]
    fn truncates_long_error_bodies_on_char_boundaries() {
        let body = "é".repeat(400);
        let out = truncate_for_error(&body);
        assert!(out.ends_with('…'));
        assert!(out.len() < body.len());
    }

    #[test]
    fn short_error_bodies_pass_through() {
        assert_eq!(truncate_for_error("nope"), "nope");
    }
}
