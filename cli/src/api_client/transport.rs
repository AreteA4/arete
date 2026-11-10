//! Sending API requests and reading their responses, with errors that say
//! what failed and what to do about it.
//!
//! A request that gets no usable answer fails with an [`ApiTransportError`]:
//! it names the operation, the method and endpoint, whether it timed out (and
//! after how long) or could not connect, and how to retry. A successful
//! response this build cannot read fails with an [`ApiResponseError`]: a body
//! of the wrong shape usually means the CLI is older than the API, so it
//! points to `a4 self update` instead of to the credentials. Under
//! `--verbose` the unreadable body is printed (credentials redacted).

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Whole-request timeout of every API request without its own (reqwest's
/// default, made explicit so a timeout error can be explained).
pub(crate) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long connecting to the API may take before the request fails, so an
/// unreachable host is reported as such instead of as a slow response.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The most body text `--verbose` prints for an unreadable response.
const MAX_VERBOSE_BODY: usize = 8 * 1024;

/// The command that updates a4, for responses this build cannot read.
const UPDATE_COMMAND: &str = "a4 self update";

/// The way a request failed before a response arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportFailure {
    /// No complete response within the request timeout.
    Timeout,
    /// The connection to the API could not be made.
    Connect,
    /// Anything else: an invalid request, a redirect loop, a broken body.
    Other,
}

/// A request to the API that got no usable response.
#[derive(Debug)]
pub struct ApiTransportError {
    operation: String,
    method: Option<String>,
    endpoint: Option<String>,
    elapsed: Duration,
    failure: TransportFailure,
    source: reqwest::Error,
}

impl ApiTransportError {
    fn new(
        operation: &str,
        method: Option<&reqwest::Method>,
        url: Option<&url::Url>,
        elapsed: Duration,
        source: reqwest::Error,
    ) -> Self {
        let failure = if source.is_timeout() {
            TransportFailure::Timeout
        } else if source.is_connect() {
            TransportFailure::Connect
        } else {
            TransportFailure::Other
        };
        ApiTransportError {
            operation: operation.to_string(),
            method: method.map(ToString::to_string),
            endpoint: url.or(source.url()).map(endpoint),
            elapsed,
            failure,
            source,
        }
    }

    #[cfg(test)]
    pub(crate) fn failure(&self) -> TransportFailure {
        self.failure
    }

    /// `GET https://api.example/path`, or what is known of it.
    fn target(&self) -> String {
        match (&self.method, &self.endpoint) {
            (Some(method), Some(endpoint)) => format!("{method} {endpoint}"),
            (None, Some(endpoint)) => endpoint.clone(),
            _ => "the API".to_string(),
        }
    }

    /// The innermost cause, for example `Connection refused (os error 61)`.
    fn root_cause(&self) -> String {
        let mut cause: &dyn std::error::Error = &self.source;
        while let Some(next) = cause.source() {
            cause = next;
        }
        cause.to_string()
    }
}

impl std::fmt::Display for ApiTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let operation = capitalize(&self.operation);
        let target = self.target();
        match self.failure {
            TransportFailure::Timeout => write!(
                formatter,
                "{operation} request timed out: {target} sent no response within {}s.\n\
                 The API may be slow or briefly unavailable. Retry in a moment; if it keeps \
                 timing out, run `a4 doctor` and check your network.",
                seconds(self.elapsed)
            ),
            TransportFailure::Connect => write!(
                formatter,
                "{operation} request could not connect to {target}: {}.\n\
                 Check your network and the API URL (--api-url or ARETE_API_URL), then retry.",
                self.root_cause()
            ),
            TransportFailure::Other => write!(
                formatter,
                "{operation} request to {target} failed after {}s: {}.\n\
                 Retry in a moment; run with --verbose for the full cause.",
                seconds(self.elapsed),
                self.root_cause()
            ),
        }
    }
}

impl std::error::Error for ApiTransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Why a successful response could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResponseProblem {
    /// The body is not JSON (or is cut short): something other than the
    /// API answered, or the connection dropped mid-body.
    NotJson { content_type: Option<String> },
    /// The body is JSON of a shape this build does not understand.
    Shape,
}

/// A successful API response this build cannot read.
#[derive(Debug)]
pub struct ApiResponseError {
    endpoint: String,
    status: u16,
    problem: ResponseProblem,
    detail: String,
}

