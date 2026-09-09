use crate::auth::{AuthConfig, AuthToken, TokenTransport};
use crate::chain::{derive_http_endpoint, ChainClient, ChainError, HttpChainClient};
use crate::config::{AreteConfig, ConnectionConfig};
use crate::connection::{ConnectionManager, ConnectionState};
use crate::entity::Stack;
use crate::error::{AreteError, SocketIssue};
use crate::gateway::create_hosted_solana_gateway_transports;
use crate::http::{HttpAuthClient, TokenSource};
use crate::instruction::{BuiltInstruction, ErrorMetadata};
use crate::operations::{
    classify_wallet_error, execute_prepared_operation, inspect_prepared_operation, ExecuteOptions,
    ExecutionHost, FailurePhase, OperationExecutionError, OperationInspection,
    OperationInspectionError, OperationReceipt, PreparedOperation, SignerRegistry,
    TransactionFailureOutcome,
};
use crate::program::Programs;
use crate::store::{SharedStore, StoreConfig};
use crate::transactions::{HttpTransactionTransport, TransactionError, TransactionTransport};
use crate::view::Views;
use crate::wallet::{
    SendOptions, TransactionInspectionOptions, WalletAdapter, WalletExecutionContext,
};
use async_trait::async_trait;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;

/// Transport mode for the client (mirror of TS `ConnectOptions.transport`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Open the streaming WebSocket (default). Views, chain reads, and
    /// execution all work.
    #[default]
    WebSocket,
    /// Skip the socket entirely: point reads, chain reads, and instruction
    /// execution work, while view subscriptions fail with
    /// [`AreteError::WebSocketDisabled`].
    Http,
}

/// Result of [`Arete::transaction`] (mirror of the TS `ExecutionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    /// Transaction signature (base58).
    pub signature: String,
    /// Slot in which the transaction landed, if the adapter reports it.
    pub slot: Option<u64>,
}

/// Options for [`Arete::transaction`] (mirror of the TS `TransactionOptions`).
///
/// Divergence from TypeScript: when `errors` is empty no program-error
/// metadata is applied (TS falls back to error metadata aggregated from every
/// stack handler; the Rust client does not hold the generated handlers).
#[derive(Clone, Default)]
pub struct TransactionOptions {
    /// Wallet override; falls back to the client's default wallet.
    pub wallet: Option<Arc<dyn WalletAdapter>>,
    /// Send options forwarded to the wallet adapter.
    pub send: SendOptions,
    /// IDL error metadata used to resolve chain failures.
    pub errors: Vec<ErrorMetadata>,
}

impl std::fmt::Debug for TransactionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionOptions")
            .field(
                "wallet",
                &self.wallet.as_ref().map(|wallet| wallet.public_key()),
            )
            .field("send", &self.send)
            .field("errors", &self.errors.len())
            .finish()
    }
}

/// Options for [`Arete::inspect_operation`] (mirror of Python's
/// `inspect_operation` keyword arguments).
#[derive(Clone, Default)]
pub struct OperationInspectionOptions {
    /// Wallet override; falls back to the client's default wallet.
    pub wallet: Option<Arc<dyn WalletAdapter>>,
    /// Inspection options forwarded to the wallet adapter.
    pub inspect: TransactionInspectionOptions,
    /// Transaction transport override; falls back to the client's transport.
    pub transaction_transport: Option<Arc<dyn TransactionTransport>>,
}

impl std::fmt::Debug for OperationInspectionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationInspectionOptions")
            .field(
                "wallet",
                &self.wallet.as_ref().map(|wallet| wallet.public_key()),
            )
            .field("inspect", &self.inspect)
            .field(
                "transaction_transport",
                &self
                    .transaction_transport
                    .as_ref()
                    .map(|_| "TransactionTransport"),
            )
            .finish()
    }
}

/// Chain client used when the client has no HTTP endpoint: every call fails
/// with a clear configuration error (mirror of the TS
/// `authenticatedStackFetch` INVALID_CONFIG throw).
struct UnconfiguredChainClient {
    stack: &'static str,
}

