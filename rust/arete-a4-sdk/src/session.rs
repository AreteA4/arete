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
//!     .program::<SplPrograms>("spl")
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
//! - **Typed standalone programs.** Generated program SDK aggregates are
//!   registered with [`SessionBuilder::program`] and recovered with
//!   [`Session::program`]. Rust keeps the concrete program type in the
//!   accessor instead of building a runtime-keyed `session.programs` object.
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
//!
//! ## Protocols
//!
//! [`SessionBuilder::protocol`] registers a protocol client under a key
//! ([`Session::protocol`]), and [`Session::protocol_client`] builds one on
//! demand (generated clients' `from_session`). Either way the client uses
//! the session's auth (unless its options carry their own) and the
//! session-wide [`SessionBuilder::protocol_key_class`], and its instance
//! handles follow the protocol's live stack view through the session's
//! connected members (TS `session.protocols`). Differences:
//!
//! - **Stacks are not hoisted.** A Rust [`ProtocolDefinition`] does not
//!   carry its stacks' generated types, so the session cannot connect them
//!   itself: register the protocol's live stack as a member under the key
//!   the definition names (`live_stack.stack`), e.g.
//!   `.stack::<MeteoraPresaleStack>("meteoraPresale")`. A protocol whose live
//!   stack is not a member still works; its handles carry no live row.
//! - **Programs are not wired** onto the protocol client: Rust protocol
//!   clients prepare through the hosted API and expose no `programs`; use
//!   [`Session::program`] for the program SDKs.
//! - **No shared fetch.** Protocol clients keep their own HTTP client unless
//!   their options pass one.

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
use crate::program::{ProgramSdk, ProgramStack};
use crate::protocol::{
    ProtocolClient, ProtocolClientOptions, ProtocolDefinition, ProtocolError, ProtocolKeyClass,
    ProtocolStackViews, StateViewSource,
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
    fn state_view_source(&self) -> StateViewSource;
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

    fn state_view_source(&self) -> StateViewSource {
        Arete::state_view_source(self)
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

type MemberFuture = Pin<
    Box<dyn Future<Output = Result<(Arc<dyn SessionMember>, Option<String>), AreteError>> + Send>,
>;
type MemberConnector = Box<dyn FnOnce(SharedMemberOptions) -> MemberFuture + Send>;

/// Builder for a multi-stack [`Session`].
#[derive(Default)]
pub struct SessionBuilder {
    members: Vec<(String, MemberConnector)>,
    shared: SharedMemberOptions,
    protocols: Vec<(String, ProtocolDefinition, ProtocolClientOptions)>,
    protocol_key_class: Option<ProtocolKeyClass>,
}

impl SessionBuilder {
    /// Add a stack member under `key` with default member options.
    pub fn stack<S: Stack>(self, key: impl Into<String>) -> Self {
        self.stack_with::<S>(key, |member| member)
    }

    /// Add a standalone generated program SDK as an HTTP-only member.
    pub fn program<P: ProgramSdk>(self, key: impl Into<String>) -> Self {
        self.program_with::<P>(key, |member| member)
    }

    /// Add and configure a standalone generated program SDK. HTTP transport
    /// is the default because standalone programs carry no live views.
    pub fn program_with<P: ProgramSdk>(
        self,
        key: impl Into<String>,
        configure: impl FnOnce(SessionMemberOptions) -> SessionMemberOptions,
    ) -> Self {
        self.stack_with::<ProgramStack<P>>(key, |member| {
            let mut member = configure(member);
            if member.transport.is_none() {
                member.transport = Some(Transport::Http);
            }
            member
        })
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
                let websocket_url = member.url.clone().or_else(|| {
                    let generated = S::url();
                    if generated.is_empty() {
                        shared.endpoints_ws.clone()
                    } else {
                        Some(generated.to_string())
                    }
                });
                if let Some(url) = websocket_url.as_deref() {
                    builder = builder.url(url);
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
                Ok((Arc::new(client) as Arc<dyn SessionMember>, websocket_url))
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

    /// Register a protocol client under `key` ([`Session::protocol`]), built
    /// at connect time like [`Session::protocol_client`].
    pub fn protocol(
        mut self,
        key: impl Into<String>,
        definition: &ProtocolDefinition,
        options: ProtocolClientOptions,
    ) -> Self {
        self.protocols.push((key.into(), *definition, options));
        self
    }

    /// The key class of the session's protocol clients when the session
    /// auth cannot tell (tokens, providers, endpoints); TS
    /// `protocolKeyClass`.
    pub fn protocol_key_class(mut self, key_class: ProtocolKeyClass) -> Self {
        self.protocol_key_class = Some(key_class);
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

        {
            let mut keys: Vec<&str> = self
                .protocols
                .iter()
                .map(|(key, ..)| key.as_str())
                .collect();
            keys.sort_unstable();
            if keys.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(AreteError::InvalidConfig(
                    "Session protocol keys must be unique".to_string(),
                ));
            }
        }

        let mut shared = self.shared;
        shared.auth = crate::auth::resolve_auth_config(shared.auth)?;
        let mut members: Vec<(String, Arc<dyn SessionMember>)> =
            Vec::with_capacity(self.members.len());
        let mut canonical_websocket_url = None;
        for (key, connector) in self.members {
            let (client, websocket_url) = connector(shared.clone()).await?;
            if members.is_empty() {
                canonical_websocket_url = websocket_url;
            }
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
                let tokens = Arc::new(HttpAuthClient::new(
                    shared.auth.clone(),
                    canonical_websocket_url,
                    http.clone(),
                )) as Arc<dyn TokenSource>;
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

        let mut session = Session {
            members,
            wallet: RwLock::new(shared.wallet),
            signer_registry: shared.signer_registry.unwrap_or_default(),
            chain,
            transactions,
            auth: shared.auth,
            protocol_key_class: self.protocol_key_class,
            protocols: Vec::with_capacity(self.protocols.len()),
        };
        for (key, definition, options) in self.protocols {
            let client = session
                .protocol_client(&definition, options)
                .map_err(|error| {
                    AreteError::InvalidConfig(format!("Session protocol '{key}': {error}"))
                })?;
            if let Some(live) = definition.live_stack {
                if !session.keys().contains(&live.stack) {
                    tracing::warn!(
                        protocol = %key,
                        stack = live.stack,
                        "protocol live stack is not a session member; its instance handles carry no live row"
                    );
                }
            }
            session.protocols.push((key, client));
        }
        Ok(session)
    }
}

/// A connected multi-stack session.
pub struct Session {
    members: Vec<(String, Arc<dyn SessionMember>)>,
    wallet: RwLock<Option<Arc<dyn WalletAdapter>>>,
    signer_registry: Arc<SignerRegistry>,
    chain: Arc<dyn ChainClient>,
    transactions: Arc<dyn TransactionTransport>,
    auth: Option<AuthConfig>,
    protocol_key_class: Option<ProtocolKeyClass>,
    protocols: Vec<(String, ProtocolClient)>,
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

    /// Typed accessor for a standalone program SDK registered with
    /// [`SessionBuilder::program`].
    pub fn program<P: ProgramSdk>(&self, key: &str) -> Result<&P, AreteError> {
        Ok(&self.stack::<ProgramStack<P>>(key)?.programs)
    }

    /// The protocol client registered under `key` with
    /// [`SessionBuilder::protocol`].
    pub fn protocol(&self, key: &str) -> Result<&ProtocolClient, AreteError> {
        self.protocols
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, client)| client)
            .ok_or_else(|| AreteError::InvalidConfig(format!("Session has no protocol '{key}'")))
    }

    /// The registered protocol keys, in registration order.
    pub fn protocol_keys(&self) -> Vec<&str> {
        self.protocols.iter().map(|(key, _)| key.as_str()).collect()
    }

    /// The session's members as live views for protocol instance handles,
    /// keyed by member key.
    pub fn live_views(&self) -> ProtocolStackViews {
        self.members
            .iter()
            .fold(ProtocolStackViews::new(), |views, (key, member)| {
                let source = member.state_view_source();
                views.with_listener(key.clone(), move |view, row| {
                    Box::pin(source(view).listen(row))
                })
            })
    }

    /// A protocol client sharing this session: the session's auth unless
    /// `options` has its own, the session's protocol key class unless
    /// `options` names one, and the session's members as live views unless
    /// `options` brings its own.
    pub fn protocol_client(
        &self,
        definition: &ProtocolDefinition,
        mut options: ProtocolClientOptions,
    ) -> Result<ProtocolClient, ProtocolError> {
        if options.auth.is_none() {
            options.auth = self.auth.clone();
        }
        if options.key_class.is_none() {
            options.key_class = self.protocol_key_class;
        }
        if options.live_views.is_none() {
            options.live_views = Some(Arc::new(self.live_views()));
        }
        ProtocolClient::new(definition, options)
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
    #[allow(clippy::result_large_err)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProgramBuilder, Programs, StackWithPrograms};

    struct TestProgramSdk;

    impl Programs for TestProgramSdk {
        fn from_builder(_builder: ProgramBuilder) -> Self {
            Self
        }
    }

    impl ProgramSdk for TestProgramSdk {
        fn name() -> &'static str {
            "test-program"
        }
    }

    struct TestLiveStack;

    impl Stack for TestLiveStack {
        type Views = ();
        type Programs = ();

        fn name() -> &'static str {
            "test-live"
        }

        fn url() -> &'static str {
            ""
        }
    }

    #[tokio::test]
    async fn standalone_programs_compose_as_http_only_session_members() {
        let session = Session::builder()
            .program::<TestProgramSdk>("test")
            .connect()
            .await
            .expect("standalone program session should connect without a live URL");

        assert_eq!(session.keys(), vec!["test"]);
        let _program: &TestProgramSdk = session
            .program("test")
            .expect("typed standalone program accessor");
        session.close().await;
    }

    #[tokio::test]
    async fn standalone_programs_attach_to_live_stack_types() {
        type EnrichedStack = StackWithPrograms<TestLiveStack, TestProgramSdk>;
        let session = Session::builder()
            .stack_with::<EnrichedStack>("live", |member| member.transport(Transport::Http))
            .connect()
            .await
            .expect("attached program stack should connect");

        let client = session
            .stack::<EnrichedStack>("live")
            .expect("enriched stack accessor");
        let _attached: &TestProgramSdk = &client.programs.attached;
        session.close().await;
    }

    const RAISE: ProtocolDefinition = ProtocolDefinition {
        name: "tokenRaise",
        version: "1.0.0",
        package: "token-raise",
        package_release_hash: "release",
        protocol_spec_hash: "spec",
        protocol_release_hash: "protocol-release",
        deployment: crate::protocol::ProtocolDeploymentDescriptor {
            deployment_id: "dep_1",
            access: crate::protocol::ProtocolDeploymentAccess::Public,
            api_url: "https://api.example.test/api",
            session_endpoint: "https://api.example.test/ws/sessions",
        },
        instance_visibility: crate::protocol::ProtocolInstanceVisibility::Account,
        steps: &[],
        events: &[],
        instance_metadata: &[],
        participant_metadata: &[],
        live_stack: Some(crate::protocol::ProtocolLiveStackDescriptor {
            stack: "raiseStack",
            view: "TokenRaiseInstance/state",
            key_account: "presale",
        }),
    };

    #[tokio::test]
    async fn protocols_share_the_session_auth_and_members() {
        let session = Session::builder()
            .stack_with::<TestLiveStack>("raiseStack", |member| member.transport(Transport::Http))
            .auth(AuthConfig::default().with_secret_key("a4_sk_test"))
            .protocol("raise", &RAISE, ProtocolClientOptions::default())
            .connect()
            .await
            .expect("session with a protocol should connect");

        assert_eq!(session.protocol_keys(), vec!["raise"]);
        let raise = session.protocol("raise").expect("registered protocol");
        assert_eq!(raise.key_class(), ProtocolKeyClass::Secret);
        assert_eq!(raise.deployment_id(), "dep_1");
        let views = raise.live_views().expect("session members as live views");
        assert!(views
            .listen("raiseStack", "TokenRaiseInstance/state", "P1")
            .is_some());
        assert!(views.listen("other", "X/state", "P1").is_none());
        assert!(session.protocol("missing").is_err());

        // On demand, with options of its own.
        let publishable = session
            .protocol_client(
                &RAISE,
                ProtocolClientOptions::default().with_publishable_key("a4_pk_test"),
            )
            .expect("protocol client");
        assert_eq!(publishable.key_class(), ProtocolKeyClass::Publishable);
        session.close().await;
    }

    #[tokio::test]
    async fn protocol_keys_are_unique() {
        let error = Session::builder()
            .program::<TestProgramSdk>("test")
            .protocol("raise", &RAISE, ProtocolClientOptions::default())
            .protocol("raise", &RAISE, ProtocolClientOptions::default())
            .connect()
            .await
            .err()
            .expect("duplicate protocol keys");
        assert!(error.to_string().contains("unique"), "{error}");
    }
}