impl ApiResponseError {
    /// Whether the body was JSON of an unexpected shape: the signature of a
    /// CLI older (or newer) than the API it talks to.
    pub(crate) fn is_version_skew(&self) -> bool {
        self.problem == ResponseProblem::Shape
    }

    /// The decoder's own message, for example ``missing field `user_id` ``.
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

impl std::fmt::Display for ApiResponseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let version = env!("CARGO_PKG_VERSION");
        match &self.problem {
            ResponseProblem::Shape => write!(
                formatter,
                "a4 {version} could not read the API's response from {} ({}): {}.\n\
                 This usually means this a4 is older than the API. Run `{UPDATE_COMMAND}` \
                 (or update a4 the way you installed it), then retry. Your credentials are \
                 not the problem. Run with --verbose to print the response body.",
                self.endpoint, self.status, self.detail
            ),
            ResponseProblem::NotJson { content_type } => write!(
                formatter,
                "The response from {} ({}, {}) is not valid JSON: {}.\n\
                 Something other than the Arete API may have answered, or the response was \
                 cut short. Check --api-url or ARETE_API_URL and any proxy, then retry. Run \
                 with --verbose to print the response body.",
                self.endpoint,
                self.status,
                content_type.as_deref().unwrap_or("no content type"),
                self.detail
            ),
        }
    }
}

impl std::error::Error for ApiResponseError {}

/// [`reqwest::blocking::RequestBuilder::send`] with a descriptive error.
pub(crate) trait SendApi {
    /// Send the request. `operation` names it in the error, in lower case
    /// ("knowledge search", "stack lookup").
    fn send_api(self, operation: &str) -> Result<reqwest::blocking::Response>;
}

impl SendApi for reqwest::blocking::RequestBuilder {
    fn send_api(self, operation: &str) -> Result<reqwest::blocking::Response> {
        let (client, request) = self.build_split();
        let request = request.map_err(|error| {
            ApiTransportError::new(operation, None, None, Duration::ZERO, error)
        })?;
        let method = request.method().clone();
        let url = request.url().clone();
        let started = Instant::now();
        let mut response = client.execute(request).map_err(|error| {
            ApiTransportError::new(
                operation,
                Some(&method),
                Some(&url),
                started.elapsed(),
                error,
            )
        })?;
        // The body is read later; a failure there still names this request
        // and the whole wait.
        response.extensions_mut().insert(SentRequest {
            operation: operation.to_string(),
            method,
            started,
        });
        Ok(response)
    }
}

/// What [`SendApi::send_api`] sent, kept on its response for errors while
/// the body is read.
#[derive(Debug, Clone)]
struct SentRequest {
    operation: String,
    method: reqwest::Method,
    started: Instant,
}

/// A successful response's body, read in full.
pub(crate) struct ResponseBody {
    endpoint: String,
    status: u16,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

impl ResponseBody {
    /// Read `response`'s body. A body that cannot be read (a timeout or a
    /// dropped connection mid-body) is an [`ApiTransportError`].
    pub(crate) fn read(response: reqwest::blocking::Response) -> Result<Self> {
        let endpoint = endpoint(response.url());
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let url = response.url().clone();
        let sent = response.extensions().get::<SentRequest>().cloned();
        let started = sent.as_ref().map_or_else(Instant::now, |sent| sent.started);
        let bytes = response.bytes().map_err(|error| {
            let (operation, method) = match &sent {
                Some(sent) => (sent.operation.as_str(), Some(&sent.method)),
                None => ("response body", None),
            };
            ApiTransportError::new(operation, method, Some(&url), started.elapsed(), error)
        })?;
        Ok(ResponseBody {
            endpoint,
            status,
            content_type,
            bytes: bytes.to_vec(),
        })
    }

