//! Multi-stack sessions (port of `typescript/core/src/session.ts`).
//!
//! A session composes multiple stack clients behind one wallet, one signer
//! registry, and one shared endpoint configuration. Every member gets its own
//! [`Arete`] client (connection + store); the **execution host** is the first
//! member in definition order, and [`Session::execute`] /
//! [`Session::transaction`] delegate to it. [`Session::set_wallet`] fans out
//! to every member and [`Session::close`] disconnects them all.
//!
//! ```ignore
//! let session = Session::builder()
//!     .stack::<OreStack>("ore")
//!     .stack_with::<OtherStack>("other", |m| m.url("wss://…").transport(Transport::Http))
//!     .wallet(wallet)
//!     .endpoints_http("http://127.0.0.1:4000")
//!     .connect()
//!     .await?;
//! let ore: &Arete<OreStack> = session.stack::<OreStack>("ore")?;
//! session.execute(&prepared, ExecuteOptions::default()).await?;
//! session.close().await;
//! ```
//!
//! ## Divergences from the TypeScript session (by design)
//!
//! - **Runtime-keyed members.** TS maps member keys to concrete client types
//!   at the type level; that requires codegen in Rust, so members are
//!   registered with a string key and recovered with the typed accessor
//!   [`Session::stack::<S>(key)`](Session::stack), which downcasts via `Any`
//!   and errors on an unknown key or a mismatched stack type.
//! - **No standalone `programs:` members.** TS promotes standalone program
//!   SDKs to synthetic HTTP-only stacks and hoists bundled programs onto
//!   `session.programs.<key>` (first-stack-wins). Rust program SDKs are
//!   generated per stack, so bundled programs are reached through their stack
//!   member: `session.stack::<S>(key)?.programs`.
//! - **Composition mode** maps to [`SessionBuilder::chain`] +
//!   [`SessionBuilder::transactions`]: the overrides are applied to every
//!   member client, so no member ever falls back to a live endpoint for chain
//!   reads or transaction dispatch (TS `mode: 'composition'`).
//! - **Signer registry scope.** The session registry participates in
//!   pre-dispatch signer validation for [`Session::execute`]. TS additionally
//!   merges registry values into the `signers` handed to `wallet.signAndSend`;
//!   Rust [`SendOptions`](crate::wallet::SendOptions) has no signers list, so
//!   adapters fetch registered signers from the shared registry instead.
//! - **Members connect sequentially** in definition order (TS connects them
//!   concurrently); the execution host is the first member either way.
//! - **Session-level chain over shared endpoints** authenticates with the
//!   session auth configuration (TS uses a raw unauthenticated `fetch` for
//!   this one path).

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::auth::AuthConfig;
use crate::chain::{ChainClient, HttpChainClient};
use crate::client::{Arete, ExecutionResult, TransactionOptions, Transport};
use crate::entity::Stack;
use crate::error::AreteError;
use crate::http::{HttpAuthClient, TokenSource};
use crate::instruction::BuiltInstruction;
use crate::operations::{
    ExecuteOptions, OperationExecutionError, OperationReceipt, PreparedOperation, SignerRegistry,
};
use crate::transactions::TransactionTransport;
use crate::wallet::WalletAdapter;

/// Per-member connection overrides (mirror of the TS `SessionMemberOptions`
/// subset expressible in Rust: `url`, `httpUrl`, `transport`, `auth`).
#[derive(Clone, Default)]
pub struct SessionMemberOptions {
    url: Option<String>,
    http_url: Option<String>,
    transport: Option<Transport>,
    auth: Option<AuthConfig>,
}

impl SessionMemberOptions {
    /// WebSocket URL override for this member.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// HTTP base URL override for this member.
    pub fn http_url(mut self, http_url: impl Into<String>) -> Self {
        self.http_url = Some(http_url.into());
        self
    }

    /// Transport override for this member.
    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Auth override for this member.
    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }
}

