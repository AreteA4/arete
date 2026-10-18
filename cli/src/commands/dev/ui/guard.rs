//! Who may talk to the dev UI server.
//!
//! The server listens on 127.0.0.1 only, but a browser on this machine also
//! runs pages from anywhere. Those pages must not drive it: the server signs
//! with dev wallets and calls the Arete API with the account's key. So every
//! request passes [`check`]:
//!
//! - **Host** must name the server itself (`127.0.0.1:<port>`,
//!   `localhost:<port>` or `[::1]:<port>`). A page on a rebound DNS name
//!   (`evil.example` resolving to 127.0.0.1) sends its own name: refused.
//! - **Origin**, when the browser sends one (every cross-origin request,
//!   every `POST`), must be the server's own origin (or the `--ui-dev-url`
//!   Vite server's). `null` and other origins are refused.
//! - **Sec-Fetch-Site** `cross-site` or `same-site` is refused on the API
//!   (`/dev/v1/*`): that catches requests without an `Origin`, such as a
//!   `<script src>` from another local port. Static UI files are exempt.
//! - Writes (`POST`, `PUT`, `PATCH`, `DELETE`) need `Content-Type:
//!   application/json`, which a cross-site page can only send after a CORS
//!   preflight; the server answers no preflight and sets no CORS headers.
//!
//! Requests without browser headers (curl, the MCP server, an agent) pass:
//! anything on this machine that can send them can read the session file
//! anyway.

use axum::http::{header, HeaderMap, Method};

/// The request's refusal: an HTTP status and a stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

/// What [`check`] allows.
#[derive(Debug, Clone)]
pub struct Policy {
    hosts: Vec<String>,
    origins: Vec<String>,
}

impl Policy {
    /// The server on `127.0.0.1:<port>`; `extra_origins` (the Vite dev
    /// server of `--ui-dev-url`) may also call it.
    pub fn new(port: u16, extra_origins: &[String]) -> Self {
        let hosts = vec![
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ];
        let mut origins: Vec<String> = hosts.iter().map(|host| format!("http://{host}")).collect();
        origins.extend(
            extra_origins
                .iter()
                .map(|origin| origin.trim_end_matches('/').to_ascii_lowercase()),
        );
        Self { hosts, origins }
    }

    /// The origin the UI is served from.
    pub fn origin(&self) -> &str {
        &self.origins[0]
    }
}

fn refuse(status: u16, code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        status,
        code,
        message: message.into(),
    }
}

