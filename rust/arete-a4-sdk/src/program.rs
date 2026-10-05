//! Program SDK binding.
//!
//! Mirrors the TypeScript `client.programs.<name>` namespace. Generated stack
//! code implements [`Programs`] with one field per bundled program; each
//! program client exposes typed instruction builders that produce
//! [`crate::instruction::BuiltInstruction`] values without any network access,
//! plus HTTP account readers built from the [`ProgramBuilder`] runtime carried
//! here.
//!
//! Stacks without bundled programs use `()` as their `Programs` type.

use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use crate::auth::AuthConfig;
use crate::chain::ChainClient;
use crate::error::AreteError;
use crate::http::HttpAuthClient;
use crate::program_read_transport::{BearerTokenSource, ProgramReadTransport};
use crate::read::{
    validate_program_read_descriptor, ProgramReadDescriptor, QueryExecutor, ReadError,
};
use crate::wallet::WalletAdapter;

/// A client's default wallet, shared by the client and the program runtime
/// it hands to generated program accessors, so
/// [`Arete::set_wallet`](crate::Arete::set_wallet) reaches every
/// [`ProgramContext`].
pub(crate) type WalletSlot = Arc<RwLock<Option<Arc<dyn WalletAdapter>>>>;

fn invalid_config(error: ReadError) -> AreteError {
    match error {
        ReadError::InvalidConfig { message } => AreteError::InvalidConfig(message),
        ReadError::Auth(inner) => inner,
        other => AreteError::InvalidConfig(other.to_string()),
    }
}

/// Runtime context handed to generated program accessors.
///
/// Mirror of the transport half of the TS connected client: it carries the
/// shared HTTP client, the effective stack HTTP base URL, and the client's
/// auth machinery so generated programs can construct release-addressed
/// account read transports ([`ProgramBuilder::account_transport`]) and query
/// executors ([`ProgramBuilder::query_executor`]).
///
/// It also carries the client's chain reader and default wallet, which a
/// [`ProgramContext`] hands to program extension functions.
///
/// [`ProgramBuilder::new`] / `Default` build a bare context (fresh
/// `reqwest::Client`, no HTTP base, no auth, no chain reader, no wallet) so
/// program-less test stacks keep working; connected clients construct it
/// internally with their shared runtime. [`ProgramBuilder::with_chain`] and
/// [`ProgramBuilder::with_wallet`] wire a bare context by hand (tests and
/// conformance harnesses).
#[derive(Clone, Default)]
pub struct ProgramBuilder {
    http: Option<reqwest::Client>,
    http_base_url: Option<String>,
    auth: Option<Arc<HttpAuthClient>>,
    auth_config: Option<AuthConfig>,
    chain: Option<Arc<dyn ChainClient>>,
    wallet: Option<WalletSlot>,
}

impl ProgramBuilder {
    /// Bare context for program-less/test stacks: lazily-created HTTP client,
    /// no HTTP base URL, no auth, no chain reader and no wallet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Context wired with the client's shared runtime.
    pub(crate) fn for_client(
        http: reqwest::Client,
        http_base_url: Option<String>,
        auth: Option<Arc<HttpAuthClient>>,
        auth_config: Option<AuthConfig>,
    ) -> Self {
        Self {
            http: Some(http),
            http_base_url,
            auth,
            auth_config,
            chain: None,
            wallet: None,
        }
    }

    /// The connected client's chain reader and its (settable) default
    /// wallet.
    pub(crate) fn with_client_runtime(
        mut self,
        chain: Arc<dyn ChainClient>,
        wallet: WalletSlot,
    ) -> Self {
        self.chain = Some(chain);
        self.wallet = Some(wallet);
        self
    }

    /// Use `chain` as the chain reader handed to program contexts.
    pub fn with_chain(mut self, chain: Arc<dyn ChainClient>) -> Self {
        self.chain = Some(chain);
        self
    }

    /// Use `wallet` as the wallet handed to program contexts.
    pub fn with_wallet(mut self, wallet: Option<Arc<dyn WalletAdapter>>) -> Self {
        self.wallet = Some(Arc::new(RwLock::new(wallet)));
        self
    }