/// Object-safe view of a connected member used by the session runtime.
#[async_trait::async_trait]
trait SessionMember: Send + Sync {
    fn set_wallet(&self, wallet: Option<Arc<dyn WalletAdapter>>);
    fn transactions(&self) -> Arc<dyn TransactionTransport>;
    fn chain(&self) -> Arc<dyn ChainClient>;
    async fn disconnect(&self);
    async fn transaction(
        &self,
        instructions: &[BuiltInstruction],
        options: TransactionOptions,
    ) -> Result<ExecutionResult, AreteError>;
    async fn execute(
        &self,
        operation: &PreparedOperation,
        options: ExecuteOptions,
    ) -> Result<OperationReceipt, OperationExecutionError>;
    fn as_any(&self) -> &dyn Any;
    fn type_label(&self) -> &'static str;
}

#[async_trait::async_trait]
impl<S: Stack> SessionMember for Arete<S> {
    fn set_wallet(&self, wallet: Option<Arc<dyn WalletAdapter>>) {
        Arete::set_wallet(self, wallet);
    }

    fn transactions(&self) -> Arc<dyn TransactionTransport> {
        Arete::transactions(self)
    }

    fn chain(&self) -> Arc<dyn ChainClient> {
        Arete::chain(self)
    }

    async fn disconnect(&self) {
        Arete::disconnect(self).await;
    }

    async fn transaction(
        &self,
        instructions: &[BuiltInstruction],
        options: TransactionOptions,
    ) -> Result<ExecutionResult, AreteError> {
        Arete::transaction(self, instructions, options).await
    }

    async fn execute(
        &self,
        operation: &PreparedOperation,
        options: ExecuteOptions,
    ) -> Result<OperationReceipt, OperationExecutionError> {
        Arete::execute(self, operation, options).await
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn type_label(&self) -> &'static str {
        std::any::type_name::<S>()
    }
}

/// Shared options applied to every member at connect time.
#[derive(Clone, Default)]
struct SharedMemberOptions {
    endpoints_ws: Option<String>,
    endpoints_http: Option<String>,
    transport: Option<Transport>,
    auth: Option<AuthConfig>,
    wallet: Option<Arc<dyn WalletAdapter>>,
    signer_registry: Option<Arc<SignerRegistry>>,
    chain: Option<Arc<dyn ChainClient>>,
    transactions: Option<Arc<dyn TransactionTransport>>,
}

type MemberFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn SessionMember>, AreteError>> + Send>>;
type MemberConnector = Box<dyn FnOnce(SharedMemberOptions) -> MemberFuture + Send>;

/// Builder for a multi-stack [`Session`].
#[derive(Default)]
pub struct SessionBuilder {
    members: Vec<(String, MemberConnector)>,
    shared: SharedMemberOptions,
}

impl SessionBuilder {
    /// Add a stack member under `key` with default member options.
    pub fn stack<S: Stack>(self, key: impl Into<String>) -> Self {
        self.stack_with::<S>(key, |member| member)
    }

    /// Add a stack member under `key`, configuring its
    /// [`SessionMemberOptions`].
    pub fn stack_with<S: Stack>(
        mut self,
        key: impl Into<String>,
        configure: impl FnOnce(SessionMemberOptions) -> SessionMemberOptions,
    ) -> Self {
        let member = configure(SessionMemberOptions::default());
        let connector: MemberConnector = Box::new(move |shared: SharedMemberOptions| {
            Box::pin(async move {
                let mut builder = Arete::<S>::builder();
                // URL precedence mirrors the TS resolveMemberConnectOptions:
                // member.url ?? (stack.endpoints.ws || shared endpoints.ws).
                if let Some(url) = member.url.as_deref() {
                    builder = builder.url(url);
                } else if S::url().is_empty() {
                    if let Some(url) = shared.endpoints_ws.as_deref() {
                        builder = builder.url(url);
                    }
                }
                if let Some(http_url) = member.http_url.or(shared.endpoints_http) {
                    builder = builder.http_url(http_url);
                }
                builder =
                    builder.transport(member.transport.or(shared.transport).unwrap_or_default());
                if let Some(auth) = member.auth.or(shared.auth) {
                    builder = builder.auth(auth);
                }
                if let Some(wallet) = shared.wallet {
                    builder = builder.wallet(wallet);
                }
                if let Some(registry) = shared.signer_registry {
                    builder = builder.signer_registry(registry);
                }
                if let Some(chain) = shared.chain {
                    builder = builder.chain(chain);
                }
                if let Some(transactions) = shared.transactions {
                    builder = builder.transactions(transactions);
                }
                let client = builder.connect().await?;
                Ok(Arc::new(client) as Arc<dyn SessionMember>)
            })
        });
        self.members.push((key.into(), connector));
        self
    }

