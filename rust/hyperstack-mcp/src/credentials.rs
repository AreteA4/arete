//! API key resolution for the `connect` tool.
//!
//! Agents should never have to paste API keys into tool calls — the key would
//! end up in the model's context window, chat transcript, and JSON-RPC wire.
//! Instead, `connect` resolves its api key through the following precedence:
//!
//! 1. **Explicit `api_key` argument** on the `connect` tool call (override,
//!    still supported for testing and multi-stack scenarios).
//! 2. **`HYPERSTACK_API_KEY` environment variable**, set once when launching
//!    the MCP server (e.g. in `.vscode/mcp.json`'s `env` block or via
//!    `claude mcp add -e HYPERSTACK_API_KEY=...`).
//! 3. **`~/.hyperstack/credentials.toml`**, the file managed by
//!    `hs auth login`. Two schemas are supported:
//!    - **New format:** `[keys]` table keyed by API URL
//!      (`https://api.usehyperstack.com`). Honors `HYPERSTACK_API_URL` for
//!      the lookup key.
//!    - **Legacy format:** a top-level `api_key = "..."` key. This is what
//!      older `hs auth login` versions wrote and what many users still have.
//!
//! If none of the three produces a key **and** the target WebSocket URL is a
//! hosted HyperStack stack (ends in `.stack.usehyperstack.com`), this module
//! returns a descriptive error so the agent can tell the user what to do.
//! Self-hosted / custom stacks are allowed to proceed without a key because
//! they may not require auth at all.

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::Deserialize;

const HOSTED_WEBSOCKET_SUFFIX: &str = ".stack.usehyperstack.com";
const DEFAULT_API_URL: &str = "https://api.usehyperstack.com";
const ENV_VAR_API_KEY: &str = "HYPERSTACK_API_KEY";
const ENV_VAR_API_URL: &str = "HYPERSTACK_API_URL";

/// Describes where a resolved api key came from. Useful for log lines and the
/// `connect` tool response so users can see which source won without revealing
/// the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Explicit,
    EnvVar,
    CredentialsFile,
    None,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            KeySource::Explicit => "explicit_argument",
            KeySource::EnvVar => "env:HYPERSTACK_API_KEY",
            KeySource::CredentialsFile => "~/.hyperstack/credentials.toml",
            KeySource::None => "none",
        }
    }
}

/// Result of a key lookup. The `key` is `None` only for self-hosted URLs
/// where proceeding without auth is legitimate.
#[derive(Debug, Clone)]
pub struct ResolvedKey {
    pub key: Option<String>,
    pub source: KeySource,
}

/// Resolve the api key to use for a `connect` call to `url`.
pub fn resolve(explicit: Option<String>, url: &str) -> Result<ResolvedKey> {
    // 1. Explicit argument wins. Trim to protect against accidental whitespace.
    if let Some(k) = explicit.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Ok(ResolvedKey {
            key: Some(k.to_string()),
            source: KeySource::Explicit,
        });
    }

    // 2. Environment variable.
    if let Ok(k) = std::env::var(ENV_VAR_API_KEY) {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(ResolvedKey {
                key: Some(k),
                source: KeySource::EnvVar,
            });
        }
    }

    // 3. Credentials file.
    if let Some(k) = load_from_credentials_file() {
        return Ok(ResolvedKey {
            key: Some(k),
            source: KeySource::CredentialsFile,
        });
    }

    // Nothing found. Decide whether that's fatal.
    if is_hosted_websocket_url(url) {
        let file = credentials_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.hyperstack/credentials.toml".to_string());
        Err(anyhow!(
            "no HyperStack api key found for hosted stack `{url}`. \
             Tried: explicit `api_key` argument, `{ENV_VAR_API_KEY}` env var, and {file}. \
             Fix: run `hs auth login`, or set `{ENV_VAR_API_KEY}=hsk_...` in your MCP \
             server environment (e.g. `.vscode/mcp.json` `env` block), or pass \
             `api_key` explicitly on the connect call."
        ))
    } else {
        Ok(ResolvedKey {
            key: None,
            source: KeySource::None,
        })
    }
}