fn header_str(headers: &HeaderMap, name: impl header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Allow or refuse one request.
pub fn check(
    policy: &Policy,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Result<(), Refusal> {
    let host = header_str(headers, header::HOST)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !policy.hosts.contains(&host) {
        return Err(refuse(
            421,
            "dev-ui-host-refused",
            format!("The dev UI only answers as {}", policy.hosts[0]),
        ));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or("null").to_ascii_lowercase();
        if !policy.origins.contains(&origin) {
            return Err(refuse(
                403,
                "dev-ui-origin-refused",
                format!("Requests from {origin} may not use the a4 dev UI server"),
            ));
        }
    }
    let api = path.starts_with("/dev/");
    if api {
        if let Some(site) = header_str(headers, "sec-fetch-site") {
            if matches!(site, "cross-site" | "same-site") {
                return Err(refuse(
                    403,
                    "dev-ui-cross-site-refused",
                    "Cross-site requests may not use the a4 dev UI server",
                ));
            }
        }
    }
    let writes = matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if writes {
        let json = header_str(headers, header::CONTENT_TYPE)
            .and_then(|value| value.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return Err(refuse(
                415,
                "dev-ui-json-required",
                "Writes take Content-Type: application/json",
            ));
        }
    }
    if *method == Method::OPTIONS {
        return Err(refuse(
            405,
            "dev-ui-preflight-refused",
            "The a4 dev UI server answers no CORS preflight",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn code(result: Result<(), Refusal>) -> Option<&'static str> {
        result.err().map(|refusal| refusal.code)
    }

    #[test]
    fn allows_the_ui_its_own_origin_and_local_tools() {
        let policy = Policy::new(50106, &[]);
        let ui = headers(&[
            ("host", "127.0.0.1:50106"),
            ("origin", "http://127.0.0.1:50106"),
            ("sec-fetch-site", "same-origin"),
            ("content-type", "application/json"),
        ]);
        assert_eq!(
            code(check(&policy, &Method::POST, "/dev/v1/steps/run", &ui)),
            None
        );
        assert_eq!(
            code(check(&policy, &Method::GET, "/dev/v1/events", &ui)),
            None
        );
        // curl / the MCP server: no browser headers.
        let tool = headers(&[("host", "localhost:50106")]);
        assert_eq!(
            code(check(&policy, &Method::GET, "/dev/v1/events", &tool)),
            None
        );
        // A typed URL (Sec-Fetch-Site: none).
        let typed = headers(&[("host", "127.0.0.1:50106"), ("sec-fetch-site", "none")]);
        assert_eq!(code(check(&policy, &Method::GET, "/", &typed)), None);
    }

    #[test]
    fn refuses_dns_rebinding() {
        let policy = Policy::new(50106, &[]);
        for host in ["evil.example:50106", "127.0.0.1:50107", "127.0.0.1", ""] {
            let request = headers(&[("host", host)]);
            assert_eq!(
                code(check(&policy, &Method::GET, "/dev/v1/protocol", &request)),
                Some("dev-ui-host-refused"),
                "{host}"
            );
        }
        // Even for the static UI.
        let request = headers(&[("host", "evil.example:50106")]);
        assert_eq!(
            code(check(&policy, &Method::GET, "/", &request)),
            Some("dev-ui-host-refused")
        );
    }

    #[test]
    fn refuses_other_origins_and_cross_site_requests() {
        let policy = Policy::new(50106, &[]);
        for origin in ["https://evil.example", "http://localhost:3000", "null"] {
            let request = headers(&[
                ("host", "127.0.0.1:50106"),
                ("origin", origin),
                ("content-type", "application/json"),
            ]);
            assert_eq!(
                code(check(&policy, &Method::POST, "/dev/v1/steps/run", &request)),
                Some("dev-ui-origin-refused"),
                "{origin}"
            );
        }
        // A no-cors GET from another local port carries no Origin.
        for site in ["cross-site", "same-site"] {
            let request = headers(&[("host", "127.0.0.1:50106"), ("sec-fetch-site", site)]);
            assert_eq!(
                code(check(&policy, &Method::GET, "/dev/v1/events", &request)),
                Some("dev-ui-cross-site-refused")
            );
            // The static UI files are not secret.
            assert_eq!(
                code(check(&policy, &Method::GET, "/assets/app.js", &request)),
                None
            );
        }
    }

    #[test]
    fn writes_need_json_and_preflights_are_refused() {
        let policy = Policy::new(50106, &[]);
        let form = headers(&[
            ("host", "127.0.0.1:50106"),
            ("content-type", "application/x-www-form-urlencoded"),
        ]);
        assert_eq!(
            code(check(&policy, &Method::POST, "/dev/v1/time-travel", &form)),
            Some("dev-ui-json-required")
        );
        let bare = headers(&[("host", "127.0.0.1:50106")]);
        assert_eq!(
            code(check(&policy, &Method::POST, "/dev/v1/time-travel", &bare)),
            Some("dev-ui-json-required")
        );
        let json = headers(&[
            ("host", "127.0.0.1:50106"),
            ("content-type", "application/json; charset=utf-8"),
        ]);
        assert_eq!(
            code(check(&policy, &Method::POST, "/dev/v1/time-travel", &json)),
            None
        );
        assert_eq!(
            code(check(
                &policy,
                &Method::OPTIONS,
                "/dev/v1/time-travel",
                &bare
            )),
            Some("dev-ui-preflight-refused")
        );
    }

    #[test]
    fn allows_the_vite_dev_server_origin() {
        let policy = Policy::new(50106, &["http://localhost:5173/".to_string()]);
        let request = headers(&[
            ("host", "127.0.0.1:50106"),
            ("origin", "http://localhost:5173"),
            ("content-type", "application/json"),
        ]);
        assert_eq!(
            code(check(&policy, &Method::POST, "/dev/v1/steps/run", &request)),
            None
        );
        assert_eq!(policy.origin(), "http://127.0.0.1:50106");
    }
}