    /// One wallet governs execution across every member.
    pub fn wallet(mut self, wallet: Arc<dyn WalletAdapter>) -> Self {
        self.shared.wallet = Some(wallet);
        self
    }

    /// Signers available to every operation executed through this session
    /// (and injected into every member client).
    pub fn signer_registry(mut self, registry: Arc<SignerRegistry>) -> Self {
        self.shared.signer_registry = Some(registry);
        self
    }

    /// Shared fallback WebSocket endpoint, used when a member (and its
    /// generated stack) defines none of its own.
    pub fn endpoints_ws(mut self, url: impl Into<String>) -> Self {
        self.shared.endpoints_ws = Some(url.into());
        self
    }

    /// Shared fallback HTTP endpoint applied to every member without an
    /// explicit `http_url`.
    pub fn endpoints_http(mut self, url: impl Into<String>) -> Self {
        self.shared.endpoints_http = Some(url.into());
        self
    }

    /// Default transport for all members (WebSocket unless overridden).
    pub fn transport(mut self, transport: Transport) -> Self {
        self.shared.transport = Some(transport);
        self
    }

    /// Shared auth configuration for members without their own.
    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.shared.auth = Some(auth);
        self
    }

    /// Explicit chain transport applied to every member (composition mode:
    /// chain reads never inherit a live member's HTTP endpoint).
    pub fn chain(mut self, chain: Arc<dyn ChainClient>) -> Self {
        self.shared.chain = Some(chain);
        self
    }

    /// Explicit transaction transport applied to every member (composition
    /// mode: transactions never inherit a live member's HTTP endpoint).
    pub fn transactions(mut self, transactions: Arc<dyn TransactionTransport>) -> Self {
        self.shared.transactions = Some(transactions);
        self
    }

    /// Connect every member (sequentially, in definition order) and assemble
    /// the session.
    pub async fn connect(self) -> Result<Session, AreteError> {
        if self.members.is_empty() {
            return Err(AreteError::InvalidConfig(
                "Session requires at least one stack member".to_string(),
            ));
        }
        {
            let mut keys: Vec<&str> = self.members.iter().map(|(key, _)| key.as_str()).collect();
            keys.sort_unstable();
            if keys.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(AreteError::InvalidConfig(
                    "Session member keys must be unique".to_string(),
                ));
            }
        }

        let shared = self.shared;
        let mut members: Vec<(String, Arc<dyn SessionMember>)> =
            Vec::with_capacity(self.members.len());
        for (key, connector) in self.members {
            let client = connector(shared.clone()).await?;
            members.push((key, client));
        }

        let execution_host = members[0].1.clone();
        // TS: chain = options.chain ?? (endpoints.http ? chain-over-endpoint
        // : executionHost.chain); transactions = options.transactions ??
        // executionHost.transactions.
        let chain: Arc<dyn ChainClient> = match (&shared.chain, &shared.endpoints_http) {
            (Some(chain), _) => chain.clone(),
            (None, Some(endpoint)) => {
                let http = reqwest::Client::new();
                let tokens = Arc::new(HttpAuthClient::new(shared.auth.clone(), None, http.clone()))
                    as Arc<dyn TokenSource>;
                Arc::new(HttpChainClient::with_http_client(
                    endpoint.clone(),
                    tokens,
                    http,
                ))
            }
            (None, None) => execution_host.chain(),
        };
        let transactions = shared
            .transactions
            .clone()
            .unwrap_or_else(|| execution_host.transactions());

        Ok(Session {
            members,
            wallet: RwLock::new(shared.wallet),
            signer_registry: shared.signer_registry.unwrap_or_default(),
            chain,
            transactions,
        })
    }
}