    /// The client's chain reader. A builder that was never connected to a
    /// client returns a reader whose every call fails with
    /// [`AreteError::InvalidConfig`].
    pub fn chain(&self) -> Arc<dyn ChainClient> {
        self.chain.clone().unwrap_or_else(|| {
            Arc::new(crate::client::UnconfiguredChainClient::detached()) as Arc<dyn ChainClient>
        })
    }

    /// The client's current default wallet, if one is set.
    pub fn wallet(&self) -> Option<Arc<dyn WalletAdapter>> {
        self.wallet
            .as_ref()
            .and_then(|slot| slot.read().expect("wallet lock poisoned").clone())
    }

    /// The shared HTTP client (created lazily for bare builders).
    pub fn http(&self) -> reqwest::Client {
        self.http.clone().unwrap_or_default()
    }

    /// The effective stack HTTP base URL, when one is configured/derivable.
    pub fn http_base_url(&self) -> Option<&str> {
        self.http_base_url.as_deref()
    }

    /// The client's shared token machinery, usable as
    /// [`crate::http::TokenSource`] or [`BearerTokenSource`].
    pub fn token_source(&self) -> Option<Arc<HttpAuthClient>> {
        self.auth.clone()
    }

    /// Build the release-addressed account read transport for one program.
    ///
    /// Validates `descriptor` ([`validate_program_read_descriptor`]) and
    /// constructs:
    ///
    /// - `local-http`: a transport over the client's HTTP base URL. Errors
    ///   with [`AreteError::InvalidConfig`] naming the program when the client
    ///   has no HTTP endpoint (mirror of the TS `INVALID_CONFIG` "requires
    ///   ConnectOptions.httpUrl" failure).
    /// - `hosted-binding`: a transport over the binding endpoint. Auth
    ///   follows the TS `hostedAuthConfig` rules: a configured runtime
    ///   strategy (token / provider / token endpoint) wins; otherwise, unless
    ///   the binding says `required == false`, tokens are minted from the
    ///   binding's `sessionEndpoint` (keeping any publishable key and custom
    ///   headers).
    pub fn account_transport(
        &self,
        program_name: &str,
        descriptor: &ProgramReadDescriptor,
    ) -> Result<ProgramReadTransport, AreteError> {
        validate_program_read_descriptor(program_name, descriptor).map_err(invalid_config)?;
        match descriptor {
            ProgramReadDescriptor::LocalHttp { release } => {
                let Some(base) = self.http_base_url.as_deref() else {
                    return Err(AreteError::InvalidConfig(format!(
                        "Program '{program_name}' local HTTP transport requires an HTTP endpoint \
                         (provide AreteBuilder::http_url or generate Stack::http_url)"
                    )));
                };
                Ok(ProgramReadTransport::local_http(
                    base,
                    release.clone(),
                    self.http(),
                ))
            }
            ProgramReadDescriptor::HostedBinding { release, binding } => {
                let auth = self.hosted_auth(binding.auth.required, &binding.auth.session_endpoint);
                Ok(ProgramReadTransport::hosted(
                    binding,
                    release.clone(),
                    auth,
                    self.http(),
                ))
            }
        }
    }

    /// Query executor over the client's HTTP base URL (stack- and
    /// program-scoped queries). Errors with [`AreteError::InvalidConfig`]
    /// when the client has no HTTP endpoint.
    pub fn query_executor(&self) -> Result<QueryExecutor, AreteError> {
        let Some(base) = self.http_base_url.as_deref() else {
            return Err(AreteError::InvalidConfig(
                "Stack queries require an HTTP endpoint (provide AreteBuilder::http_url or \
                 generate Stack::http_url)"
                    .to_string(),
            ));
        };
        let mut executor = QueryExecutor::new(base, self.http());
        if let Some(auth) = &self.auth {
            executor = executor.with_auth(auth.clone() as Arc<dyn BearerTokenSource>);
        }
        Ok(executor)
    }

