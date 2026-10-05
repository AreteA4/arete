use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex, Weak};

use arete_auth::SessionClaims;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::protocol::Role;

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
    deny_reserve: AtomicBool,
    deny_inbound: AtomicBool,
    deny_egress: AtomicBool,
    refresh_entered: Mutex<Option<sync_mpsc::Sender<()>>>,
    refresh_release: Mutex<Option<sync_mpsc::Receiver<()>>>,
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

    fn block_next_refresh(&self, entered: sync_mpsc::Sender<()>, release: sync_mpsc::Receiver<()>) {
        *self.refresh_entered.lock().unwrap() = Some(entered);
        *self.refresh_release.lock().unwrap() = Some(release);
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
        if self.deny_reserve.load(Ordering::Relaxed) {
            return Err(AuthDeny::new(
                AuthErrorCode::InternalError,
                "connection rejected by provider",
            ));
        }
        Ok(Some(self.permit(context)))
    }

    fn refresh_connection(
        &self,
        current_permit: Option<Arc<dyn WebSocketConnectionPermit>>,
        context: &AuthContext,
        active_subscriptions: &[String],
    ) -> Result<Option<Arc<dyn WebSocketConnectionPermit>>, AuthDeny> {
        if let Some(entered) = self.refresh_entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.refresh_release.lock().unwrap().take() {
            let _ = release.recv();
        }
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

async fn test_websocket_sender() -> WebSocketSender {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (accepted, connected) = tokio::join!(listener.accept(), TcpStream::connect(addr));
    let (server_stream, _) = accepted.unwrap();
    let _client_stream = connected.unwrap();
    let socket = WebSocketStream::from_raw_socket(server_stream, Role::Server, None).await;
    socket.split().0
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

#[tokio::test]
async fn public_add_client_returns_connection_denial() {
    let provider = Arc::new(FakeAdmission::default());
    provider.deny_reserve.store(true, Ordering::Relaxed);
    let manager = ClientManager::new().with_admission_provider(Arc::new(provider));
    let client_id = Uuid::new_v4();

    let deny = manager
        .add_client(
            client_id,
            test_websocket_sender().await,
            Some(context("actor:denied")),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1234),
        )
        .unwrap_err();

    assert_eq!(deny.reason, "connection rejected by provider");
    assert!(!manager.has_client(client_id));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_serializes_subscription_changes_without_holding_the_registry() {
    let provider = Arc::new(FakeAdmission::default());
    let manager =
        Arc::new(ClientManager::new().with_admission_provider(Arc::new(provider.clone())));
    let client_id = insert_client(&manager, &provider, context("actor:old"));
    let (entered_tx, entered_rx) = sync_mpsc::channel();
    let (release_tx, release_rx) = sync_mpsc::channel();
    provider.block_next_refresh(entered_tx, release_rx);

    let refresh_manager = manager.clone();
    let refresh = tokio::task::spawn_blocking(move || {
        refresh_manager.try_update_client_auth(client_id, context("actor:new"))
    });
    tokio::task::spawn_blocking(move || entered_rx.recv_timeout(Duration::from_secs(1)))
        .await
        .unwrap()
        .unwrap();

    let probe_manager = manager.clone();
    let registry_available =
        tokio::task::spawn_blocking(move || probe_manager.has_client(client_id));
    assert!(
        tokio::time::timeout(Duration::from_millis(250), registry_available)
            .await
            .unwrap()
            .unwrap()
    );

    let add_manager = manager.clone();
    let mut add = tokio::spawn(async move {
        add_manager
            .try_add_client_subscription(
                client_id,
                "sub-during-refresh".into(),
                CancellationToken::new(),
            )
            .await
    });
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut add)
        .await
        .is_err());

    release_tx.send(()).unwrap();
    assert!(refresh.await.unwrap().unwrap());
    assert!(add.await.unwrap().unwrap());
    let reservations = provider.reservations.lock().unwrap();
    let reservation = reservations.values().next().unwrap();
    assert_eq!(reservation.context.subject, "actor:new");
    assert_eq!(
        reservation.subscriptions,
        HashSet::from(["sub-during-refresh".to_string()])
    );
}

#[tokio::test]
async fn inbound_denial_remains_deliverable_while_egress_denial_disconnects() {
    let provider = Arc::new(FakeAdmission::default());
    let manager = ClientManager::new().with_admission_provider(Arc::new(provider.clone()));
    let inbound = Uuid::new_v4();
    let (sender, mut receiver) = mpsc::channel(4);
    let inbound_context = context("actor:inbound");
    let mut client = ClientInfo::new(
        inbound,
        sender,
        Some(inbound_context.clone()),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1234),
    );
    client.admission_permit = provider.reserve_connection(&inbound_context).unwrap();
    manager.clients.insert(inbound, client);
    provider.deny_inbound.store(true, Ordering::Relaxed);
    let deny = manager.check_inbound_message_allowed(inbound).unwrap_err();
    assert!(manager.has_client(inbound));
    manager
        .send_text_to_client(inbound, deny.reason.clone())
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(Message::Text(deny.reason.into()))
    );
    manager.remove_client(inbound);

    provider.deny_inbound.store(false, Ordering::Relaxed);
    let egress = insert_client(&manager, &provider, context("actor:egress"));
    provider.deny_egress.store(true, Ordering::Relaxed);
    assert!(manager.enforce_egress_budgets(egress, 32).is_err());
    assert!(!manager.has_client(egress));
}
