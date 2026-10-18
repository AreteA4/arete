//! The UI's static files: built into `a4` (the Vite build of
//! `typescript/dev-ui`, gzipped by `build.rs`), or proxied from a Vite dev
//! server with `--ui-dev-url` (UI development; its hot-reload socket
//! connects to Vite directly). An `a4` built without the UI serves a
//! placeholder page.

use std::io::Read as _;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{Html, IntoResponse, Response};

use super::UiState;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/dev_ui_assets.rs"));
}

/// Whether this `a4` has the dev UI built in.
pub fn built() -> bool {
    embedded::BUILT
}

/// The embedded file for a request path: the file itself, else (a client
/// route) `index.html`. Paths with an extension that match nothing are 404.
pub fn lookup(path: &str) -> Option<(&'static str, &'static [u8])> {
    let find = |wanted: &str| {
        embedded::ASSETS
            .iter()
            .find(|(path, _, _)| *path == wanted)
            .map(|(_, mime, bytes)| (*mime, *bytes))
    };
    if let Some(found) = find(path) {
        return Some(found);
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return None;
    }
    find("/index.html")
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim().starts_with("gzip")))
}

fn embedded_response(path: &str, headers: &HeaderMap) -> Option<Response> {
    let (mime, gzipped) = lookup(path)?;
    let mut response = if accepts_gzip(headers) {
        let mut response = Response::new(Body::from(gzipped));
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        response
    } else {
        let mut plain = Vec::new();
        flate2::read::GzDecoder::new(gzipped)
            .read_to_end(&mut plain)
            .ok()?;
        Response::new(Body::from(plain))
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    if mime.starts_with("text/html") {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'; object-src 'none'; base-uri 'none'"),
        );
    }
    Some(response)
}

const PLACEHOLDER: &str = "<!doctype html><meta charset=utf-8><title>a4 dev</title><body style=\"font-family:system-ui;margin:3rem\"><h1>a4 dev</h1><p>This a4 build has no dev UI. The event feed is at <a href=\"/dev/v1/events\">/dev/v1/events</a> and the protocol at <a href=\"/dev/v1/protocol\">/dev/v1/protocol</a>.</p>";

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"error\":\"Not found\",\"code\":\"dev-ui-not-found\"}",
    )
        .into_response()
}

pub async fn serve(State(state): State<Arc<UiState>>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    if path.starts_with("/dev/") {
        return not_found();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "").into_response();
    }
    if let Some(vite) = &state.ui_dev_url {
        return proxy(&state, vite, request).await;
    }
    if built() {
        return embedded_response(&path, request.headers()).unwrap_or_else(not_found);
    }
    Html(PLACEHOLDER).into_response()
}

/// Forward a GET to the Vite dev server.
async fn proxy(state: &UiState, vite: &str, request: Request) -> Response {
    let target = format!(
        "{vite}{}",
        request
            .uri()
            .path_and_query()
            .map(|path| path.as_str())
            .unwrap_or("/")
    );
    let upstream = match state.http.get(&target).send().await {
        Ok(upstream) => upstream,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("The Vite dev server at {vite} is unreachable: {error}"),
            )
                .into_response()
        }
    };
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = upstream.bytes().await.unwrap_or_default();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| not_found())
}