/// A connected multi-stack session.
pub struct Session {
    members: Vec<(String, Arc<dyn SessionMember>)>,
    wallet: RwLock<Option<Arc<dyn WalletAdapter>>>,
    signer_registry: Arc<SignerRegistry>,
    chain: Arc<dyn ChainClient>,
    transactions: Arc<dyn TransactionTransport>,
}

impl Session {
    /// Create a [`SessionBuilder`].
    pub fn builder() -> SessionBuilder {
        SessionBuilder::default()
    }

    /// Typed accessor for the member registered under `key`.
    ///
    /// Errors with [`AreteError::InvalidConfig`] when the key is unknown or
    /// the member was registered with a different stack type.
    pub fn stack<S: Stack>(&self, key: &str) -> Result<&Arete<S>, AreteError> {
        let member = self
            .members
            .iter()
            .find(|(member_key, _)| member_key == key)
            .map(|(_, member)| member)
            .ok_or_else(|| {
                AreteError::InvalidConfig(format!("Session has no stack member '{key}'"))
            })?;
        member.as_any().downcast_ref::<Arete<S>>().ok_or_else(|| {
            AreteError::InvalidConfig(format!(
                "Session member '{key}' is bound to stack type {}, not {}",
                member.type_label(),
                std::any::type_name::<S>()
            ))
        })
    }

    /// The member keys, in definition (connection) order.
    pub fn keys(&self) -> Vec<&str> {
        self.members.iter().map(|(key, _)| key.as_str()).collect()
    }

    /// The session's shared wallet, if one is set.
    pub fn wallet(&self) -> Option<Arc<dyn WalletAdapter>> {
        self.wallet
            .read()
            .expect("session wallet lock poisoned")
            .clone()
    }

    /// Set (or clear) the shared wallet; fans out to every member.
    pub fn set_wallet(&self, wallet: Option<Arc<dyn WalletAdapter>>) {
        *self.wallet.write().expect("session wallet lock poisoned") = wallet.clone();
        for (_, member) in &self.members {
            member.set_wallet(wallet.clone());
        }
    }

    /// Signers available to every operation executed through this session.
    pub fn signer_registry(&self) -> Arc<SignerRegistry> {
        self.signer_registry.clone()
    }

    /// The session's canonical chain reader.
    pub fn chain(&self) -> Arc<dyn ChainClient> {
        self.chain.clone()
    }

    /// The session's transaction transport.
    pub fn transactions(&self) -> Arc<dyn TransactionTransport> {
        self.transactions.clone()
    }

    /// Sign and send instructions through the execution host (the first
    /// connected member).
    pub async fn transaction(
        &self,
        instructions: &[BuiltInstruction],
        options: TransactionOptions,
    ) -> Result<ExecutionResult, AreteError> {
        self.members[0].1.transaction(instructions, options).await
    }

    /// Execute a prepared operation through the execution host (the first
    /// connected member). The session signer registry is merged in unless
    /// the call provides its own (call options win).
    pub async fn execute(
        &self,
        operation: &PreparedOperation,
        options: ExecuteOptions,
    ) -> Result<OperationReceipt, OperationExecutionError> {
        let mut options = options;
        if options.signer_registry.is_none() {
            options.signer_registry = Some(self.signer_registry.clone());
        }
        self.members[0].1.execute(operation, options).await
    }

    /// Disconnect every member.
    pub async fn close(&self) {
        for (_, member) in &self.members {
            member.disconnect().await;
        }
    }
}
