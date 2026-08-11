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
    pub async fn list_stacks(&self) -> Result<String> {
        self.get("/api/registry").await
    }

    /// The pinned install descriptor for one stack — the exact identities
    /// `a4 install` would consume.
    pub async fn stack_install(&self, stack: &str) -> Result<String> {
        let stack = path_segment(stack, "stack")?;
        self.get(&format!("/api/registry/stacks/{stack}/install"))
            .await
    }

    /// Entity and view schema for one stack. This is where an agent gets the
    /// `<EntityName>/<view>` ids that `subscribe` expects.
    pub async fn stack_schema(&self, stack: &str) -> Result<String> {
        let stack = path_segment(stack, "stack")?;
        self.get(&format!("/api/registry/{stack}/schema")).await
    }

    /// List installable standalone programs.
    pub async fn list_programs(&self) -> Result<String> {
        self.get("/api/registry/programs").await
    }

    /// The pinned install descriptor for one standalone program.
    pub async fn program_install(&self, program: &str) -> Result<String> {
        let program = path_segment(program, "program")?;
        self.get(&format!("/api/registry/programs/{program}/install"))
            .await
    }

    /// Fetch a content-addressed artifact by kind and hash.
    pub async fn artifact(&self, kind: &str, hash: &str) -> Result<String> {
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

    /// Returns the response body verbatim rather than a parsed [`Value`].
    ///
    /// Two reasons, and both are contract-level. The body is already bounded by
    /// [`MAX_RESPONSE_BYTES`]; re-serializing a parsed `Value` is *not* bounded by
    /// that, because JSON number formatting is not length-preserving — `1e9`
    /// round-trips to `1000000000.0`, so an array of those grows over 3x and can
    /// carry a legal 512 KiB body past the limit the cap exists to enforce.
    /// Returning the bytes we accepted keeps the advertised bound true by
    /// construction. It also makes the documented raw pass-through actually raw:
    /// a round-trip through `Value` rewrites numbers and reorders nothing
    /// usefully, and agents comparing a hash-relevant artifact against the CLI
    /// would see a body the platform never sent.
    async fn get(&self, path: &str) -> Result<String> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self.http.get(&url);

        // Best-effort auth, but only ever to an Arete origin. `ARETE_API_URL` can
        // point anywhere, and `ARETE_API_KEY` (unlike the credentials file, which
        // is keyed by API URL) is not scoped to a destination — so attaching it
        // unconditionally would ship the user's key to whatever host that variable
        // names. Every endpoint reached here is public, so the key only ever buys
        // the global-stack widening on `GET /api/registry`. That is not worth
        // leaking a credential for: when the destination is not recognised, send
        // the request unauthenticated rather than failing, and the public subset
        // still comes back.
        //
        // The empty target passed to `resolve` keeps a missing key non-fatal — it
        // is a hosted *stack* URL that makes absence an error, which is a `connect`
        // concern, not ours.
        if is_arete_origin(&self.base_url) {
            if let Ok(resolved) = credentials::resolve(None, "") {
                if let Some(key) = resolved.key {
                    request = request.bearer_auth(key);
                }
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| anyhow!("registry request to {url} failed: {e}"))?;

        let status = response.status();
        let body = read_capped_body(response, path).await?;

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

        // Parse only to validate: a proxy's HTML error page must not reach the
        // agent as though it were a registry response. The parsed value is
        // discarded and the original bytes are returned.
        serde_json::from_str::<Value>(&body)
            .map_err(|e| anyhow!("registry returned invalid JSON for {path}: {e}"))?;
        Ok(body)
    }
}

/// Read a response body, aborting as soon as it exceeds [`MAX_RESPONSE_BYTES`].
///
/// Buffering first and measuring afterwards would defeat the point of the cap:
/// a multi-gigabyte artifact would be fully downloaded and allocated before we
/// declined it, which is the exact failure the limit exists to prevent. So we
/// check the declared `Content-Length` when the server offers one, then stream
/// chunk by chunk and stop at the first chunk that crosses the line.
async fn read_capped_body(mut response: reqwest::Response, path: &str) -> Result<String> {
    // Fail before transferring anything when the server declares an oversized body.
    check_size(response.content_length(), path)?;

    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow!("could not read registry response for {path}: {e}"))?
    {
        check_size(Some((buf.len() + chunk.len()) as u64), path)?;
        buf.extend_from_slice(&chunk);
    }

    String::from_utf8(buf).map_err(|e| anyhow!("registry response for {path} is not UTF-8: {e}"))
}