    /// TS `hostedAuthConfig`: runtime strategy wins; `required == false`
    /// falls back to whatever runtime auth exists (which may mint nothing);
    /// otherwise mint from the binding session endpoint.
    fn hosted_auth(
        &self,
        required: Option<bool>,
        session_endpoint: &str,
    ) -> Option<Arc<dyn BearerTokenSource>> {
        let runtime_strategy_configured = self.auth_config.as_ref().is_some_and(|auth| {
            auth.token.is_some() || auth.get_token.is_some() || auth.token_endpoint.is_some()
        });
        if runtime_strategy_configured || required == Some(false) {
            return self
                .auth
                .clone()
                .map(|auth| auth as Arc<dyn BearerTokenSource>);
        }
        let mut config = self.auth_config.clone().unwrap_or_default();
        config.token_endpoint = Some(session_endpoint.to_string());
        Some(
            Arc::new(HttpAuthClient::new(Some(config), None, self.http()))
                as Arc<dyn BearerTokenSource>,
        )
    }
}

/// Trait for generated program accessor structs, mirroring [`crate::view::Views`].
pub trait Programs: Sized + Send + Sync + 'static {
    fn from_builder(builder: ProgramBuilder) -> Self;
}

/// One generated program accessor (`client.programs.<name>`, or the
/// `<Name>Program` field of a standalone program SDK aggregate).
///
/// Every generated accessor implements it, which is what lets any accessor
/// produce a [`ProgramContext`], whether it belongs to a standalone program
/// crate or to a stack client.
pub trait ProgramAccessor {
    /// The client runtime the accessor was built from.
    fn program_builder(&self) -> &ProgramBuilder;

    /// The context program extension functions take: the client's chain
    /// reader, its current wallet, and this accessor.
    fn context(&self) -> ProgramContext<'_, Self>
    where
        Self: Sized,
    {
        ProgramContext::new(self)
    }
}

/// What a program extension function receives (the Rust counterpart of the
/// TypeScript and Python `ProgramOperationContext`): the connected client's
/// chain reader, its wallet, and the generated program accessor, with its
/// typed instruction builders and account readers.
///
/// Program extension bundles take it first:
///
/// ```ignore
/// pub async fn deploy(
///     ctx: &ProgramContext<'_, OreProgram>,
///     input: DeployInput,
/// ) -> Result<PreparedOperation, AreteError> { … }
///
/// // Standalone: client.programs.ore.context()
/// // In a stack: a4.programs.ore.context()
/// let prepared = ore_program::transactions::mining::deploy(&a4.programs.ore.context(), input).await?;
/// ```
pub struct ProgramContext<'a, P: ?Sized> {
    program: &'a P,
    chain: Arc<dyn ChainClient>,
    wallet: WalletSlot,
}

impl<'a, P: ProgramAccessor + ?Sized> ProgramContext<'a, P> {
    /// The context of a generated program accessor: its client's chain
    /// reader and default wallet (read when used, so a later
    /// [`Arete::set_wallet`](crate::Arete::set_wallet) is seen).
    pub fn new(program: &'a P) -> Self {
        let builder = program.program_builder();
        Self {
            program,
            chain: builder.chain(),
            wallet: builder
                .wallet
                .clone()
                .unwrap_or_else(|| Arc::new(RwLock::new(None))),
        }
    }
}

impl<'a, P: ?Sized> ProgramContext<'a, P> {
    /// A context from explicit parts (tests and conformance harnesses).
    pub fn from_parts(
        program: &'a P,
        chain: Arc<dyn ChainClient>,
        wallet: Option<Arc<dyn WalletAdapter>>,
    ) -> Self {
        Self {
            program,
            chain,
            wallet: Arc::new(RwLock::new(wallet)),
        }
    }

    /// This context with another chain reader.
    pub fn with_chain(mut self, chain: Arc<dyn ChainClient>) -> Self {
        self.chain = chain;
        self
    }

    /// This context with another wallet, fixed for its lifetime.
    pub fn with_wallet(mut self, wallet: Option<Arc<dyn WalletAdapter>>) -> Self {
        self.wallet = Arc::new(RwLock::new(wallet));
        self
    }

