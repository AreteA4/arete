//! Deadlines for the server's network calls.
//!
//! Two layers, so no tool call can wait on the network forever:
//!
//! - Each request to the Arete API or a stack is abandoned after
//!   [`REQUEST_TIMEOUT`] (the CLI's client default), connecting, TLS,
//!   headers and the whole body included, and fails with [`TimedOut`].
//!   Clients that already bound their own requests keep their bounds.
//! - Each tool call as a whole is abandoned after [`tool_call_timeout`],
//!   which covers tools that make several requests in a row. Tools that
//!   sign or send a transaction ([`SENDING_TOOLS`]) are never abandoned
//!   partway: every request and wait they make is bounded already, and
//!   their own errors say how to find out whether the action ran. Other
//!   tools that change something ([`CHANGING_TOOLS`]) may be abandoned, but
//!   their timeout is not called retryable: the change may have been made.
//!
//! A call that times out is answered with an error rather than holding its
//! request open, so it also gives back its place under the stdio cap on
//! requests awaiting a reply.

use std::future::Future;
use std::time::Duration;

/// How long one request to the API or a stack may take when its caller sets
/// no other bound: the CLI's default request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a tool call may take when [`tool_call_timeout`] names no other
/// bound: two [`REQUEST_TIMEOUT`]s, for a tool that makes a request after a
/// lookup. A healthy call takes seconds; this only ends stalled ones.
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Tools that may sign or send a transaction. Abandoning one partway would
/// answer with a bare retryable error while the action may still land, and
/// lose what the tool's own answer carries (the idempotency key it
/// generated, the execution id, the hint to look the action up rather than
/// run it again), so these get no tool deadline: each of their requests
/// and waits is bounded by its client (sandbox actions 90 s, protocol API
/// requests 60 s, a settle wait at most 120 s, a dev step's wait 90 s).
pub const SENDING_TOOLS: [&str; 4] = [
    "sandbox_invoke",
    "protocol_submit",
    "protocol_execute",
    "dev_step",
];

/// Tools other than [`SENDING_TOOLS`] that change state on the API or in a
/// dev environment. The tool deadline can abandon one with its change in
/// flight (a relative `dev_warp`, a `dev_branch`), so its timeout is
/// answered with `retryable: false` and asks for the state to be checked
/// before the call is repeated.
pub const CHANGING_TOOLS: [&str; 18] = [
    "dev_wake",
    "dev_sleep",
    "dev_delete",
    "dev_checkpoint",
    "dev_branch",
    "dev_restore",
    "dev_rebase",
    "dev_reset",
    "dev_reset_account",
    "dev_pin",
    "dev_follow",
    "dev_fund",
    "dev_warp",
    "dev_scenario",
    "protocol_prepare",
    "squads_attach",
    "rename_agent",
    "create_claim_link",
];

/// How long a call to `tool` may take, or `None` for [`SENDING_TOOLS`]. A
/// tool whose own requests or waits may legitimately run longer than
/// [`TOOL_CALL_TIMEOUT`] gets a bound above them, so its own, more specific
/// error is the one an agent sees:
///
/// - `dev_capture` and `dev_scenario`: 15 minutes. A capture pages through
///   up to `max_transactions` transactions at a paced request rate, and a
///   scenario runs many steps; both are bounded per request already. Larger
///   captures belong to `a4 dev capture`.
/// - Other `dev_*` tools: 300 s. Cloud dev environment requests are bounded
///   at 150 s (a sleeping environment wakes first).
/// - `sandbox_preview`: 300 s, above the 270 s its requests can take in a
///   row: listing the grants and minting a token (30 s each), the dry run
///   (90 s), and a fresh token and the dry run again after a stale one.
/// - `protocol_prepare`: 240 s, above the 210 s its requests can take in a
///   row: the protocol's registry lookup (30 s), the session mint and the
///   prepare (60 s each), and simulating the step (60 s as a whole).
/// - Other `sandbox_*`, `protocol_*` and `squads_*` tools: 120 s, above any
///   one of their requests (the sandbox's 90 s, the protocol API's 60 s).
///   A read that makes several in a row can be cut off by it; its timeout
///   is retryable.
/// - `read_view`: 90 s, above its own `timeout_secs` (at most 60 s).
pub fn tool_call_timeout(tool: &str) -> Option<Duration> {
    if SENDING_TOOLS.contains(&tool) {
        return None;
    }
    Some(match tool {
        "dev_capture" | "dev_scenario" => Duration::from_secs(15 * 60),
        "read_view" => Duration::from_secs(90),
        "sandbox_preview" => Duration::from_secs(300),
        "protocol_prepare" => Duration::from_secs(240),
        _ if tool.starts_with("dev_") => Duration::from_secs(300),
        _ if ["sandbox_", "protocol_", "squads_"]
            .iter()
            .any(|prefix| tool.starts_with(prefix)) =>
        {
            Duration::from_secs(120)
        }
        _ => TOOL_CALL_TIMEOUT,
    })
}