    /// The body as `T`.
    pub(crate) fn parse<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.bytes).map_err(|error| self.error(error))
    }

    /// An already-parsed body `value` as `T`, with the same error as
    /// [`Self::parse`].
    pub(crate) fn convert<T: DeserializeOwned>(&self, value: Value) -> Result<T> {
        serde_json::from_value(value).map_err(|error| self.error(error))
    }

    fn error(&self, error: serde_json::Error) -> anyhow::Error {
        let problem = match error.classify() {
            serde_json::error::Category::Data => ResponseProblem::Shape,
            _ => ResponseProblem::NotJson {
                content_type: self.content_type.clone(),
            },
        };
        if super::verbose_api_errors() {
            eprintln!(
                "API response body ({}, {}):\n{}",
                self.status,
                self.endpoint,
                verbose_body(&self.bytes)
            );
        }
        ApiResponseError {
            endpoint: self.endpoint.clone(),
            status: self.status,
            problem,
            detail: error.to_string(),
        }
        .into()
    }
}

/// `scheme://host[:port]/path` of `url`: no credentials, query or fragment.
fn endpoint(url: &url::Url) -> String {
    format!("{}{}", url.origin().ascii_serialization(), url.path())
}

fn seconds(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 10.0 {
        format!("{seconds:.1}")
    } else {
        format!("{seconds:.0}")
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The body for `--verbose`: pretty JSON with credential values redacted,
/// or the raw text; at most [`MAX_VERBOSE_BODY`] characters either way.
pub(crate) fn verbose_body(bytes: &[u8]) -> String {
    let text = match serde_json::from_slice::<Value>(bytes) {
        Ok(mut value) => {
            redact_credentials(&mut value);
            serde_json::to_string_pretty(&value).unwrap_or_default()
        }
        Err(_) => redact_credential_text(&String::from_utf8_lossy(bytes)),
    };
    let total = text.chars().count();
    if total <= MAX_VERBOSE_BODY {
        return text;
    }
    let mut shown: String = text.chars().take(MAX_VERBOSE_BODY).collect();
    shown.push_str(&format!(
        "\n... ({} more characters not shown)",
        total - MAX_VERBOSE_BODY
    ));
    shown
}

/// Replace every string that is, or sits under a field named like, a
/// credential with `<redacted>`.
fn redact_credentials(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if is_credential_field(key) && value.is_string() {
                    *value = Value::String("<redacted>".to_string());
                } else {
                    redact_credentials(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_credentials),
        Value::String(text) => {
            let redacted = redact_credential_text(text);
            if redacted != *text {
                *value = Value::String(redacted);
            }
        }
        _ => {}
    }
}

/// Redact credentials in text that is not valid JSON (for example a body cut
/// short): the string value of every `"field": "..."` pair named like a
/// credential, even unterminated, and anything shaped like an Arete key, a
/// JWT or a bearer token.
fn redact_credential_text(text: &str) -> String {
    use std::sync::LazyLock;
    static FIELD: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#""([^"\\]{1,64})"(\s*:\s*)"(?:[^"\\]|\\.)*("|\\?$)"#).unwrap()
    });
    static TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(concat!(
            r"a4_[A-Za-z]{2,}_[A-Za-z0-9_\-]{4,}",
            r"|eyJ[A-Za-z0-9_\-]{8,}(?:\.[A-Za-z0-9_\-]*){0,2}",
            r"|(?i:bearer)\s+[A-Za-z0-9._~+/=\-]{8,}",
        ))
        .unwrap()
    });
    let fields = FIELD.replace_all(text, |captures: &regex::Captures| {
        if is_credential_field(&captures[1]) {
            format!(
                "\"{}\"{}\"<redacted>{}",
                &captures[1], &captures[2], &captures[3]
            )
        } else {
            captures[0].to_string()
        }
    });
    TOKEN.replace_all(&fields, "<redacted>").into_owned()
}

