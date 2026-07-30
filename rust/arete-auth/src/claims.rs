use serde::{Deserialize, Serialize};

/// Key classification for metering and policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyClass {
    /// Secret API key - long-lived, high trust
    Secret,
    /// Publishable key - safe for browsers, constrained
    Publishable,
}

/// Kind of resource targeted by a signed session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    /// A legacy stack deployment.
    Deployment,
    /// A hosted program-read binding.
    ProgramReadBinding,
    /// A regional Solana RPC gateway binding.
    SolanaGatewayBinding,
}

/// Resource limits for a session
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum concurrent connections for this subject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u32>,
    /// Maximum subscriptions per connection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_subscriptions: Option<u32>,
    /// Maximum snapshot rows per request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_snapshot_rows: Option<u32>,
    /// Maximum messages per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_messages_per_minute: Option<u32>,
    /// Maximum egress bytes per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes_per_minute: Option<u64>,
    /// Maximum HTTP read requests per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_http_requests_per_minute: Option<u32>,
    /// Maximum account addresses accepted in one HTTP batch read
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_http_batch_addresses: Option<u32>,
    /// Maximum transaction inspection requests per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_inspect_requests_per_minute: Option<u32>,
    /// Maximum transaction submissions per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_send_requests_per_minute: Option<u32>,
    /// Maximum signature status requests per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_status_requests_per_minute: Option<u32>,
    /// Maximum encoded HTTP request body size for transaction routes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_request_bytes: Option<u32>,
    /// Maximum decoded Solana message or transaction size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_bytes: Option<u32>,
    /// Maximum concurrent transaction operations for this subject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transaction_concurrency: Option<u32>,
}

/// Session token claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// Issuer - who issued this token
    pub iss: String,
    /// Subject - who this token is for
    pub sub: String,
    /// Audience - intended recipient (e.g., deployment ID)
    pub aud: String,
    /// Issued at (Unix timestamp)
    pub iat: u64,
    /// Not valid before (Unix timestamp)
    pub nbf: u64,
    /// Expiration time (Unix timestamp)
    pub exp: u64,
    /// JWT ID - unique identifier for this token
    pub jti: String,
    /// Scope - permissions granted
    pub scope: String,
    /// Metering key - for usage attribution
    pub metering_key: String,
    /// Deployment ID (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    /// Typed resource target (optional for legacy deployment tokens)
    #[serde(
        default,
        rename = "targetKind",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_kind: Option<TargetKind>,
    /// Public target identifier (optional for legacy deployment tokens)
    #[serde(default, rename = "targetId", skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Program allowed by this token
    #[serde(default, rename = "programId", skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Exact immutable program release allowed by this token
    #[serde(
        default,
        rename = "programReleaseHash",
        skip_serializing_if = "Option::is_none"
    )]
    pub program_release_hash: Option<String>,
    /// Origin binding (optional, defense-in-depth)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Client IP binding (optional, for high-security scenarios)
    #[serde(skip_serializing_if = "Option::is_none", rename = "client_ip")]
    pub client_ip: Option<String>,
    /// Resource limits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// Plan identifier (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Key class (secret vs publishable)
    #[serde(rename = "key_class")]
    pub key_class: KeyClass,
}

impl SessionClaims {
    /// Create a new session claims builder
    pub fn builder(
        iss: impl Into<String>,
        sub: impl Into<String>,
        aud: impl Into<String>,
    ) -> SessionClaimsBuilder {
        SessionClaimsBuilder::new(iss, sub, aud)
    }

    /// Create claims for one exact program-read binding, program, and release.
    pub fn program_read_builder(
        iss: impl Into<String>,
        sub: impl Into<String>,
        target_id: impl Into<String>,
        program_id: impl Into<String>,
        program_release_hash: impl Into<String>,
    ) -> SessionClaimsBuilder {
        SessionClaimsBuilder::new(iss, sub, crate::PROGRAM_READ_AUDIENCE).with_program_read_binding(
            target_id,
            program_id,
            program_release_hash,
        )
    }

    /// Create claims for one regional Solana gateway binding.
    pub fn solana_gateway_builder(
        iss: impl Into<String>,
        sub: impl Into<String>,
        target_id: impl Into<String>,
    ) -> SessionClaimsBuilder {
        SessionClaimsBuilder::new(iss, sub, crate::SOLANA_GATEWAY_AUDIENCE)
            .with_solana_gateway_binding(target_id)
    }

    /// Check if the token is expired
    pub fn is_expired(&self, now: u64) -> bool {
        self.exp <= now
    }

    /// Check if the token is valid (not before issued)
    pub fn is_valid(&self, now: u64) -> bool {
        self.nbf <= now && self.iat <= now
    }
}

