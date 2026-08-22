//! Shared account policy state for runtime enforcement.
//!
//! A signed session token carries a monotonic `policy_version` and an
//! aggregate `account_limits` object for its billing account. Each runtime
//! process keeps one [`AccountPolicyRegistry`] per enforcement surface and
//! observes every non-legacy token at admission time:
//!
//! - the first token for an account creates state;
//! - a higher policy version atomically replaces the stored limits;
//! - a lower policy version is rejected as stale once a newer version has
//!   been observed;
//! - the same version with different limits is rejected and logged as a
//!   signing/configuration fault.
//!
//! A downgrade affects new admissions immediately but never kills an
//! existing socket; normal token refresh/expiry drains it. Entries are
//! bounded and evicted after an idle TTL so signed-but-idle accounts cannot
//! grow the map forever.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use arete_auth::Limits;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;

/// Default bound on tracked account policy entries per registry.
pub const DEFAULT_MAX_TRACKED_ACCOUNTS: usize = 100_000;

/// Default idle TTL after which unused account state is evicted.
pub const DEFAULT_ACCOUNT_IDLE_TTL: Duration = Duration::from_secs(15 * 60);

/// Hash an account/consumer identity for routine logs.
///
/// Raw identities must not appear in routine logs; this produces a stable
/// low-cardinality token suitable for correlation.
pub fn redact_identity(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Failure to admit a token against previously observed account policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountPolicyError {
    /// The token carries an older policy version than this runtime has seen.
    #[error("token policy version {presented} is stale; runtime has observed {current}")]
    StaleVersion { presented: u32, current: u32 },
    /// The token repeats an observed policy version with different limits.
    #[error("token policy version {version} conflicts with previously observed limits")]
    ConflictingLimits { version: u32 },
    /// The registry is full and no idle entry could be evicted.
    #[error("account policy state is at capacity")]
    CapacityExhausted,
}

#[derive(Debug, Clone)]
struct AccountPolicyEntry {
    version: u32,
    limits: Limits,
    last_seen: Instant,
}

/// Bounded per-process registry of account policy versions and limits.
#[derive(Debug)]
pub struct AccountPolicyRegistry {
    entries: DashMap<String, AccountPolicyEntry>,
    max_entries: usize,
    idle_ttl: Duration,
    legacy_tokens: AtomicU64,
}

impl Default for AccountPolicyRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_TRACKED_ACCOUNTS, DEFAULT_ACCOUNT_IDLE_TTL)
    }
}

