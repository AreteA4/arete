use arete::runtime::yellowstone_grpc_proto::{
    geyser::*,
    tonic::{self, Request, Response, Status},
};
use arete::runtime::{
    futures::{self, StreamExt},
    shipstern::{
        self,
        sources::{SourceExitStatus, SourceTrait},
    },
    shipstern_core, tokio,
};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

__managed_helpers!();

#[derive(Clone, Default)]
struct Service {
    connections: Arc<
        Mutex<
            Vec<(
                SubscribeRequest,
                tokio::sync::mpsc::Sender<Result<SubscribeUpdate, Status>>,
            )>,
        >,
    >,
    connected: Arc<tokio::sync::Notify>,
}
type Updates = Pin<Box<dyn futures::Stream<Item = Result<SubscribeUpdate, Status>> + Send>>;
#[tonic::async_trait]
impl geyser_server::Geyser for Service {
    type SubscribeStream = Updates;
    async fn subscribe(
        &self,
        request: Request<tonic::Streaming<SubscribeRequest>>,
    ) -> Result<Response<Updates>, Status> {
        let mut incoming = request.into_inner();
        let request = incoming.message().await?.unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        self.connections.lock().unwrap().push((request, tx));
        self.connected.notify_one();
        // Keep the bidirectional request stream alive while the subscription runs.
        tokio::spawn(async move { while incoming.message().await.ok().flatten().is_some() {} });
        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
    }
    type SubscribeDeshredStream =
        Pin<Box<dyn futures::Stream<Item = Result<SubscribeUpdateDeshred, Status>> + Send>>;
    async fn subscribe_deshred(
        &self,
        _: Request<tonic::Streaming<SubscribeDeshredRequest>>,
    ) -> Result<Response<Self::SubscribeDeshredStream>, Status> {
        Err(Status::unimplemented("test"))
    }
    type SubscribeGossipStream =
        Pin<Box<dyn futures::Stream<Item = Result<SubscribeUpdateGossip, Status>> + Send>>;
    async fn subscribe_gossip(
        &self,
        _: Request<SubscribeGossipRequest>,
    ) -> Result<Response<Self::SubscribeGossipStream>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn subscribe_replay_info(
        &self,
        _: Request<SubscribeReplayInfoRequest>,
    ) -> Result<Response<SubscribeReplayInfoResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn ping(&self, _: Request<PingRequest>) -> Result<Response<PongResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn get_latest_blockhash(
        &self,
        _: Request<GetLatestBlockhashRequest>,
    ) -> Result<Response<GetLatestBlockhashResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn get_block_height(
        &self,
        _: Request<GetBlockHeightRequest>,
    ) -> Result<Response<GetBlockHeightResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn get_slot(
        &self,
        _: Request<GetSlotRequest>,
    ) -> Result<Response<GetSlotResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn is_blockhash_valid(
        &self,
        _: Request<IsBlockhashValidRequest>,
    ) -> Result<Response<IsBlockhashValidResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
    async fn get_version(
        &self,
        _: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        Err(Status::unimplemented("test"))
    }
}
async fn connection(
    service: &Service,
    count: usize,
) -> (
    SubscribeRequest,
    tokio::sync::mpsc::Sender<Result<SubscribeUpdate, Status>>,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notified = service.connected.notified();
            if let Some(pair) = service.connections.lock().unwrap().get(count - 1).cloned() {
                return pair;
            }
            notified.await;
        }
    })
    .await
    .unwrap()
}
fn source(endpoint: &str, from_slot: u64) -> ManagedYellowstoneGrpcSource {
    ManagedYellowstoneGrpcSource::new(
        arete::runtime::shipstern_yellowstone_grpc_source::YellowstoneGrpcConfig {
            endpoint: endpoint.into(),
            x_token: None,
            timeout: 10,
            commitment_level: None,
            from_slot: Some(from_slot),
            accept_compression: None,
            max_decoding_message_size: None,
            accounts_data_slice: vec![],
            auto_reconnect: false,
            reconnect_max_retries: None,
            reconnect_slot_retention: None,
        },
        shipstern_core::Filters::new(Default::default()),
    )
}
fn update(slot: u64) -> SubscribeUpdate {
    SubscribeUpdate {
        update_oneof: Some(subscribe_update::UpdateOneof::Slot(SubscribeUpdateSlot {
            slot,
            ..Default::default()
        })),
        ..Default::default()
    }
}
async fn run() {
    let service = Service::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(geyser_server::GeyserServer::new(service.clone()))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = stop_rx.await;
                },
            ),
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let (status_tx, status_rx) = tokio::sync::oneshot::channel();
    let first_source = source(&endpoint, 100);
    let first = tokio::spawn(async move { first_source.connect(tx, status_tx).await });
    let (request, upstream) = connection(&service, 1).await;
    assert_eq!(request.from_slot, Some(100));
    // No client-side dedup or replay quarantine is permitted to change Arete's
    // input order, including duplicates and partial slots on reconnect.
    upstream.send(Ok(update(100))).await.unwrap();
    upstream.send(Ok(update(100))).await.unwrap();
    upstream.send(Ok(update(101))).await.unwrap();
    for expected in [100, 100, 101] {
        let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(received, update(expected));
    }
    upstream
        .send(Err(Status::unavailable("disconnect")))
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), status_rx)
            .await
            .unwrap()
            .unwrap(),
        SourceExitStatus::Completed
    ));
    first.await.unwrap().unwrap();
    assert_eq!(
        service.connections.lock().unwrap().len(),
        1,
        "Arete owns reconnects"
    );

    let processed = arete::server::SlotTracker::new();
    processed.record(100);
    let resume = arete::server::snapshot::select_reconnect_from_slot(
        None,
        processed.get(),
        0,
        FROM_SLOT_LIVE_FALLBACK_ATTEMPTS,
    );
    assert_eq!(resume, Some(100));
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let (status_tx, status_rx) = tokio::sync::oneshot::channel();
    let resumed_source = source(&endpoint, resume.unwrap());
    let resumed = tokio::spawn(async move { resumed_source.connect(tx, status_tx).await });
    let (request, upstream) = connection(&service, 2).await;
    assert_eq!(request.from_slot, Some(100));
    upstream.send(Ok(update(100))).await.unwrap();
    upstream.send(Ok(update(102))).await.unwrap();
    // The bounded downstream queue permits only one update while the consumer stalls.
    tokio::task::yield_now().await;
    assert!(rx.len() <= 1);
    for expected in [100, 102] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            update(expected)
        );
    }
    // Cancellation also works while the upstream is idle.
    drop(rx);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), status_rx)
            .await
            .unwrap()
            .unwrap(),
        SourceExitStatus::ReceiverDropped
    ));
    resumed.await.unwrap().unwrap();

    // The independent slot subscription must still include a valid SlotHashes filter.
    let slot_tracker = arete::server::SlotTracker::new();
    let x_token: Option<String> = None;
    let health_monitor: Option<arete::server::HealthMonitor> = None;
    let mut reconnection_config = arete::server::ReconnectionConfig::default();
    reconnection_config.max_attempts = Some(1);
    __slot_task!();
    let (request, upstream) = connection(&service, 3).await;
    assert!(request.from_slot.is_none());
    assert_eq!(request.slots.len(), 1);
    assert_eq!(
        request.accounts["slot_hashes_sysvar"].account,
        ["SysvarS1otHashes111111111111111111111111111"]
    );
    let mut data = 1u64.to_le_bytes().to_vec();
    data.extend_from_slice(&102u64.to_le_bytes());
    data.extend_from_slice(&[7u8; 32]);
    upstream
        .send(Ok(SubscribeUpdate {
            update_oneof: Some(subscribe_update::UpdateOneof::Account(
                SubscribeUpdateAccount {
                    slot: 103,
                    account: Some(SubscribeUpdateAccountInfo {
                        pubkey: arete::runtime::bs58::decode(
                            "SysvarS1otHashes111111111111111111111111111",
                        )
                        .into_vec()
                        .unwrap(),
                        data,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }))
        .await
        .unwrap();
    upstream.send(Ok(update(103))).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if slot_tracker.get() == 103 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(arete::interpreter::get_slot_hash(102).is_some());
    upstream
        .send(Err(Status::unavailable("done")))
        .await
        .unwrap();
    // Background gRPC streams may hold the server until their tasks shut down.
    let _ = stop_tx.send(());
    server.abort();
}
fn main() {
    tokio::runtime::Runtime::new().unwrap().block_on(run());
}
