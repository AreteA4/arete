use axum::{body::Body, http::Request, middleware::Next, response::Response};

/// Request logging middleware
#[allow(dead_code)]
pub async fn logging_middleware(req: Request<Body>, next: Next) -> Response {
    let start = std::time::Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();

    let response = next.run(req).await;

    let duration = start.elapsed();
    let status = response.status();

    tracing::info!("{} {} - {} in {:?}", method, uri, status.as_u16(), duration);

    response
}

/// Rate limiting middleware (placeholder for now)
///
/// In production, this would use a proper rate limiter like governor
#[allow(dead_code)]
pub async fn rate_limit_middleware(req: Request<Body>, next: Next) -> Response {
    // For now, just pass through
    // In production, check API key rate limits here
    next.run(req).await
}