impl UnconfiguredChainClient {
    fn error(&self) -> ChainError {
        ChainError::Sdk(AreteError::InvalidConfig(format!(
            "Stack '{}' has no HTTP endpoint; chain reads require AreteBuilder::http_url or a \
             WebSocket URL to derive it from",
            self.stack
        )))
    }
}

#[async_trait]
impl ChainClient for UnconfiguredChainClient {
    async fn exists(&self, _address: &str) -> Result<bool, ChainError> {
        Err(self.error())
    }

    async fn lamports(&self, _address: &str) -> Result<u64, ChainError> {
        Err(self.error())
    }

    async fn native_balance(
        &self,
        _address: &str,
        _options: crate::chain::ContextSlotOptions,
    ) -> Result<crate::chain::NativeBalanceInfo, ChainError> {
        Err(self.error())
    }

    async fn minimum_balance_for_rent_exemption(&self, _space: u64) -> Result<u64, ChainError> {
        Err(self.error())
    }

    async fn clock(&self) -> Result<crate::chain::ChainClock, ChainError> {
        Err(self.error())
    }

    async fn account(
        &self,
        _address: &str,
    ) -> Result<Option<crate::chain::RawAccountInfo>, ChainError> {
        Err(self.error())
    }

    async fn accounts(
        &self,
        _addresses: &[String],
    ) -> Result<Vec<Option<crate::chain::RawAccountInfo>>, ChainError> {
        Err(self.error())
    }

    async fn mint(
        &self,
        _address: &str,
    ) -> Result<Option<crate::chain::MintAccountInfo>, ChainError> {
        Err(self.error())
    }

    async fn token_account(
        &self,
        _address: &str,
    ) -> Result<Option<crate::chain::TokenAccountInfo>, ChainError> {
        Err(self.error())
    }

    async fn balance(
        &self,
        _input: &crate::chain::TokenBalanceInput,
        _options: crate::chain::ContextSlotOptions,
    ) -> Result<crate::chain::TokenBalanceInfo, ChainError> {
        Err(self.error())
    }
}

/// Transaction transport used when the client has no HTTP endpoint.
struct UnconfiguredTransactionTransport {
    stack: &'static str,
}

impl UnconfiguredTransactionTransport {
    fn error(&self) -> TransactionError {
        TransactionError::Sdk(AreteError::InvalidConfig(format!(
            "Stack '{}' has no HTTP endpoint; the transaction relay requires \
             AreteBuilder::http_url or a WebSocket URL to derive it from",
            self.stack
        )))
    }
}

#[async_trait]
impl TransactionTransport for UnconfiguredTransactionTransport {
    async fn latest_blockhash(
        &self,
        _options: crate::transactions::TransactionRequestContext,
    ) -> Result<crate::transactions::LatestBlockhashResult, TransactionError> {
        Err(self.error())
    }

    async fn fee(
        &self,
        _message: &str,
        _options: crate::transactions::TransactionRequestContext,
    ) -> Result<crate::transactions::TransactionFeeResult, TransactionError> {
        Err(self.error())
    }

    async fn simulate(
        &self,
        _transaction: &str,
        _options: crate::transactions::TransactionSimulationOptions,
    ) -> Result<crate::transactions::TransactionSimulationResult, TransactionError> {
        Err(self.error())
    }

    async fn send(
        &self,
        _transaction: &str,
        _options: crate::transactions::TransactionSendOptions,
    ) -> Result<crate::transactions::TransactionSendResult, TransactionError> {
        Err(self.error())
    }

    async fn signature_status(
        &self,
        _signature: &str,
        _options: crate::transactions::SignatureStatusOptions,
    ) -> Result<Option<crate::transactions::TransactionSignatureStatus>, TransactionError> {
        Err(self.error())
    }

    async fn signature_statuses(
        &self,
        _signatures: &[String],
        _options: crate::transactions::SignatureStatusOptions,
    ) -> Result<Vec<Option<crate::transactions::TransactionSignatureStatus>>, TransactionError>
    {
        Err(self.error())
    }

    async fn block_height(
        &self,
        _options: crate::transactions::TransactionRequestContext,
    ) -> Result<u64, TransactionError> {
        Err(self.error())
    }

