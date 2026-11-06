//! Outbound request guard for URL resolvers.
//!
//! URL resolver inputs come from stack definitions and on-chain data, so the
//! host they point at is untrusted. This module keeps those requests on the
//! public internet:
//!
//! - only `http` and `https` URLs are fetched;
//! - every address a hostname resolves to is checked before connecting, and
//!   the connection uses exactly the checked addresses, so a DNS answer that
//!   changes between check and connect cannot reach an internal address;
//! - IP-literal hosts are checked directly (they skip DNS);
//! - concurrent requests are capped overall and per host.
//!
//! The URL resolver client follows redirects itself, so every hop goes
//! through the same checks and takes its own host's concurrency slot.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use futures::future::BoxFuture;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::sync::{Semaphore, SemaphorePermit};

const ALLOWED_SCHEMES: &[&str] = &["http", "https"];
const HOST_LIMITER_STRIPES: usize = 64;

/// Returns true when `ip` is a publicly routable unicast address.
///
/// Loopback, private, shared (CGNAT), link-local (including cloud metadata
/// endpoints), unspecified, multicast, broadcast, documentation and other
/// special-purpose ranges are not public. IPv6 forms that embed an IPv4
/// address (mapped, compatible, NAT64, 6to4) are judged by that IPv4 address.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => is_public_ipv6(v6),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let special = a == 0 // "this network", 0.0.0.0/8
        || a == 10 // private
        || a == 127 // loopback
        || (a == 100 && (64..=127).contains(&b)) // shared address space, 100.64.0.0/10
        || (a == 169 && b == 254) // link-local, incl. 169.254.169.254
        || (a == 172 && (16..=31).contains(&b)) // private
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2) // documentation
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast
        || (a == 192 && b == 168) // private
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || (a == 198 && b == 51 && c == 100) // documentation
        || (a == 203 && b == 0 && c == 113) // documentation
        || a >= 224; // multicast, reserved, broadcast
    !special
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_ipv4(ip) {
        return is_public_ipv4(v4);
    }
    let s = ip.segments();
    // Only global unicast (2000::/3) is public, minus its special blocks.
    let global_unicast = (s[0] & 0xe000) == 0x2000;
    global_unicast
        && !(s[0] == 0x2001 && s[1] < 0x0200) // IETF protocol assignments, incl. Teredo
        && !(s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        && !(s[0] == 0x3fff && s[1] < 0x1000) // documentation
}

/// The IPv4 address carried by an IPv6 address, for the forms that route to
/// it: IPv4-mapped `::ffff:a.b.c.d`, IPv4-compatible `::a.b.c.d` (which also
/// covers `::` and `::1`), NAT64 `64:ff9b::a.b.c.d` and 6to4 `2002:aabb:ccdd::`.
fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let low =
        |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    match s {
        [0, 0, 0, 0, 0, 0xffff, hi, lo] | [0, 0, 0, 0, 0, 0, hi, lo] => Some(low(hi, lo)),
        [0x64, 0xff9b, 0, 0, 0, 0, hi, lo] => Some(low(hi, lo)),
        [0x2002, hi, lo, ..] => Some(low(hi, lo)),
        _ => None,
    }
}

/// The error returned when a URL resolver request is refused before it is sent.
#[derive(Debug)]
pub struct BlockedUrlError(pub(crate) String);

impl std::fmt::Display for BlockedUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "URL resolver request blocked: {}", self.0)
    }
}

impl std::error::Error for BlockedUrlError {}

/// Which destinations URL resolver requests may reach.
#[derive(Debug, Clone, Default)]
pub(crate) struct AddressPolicy {
    /// Skip the public-address check. For local development only.
    pub(crate) allow_private_networks: bool,
    /// Individual addresses exempt from the check, so tests can talk to a
    /// local server while every other private address stays blocked.
    pub(crate) exempt: Vec<IpAddr>,
}

impl AddressPolicy {
    pub(crate) fn permits(&self, ip: IpAddr) -> bool {
        self.allow_private_networks || is_public_ip(ip) || self.exempt.contains(&ip)
    }

    /// Checks the parts of a URL that can be judged without DNS: its scheme
    /// and, for IP-literal hosts, the address itself. Hostnames are checked by
    /// [`GuardedResolver`] when they are resolved.
    pub(crate) fn check_url(&self, url: &reqwest::Url) -> Result<(), BlockedUrlError> {
        if !ALLOWED_SCHEMES.contains(&url.scheme()) {
            return Err(BlockedUrlError(format!(
                "scheme '{}' is not allowed",
                url.scheme()
            )));
        }
        let host = url
            .host_str()
            .ok_or_else(|| BlockedUrlError("URL has no host".to_string()))?;
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = literal.parse::<IpAddr>() {
            if !self.permits(ip) {
                return Err(BlockedUrlError(format!("{ip} is not a public address")));
            }
        }
        Ok(())
    }
}

pub(crate) type LookupFn =
    Arc<dyn Fn(String) -> BoxFuture<'static, std::io::Result<Vec<IpAddr>>> + Send + Sync>;

/// Resolves with the system resolver.
pub(crate) fn system_lookup() -> LookupFn {
    Arc::new(|host: String| {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addrs.map(|addr| addr.ip()).collect())
        })
    })
}

