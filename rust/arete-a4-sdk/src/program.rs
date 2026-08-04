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

use std::sync::Arc;

use crate::auth::AuthConfig;
use crate::error::AreteError;
use crate::http::HttpAuthClient;
use crate::program_read_transport::{BearerTokenSource, ProgramReadTransport};
use crate::read::{
    validate_program_read_descriptor, ProgramReadDescriptor, QueryExecutor, ReadError,
};

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
/// [`ProgramBuilder::new`] / `Default` build a bare context (fresh
/// `reqwest::Client`, no HTTP base, no auth) so program-less test stacks keep
/// working; connected clients construct it internally with their shared
/// runtime.
#[derive(Clone, Default)]
pub struct ProgramBuilder {
    http: Option<reqwest::Client>,
    http_base_url: Option<String>,
    auth: Option<Arc<HttpAuthClient>>,
    auth_config: Option<AuthConfig>,
}

impl ProgramBuilder {
    /// Bare context for program-less/test stacks: lazily-created HTTP client,
    /// no HTTP base URL, and no auth.
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
        }
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
        Some(Arc::new(HttpAuthClient::new(Some(config), None, self.http()))
            as Arc<dyn BearerTokenSource>)
    }
}

/// Trait for generated program accessor structs, mirroring [`crate::view::Views`].
pub trait Programs: Sized + Send + Sync + 'static {
    fn from_builder(builder: ProgramBuilder) -> Self;
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