/// Builder for SessionClaims
pub struct SessionClaimsBuilder {
    iss: String,
    sub: String,
    aud: String,
    iat: u64,
    nbf: u64,
    exp: u64,
    jti: String,
    scope: String,
    metering_key: String,
    deployment_id: Option<String>,
    target_kind: Option<TargetKind>,
    target_id: Option<String>,
    program_id: Option<String>,
    program_release_hash: Option<String>,
    origin: Option<String>,
    client_ip: Option<String>,
    limits: Option<Limits>,
    plan: Option<String>,
    key_class: KeyClass,
}

impl SessionClaimsBuilder {
    fn new(iss: impl Into<String>, sub: impl Into<String>, aud: impl Into<String>) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should not be before epoch")
            .as_secs();

        Self {
            iss: iss.into(),
            sub: sub.into(),
            aud: aud.into(),
            iat: now,
            nbf: now,
            exp: now + crate::DEFAULT_SESSION_TTL_SECONDS,
            jti: uuid::Uuid::new_v4().to_string(),
            scope: crate::SCOPE_READ.to_string(),
            metering_key: String::new(),
            deployment_id: None,
            target_kind: None,
            target_id: None,
            program_id: None,
            program_release_hash: None,
            origin: None,
            client_ip: None,
            limits: None,
            plan: None,
            key_class: KeyClass::Publishable,
        }
    }

    pub fn with_ttl(mut self, ttl_seconds: u64) -> Self {
        self.exp = self.iat + ttl_seconds;
        self
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    pub fn with_metering_key(mut self, key: impl Into<String>) -> Self {
        self.metering_key = key.into();
        self
    }

    pub fn with_deployment_id(mut self, id: impl Into<String>) -> Self {
        self.deployment_id = Some(id.into());
        self
    }

    /// Bind claims to a typed target.
    pub fn with_target(mut self, kind: TargetKind, id: impl Into<String>) -> Self {
        self.target_kind = Some(kind);
        self.target_id = Some(id.into());
        self
    }

    /// Allow reads for a program.
    pub fn with_program_id(mut self, program_id: impl Into<String>) -> Self {
        self.program_id = Some(program_id.into());
        self
    }

    /// Restrict reads to one immutable program release.
    pub fn with_program_release_hash(mut self, hash: impl Into<String>) -> Self {
        self.program_release_hash = Some(hash.into());
        self
    }

    /// Configure an exact program-read target and its immutable release.
    pub fn with_program_read_binding(
        mut self,
        target_id: impl Into<String>,
        program_id: impl Into<String>,
        program_release_hash: impl Into<String>,
    ) -> Self {
        self.aud = crate::PROGRAM_READ_AUDIENCE.to_string();
        self.scope = crate::SCOPE_READ.to_string();
        self.target_kind = Some(TargetKind::ProgramReadBinding);
        self.target_id = Some(target_id.into());
        self.program_id = Some(program_id.into());
        self.program_release_hash = Some(program_release_hash.into());
        self
    }

    /// Configure a typed regional Solana gateway target.
    pub fn with_solana_gateway_binding(mut self, target_id: impl Into<String>) -> Self {
        self.aud = crate::SOLANA_GATEWAY_AUDIENCE.to_string();
        self.scope = crate::SCOPE_READ.to_string();
        self.target_kind = Some(TargetKind::SolanaGatewayBinding);
        self.target_id = Some(target_id.into());
        self
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    pub fn with_client_ip(mut self, client_ip: impl Into<String>) -> Self {
        self.client_ip = Some(client_ip.into());
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    pub fn with_plan(mut self, plan: impl Into<String>) -> Self {
        self.plan = Some(plan.into());
        self
    }

    pub fn with_key_class(mut self, key_class: KeyClass) -> Self {
        self.key_class = key_class;
        self
    }

    pub fn with_jti(mut self, jti: impl Into<String>) -> Self {
        self.jti = jti.into();
        self
    }

    pub fn build(self) -> SessionClaims {
        SessionClaims {
            iss: self.iss,
            sub: self.sub,
            aud: self.aud,
            iat: self.iat,
            nbf: self.nbf,
            exp: self.exp,
            jti: self.jti,
            scope: self.scope,
            metering_key: self.metering_key,
            deployment_id: self.deployment_id,
            target_kind: self.target_kind,
            target_id: self.target_id,
            program_id: self.program_id,
            program_release_hash: self.program_release_hash,
            origin: self.origin,
            client_ip: self.client_ip,
            limits: self.limits,
            plan: self.plan,
            key_class: self.key_class,
        }
    }
}

/// Auth context extracted from a verified token
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Subject identifier
    pub subject: String,
    /// Issuer
    pub issuer: String,
    /// Verified JWT audience
    pub audience: String,
    /// Key class (secret vs publishable)
    pub key_class: KeyClass,
    /// Metering key for usage attribution
    pub metering_key: String,
    /// Deployment ID binding
    pub deployment_id: Option<String>,
    /// Typed resource target
    pub target_kind: Option<TargetKind>,
    /// Public target identifier
    pub target_id: Option<String>,
    /// Program allowed by the token
    pub program_id: Option<String>,
    /// Exact immutable program release allowed by the token
    pub program_release_hash: Option<String>,
    /// Token expiration time
    pub expires_at: u64,
    /// Granted scope
    pub scope: String,
    /// Resource limits
    pub limits: Limits,
    /// Plan or access tier associated with the session
    pub plan: Option<String>,
    /// Origin binding
    pub origin: Option<String>,
    /// Client IP binding
    pub client_ip: Option<String>,
    /// JWT ID
    pub jti: String,
}

impl AuthContext {
    /// Test an exact whitespace-delimited scope. Scopes never imply one another.
    pub fn has_scope(&self, required: &str) -> bool {
        self.scope.split_whitespace().any(|scope| scope == required)
    }

    /// Create AuthContext from verified claims
    pub fn from_claims(claims: SessionClaims) -> Self {
        Self {
            subject: claims.sub,
            issuer: claims.iss,
            audience: claims.aud,
            key_class: claims.key_class,
            metering_key: claims.metering_key,
            deployment_id: claims.deployment_id,
            target_kind: claims.target_kind,
            target_id: claims.target_id,
            program_id: claims.program_id,
            program_release_hash: claims.program_release_hash,
            expires_at: claims.exp,
            scope: claims.scope,
            limits: claims.limits.unwrap_or_default(),
            plan: claims.plan,
            origin: claims.origin,
            client_ip: claims.client_ip,
            jti: claims.jti,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_exact_and_independent() {
        let context = AuthContext::from_claims(
            SessionClaims::builder("issuer", "subject", "audience")
                .with_scope("read transaction:inspect transaction:send-extra")
                .build(),
        );

        assert!(context.has_scope("read"));
        assert!(context.has_scope("transaction:inspect"));
        assert!(!context.has_scope("transaction:send"));
        assert!(!context.has_scope("transaction"));
    }

    #[test]
    fn old_limits_claims_remain_deserializable() {
        let limits: Limits = serde_json::from_value(serde_json::json!({
            "max_connections": 2
        }))
        .unwrap();

        assert_eq!(limits.max_connections, Some(2));
        assert_eq!(limits.max_transaction_bytes, None);
    }

    #[test]
    fn transaction_limits_round_trip_additively() {
        let limits = Limits {
            max_transaction_inspect_requests_per_minute: Some(120),
            max_transaction_send_requests_per_minute: Some(12),
            max_transaction_status_requests_per_minute: Some(240),
            max_transaction_request_bytes: Some(4096),
            max_transaction_bytes: Some(1232),
            max_transaction_concurrency: Some(4),
            ..Limits::default()
        };
        let value = serde_json::to_value(&limits).unwrap();
        let decoded: Limits = serde_json::from_value(value).unwrap();

        assert_eq!(decoded.max_transaction_bytes, Some(1232));
        assert_eq!(decoded.max_transaction_concurrency, Some(4));
    }

    #[test]
    fn program_read_claims_use_camel_case_fields() {
        let claims = SessionClaims::program_read_builder(
            "issuer",
            "subject",
            "binding-1",
            "program-1",
            "release-1",
        )
        .build();
        let value = serde_json::to_value(claims).unwrap();

        assert_eq!(value["aud"], crate::PROGRAM_READ_AUDIENCE);
        assert_eq!(value["targetKind"], "program-read-binding");
        assert_eq!(value["targetId"], "binding-1");
        assert_eq!(value["programId"], "program-1");
        assert_eq!(value["programReleaseHash"], "release-1");
        assert!(value.get("target_kind").is_none());
    }

    #[test]
    fn gateway_claims_use_stable_audience_target_and_default_scope() {
        let claims =
            SessionClaims::solana_gateway_builder("issuer", "subject", "gateway-us-east-1").build();
        let value = serde_json::to_value(claims).unwrap();

        assert_eq!(value["aud"], crate::SOLANA_GATEWAY_AUDIENCE);
        assert_eq!(value["targetKind"], "solana-gateway-binding");
        assert_eq!(value["targetId"], "gateway-us-east-1");
        assert_eq!(value["scope"], crate::SCOPE_READ);
    }

    #[test]
    fn legacy_deployment_claims_remain_untyped() {
        let claims = SessionClaims::builder("issuer", "subject", "deployment-1")
            .with_deployment_id("deployment-1")
            .build();
        let value = serde_json::to_value(&claims).unwrap();
        let decoded: SessionClaims = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(decoded.deployment_id.as_deref(), Some("deployment-1"));
        assert_eq!(decoded.target_kind, None);
        assert!(value.get("targetKind").is_none());
    }
}
