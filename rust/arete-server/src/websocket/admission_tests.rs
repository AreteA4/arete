use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arete_auth::SessionClaims;

use super::*;

#[derive(Debug)]
struct Reservation {
    context: AuthContext,
    subscriptions: HashSet<String>,
}

#[derive(Debug, Default)]
struct FakeAdmission {
    reservations: Mutex<HashMap<Uuid, Reservation>>,
    deny_refresh: AtomicBool,
    deny_inbound: AtomicBool,
    deny_egress: AtomicBool,
}

impl FakeAdmission {
    fn permit(self: &Arc<Self>, context: &AuthContext) -> Arc<dyn WebSocketConnectionPermit> {
        let id = Uuid::new_v4();
        self.reservations.lock().unwrap().insert(
            id,
            Reservation {
                context: context.clone(),
                subscriptions: HashSet::new(),
            },
        );
        Arc::new(FakePermit {
            state: Arc::downgrade(self),
            id,
        })
    }

    fn reservation_count(&self) -> usize {
        self.reservations.lock().unwrap().len()
    }
}

#[derive(Debug)]
struct FakePermit {
    state: Weak<FakeAdmission>,
    id: Uuid,
}

impl WebSocketConnectionPermit for FakePermit {
    fn id(&self) -> Uuid {
        self.id
    }
}

impl Drop for FakePermit {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.reservations.lock().unwrap().remove(&self.id);
        }
    }
}

impl WebSocketAdmissionProvider for Arc<FakeAdmission> {
    fn reserve_connection(
        &self,
        context: &AuthContext,
    ) -> Result<Option<Arc<dyn WebSocketConnectionPermit>>, AuthDeny> {
        Ok(Some(self.permit(context)))
    }

    fn refresh_connection(
        &self,
        current_permit: Option<Arc<dyn WebSocketConnectionPermit>>,
        context: &AuthContext,
        active_subscriptions: &[String],
    ) -> Result<Option<Arc<dyn WebSocketConnectionPermit>>, AuthDeny> {
        if self.deny_refresh.load(Ordering::Relaxed) {
            return Err(AuthDeny::new(
                AuthErrorCode::TokenExpired,
                "refresh rejected",
            ));
        }
        let permit = current_permit.unwrap_or_else(|| self.permit(context));
        let mut reservations = self.reservations.lock().unwrap();
        let reservation = reservations.get_mut(&permit.id()).unwrap();
        reservation.context = context.clone();
        reservation.subscriptions = active_subscriptions.iter().cloned().collect();
        drop(reservations);
        Ok(Some(permit))
    }

    fn add_subscription(&self, permit: Uuid, subscription_id: &str) -> Result<bool, AuthDeny> {
        let mut reservations = self.reservations.lock().unwrap();
        let reservation = reservations.get_mut(&permit).unwrap();
        Ok(reservation
            .subscriptions
            .insert(subscription_id.to_string()))
    }

    fn remove_subscription(&self, permit: Uuid, subscription_id: &str) {
        if let Some(reservation) = self.reservations.lock().unwrap().get_mut(&permit) {
            reservation.subscriptions.remove(subscription_id);
        }
    }

    fn check_inbound_message(&self, _permit: Uuid) -> Result<(), AuthDeny> {
        if self.deny_inbound.load(Ordering::Relaxed) {
            Err(AuthDeny::rate_limited(
                Duration::from_secs(60),
                "provider inbound",
            ))
        } else {
            Ok(())
        }
    }

    fn check_egress(&self, _permit: Uuid, _bytes: usize) -> Result<(), AuthDeny> {
        if self.deny_egress.load(Ordering::Relaxed) {
            Err(AuthDeny::rate_limited(
                Duration::from_secs(60),
                "provider egress",
            ))
        } else {
            Ok(())
        }
    }
}

fn context(subject: &str) -> AuthContext {
    AuthContext::from_claims(
        SessionClaims::builder("issuer", subject, "audience")
            .with_metering_key(subject)
            .build(),
    )
}

fn insert_client(
    manager: &ClientManager,
    provider: &Arc<FakeAdmission>,
    context: AuthContext,
) -> Uuid {
    let client_id = Uuid::new_v4();
    let (sender, _receiver) = mpsc::channel(4);
    let mut client = ClientInfo::new(
        client_id,
        sender,
        Some(context.clone()),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1234),
    );
    client.admission_permit = provider.reserve_connection(&context).unwrap();
    manager.clients.insert(client_id, client);
    client_id
}

#[tokio::test]
async fn provider_reservations_follow_connection_and_subscription_lifecycle() {
    let provider = Arc::new(FakeAdmission::default());
    let manager = ClientManager::new().with_admission_provider(Arc::new(provider.clone()));
    let client_id = insert_client(&manager, &provider, context("actor:1"));
    assert_eq!(provider.reservation_count(), 1);

    let token = CancellationToken::new();
    assert!(manager
        .try_add_client_subscription(client_id, "sub-1".into(), token)
        .await
        .unwrap());
    assert_eq!(
        provider
            .reservations
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .subscriptions
            .len(),
        1
    );
    assert!(manager.remove_client_subscription(client_id, "sub-1").await);
    manager.remove_client(client_id);
    assert_eq!(provider.reservation_count(), 0);
}

#[test]
fn rejected_refresh_preserves_the_existing_context_and_permit() {
    let provider = Arc::new(FakeAdmission::default());
    let manager = ClientManager::new().with_admission_provider(Arc::new(provider.clone()));
    let client_id = insert_client(&manager, &provider, context("actor:old"));
    provider.deny_refresh.store(true, Ordering::Relaxed);

    assert!(manager
        .try_update_client_auth(client_id, context("actor:new"))
        .is_err());
    assert_eq!(
        manager
            .clients
            .get(&client_id)
            .unwrap()
            .auth_context
            .as_ref()
            .unwrap()
            .subject,
        "actor:old"
    );
    assert_eq!(provider.reservation_count(), 1);
}

#[test]
fn provider_denials_disconnect_before_inbound_or_egress_delivery() {
    let provider = Arc::new(FakeAdmission::default());
    let manager = ClientManager::new().with_admission_provider(Arc::new(provider.clone()));
    let inbound = insert_client(&manager, &provider, context("actor:inbound"));
    provider.deny_inbound.store(true, Ordering::Relaxed);
    assert!(manager.check_inbound_message_allowed(inbound).is_err());
    assert!(!manager.has_client(inbound));

    provider.deny_inbound.store(false, Ordering::Relaxed);
    let egress = insert_client(&manager, &provider, context("actor:egress"));
    provider.deny_egress.store(true, Ordering::Relaxed);
    assert!(manager.enforce_egress_budgets(egress, 32).is_err());
    assert!(!manager.has_client(egress));
}