    async fn transaction(
        &self,
        _signature: &str,
        _options: crate::transactions::TransactionInspectOptions,
    ) -> Result<Option<crate::transactions::ConfirmedTransaction>, TransactionError> {
        Err(self.error())
    }

    async fn signatures(
        &self,
        _address: &str,
        _options: crate::transactions::SignaturePageOptions,
    ) -> Result<Vec<crate::transactions::SignaturePageEntry>, TransactionError> {
        Err(self.error())
    }
}

/// Arete client with typed views access.
///
/// ```ignore
/// use arete_sdk::prelude::*;
/// use arete_stacks::ore::OreStack;
///
/// let a4 = Arete::<OreStack>::connect().await?;
/// let rounds = a4.views.latest().get().await;
/// ```
pub struct Arete<S: Stack> {
    connection: ConnectionManager,
    store: SharedStore,
    #[allow(dead_code)]
    config: AreteConfig,
    transport: Transport,
    http_base_url: Option<String>,
    auth_client: Arc<HttpAuthClient>,
    chain: Arc<dyn ChainClient>,
    transactions: Arc<dyn TransactionTransport>,
    wallet: RwLock<Option<Arc<dyn WalletAdapter>>>,
    signer_registry: Arc<SignerRegistry>,
    pub views: S::Views,
    pub programs: S::Programs,
    _stack: PhantomData<S>,
}

impl<S: Stack> Arete<S> {
    /// Connect to the stack's default URL.
    pub async fn connect() -> Result<Self, AreteError> {
        Self::builder().connect().await
    }

    /// Connect with custom URL.
    pub async fn connect_url(url: &str) -> Result<Self, AreteError> {
        Self::builder().url(url).connect().await
    }

    /// Create a builder for custom configuration.
    pub fn builder() -> AreteBuilder<S> {
        AreteBuilder::new()
    }

    pub async fn connection_state(&self) -> ConnectionState {
        self.connection.state().await
    }

    pub async fn last_error(&self) -> Option<Arc<AreteError>> {
        self.connection.last_error().await
    }

    pub async fn last_socket_issue(&self) -> Option<SocketIssue> {
        self.connection.last_socket_issue().await
    }

    pub fn subscribe_socket_issues(&self) -> broadcast::Receiver<SocketIssue> {
        self.connection.subscribe_socket_issues()
    }

    pub async fn disconnect(&self) {
        self.connection.disconnect().await;
    }

    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    /// The transport mode this client was connected with.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// The effective HTTP base URL (explicit `http_url`, generated
    /// `Stack::http_url`, or derived from the WebSocket URL), if any.
    pub fn http_base_url(&self) -> Option<&str> {
        self.http_base_url.as_deref()
    }

    /// The client's shared HTTP token machinery.
    pub fn token_source(&self) -> Arc<dyn TokenSource> {
        self.auth_client.clone()
    }

    /// Chain read client (`/chain/*`). Always available; when the client has
    /// no HTTP endpoint and no explicit override was configured, every call
    /// fails with [`AreteError::InvalidConfig`].
    pub fn chain(&self) -> Arc<dyn ChainClient> {
        self.chain.clone()
    }

    /// Transaction relay transport (`/transactions/v1/*`). Always available;
    /// when the client has no HTTP endpoint and no explicit override was
    /// configured, every call fails with [`AreteError::InvalidConfig`].
    pub fn transactions(&self) -> Arc<dyn TransactionTransport> {
        self.transactions.clone()
    }

    /// The default wallet adapter, if one is configured.
    pub fn wallet(&self) -> Option<Arc<dyn WalletAdapter>> {
        self.wallet.read().expect("wallet lock poisoned").clone()
    }

    /// Set (or clear) the default wallet adapter used for execution.
    pub fn set_wallet(&self, wallet: Option<Arc<dyn WalletAdapter>>) {
        *self.wallet.write().expect("wallet lock poisoned") = wallet;
    }

    /// The connected wallet address, if a default wallet is configured.
    pub fn public_key(&self) -> Option<String> {
        self.wallet().map(|wallet| wallet.public_key())
    }