/// Shared size guard for both the declared-length and streaming paths, so the
/// two cannot drift apart. `None` means the server declared nothing, which is
/// not itself a failure — the streaming path still bounds it.
fn check_size(bytes: Option<u64>, path: &str) -> Result<()> {
    match bytes {
        Some(n) if n > MAX_RESPONSE_BYTES as u64 => Err(anyhow!(
            "registry response for {path} is at least {n} bytes, over the \
             {MAX_RESPONSE_BYTES} byte limit for a single tool result. Use `a4 explore` \
             or `a4 install` on the command line for payloads this large."
        )),
        _ => Ok(()),
    }
}

/// Whether a resolved API key may be attached to requests for `base_url`.
///
/// Deliberately an allowlist. `ARETE_API_URL` is free-form, and a bearer token
/// sent to the wrong host is unrecoverable — the key cannot be un-leaked. Local
/// loopback is permitted so development against a local control plane keeps
/// working; anything else gets unauthenticated requests, which still succeed
/// because every endpoint this client touches is public.
///
/// The right host over the wrong scheme leaks just as badly, so HTTPS is
/// required for Arete hosts: `ARETE_API_URL=http://api.arete.run` is an easy
/// thing to have lying around in a shell profile, and it would put the key on
/// the wire in cleartext. Loopback is the one exception — the request never
/// reaches a network — so local development over plain HTTP keeps working.
fn is_arete_origin(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    // Trailing dot is the fully-qualified form of the same name, so strip it
    // before comparing — `api.arete.run.` must not slip past the allowlist.
    let host = host.trim_end_matches('.').to_ascii_lowercase();

    // `host_str` keeps the brackets on IPv6 literals (`[::1]`), which do not
    // parse as an address. Strip them or loopback IPv6 is wrongly refused.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host);
    let loopback = match bare.parse::<std::net::IpAddr>() {
        // An explicit IP is only ever loopback or foreign; it is never an
        // Arete hostname, so this decides the answer on its own.
        Ok(ip) => ip.is_loopback(),
        Err(_) => host == "localhost",
    };

    match url.scheme() {
        "http" => loopback,
        "https" => loopback || host == "arete.run" || host.ends_with(".arete.run"),
        _ => false,
    }
}

