use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Authentication metrics for observability
#[derive(Debug, Default)]
pub struct AuthMetrics {
    /// Total authentication attempts
    total_attempts: AtomicU64,
    /// Successful authentications
    success_count: AtomicU64,
    /// Failed authentications by error code
    failure_counts: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    /// JWKS fetch count
    jwks_fetch_count: AtomicU64,
    /// JWKS fetch latency in microseconds (last value)
    jwks_fetch_latency_us: AtomicU64,
    /// JWKS fetch failures
    jwks_fetch_failures: AtomicU64,
    /// Token verification latency in microseconds (last value)
    verification_latency_us: AtomicU64,
}

impl AuthMetrics {
    /// Create new auth metrics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an authentication attempt
    pub fn record_attempt(&self) {
        self.total_attempts.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a successful authentication
    pub fn record_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a failed authentication
    pub fn record_failure(&self, error_code: &crate::AuthErrorCode) {
        let mut counts = self.failure_counts.lock().unwrap();
        *counts.entry(error_code.to_string()).or_insert(0) += 1;
    }

    /// Record JWKS fetch with latency
    pub fn record_jwks_fetch(&self, latency: std::time::Duration, success: bool) {
        self.jwks_fetch_count.fetch_add(1, Ordering::Relaxed);
        self.jwks_fetch_latency_us
            .store(latency.as_micros() as u64, Ordering::Relaxed);
        if !success {
            self.jwks_fetch_failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record token verification latency
    pub fn record_verification_latency(&self, latency: std::time::Duration) {
        self.verification_latency_us
            .store(latency.as_micros() as u64, Ordering::Relaxed);
    }

    /// Get total attempts
    pub fn total_attempts(&self) -> u64 {
        self.total_attempts.load(Ordering::Relaxed)
    }

    /// Get success count
    pub fn success_count(&self) -> u64 {
        self.success_count.load(Ordering::Relaxed)
    }

    /// Get success rate (0.0 - 1.0)
    pub fn success_rate(&self) -> f64 {
        let total = self.total_attempts();
        if total == 0 {
            0.0
        } else {
            self.success_count() as f64 / total as f64
        }
    }

    /// Get failure counts by error code
    pub fn failure_counts(&self) -> std::collections::HashMap<String, u64> {
        self.failure_counts.lock().unwrap().clone()
    }

    /// Get JWKS fetch count
    pub fn jwks_fetch_count(&self) -> u64 {
        self.jwks_fetch_count.load(Ordering::Relaxed)
    }

    /// Get JWKS fetch latency in microseconds
    pub fn jwks_fetch_latency_us(&self) -> u64 {
        self.jwks_fetch_latency_us.load(Ordering::Relaxed)
    }

    /// Get number of JWKS fetch failures
    pub fn jwks_fetch_failures(&self) -> u64 {
        self.jwks_fetch_failures.load(Ordering::Relaxed)
    }

    /// Get verification latency in microseconds
    pub fn verification_latency_us(&self) -> u64 {
        self.verification_latency_us.load(Ordering::Relaxed)
    }

    /// Get metrics as a serializable snapshot
    pub fn snapshot(&self) -> AuthMetricsSnapshot {
        AuthMetricsSnapshot {
            total_attempts: self.total_attempts(),
            success_count: self.success_count(),
            success_rate: self.success_rate(),
            failure_counts: self.failure_counts(),
            jwks_fetch_count: self.jwks_fetch_count(),
            jwks_fetch_latency_us: self.jwks_fetch_latency_us(),
            jwks_fetch_failures: self.jwks_fetch_failures(),
            verification_latency_us: self.verification_latency_us(),
        }
    }
}

/// Serializable snapshot of auth metrics
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthMetricsSnapshot {
    pub total_attempts: u64,
    pub success_count: u64,
    pub success_rate: f64,
    pub failure_counts: std::collections::HashMap<String, u64>,
    pub jwks_fetch_count: u64,
    pub jwks_fetch_latency_us: u64,
    pub jwks_fetch_failures: u64,
    pub verification_latency_us: u64,
}

/// Trait for collecting auth metrics
pub trait AuthMetricsCollector: Send + Sync {
    /// Get auth metrics
    fn metrics(&self) -> Option<&AuthMetrics> {
        None
    }

    /// Record an authentication attempt with metrics
    fn record_auth_attempt(&self, success: bool, error_code: Option<&crate::AuthErrorCode>) {
        if let Some(metrics) = self.metrics() {
            metrics.record_attempt();
            if success {
                metrics.record_success();
            } else if let Some(code) = error_code {
                metrics.record_failure(code);
            }
        }
    }

    /// Time a JWKS fetch operation
    fn time_jwks_fetch<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = f();
        if let Some(metrics) = self.metrics() {
            metrics.record_jwks_fetch(start.elapsed(), true);
        }
        result
    }

    /// Time a token verification operation
    fn time_verification<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = f();
        if let Some(metrics) = self.metrics() {
            metrics.record_verification_latency(start.elapsed());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_metrics() {
        let metrics = AuthMetrics::new();

        metrics.record_attempt();
        metrics.record_success();

        metrics.record_attempt();
        metrics.record_failure(&crate::AuthErrorCode::TokenExpired);

        assert_eq!(metrics.total_attempts(), 2);
        assert_eq!(metrics.success_count(), 1);
        assert_eq!(metrics.success_rate(), 0.5);

        let failures = metrics.failure_counts();
        assert_eq!(failures.get("token-expired"), Some(&1));
    }

    #[test]
    fn test_metrics_snapshot() {
        let metrics = AuthMetrics::new();
        metrics.record_attempt();
        metrics.record_success();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_attempts, 1);
        assert_eq!(snapshot.success_count, 1);
        assert_eq!(snapshot.success_rate, 1.0);
    }
}