    /// Signers available to every operation executed through this client.
    pub fn signer_registry(&self) -> Arc<SignerRegistry> {
        self.signer_registry.clone()
    }

    /// Sign and send a batch of pre-built instructions as a single
    /// transaction (mirror of the TS `client.transaction`).
    ///
    /// The wallet (call override or client default) receives the client's
    /// transaction transport via [`WalletExecutionContext`]. On failure the
    /// error is [`AreteError::TransactionFailed`] carrying a structured
    /// [`TransactionFailureOutcome`]; chain failures are resolved against
    /// `options.errors` metadata when provided.
    pub async fn transaction(
        &self,
        instructions: &[BuiltInstruction],
        options: TransactionOptions,
    ) -> Result<ExecutionResult, AreteError> {
        let wallet = options.wallet.clone().or_else(|| self.wallet());
        let Some(wallet) = wallet else {
            return Err(AreteError::TransactionFailed(Box::new(
                TransactionFailureOutcome::NotSubmitted {
                    phase: FailurePhase::Wallet,
                    message: "Wallet required to sign and send transaction".to_string(),
                },
            )));
        };
        // Pre-dispatch (contract §2/§3): refuse an undeclared explicit
        // version or a wrong-version fee option instead of downgrading.
        if let Err(error) = wallet
            .validate_transaction_options(options.send.transaction_version, &options.send.resources)
        {
            return Err(AreteError::TransactionFailed(Box::new(
                TransactionFailureOutcome::NotSubmitted {
                    phase: FailurePhase::Build,
                    message: error.to_string(),
                },
            )));
        }
        let context = WalletExecutionContext::new(Some(self.transactions.clone()));
        match wallet
            .sign_and_send(instructions, &options.send, &context)
            .await
        {
            Ok(result) => Ok(ExecutionResult {
                signature: result.signature,
                slot: result.slot,
            }),
            Err(error) => Err(AreteError::TransactionFailed(Box::new(
                classify_wallet_error(error, &options.errors),
            ))),
        }
    }

    /// Execute a prepared operation through the client's wallet (mirror of
    /// the TS `client.execute`).
    ///
    /// The client's wallet, signer registry, and transaction transport are
    /// merged into the execution; call options win (an explicit
    /// `options.signer_registry` replaces the client's registry).
    #[allow(clippy::result_large_err)]
    pub async fn execute(
        &self,
        operation: &PreparedOperation,
        options: ExecuteOptions,
    ) -> Result<OperationReceipt, OperationExecutionError> {
        let wallet = self.wallet();
        let host = ExecutionHost {
            wallet: wallet.as_deref(),
            available_signer_addresses: Vec::new(),
            transaction_transport: Some(self.transactions.clone()),
        };
        let mut options = options;
        if options.signer_registry.is_none() {
            options.signer_registry = Some(self.signer_registry.clone());
        }
        execute_prepared_operation(&host, operation, &options).await
    }

    /// Inspect a prepared operation without signing or submitting it (mirror
    /// of Python's `client.inspect_operation`).
    ///
    /// The wallet (call override or client default) receives the client's
    /// transaction transport via [`WalletExecutionContext`]; nothing on this
    /// path can reach the adapter's signing entry point.
    pub async fn inspect_operation(
        &self,
        operation: &PreparedOperation,
        options: OperationInspectionOptions,
    ) -> Result<OperationInspection, OperationInspectionError> {
        let wallet = options.wallet.clone().or_else(|| self.wallet());
        let context = WalletExecutionContext::new(Some(
            options
                .transaction_transport
                .clone()
                .unwrap_or_else(|| self.transactions.clone()),
        ));
        inspect_prepared_operation(wallet.as_deref(), operation, &options.inspect, &context).await
    }
}

/// Builder for Arete with custom configuration.
pub struct AreteBuilder<S: Stack> {
    url: String,
    http_url: Option<String>,
    transport: Transport,
    config: AreteConfig,
    wallet: Option<Arc<dyn WalletAdapter>>,
    chain: Option<Arc<dyn ChainClient>>,
    transactions: Option<Arc<dyn TransactionTransport>>,
    signer_registry: Option<Arc<SignerRegistry>>,
    _stack: PhantomData<S>,
}