/// A request that did not finish in time. It names the origin only, never
/// a path, query, credential or key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedOut {
    pub origin: String,
    pub after: Duration,
}

impl TimedOut {
    pub fn new(url: &str, after: Duration) -> Self {
        Self {
            origin: origin(url),
            after,
        }
    }
}

impl std::fmt::Display for TimedOut {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "request to {} timed out after {:?}",
            self.origin, self.after
        )
    }
}

impl std::error::Error for TimedOut {}

/// `scheme://host[:port]` of `url`, without credentials, path or query.
pub fn origin(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(url) if url.has_host() => url.origin().ascii_serialization(),
        _ => "the configured API".to_string(),
    }
}

/// `request`, failing with [`TimedOut`] for `url` when it takes longer than
/// `after`. Dropping `request` closes its connection.
pub async fn within<T>(
    after: Duration,
    url: &str,
    request: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    tokio::time::timeout(after, request)
        .await
        .map_err(|_| TimedOut::new(url, after))?
}

/// `http://<address>` of a server that accepts every connection and never
/// answers, for tests of the deadlines above.
#[cfg(test)]
pub(crate) fn blackhole() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    format!("http://{address}")
}

/// `test`, failing it once `limit` passes: a deadline test whose deadline
/// regresses fails instead of hanging the suite. Under a paused clock pick
/// a `limit` above the deadline under test, so the clock reaches that first.
#[cfg(test)]
pub(crate) async fn watchdog<T>(limit: Duration, test: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, test)
        .await
        .unwrap_or_else(|_| panic!("no answer within {limit:?}: the deadline did not fire"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_drops_credentials_path_and_query() {
        assert_eq!(
            origin("https://user:a4_sk_secret@api.arete.run:8443/api/registry?key=a4_sk_secret"),
            "https://api.arete.run:8443"
        );
        assert_eq!(origin("ws://127.0.0.1:9000/stack"), "ws://127.0.0.1:9000");
        assert_eq!(origin("not a url"), "the configured API");
    }

    #[test]
    fn long_running_tools_get_bounds_above_their_own_waits() {
        let secs = |secs| Some(Duration::from_secs(secs));
        assert_eq!(
            tool_call_timeout("explore_programs"),
            Some(TOOL_CALL_TIMEOUT)
        );
        assert_eq!(tool_call_timeout("read_view"), secs(90));
        assert_eq!(tool_call_timeout("sandbox_status"), secs(120));
        assert_eq!(tool_call_timeout("dev_wake"), secs(300));
        assert_eq!(tool_call_timeout("dev_capture"), secs(900));
    }

    /// Exactly these tools sign or send, and none of them has a tool
    /// deadline that could cut an action off mid-flight.
    #[test]
    fn sending_tools_are_exactly_the_four_and_never_cut_off() {
        assert_eq!(
            SENDING_TOOLS,
            [
                "sandbox_invoke",
                "protocol_submit",
                "protocol_execute",
                "dev_step"
            ]
        );
        for tool in SENDING_TOOLS {
            assert_eq!(tool_call_timeout(tool), None, "{tool}");
        }
    }

    /// Each tool's bound is at least the sum of the request bounds it can
    /// hit in a row, so its own, more specific error reaches the agent.
    #[test]
    fn ceilings_cover_the_requests_each_tool_makes_in_a_row() {
        use crate::protocol_agent::{API_TIMEOUT, SIMULATION_BUDGET};
        use crate::sandbox::{ACTION_TIMEOUT, READ_TIMEOUT};
        let table: [(&str, Vec<Duration>); 3] = [
            (
                // Grants, mint, dry run; a fresh mint and the dry run again
                // after a stale token.
                "sandbox_preview",
                vec![
                    READ_TIMEOUT,
                    READ_TIMEOUT,
                    ACTION_TIMEOUT,
                    READ_TIMEOUT,
                    ACTION_TIMEOUT,
                ],
            ),
            (
                // Registry lookup of `protocol`, session mint, prepare,
                // simulation as a whole.
                "protocol_prepare",
                vec![REQUEST_TIMEOUT, API_TIMEOUT, API_TIMEOUT, SIMULATION_BUDGET],
            ),
            (
                // The attach, then the proposer read.
                "squads_attach",
                vec![API_TIMEOUT, API_TIMEOUT],
            ),
        ];
        for (tool, requests) in table {
            let sum: Duration = requests.iter().sum();
            let ceiling = tool_call_timeout(tool).unwrap();
            assert!(ceiling >= sum, "{tool}: {ceiling:?} < {sum:?}");
        }
    }
}