    /// The generated program accessor: typed instruction builders and
    /// account readers.
    pub fn program(&self) -> &'a P {
        self.program
    }

    /// Chain reads (`/chain/*`) of the client the program is connected to.
    pub fn chain(&self) -> Arc<dyn ChainClient> {
        self.chain.clone()
    }

    /// The client's current default wallet, if one is set.
    pub fn wallet(&self) -> Option<Arc<dyn WalletAdapter>> {
        self.wallet.read().expect("wallet lock poisoned").clone()
    }

    /// The current wallet's address, if a wallet is set.
    pub fn public_key(&self) -> Option<String> {
        self.wallet().map(|wallet| wallet.public_key())
    }
}

impl<P: ?Sized> Clone for ProgramContext<'_, P> {
    fn clone(&self) -> Self {
        Self {
            program: self.program,
            chain: self.chain.clone(),
            wallet: self.wallet.clone(),
        }
    }
}

impl<P: ?Sized> std::fmt::Debug for ProgramContext<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgramContext")
            .field("program", &std::any::type_name::<P>())
            .field("wallet", &self.public_key())
            .finish_non_exhaustive()
    }
}

/// A generated standalone program-SDK aggregate.
///
/// Program SDK crates implement this trait on their exported aggregate so it
/// can be connected through [`ProgramStack`] or registered directly with a
/// multi-member [`Session`](crate::Session).
pub trait ProgramSdk: Programs {
    fn name() -> &'static str;

    fn gateway() -> Option<crate::HostedSolanaGatewayBindings> {
        None
    }

    /// The package release this SDK was generated from, when the generator
    /// knew it. Local builds leave it unset. Two SDKs carrying the same one
    /// are the same program; see [`same_program`].
    fn package_release_hash() -> Option<&'static str> {
        None
    }
}

/// Canonical program identity (`docs/internal/sdk-core-api.md` §9): the same
/// generated type, or two SDKs that both carry a
/// [`package_release_hash`](ProgramSdk::package_release_hash) and agree on
/// it. Names never decide it.
///
/// Rust reaches every program through a typed path — a stack's `programs`
/// fields, [`AttachedPrograms::stack`] / [`AttachedPrograms::attached`], or
/// [`Session::program`](crate::Session::program) under its own member key —
/// so no runtime key can name two programs and the `PROGRAM_KEY_CONFLICT`
/// case of the other SDKs cannot arise. Use this to decide whether two
/// generated SDKs are one release, e.g. before holding both.
pub fn same_program<A: ProgramSdk, B: ProgramSdk>() -> bool {
    if std::any::TypeId::of::<A>() == std::any::TypeId::of::<B>() {
        return true;
    }
    matches!(
        (A::package_release_hash(), B::package_release_hash()),
        (Some(left), Some(right)) if !left.is_empty() && left == right
    )
}

/// HTTP-only stack adapter used internally to connect a standalone program
/// SDK through the same client/runtime machinery as a live stack. It carries
/// no view definition or view data.
pub struct ProgramStack<P: ProgramSdk>(PhantomData<fn() -> P>);

impl<P: ProgramSdk> crate::Stack for ProgramStack<P> {
    type Views = ();
    type Programs = P;

    fn name() -> &'static str {
        P::name()
    }

    fn url() -> &'static str {
        ""
    }

    fn gateway() -> Option<crate::HostedSolanaGatewayBindings> {
        P::gateway()
    }
}

/// Program namespace produced by [`StackWithPrograms`]. Existing stack
/// programs remain under `stack`; the attached standalone SDK is under
/// `attached`, avoiding compile-time field-name collisions.
pub struct AttachedPrograms<Base: Programs, Attached: ProgramSdk> {
    pub stack: Base,
    pub attached: Attached,
}

impl<Base: Programs, Attached: ProgramSdk> Programs for AttachedPrograms<Base, Attached> {
    fn from_builder(builder: ProgramBuilder) -> Self {
        Self {
            stack: Base::from_builder(builder.clone()),
            attached: Attached::from_builder(builder),
        }
    }
}