impl<S: Stack> AreteBuilder<S> {
    fn new() -> Self {
        Self {
            url: S::url().to_string(),
            http_url: None,
            transport: Transport::default(),
            config: AreteConfig::default(),
            wallet: None,
            chain: None,
            transactions: None,
            signer_registry: None,
            _stack: PhantomData,
        }
    }

    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Explicit HTTP base URL for chain reads, the transaction relay, stack
    /// queries, and local program reads. When unset, the generated
    /// `Stack::http_url` is used, and failing that the base is derived from
    /// the WebSocket URL ([`derive_http_endpoint`]).
    pub fn http_url(mut self, http_url: impl Into<String>) -> Self {
        self.http_url = Some(http_url.into());
        self
    }

    /// Transport mode. [`Transport::Http`] skips the socket entirely: no
    /// connection is opened and view subscriptions fail with
    /// [`AreteError::WebSocketDisabled`].
    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = transport;
        self
    }

    /// Default wallet adapter used for instruction execution.
    pub fn wallet(mut self, wallet: Arc<dyn WalletAdapter>) -> Self {
        self.wallet = Some(wallet);
        self
    }

    /// Explicit chain transport override (used by composition sessions).
    pub fn chain(mut self, chain: Arc<dyn ChainClient>) -> Self {
        self.chain = Some(chain);
        self
    }

    /// Explicit transaction transport override (used by composition
    /// sessions).
    pub fn transactions(mut self, transactions: Arc<dyn TransactionTransport>) -> Self {
        self.transactions = Some(transactions);
        self
    }

    /// Shared signer registry merged into every executed operation.
    pub fn signer_registry(mut self, registry: Arc<SignerRegistry>) -> Self {
        self.signer_registry = Some(registry);
        self
    }

    pub fn auto_reconnect(mut self, enabled: bool) -> Self {
        self.config.auto_reconnect = enabled;
        self
    }

    pub fn reconnect_intervals(mut self, intervals: Vec<Duration>) -> Self {
        self.config.reconnect_intervals = intervals;
        self
    }

    pub fn max_reconnect_attempts(mut self, max: u32) -> Self {
        self.config.max_reconnect_attempts = max;
        self
    }

    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.config.ping_interval = interval;
        self
    }

    pub fn initial_data_timeout(mut self, timeout: Duration) -> Self {
        self.config.initial_data_timeout = timeout;
        self
    }

    pub fn max_entries_per_view(mut self, max: usize) -> Self {
        self.config.max_entries_per_view = Some(max);
        self
    }

    pub fn unlimited_entries(mut self) -> Self {
        self.config.max_entries_per_view = None;
        self
    }

    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.config.auth = Some(auth);
        self
    }

    pub fn auth_token(mut self, token: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token(token);
        self.config.auth = Some(auth);
        self
    }

    pub fn publishable_key(mut self, publishable_key: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_publishable_key(publishable_key);
        self.config.auth = Some(auth);
        self
    }

    /// Alias for `publishable_key` - use this for server-side code where
    /// the key could be either a secret key or a publishable key.
    pub fn api_key(self, api_key: impl Into<String>) -> Self {
        self.publishable_key(api_key)
    }

    pub fn token_endpoint(mut self, token_endpoint: impl Into<String>) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_endpoint(token_endpoint);
        self.config.auth = Some(auth);
        self
    }

    pub fn token_endpoint_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_endpoint_header(key, value);
        self.config.auth = Some(auth);
        self
    }

    /// Set a header sent on the WebSocket upgrade request.
    ///
    /// Required for non-browser clients (Rust CLIs, server-side code) talking
    /// to a hosted deployment with an origin-bound publishable key — the
    /// server demands an `Origin` header that matches the token's bound
    /// origin, and unlike browsers, tungstenite does not add one automatically.
    ///
    /// Header names are case-insensitive on the wire, so keys are normalized
    /// to lowercase on insertion. `Authorization` is reserved when using
    /// `TokenTransport::Bearer` — the SDK-managed Bearer token will overwrite
    /// any user-provided `Authorization` value to keep auth state consistent.
    pub fn handshake_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config
            .handshake_headers
            .insert(key.into().to_ascii_lowercase(), value.into());
        self
    }

    pub fn token_transport(mut self, transport: TokenTransport) -> Self {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_transport(transport);
        self.config.auth = Some(auth);
        self
    }

    pub fn get_token<F, Fut>(mut self, provider: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AuthToken, AreteError>> + Send + 'static,
    {
        let auth = self
            .config
            .auth
            .take()
            .unwrap_or_default()
            .with_token_provider(provider);
        self.config.auth = Some(auth);
        self
    }

    pub async fn connect(self) -> Result<Arete<S>, AreteError> {
        let AreteBuilder {
            url,
            http_url,
            transport,
            config,
            wallet,
            chain,
            transactions,
            signer_registry,
            _stack: _,
        } = self;

        if transport == Transport::WebSocket && url.is_empty() {
            return Err(AreteError::MissingUrl);
        }

        // Effective HTTP base: explicit builder URL > generated
        // Stack::http_url > derived from the WebSocket URL. (TS requires an
        // explicit httpUrl/endpoints.http; the Rust client derives it from
        // the WebSocket URL as documented on Stack::http_url.)
        let http_base_url = http_url
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                let generated = S::http_url();
                (!generated.trim().is_empty()).then(|| generated.to_string())
            })
            .or_else(|| (!url.is_empty()).then(|| derive_http_endpoint(&url)));

        let http = reqwest::Client::new();
        let auth_client = Arc::new(HttpAuthClient::new(
            config.auth.clone(),
            (!url.is_empty()).then(|| url.clone()),
            http.clone(),
        ));

        let (hosted_chain, hosted_transactions) = if chain.is_none() || transactions.is_none() {
            match S::gateway() {
                Some(bindings) => {
                    let (gateway_chain, gateway_transactions) =
                        create_hosted_solana_gateway_transports(
                            &bindings,
                            config.auth.clone(),
                            Some(http.clone()),
                        )?;
                    (Some(gateway_chain), Some(gateway_transactions))
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };
        let chain = chain.or(hosted_chain);
        let transactions = transactions.or(hosted_transactions);

        let chain: Arc<dyn ChainClient> = match chain {
            Some(chain) => chain,
            None => match &http_base_url {
                Some(base) => Arc::new(HttpChainClient::with_http_client(
                    base.clone(),
                    auth_client.clone() as Arc<dyn TokenSource>,
                    http.clone(),
                )),
                None => Arc::new(UnconfiguredChainClient { stack: S::name() }),
            },
        };
        let transactions: Arc<dyn TransactionTransport> = match transactions {
            Some(transactions) => transactions,
            None => match &http_base_url {
                Some(base) => Arc::new(HttpTransactionTransport::with_http_client(
                    base.clone(),
                    auth_client.clone() as Arc<dyn TokenSource>,
                    http.clone(),
                )),
                None => Arc::new(UnconfiguredTransactionTransport { stack: S::name() }),
            },
        };

        let store_config = StoreConfig {
            max_entries_per_view: config.max_entries_per_view,
        };
        let store = SharedStore::with_config(store_config);
        let connection_config: ConnectionConfig = config.clone().into();
        let connection = match transport {
            Transport::WebSocket => {
                ConnectionManager::new(url, connection_config, store.clone()).await?
            }
            Transport::Http => ConnectionManager::new_disabled(connection_config, store.clone()),
        };

        let view_builder = crate::view::ViewBuilder::new(
            connection.clone(),
            store.clone(),
            config.initial_data_timeout,
        );
        let views = S::Views::from_builder(view_builder);
        let program_builder = crate::program::ProgramBuilder::for_client(
            http,
            http_base_url.clone(),
            Some(auth_client.clone()),
            config.auth.clone(),
        );
        let programs = S::Programs::from_builder(program_builder);

        Ok(Arete {
            connection,
            store,
            config,
            transport,
            http_base_url,
            auth_client,
            chain,
            transactions,
            wallet: RwLock::new(wallet),
            signer_registry: signer_registry.unwrap_or_default(),
            views,
            programs,
            _stack: PhantomData,
        })
    }
}
