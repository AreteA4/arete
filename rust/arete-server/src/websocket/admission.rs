//! Optional host-provided admission hooks for WebSocket lifecycle events.
//!
//! The server owns protocol and connection lifecycles, but it does not need to
//! know how an embedder groups callers or stores quota state. Implementations
//! may keep process-local state, delegate to a shared service, or decline to
//! manage a context by returning no connection permit.

use std::fmt::Debug;
use std::sync::Arc;

use uuid::Uuid;

use super::auth::{AuthContext, AuthDeny};

/// An opaque connection reservation owned by an admission provider.
///
/// The server retains this value for the lifetime of the connection. Dropping
/// it must release any reservation held by the provider.
pub trait WebSocketConnectionPermit: Debug + Send + Sync {
    /// Stable identifier used for later lifecycle callbacks.
    fn id(&self) -> Uuid;
}

/// Embedder-owned admission policy for lifecycle events the server controls.
///
/// Implementations must make `refresh_connection` transactional: an error
/// leaves the existing permit and its policy unchanged. When it returns a new
/// permit, dropping the previous permit must not release the new reservation.
/// The server serializes refresh and subscription callbacks for each
/// connection and invokes them without holding its client-registry locks.
#[allow(clippy::result_large_err)]
pub trait WebSocketAdmissionProvider: Debug + Send + Sync {
    /// Reserve a newly authenticated connection.
    ///
    /// Returning `None` leaves this connection unmanaged by the provider.
    fn reserve_connection(
        &self,
        context: &AuthContext,
    ) -> Result<Option<Arc<dyn WebSocketConnectionPermit>>, AuthDeny>;

    /// Atomically refresh admission for a live connection.
    ///
    /// `active_subscriptions` is the complete current set. This lets a provider
    /// move an unmanaged connection into managed state without racing a series
    /// of subscription callbacks.
    fn refresh_connection(
        &self,
        current_permit: Option<Arc<dyn WebSocketConnectionPermit>>,
        context: &AuthContext,
        active_subscriptions: &[String],
    ) -> Result<Option<Arc<dyn WebSocketConnectionPermit>>, AuthDeny>;

    /// Reserve a subscription. Return `false` when the identifier is already
    /// present for this connection.
    fn add_subscription(&self, permit: Uuid, subscription_id: &str) -> Result<bool, AuthDeny>;

    /// Release a subscription reservation.
    fn remove_subscription(&self, permit: Uuid, subscription_id: &str);

    /// Admit one inbound protocol message.
    fn check_inbound_message(&self, permit: Uuid) -> Result<(), AuthDeny>;

    /// Admit an outbound payload of `bytes` bytes.
    fn check_egress(&self, permit: Uuid, bytes: usize) -> Result<(), AuthDeny>;

    /// Periodic bounded-state maintenance.
    fn cleanup(&self) {}
}