impl AccountPolicyRegistry {
    /// Create a registry bounded to `max_entries` with the given idle TTL.
    pub fn new(max_entries: usize, idle_ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            max_entries,
            idle_ttl,
            legacy_tokens: AtomicU64::new(0),
        }
    }

    /// Apply the policy-version rules to an entry already held under lock.
    fn apply(
        entry: &mut AccountPolicyEntry,
        version: u32,
        limits: &Limits,
    ) -> Result<(), AccountPolicyError> {
        entry.last_seen = Instant::now();
        match version.cmp(&entry.version) {
            std::cmp::Ordering::Greater => {
                entry.version = version;
                entry.limits = limits.clone();
                Ok(())
            }
            std::cmp::Ordering::Less => Err(AccountPolicyError::StaleVersion {
                presented: version,
                current: entry.version,
            }),
            std::cmp::Ordering::Equal => {
                if &entry.limits == limits {
                    Ok(())
                } else {
                    Err(AccountPolicyError::ConflictingLimits { version })
                }
            }
        }
    }

    /// Observe a signed (account, policy version, account limits) tuple and
    /// apply the policy-version conflict rules.
    pub fn observe(
        &self,
        account: &str,
        version: u32,
        limits: &Limits,
    ) -> Result<(), AccountPolicyError> {
        // Fast path: an existing entry is updated under its own lock, and
        // avoids allocating the owned key the entry API needs.
        if let Some(mut entry) = self.entries.get_mut(account) {
            return Self::apply(&mut entry, version, limits);
        }

        // Make room before taking an entry lock: `evict_idle` retains over
        // the whole map and must not run while a lock is held.
        if self.entries.len() >= self.max_entries {
            self.evict_idle(|_| false);
            if self.entries.len() >= self.max_entries {
                return Err(AccountPolicyError::CapacityExhausted);
            }
        }

        // Re-check under the entry lock. Two first admissions for one account
        // can both miss the fast path; without this the later insert would
        // clobber a newer version with an older one and leave stale limits
        // registered. The capacity bound above is a backstop, so overshooting
        // it by the number of racing threads is acceptable.
        match self.entries.entry(account.to_string()) {
            Entry::Occupied(mut occupied) => Self::apply(occupied.get_mut(), version, limits),
            Entry::Vacant(vacant) => {
                vacant.insert(AccountPolicyEntry {
                    version,
                    limits: limits.clone(),
                    last_seen: Instant::now(),
                });
                Ok(())
            }
        }
    }

    /// Stored limits for an account, if this runtime has observed one.
    pub fn limits_for(&self, account: &str) -> Option<Limits> {
        self.entries.get(account).map(|entry| entry.limits.clone())
    }

    /// Record one legacy (pre-v2) token observation and return the total.
    pub fn record_legacy_token(&self) -> u64 {
        self.legacy_tokens.fetch_add(1, AtomicOrdering::Relaxed) + 1
    }

    /// Total legacy tokens observed since process start.
    pub fn legacy_token_count(&self) -> u64 {
        self.legacy_tokens.load(AtomicOrdering::Relaxed)
    }

    /// Number of tracked accounts.
    pub fn tracked_accounts(&self) -> usize {
        self.entries.len()
    }

    /// Drop entries that are idle past the TTL and not reported live.
    pub fn evict_idle(&self, is_live: impl Fn(&str) -> bool) {
        let ttl = self.idle_ttl;
        self.entries
            .retain(|account, entry| is_live(account) || entry.last_seen.elapsed() < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_connections: u32) -> Limits {
        Limits {
            max_connections: Some(max_connections),
            ..Limits::default()
        }
    }

    #[test]
    fn version_upgrade_replaces_limits_and_stale_or_conflicting_tokens_reject() {
        let registry = AccountPolicyRegistry::default();

        registry.observe("account:1", 1, &limits(5)).unwrap();
        assert_eq!(registry.limits_for("account:1"), Some(limits(5)));

        // Same version, same limits: fine.
        registry.observe("account:1", 1, &limits(5)).unwrap();

        // Higher version replaces the limits atomically.
        registry.observe("account:1", 3, &limits(2)).unwrap();
        assert_eq!(registry.limits_for("account:1"), Some(limits(2)));

        // Lower version is stale once a newer version has been observed.
        assert_eq!(
            registry.observe("account:1", 2, &limits(9)),
            Err(AccountPolicyError::StaleVersion {
                presented: 2,
                current: 3
            })
        );

        // Same version with different limits is a signing/config fault.
        assert_eq!(
            registry.observe("account:1", 3, &limits(9)),
            Err(AccountPolicyError::ConflictingLimits { version: 3 })
        );

        // Other accounts are unaffected.
        registry.observe("account:2", 1, &limits(1)).unwrap();
    }

    #[test]
    fn concurrent_first_admissions_never_register_an_older_version() {
        use std::sync::Arc;

        // Racing first admissions for one account: every thread misses the
        // fast path, so the insert must not clobber a newer version.
        for _ in 0..64 {
            let registry = Arc::new(AccountPolicyRegistry::default());
            let threads: Vec<_> = (1..=8u32)
                .map(|version| {
                    let registry = Arc::clone(&registry);
                    std::thread::spawn(move || {
                        // Limits vary with the version so a clobber is visible.
                        let _ = registry.observe("account:1", version, &limits(version));
                    })
                })
                .collect();
            for thread in threads {
                thread.join().expect("observer thread");
            }

            // The highest version always lands, and nothing may downgrade it.
            assert_eq!(
                registry.limits_for("account:1"),
                Some(limits(8)),
                "a lower policy version overwrote a newer one"
            );
            assert_eq!(
                registry.observe("account:1", 7, &limits(7)),
                Err(AccountPolicyError::StaleVersion {
                    presented: 7,
                    current: 8
                })
            );
        }
    }

    #[test]
    fn idle_entries_evict_and_capacity_is_bounded() {
        let registry = AccountPolicyRegistry::new(2, Duration::from_secs(0));
        registry.observe("account:1", 1, &limits(1)).unwrap();
        registry.observe("account:2", 1, &limits(1)).unwrap();

        // At capacity with a zero TTL: the idle entries are evicted to make
        // room instead of failing.
        registry.observe("account:3", 1, &limits(1)).unwrap();
        assert!(registry.tracked_accounts() <= 2);

        // With every entry live, capacity is a hard bound.
        let full = AccountPolicyRegistry::new(1, Duration::from_secs(3600));
        full.observe("account:1", 1, &limits(1)).unwrap();
        assert_eq!(
            full.observe("account:2", 1, &limits(1)),
            Err(AccountPolicyError::CapacityExhausted)
        );

        full.evict_idle(|_| false);
        assert_eq!(full.tracked_accounts(), 1, "live TTL keeps entries");
    }

    #[test]
    fn legacy_counter_accumulates() {
        let registry = AccountPolicyRegistry::default();
        assert_eq!(registry.legacy_token_count(), 0);
        assert_eq!(registry.record_legacy_token(), 1);
        assert_eq!(registry.record_legacy_token(), 2);
        assert_eq!(registry.legacy_token_count(), 2);
    }

    #[test]
    fn redacted_identities_are_stable_and_not_raw() {
        let redacted = redact_identity("account:42");
        assert_eq!(redacted, redact_identity("account:42"));
        assert_ne!(redacted, "account:42");
        assert_eq!(redacted.len(), 16);
        assert!(redacted.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