fn is_credential_field(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    key == "key"
        || key == "authorization"
        || key == "password"
        || key.ends_with("apikey")
        || key.ends_with("secretkey")
        || key.ends_with("secret")
        || key.ends_with("token")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::test_support::MockServer;
    use crate::api_client::{error_report, ApiClient};
    use anyhow::Context;
    use serde::Deserialize;
    use serde_json::json;

    fn body(bytes: &str) -> ResponseBody {
        ResponseBody {
            endpoint: "https://api.example/api/specs".to_string(),
            status: 200,
            content_type: Some("text/html".to_string()),
            bytes: bytes.as_bytes().to_vec(),
        }
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Spec {
        user_id: i32,
    }

    #[test]
    fn a_body_of_another_shape_points_to_self_update_not_credentials() {
        let error = body(r#"[{"id":1}]"#).parse::<Vec<Spec>>().unwrap_err();
        let response = error.downcast_ref::<ApiResponseError>().unwrap();
        assert!(response.is_version_skew());
        let message = error.to_string();
        assert!(
            message.contains("https://api.example/api/specs (200)"),
            "{message}"
        );
        assert!(message.contains("missing field `user_id`"), "{message}");
        assert!(message.contains("a4 self update"), "{message}");
        assert!(message.contains(env!("CARGO_PKG_VERSION")), "{message}");
        assert!(
            !message.contains("Failed to parse response JSON"),
            "{message}"
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_not_called_version_skew() {
        let error = body("<html>gateway</html>")
            .parse::<Vec<Spec>>()
            .unwrap_err();
        let response = error.downcast_ref::<ApiResponseError>().unwrap();
        assert!(!response.is_version_skew());
        let message = error.to_string();
        assert!(message.contains("is not valid JSON"), "{message}");
        assert!(message.contains("text/html"), "{message}");
        assert!(!message.contains("a4 self update"), "{message}");
    }

    #[test]
    fn verbose_bodies_redact_credentials_and_are_bounded() {
        let shown = verbose_body(
            json!({
                "key": "a4_pk_abcdefghijkl",
                "accessToken": "eyJhbGciOi",
                "nested": [{"api_key": "plain", "note": "a4_sk_0123456789abcdef"}],
                "user_id": 3,
                "keys": 2,
                "name": "web"
            })
            .to_string()
            .as_bytes(),
        );
        assert!(!shown.contains("a4_pk_abcdefghijkl"), "{shown}");
        assert!(!shown.contains("eyJhbGciOi"), "{shown}");
        assert!(!shown.contains("plain"), "{shown}");
        assert!(!shown.contains("a4_sk_0123456789abcdef"), "{shown}");
        assert!(shown.contains("\"user_id\": 3"), "{shown}");
        assert!(shown.contains("\"name\": \"web\""), "{shown}");

        // A body cut short is not JSON, and is redacted all the same.
        for truncated in [
            r#"{"id": 8, "key": "a4_sk_0123456789abcdef", "name": "web", "id":"#,
            r#"{"id": 8, "name": "web", "secretKey": "plain-secret-valu"#,
            r#"{"items": [{"note": "a4_pk_abcdefghijkl" ,"#,
            "upstream said: Authorization: Bearer abcdefghijklmnop.qrs",
            r#"{"session": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln", "#,
        ] {
            let shown = verbose_body(truncated.as_bytes());
            for secret in [
                "a4_sk_0123456789abcdef",
                "plain-secret-valu",
                "a4_pk_abcdefghijkl",
                "abcdefghijklmnop",
                "eyJhbGciOiJIUzI1NiJ9",
            ] {
                assert!(!shown.contains(secret), "{truncated} -> {shown}");
            }
            assert!(shown.contains("<redacted>"), "{truncated} -> {shown}");
        }
        let shown = verbose_body(br#"{"id": 8, "name": "web", "#);
        assert_eq!(shown, r#"{"id": 8, "name": "web", "#);

        let long = "x".repeat(MAX_VERBOSE_BODY + 10);
        let shown = verbose_body(long.as_bytes());
        assert!(shown.ends_with("(10 more characters not shown)"), "{shown}");
    }

    #[test]
    fn endpoints_drop_credentials_query_and_fragment() {
        let url = url::Url::parse("https://user:pw@api.example:8443/api/x?q=secret#frag").unwrap();
        assert_eq!(endpoint(&url), "https://api.example:8443/api/x");
    }

    /// A base URL nothing listens on, so connecting fails at once.
    fn closed_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    fn keyed(base_url: &str) -> ApiClient {
        ApiClient::with_base_url(base_url).with_api_key("a4_sk_test".to_string())
    }

    #[test]
    fn a_failed_read_names_its_operation_endpoint_and_a_retry() {
        let base = closed_base_url();
        let error = keyed(&base)
            .knowledge_search(Some("lending yield rebalance"), None, None, Some(3))
            .unwrap_err();
        let transport = error.downcast_ref::<ApiTransportError>().unwrap();
        assert_eq!(transport.failure(), TransportFailure::Connect);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "Knowledge search request could not connect to GET {base}/api/registry/knowledge/search: "
            )),
            "{message}"
        );
        assert!(message.contains("then retry"), "{message}");
        // The query string (search terms, filters) stays out of the message.
        assert!(!message.contains("lending"), "{message}");
        assert!(!message.contains("Failed to send"), "{message}");
    }

    #[test]
    fn reading_a_stack_is_a_lookup_not_an_install() {
        let error = keyed(&closed_base_url())
            .get_registry_stack_install("lending-rates", None)
            .unwrap_err();
        let message = error.to_string();
        assert!(message.starts_with("Stack lookup request"), "{message}");
        assert!(!message.contains("install request"), "{message}");

        let error = keyed(&closed_base_url())
            .list_deployments_page(100, 0)
            .unwrap_err();
        assert!(
            error.to_string().starts_with("List deployments request"),
            "{error}"
        );
    }

    #[test]
    fn a_timeout_says_how_long_it_waited() {
        let server = MockServer::json_delayed(200, "[]", Duration::from_secs(1));
        let error = reqwest::blocking::Client::new()
            .get(format!("{}/api/registry", server.base_url()))
            .timeout(Duration::from_millis(50))
            .send_api("registry list")
            .unwrap_err();
        let transport = error.downcast_ref::<ApiTransportError>().unwrap();
        assert_eq!(transport.failure(), TransportFailure::Timeout);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "Registry list request timed out: GET {}/api/registry sent no response within 0.",
                server.base_url()
            )),
            "{message}"
        );
        assert!(message.contains("Retry in a moment"), "{message}");
    }

    #[test]
    fn a_body_timeout_names_the_request_and_the_whole_wait() {
        // Headers arrive after 200ms; the body never completes before the
        // deadline, so the failure happens while it is read.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_millis(200));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n[",
            );
            let _ = stream.flush();
            std::thread::sleep(Duration::from_millis(1500));
        });
        let response = reqwest::blocking::Client::new()
            .get(format!("{base}/api/deployments?limit=100"))
            .timeout(Duration::from_millis(300))
            .send_api("list deployments")
            .unwrap();
        let error = ResponseBody::read(response).err().unwrap();
        server.join().unwrap();
        let transport = error.downcast_ref::<ApiTransportError>().unwrap();
        assert_eq!(transport.failure(), TransportFailure::Timeout);
        let message = error.to_string();
        let prefix = format!(
            "List deployments request timed out: GET {base}/api/deployments sent no response within "
        );
        assert!(message.starts_with(&prefix), "{message}");
        // The wait counts from when the request was sent, not from when the
        // headers arrived.
        let waited: f64 = message[prefix.len()..]
            .split('s')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(waited >= 0.3, "{message}");
    }

    #[test]
    fn a_response_of_another_shape_is_reported_as_version_skew() {
        let server = MockServer::json(200, r#"[{"id": 1, "name": "ore"}]"#);
        let error = keyed(server.base_url()).list_specs().unwrap_err();
        let response = error.downcast_ref::<ApiResponseError>().unwrap();
        assert!(response.is_version_skew());
        let message = error.to_string();
        assert!(message.contains("/api/specs (200)"), "{message}");
        assert!(message.contains("a4 self update"), "{message}");
        assert!(
            !message.contains("Failed to parse response JSON"),
            "{message}"
        );
    }

    #[test]
    fn error_reports_surface_api_failures_under_caller_context() {
        let error = keyed(&closed_base_url())
            .get_registry_stack_install("lending-rates", None)
            .context("Unable to assemble the install descriptor for stack 'lending-rates'")
            .unwrap_err();

        let report = error_report(&error, false);
        let mut lines = report.lines();
        assert_eq!(
            lines.next(),
            Some("Unable to assemble the install descriptor for stack 'lending-rates'")
        );
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("Stack lookup request could not connect to GET "),
            "{report}"
        );

        // --verbose prints every cause, down to the socket error.
        let verbose = error_report(&error, true);
        assert!(
            verbose.contains("\n  caused by: Stack lookup request"),
            "{verbose}"
        );
        assert!(verbose.matches("\n  caused by: ").count() >= 3, "{verbose}");

        // Errors without an API failure underneath print as before.
        let plain = anyhow::anyhow!("inner").context("outer");
        assert_eq!(error_report(&plain, false), "outer");
        assert_eq!(error_report(&plain, true), "outer\n  caused by: inner");
    }
}
