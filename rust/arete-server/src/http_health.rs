use crate::health::HealthMonitor;
use anyhow::Result;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

/// Configuration for the HTTP health server
#[derive(Clone, Debug)]
pub struct HttpHealthConfig {
    pub bind_address: SocketAddr,
}

impl Default for HttpHealthConfig {
    fn default() -> Self {
        Self {
            bind_address: "[::]:8081".parse().expect("valid socket address"),
        }
    }
}

impl HttpHealthConfig {
    pub fn new(bind_address: impl Into<SocketAddr>) -> Self {
        Self {
            bind_address: bind_address.into(),
        }
    }
}

/// HTTP server that exposes health endpoints
pub struct HttpHealthServer {
    bind_addr: SocketAddr,
    health_monitor: Option<HealthMonitor>,
}

impl HttpHealthServer {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            health_monitor: None,
        }
    }

    pub fn with_health_monitor(mut self, monitor: HealthMonitor) -> Self {
        self.health_monitor = Some(monitor);
        self
    }

    pub async fn start(self) -> Result<()> {
        info!("Starting HTTP health server on {}", self.bind_addr);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("HTTP health server listening on {}", self.bind_addr);

        let health_monitor = Arc::new(self.health_monitor);

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let io = TokioIo::new(stream);
                    let monitor = health_monitor.clone();

                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let monitor = monitor.clone();
                            async move { handle_request(req, monitor).await }
                        });

                        if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                            error!("HTTP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept HTTP connection: {}", e);
                }
            }
        }
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    health_monitor: Arc<Option<HealthMonitor>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path();

    match path {
        "/health" | "/healthz" => {
            // Basic health check - server is running
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Full::new(Bytes::from("OK")))
                .unwrap())
        }
        "/ready" | "/readiness" => {
            // Readiness check - check if stream is healthy
            if let Some(monitor) = health_monitor.as_ref() {
                if monitor.is_healthy().await {
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/plain")
                        .body(Full::new(Bytes::from("READY")))
                        .unwrap())
                } else {
                    Ok(Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header("Content-Type", "text/plain")
                        .body(Full::new(Bytes::from("NOT READY")))
                        .unwrap())
                }
            } else {
                // No health monitor configured, assume ready
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain")
                    .body(Full::new(Bytes::from("READY")))
                    .unwrap())
            }
        }
        "/status" => {
            // Detailed status endpoint
            if let Some(monitor) = health_monitor.as_ref() {
                let status = monitor.status().await;
                let error_count = monitor.error_count().await;
                let is_healthy = monitor.is_healthy().await;

                let status_json = serde_json::json!({
                    "healthy": is_healthy,
                    "status": format!("{:?}", status),
                    "error_count": error_count
                });

                let status_code = if is_healthy {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };

                Ok(Response::builder()
                    .status(status_code)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(status_json.to_string())))
                    .unwrap())
            } else {
                let status_json = serde_json::json!({
                    "healthy": true,
                    "status": "no_monitor",
                    "error_count": 0
                });

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(status_json.to_string())))
                    .unwrap())
            }
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()),
    }
}