/// Validate a value destined for a URL path segment.
///
/// Rejecting rather than percent-encoding is deliberate: a `/` or `?` in a
/// stack reference means the agent has confused a reference with a path or a
/// URL, and silently encoding it would produce a confusing 404 instead of an
/// error that says what went wrong.
///
/// This is also a security boundary, not just ergonomics. The WHATWG parser
/// behind `reqwest::Url` treats `\` as a path separator for http(s) and
/// normalizes `..` away, so an unvalidated segment does not stay under
/// `/api/registry/`: a hash shaped like `..\..\..\agents\me` resolves to
/// `/api/agents/me`. Since [`RegistryClient::get`] attaches the bearer token
/// before sending, that would let a public-registry tool issue authenticated
/// requests against arbitrary routes.
fn path_segment(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("`{field}` must not be empty"));
    }
    if let Some(bad) = trimmed
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || "/\\?#%&".contains(*c))
    {
        return Err(anyhow!(
            "`{field}` contains an invalid character {bad:?}. Pass a bare reference \
             (e.g. `ore`), not a URL or path."
        ));
    }
    // A dot-only segment is a relative path component, never a reference.
    if trimmed.chars().all(|c| c == '.') {
        return Err(anyhow!(
            "`{field}` must not be a relative path segment. Pass a bare reference \
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

    #[test]
    fn rejects_path_traversal_shapes() {
        // `\` is a path separator to the WHATWG parser behind `reqwest::Url`,
        // and `..` is normalized away, so neither may reach a URL.
        for bad in [
            r"..\..\..\agents\me",
            r"ore\..\..\agents",
            "..",
            ".",
            "%2e%2e%2fagents",
        ] {
            assert!(
                path_segment(bad, "hash").is_err(),
                "expected {bad:?} to be refused"
            );
        }
    }

    #[test]
    fn validated_segments_cannot_escape_the_registry_prefix() {
        // Why that guard is load-bearing, asserted on the normalized request
        // URL: unvalidated, this segment walks out of `/api/registry` and lands
        // on an authenticated route that `get` has already attached a bearer
        // token for.
        let escaped = reqwest::Url::parse(
            r"https://api.arete.run/api/registry/artifacts/program-spec/..\..\..\agents\me",
        )
        .unwrap();
        assert_eq!(escaped.path(), "/api/agents/me");

        // A reference that passes validation stays under the prefix.
        let hash = path_segment("2f8a9c", "hash").unwrap();
        let url = reqwest::Url::parse(&format!(
            "https://api.arete.run/api/registry/artifacts/program-spec/{hash}"
        ))
        .unwrap();
        assert_eq!(url.path(), "/api/registry/artifacts/program-spec/2f8a9c");
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

    #[test]
    fn attaches_credentials_only_to_arete_origins() {
        for ok in [
            "https://api.arete.run",
            "https://arete.run",
            "https://API.Arete.Run",
            "https://api.arete.run.", // fully-qualified form of the same host
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
        ] {
            assert!(is_arete_origin(ok), "expected {ok} to be allowed");
        }
    }

    #[test]
    fn refuses_credentials_for_foreign_origins() {
        for bad in [
            "https://evil.example",
            // Suffix-confusion attempts against a naive `contains`/`starts_with`.
            "https://arete.run.evil.example",
            "https://notarete.run",
            "https://api.arete.run.evil.example",
            // Userinfo cannot smuggle the real host into the authority.
            "https://api.arete.run@evil.example",
            // Non-HTTP schemes never carry a bearer token.
            "file:///etc/passwd",
            "ftp://api.arete.run",
            "not a url",
            "",
        ] {
            assert!(!is_arete_origin(bad), "expected {bad} to be refused");
        }
    }

    #[test]
    fn requires_https_for_arete_hosts() {
        // The right host over the wrong scheme leaks just as badly — the key
        // would go out in cleartext.
        for bad in [
            "http://api.arete.run",
            "http://arete.run",
            "http://ore.stack.arete.run",
        ] {
            assert!(!is_arete_origin(bad), "expected {bad} to be refused");
        }
        // Loopback is the exception, since the request never reaches a network.
        for ok in [
            "http://localhost:3000",
            "https://localhost:3000",
            "https://127.0.0.1",
        ] {
            assert!(is_arete_origin(ok), "expected {ok} to be allowed");
        }
    }

    #[test]
    fn public_ips_are_not_treated_as_loopback() {
        assert!(!is_arete_origin("http://8.8.8.8:3000"));
        assert!(!is_arete_origin("http://[2001:4860:4860::8888]:3000"));
    }

    #[test]
    fn declared_length_over_cap_is_refused() {
        // Guards the pre-transfer path: a server-declared Content-Length above
        // the cap must fail before any body is read.
        let err = check_size(Some(MAX_RESPONSE_BYTES as u64 + 1), "/x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("over the"), "unexpected error: {err}");
    }

    #[test]
    fn declared_length_at_or_under_cap_is_allowed() {
        assert!(check_size(Some(MAX_RESPONSE_BYTES as u64), "/x").is_ok());
        assert!(check_size(Some(0), "/x").is_ok());
    }

    #[test]
    fn absent_declared_length_is_not_a_failure() {
        // Chunked responses declare nothing; the streaming path bounds those.
        assert!(check_size(None, "/x").is_ok());
    }

    #[tokio::test]
    async fn oversized_undeclared_body_is_refused_while_streaming() {
        // No Content-Length: the cap has to hold on the streaming path too.
        let body = "x".repeat(MAX_RESPONSE_BYTES + 1024);
        let response = http::Response::builder().status(200).body(body).unwrap();
        let err = read_capped_body(reqwest::Response::from(response), "/x")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("over the"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn body_at_the_limit_is_accepted() {
        let body = "x".repeat(MAX_RESPONSE_BYTES);
        let response = http::Response::builder().status(200).body(body).unwrap();
        let out = read_capped_body(reqwest::Response::from(response), "/x")
            .await
            .unwrap();
        assert_eq!(out.len(), MAX_RESPONSE_BYTES);
    }
}