/// A DNS resolver that refuses a hostname if any address it resolves to is
/// not permitted. The HTTP client connects only to the addresses returned
/// here, so the checked answer is the one used.
pub(crate) struct GuardedResolver {
    lookup: LookupFn,
    policy: Arc<AddressPolicy>,
}

impl GuardedResolver {
    pub(crate) fn new(lookup: LookupFn, policy: Arc<AddressPolicy>) -> Self {
        Self { lookup, policy }
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let lookup = self.lookup.clone();
        let policy = self.policy.clone();
        Box::pin(async move {
            let ips = lookup(host.clone()).await?;
            if ips.is_empty() {
                return Err(format!("no addresses found for {host}").into());
            }
            if let Some(ip) = ips.iter().find(|ip| !policy.permits(**ip)) {
                return Err(Box::new(BlockedUrlError(format!(
                    "{host} resolves to {ip}, which is not a public address"
                )))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// Caps concurrent requests overall and per host. Hosts share a fixed set of
/// semaphores by hash, so memory stays bounded however many hosts are seen.
pub(crate) struct HostLimiter {
    total: Semaphore,
    per_host: Vec<Semaphore>,
}

impl HostLimiter {
    pub(crate) fn new(max_total: usize, max_per_host: usize) -> Self {
        Self {
            total: Semaphore::new(max_total.max(1)),
            per_host: (0..HOST_LIMITER_STRIPES)
                .map(|_| Semaphore::new(max_per_host.max(1)))
                .collect(),
        }
    }

    pub(crate) async fn acquire(&self, host: &str) -> (SemaphorePermit<'_>, SemaphorePermit<'_>) {
        let mut hasher = DefaultHasher::new();
        host.to_ascii_lowercase().hash(&mut hasher);
        let stripe = &self.per_host[(hasher.finish() as usize) % self.per_host.len()];
        // Wait for the host's slot first so a busy host does not hold a
        // global slot while it queues.
        let host_permit = stripe.acquire().await.expect("semaphore is never closed");
        let total_permit = self
            .total
            .acquire()
            .await
            .expect("semaphore is never closed");
        (host_permit, total_permit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn internal_ipv4_addresses_are_not_public() {
        for addr in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "192.0.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public_ip(ip(addr)), "{addr} should be blocked");
        }
    }

    #[test]
    fn internal_ipv6_addresses_are_not_public() {
        for addr in [
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001::1",
            "64:ff9b:1::1",
            // IPv4 forms embedded in IPv6.
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::1",
            "2002:c0a8:0101::1",
        ] {
            assert!(!is_public_ip(ip(addr)), "{addr} should be blocked");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for addr in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "172.32.0.1",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(is_public_ip(ip(addr)), "{addr} should be allowed");
        }
    }

    #[test]
    fn check_url_rejects_other_schemes_and_internal_literals() {
        let policy = AddressPolicy::default();
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://example.com/",
            "http://127.0.0.1/",
            "http://localhost.:80@127.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1:8080/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fd00::1]/",
            // Integer and hex forms normalise to 127.0.0.1.
            "http://2130706433/",
            "http://0x7f000001/",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert!(
                policy.check_url(&parsed).is_err(),
                "{url} should be blocked"
            );
        }

        for url in ["https://example.com/a.json", "http://8.8.8.8/"] {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert!(policy.check_url(&parsed).is_ok(), "{url} should be allowed");
        }
    }

    #[test]
    fn allow_private_networks_disables_the_address_check() {
        let policy = AddressPolicy {
            allow_private_networks: true,
            exempt: Vec::new(),
        };
        let parsed = reqwest::Url::parse("http://127.0.0.1:8080/").unwrap();
        assert!(policy.check_url(&parsed).is_ok());
        let parsed = reqwest::Url::parse("file:///etc/passwd").unwrap();
        assert!(policy.check_url(&parsed).is_err());
    }

    #[tokio::test]
    async fn resolver_refuses_names_with_any_internal_address() {
        let lookup: LookupFn = Arc::new(|host: String| {
            Box::pin(async move {
                Ok(match host.as_str() {
                    "public.test" => vec!["93.184.216.34".parse().unwrap()],
                    "mixed.test" => vec![
                        "93.184.216.34".parse().unwrap(),
                        "10.0.0.1".parse().unwrap(),
                    ],
                    _ => vec!["169.254.169.254".parse().unwrap()],
                })
            })
        });
        let resolver = GuardedResolver::new(lookup, Arc::new(AddressPolicy::default()));

        let addrs: Vec<SocketAddr> = resolver
            .resolve("public.test".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, vec!["93.184.216.34:0".parse().unwrap()]);

        assert!(resolver
            .resolve("mixed.test".parse().unwrap())
            .await
            .is_err());
        assert!(resolver
            .resolve("metadata.test".parse().unwrap())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn host_limiter_caps_concurrency_per_host() {
        let limiter = HostLimiter::new(8, 1);
        let first = limiter.acquire("a.example").await;
        // A second request to the same host waits for the first to finish.
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            limiter.acquire("A.example"),
        )
        .await;
        assert!(blocked.is_err());
        drop(first);
        let _second = limiter.acquire("a.example").await;
    }
}