/// A live stack with an independently generated program SDK attached.
///
/// ```ignore
/// type OreWithSpl = StackWithPrograms<OreStack, SplPrograms>;
/// let client = Arete::<OreWithSpl>::connect().await?;
/// let ix = client.programs.attached.spl.transfer(params)?;
/// ```
///
/// Views and endpoints come exclusively from `S`; attaching a program never
/// creates or changes view data.
pub struct StackWithPrograms<S: crate::Stack, P: ProgramSdk>(PhantomData<fn() -> (S, P)>);

impl<S: crate::Stack, P: ProgramSdk> crate::Stack for StackWithPrograms<S, P> {
    type Views = S::Views;
    type Programs = AttachedPrograms<S::Programs, P>;

    fn name() -> &'static str {
        S::name()
    }

    fn url() -> &'static str {
        S::url()
    }

    fn http_url() -> &'static str {
        S::http_url()
    }

    fn gateway() -> Option<crate::HostedSolanaGatewayBindings> {
        S::gateway().or_else(P::gateway)
    }

    fn stack_manifest_hash() -> Option<&'static str> {
        S::stack_manifest_hash()
    }

    fn live_alias() -> Option<&'static str> {
        S::live_alias()
    }
}

/// Program-less stacks bind `()`.
impl Programs for () {
    fn from_builder(_builder: ProgramBuilder) -> Self {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::ProgramReleaseReference;

    fn release() -> ProgramReleaseReference {
        ProgramReleaseReference {
            program_release_hash: "arete:h1:release".to_string(),
            program_spec_hash: "arete:h1:spec".to_string(),
        }
    }

    #[test]
    fn bare_builder_rejects_local_http_transport_naming_the_program() {
        let builder = ProgramBuilder::new();
        let descriptor = ProgramReadDescriptor::LocalHttp { release: release() };
        let error = builder.account_transport("ore", &descriptor).err().unwrap();
        match error {
            AreteError::InvalidConfig(message) => {
                assert!(message.contains("Program 'ore'"), "message: {message}");
                assert!(message.contains("HTTP endpoint"), "message: {message}");
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn local_http_transport_uses_the_client_base_url() {
        let builder = ProgramBuilder::for_client(
            reqwest::Client::new(),
            Some("http://127.0.0.1:4000".to_string()),
            None,
            None,
        );
        let descriptor = ProgramReadDescriptor::LocalHttp { release: release() };
        let transport = builder.account_transport("ore", &descriptor).unwrap();
        assert_eq!(transport.endpoint(), "http://127.0.0.1:4000");
        assert_eq!(transport.release(), &release());
    }

    #[test]
    fn invalid_descriptor_fails_validation() {
        let builder = ProgramBuilder::for_client(
            reqwest::Client::new(),
            Some("http://127.0.0.1:4000".to_string()),
            None,
            None,
        );
        let descriptor = ProgramReadDescriptor::LocalHttp {
            release: ProgramReleaseReference {
                program_release_hash: " ".to_string(),
                program_spec_hash: String::new(),
            },
        };
        let error = builder.account_transport("ore", &descriptor).err().unwrap();
        assert!(matches!(error, AreteError::InvalidConfig(_)));
    }

    struct Released<const N: u8>;

    impl<const N: u8> Programs for Released<N> {
        fn from_builder(_builder: ProgramBuilder) -> Self {
            Self
        }
    }

    impl<const N: u8> ProgramSdk for Released<N> {
        fn name() -> &'static str {
            "ore"
        }

        fn package_release_hash() -> Option<&'static str> {
            match N {
                1 | 2 => Some("pkg:ore@1"),
                3 => Some("pkg:ore@2"),
                5 | 6 => Some(""),
                _ => None,
            }
        }
    }

    #[test]
    fn programs_are_matched_by_package_release_not_name() {
        // The same type is the same program, released or not.
        assert!(same_program::<Released<0>, Released<0>>());
        assert!(same_program::<Released<1>, Released<1>>());
        // Separately generated copies of one release are one program.
        assert!(same_program::<Released<1>, Released<2>>());
        // Same name, different releases.
        assert!(!same_program::<Released<1>, Released<3>>());
        // A local build is only ever itself.
        assert!(!same_program::<Released<0>, Released<1>>());
        assert!(!same_program::<Released<0>, Released<4>>());
        // An empty hash names no release.
        assert!(!same_program::<Released<5>, Released<6>>());
    }

    struct TestWallet(&'static str);

    #[async_trait::async_trait]
    impl WalletAdapter for TestWallet {
        fn public_key(&self) -> String {
            self.0.to_string()
        }

        async fn sign_and_send(
            &self,
            _instructions: &[crate::instruction::BuiltInstruction],
            _options: &crate::wallet::SendOptions,
            _context: &crate::wallet::WalletExecutionContext,
        ) -> Result<crate::wallet::SendResult, crate::wallet::WalletError> {
            unreachable!("program contexts never send")
        }
    }

    /// Stand-in for a generated `<Name>Program` accessor.
    struct TestProgram {
        builder: ProgramBuilder,
    }

    impl ProgramAccessor for TestProgram {
        fn program_builder(&self) -> &ProgramBuilder {
            &self.builder
        }
    }

    #[tokio::test]
    async fn a_detached_program_context_has_no_chain_reader_or_wallet() {
        let program = TestProgram {
            builder: ProgramBuilder::new(),
        };
        let context = program.context();
        assert!(std::ptr::eq(context.program(), &program));
        assert!(context.wallet().is_none());
        let error = context.chain().clock().await.err().unwrap();
        assert!(
            error.to_string().contains("not connected to a client"),
            "error: {error}"
        );
    }

    #[test]
    fn a_program_context_uses_the_client_chain_and_its_current_wallet() {
        let chain: Arc<dyn ChainClient> =
            Arc::new(crate::client::UnconfiguredChainClient::for_stack("test"));
        let wallet: WalletSlot = Arc::new(RwLock::new(None));
        let program = TestProgram {
            builder: ProgramBuilder::new().with_client_runtime(chain.clone(), wallet.clone()),
        };
        let context = ProgramContext::new(&program);
        assert!(Arc::ptr_eq(&context.chain(), &chain));
        assert!(context.wallet().is_none());

        // The client's `set_wallet` writes the shared slot; a context made
        // before it sees the new wallet.
        *wallet.write().unwrap() = Some(Arc::new(TestWallet("payer")));
        assert_eq!(context.public_key().as_deref(), Some("payer"));
        assert_eq!(program.builder.wallet().unwrap().public_key(), "payer");

        // A per-call override does not touch the client's wallet.
        let overridden = context
            .clone()
            .with_wallet(Some(Arc::new(TestWallet("other"))));
        assert_eq!(overridden.public_key().as_deref(), Some("other"));
        assert_eq!(context.public_key().as_deref(), Some("payer"));
    }

    #[test]
    fn a_bare_builder_can_be_wired_by_hand() {
        let chain: Arc<dyn ChainClient> =
            Arc::new(crate::client::UnconfiguredChainClient::for_stack("fixture"));
        let program = TestProgram {
            builder: ProgramBuilder::new()
                .with_chain(chain.clone())
                .with_wallet(Some(Arc::new(TestWallet("fixture-wallet")))),
        };
        let context = program.context();
        assert!(Arc::ptr_eq(&context.chain(), &chain));
        assert_eq!(context.public_key().as_deref(), Some("fixture-wallet"));
        let parts = ProgramContext::from_parts(&program, chain, None);
        assert!(parts.wallet().is_none());
    }

    #[test]
    fn query_executor_requires_a_base_url() {
        assert!(matches!(
            ProgramBuilder::new().query_executor(),
            Err(AreteError::InvalidConfig(_))
        ));
        let builder = ProgramBuilder::for_client(
            reqwest::Client::new(),
            Some("http://127.0.0.1:4000".to_string()),
            None,
            None,
        );
        assert!(builder.query_executor().is_ok());
    }
}