/// Whether the URL points at a HyperStack-hosted WebSocket endpoint.
/// Mirrors `hyperstack_sdk::auth::is_hosted_hyperstack_websocket_url`, which is
/// `pub(crate)` in the SDK and not reachable from here. Kept in sync with the
/// SDK's `HOSTED_WEBSOCKET_SUFFIX` constant.
fn is_hosted_websocket_url(url: &str) -> bool {
    // We only need the host portion; a full URL parser is overkill. Strip the
    // scheme, then cut at the first path/query/port character and compare the
    // trailing substring against the hosted suffix.
    let rest = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let host_end = rest
        .find(|c: char| c == '/' || c == ':' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    rest[..host_end].ends_with(HOSTED_WEBSOCKET_SUFFIX)
}

fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".hyperstack").join("credentials.toml"))
}

/// Read the key from `~/.hyperstack/credentials.toml`. Supports both the new
/// `[keys] "<api_url>" = "..."` schema (preferred) and the legacy top-level
/// `api_key = "..."` schema. Returns `None` on any read/parse error — we
/// fail soft here because the caller will decide whether absence is fatal.
fn load_from_credentials_file() -> Option<String> {
    let path = credentials_path()?;
    let content = fs::read_to_string(&path).ok()?;

    // Try the new format first.
    if let Ok(parsed) = toml::from_str::<NewFormat>(&content) {
        if let Some(keys) = parsed.keys {
            let api_url =
                std::env::var(ENV_VAR_API_URL).unwrap_or_else(|_| DEFAULT_API_URL.to_string());
            if let Some(key) = keys.get(&api_url) {
                let k = key.trim();
                if !k.is_empty() {
                    return Some(k.to_string());
                }
            }
        }
    }

    // Fall back to legacy top-level `api_key = "..."`.
    if let Ok(parsed) = toml::from_str::<LegacyFormat>(&content) {
        if let Some(k) = parsed.api_key {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Some(k);
            }
        }
    }

    None
}

#[derive(Deserialize)]
struct NewFormat {
    keys: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize)]
struct LegacyFormat {
    api_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_argument_wins() {
        let r = resolve(Some("hsk_explicit".into()), "wss://foo.stack.usehyperstack.com").unwrap();
        assert_eq!(r.source, KeySource::Explicit);
        assert_eq!(r.key.as_deref(), Some("hsk_explicit"));
    }

    #[test]
    fn whitespace_only_explicit_falls_through() {
        // If the caller passed "" or "   ", don't treat it as a key. Resolver
        // should continue to env/file lookups. We can't assert the resolved
        // value here without mocking HOME, but we can assert it didn't short-
        // circuit on Explicit.
        std::env::remove_var(ENV_VAR_API_KEY);
        let r = resolve(Some("  ".into()), "wss://self.hosted.example").unwrap();
        assert_ne!(r.source, KeySource::Explicit);
    }

    #[test]
    fn hosted_url_without_key_is_error() {
        // Ensure neither env nor file can rescue us in this test.
        std::env::remove_var(ENV_VAR_API_KEY);
        // Point HOME at a nonexistent dir so the file lookup also fails.
        std::env::set_var("HOME", "/nonexistent/hs-mcp-test");
        let err = resolve(None, "wss://any.stack.usehyperstack.com").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no HyperStack api key"));
        assert!(msg.contains("hs auth login"));
        assert!(msg.contains("HYPERSTACK_API_KEY"));
    }

    #[test]
    fn self_hosted_url_without_key_is_ok_with_none() {
        std::env::remove_var(ENV_VAR_API_KEY);
        std::env::set_var("HOME", "/nonexistent/hs-mcp-test");
        let r = resolve(None, "wss://my.self.hosted.example").unwrap();
        assert_eq!(r.source, KeySource::None);
        assert!(r.key.is_none());
    }

    #[test]
    fn hosted_url_detection() {
        assert!(is_hosted_websocket_url(
            "wss://foo.stack.usehyperstack.com"
        ));
        assert!(is_hosted_websocket_url(
            "wss://a-b-c.stack.usehyperstack.com"
        ));
        assert!(!is_hosted_websocket_url("wss://example.com"));
        assert!(!is_hosted_websocket_url("ws://localhost:8878"));
        assert!(!is_hosted_websocket_url("not a url"));
    }
}
