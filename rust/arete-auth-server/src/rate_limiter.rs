use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Simple in-memory sliding-window rate limiter for the reference auth server.
///
/// This is intentionally process-local. It gives self-hosters a real default
/// limiter without introducing Redis or other shared infrastructure.
pub struct MintRateLimiter {
    window: Duration,
    buckets: Mutex<HashMap<String, Vec<Instant>>>,
}

impl MintRateLimiter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub fn check(&self, key: &str, limit: u32) -> bool {
        self.check_all(&[(key, limit)])
    }

    /// Record one request against every `(bucket, limit)` pair, or none of
    /// them if any bucket is already full.
    pub fn check_all(&self, buckets_to_check: &[(&str, u32)]) -> bool {
        let now = Instant::now();
        let mut buckets = self
            .buckets
            .lock()
            .expect("mint rate limiter lock poisoned");

        // Keep this simple in-memory limiter bounded without a background task.
        buckets.retain(|_, bucket| {
            bucket.retain(|instant| now.duration_since(*instant) < self.window);
            !bucket.is_empty()
        });

        let full = buckets_to_check
            .iter()
            .any(|(key, limit)| buckets.get(*key).map_or(0, Vec::len) >= *limit as usize);
        if full {
            return false;
        }

        for (key, _) in buckets_to_check {
            buckets.entry((*key).to_string()).or_default().push(now);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_until_limit_then_denies() {
        let limiter = MintRateLimiter::new(Duration::from_secs(60));
        assert!(limiter.check("key", 2));
        assert!(limiter.check("key", 2));
        assert!(!limiter.check("key", 2));
    }

    #[test]
    fn tracks_keys_independently() {
        let limiter = MintRateLimiter::new(Duration::from_secs(60));
        assert!(limiter.check("key-a", 1));
        assert!(!limiter.check("key-a", 1));
        assert!(limiter.check("key-b", 1));
    }

    #[test]
    fn check_all_consumes_nothing_when_any_bucket_is_full() {
        let limiter = MintRateLimiter::new(Duration::from_secs(60));
        assert!(limiter.check_all(&[("ip", 5), ("key", 1)]));
        assert!(!limiter.check_all(&[("ip", 5), ("key", 1)]));

        let buckets = limiter
            .buckets
            .lock()
            .expect("mint rate limiter lock poisoned");
        assert_eq!(buckets.get("ip").map(Vec::len), Some(1));
    }

    #[test]
    fn zero_limit_denies() {
        let limiter = MintRateLimiter::new(Duration::from_secs(60));
        assert!(!limiter.check_all(&[("key", 0)]));
    }

    #[test]
    fn prunes_stale_buckets_on_check() {
        let limiter = MintRateLimiter::new(Duration::from_secs(60));
        let stale = Instant::now() - Duration::from_secs(120);

        {
            let mut buckets = limiter
                .buckets
                .lock()
                .expect("mint rate limiter lock poisoned");
            buckets.insert("stale".to_string(), vec![stale]);
        }

        assert!(limiter.check("fresh", 1));

        let buckets = limiter
            .buckets
            .lock()
            .expect("mint rate limiter lock poisoned");
        assert!(!buckets.contains_key("stale"));
        assert_eq!(buckets.get("fresh").map(Vec::len), Some(1));
    }
}
