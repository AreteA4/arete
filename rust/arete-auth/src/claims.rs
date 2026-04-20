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
            scope: "read".to_string(),
            metering_key: String::new(),
            deployment_id: None,
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
    /// Key class (secret vs publishable)
    pub key_class: KeyClass,
    /// Metering key for usage attribution
    pub metering_key: String,
    /// Deployment ID binding
    pub deployment_id: Option<String>,
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
    /// Create AuthContext from verified claims
    pub fn from_claims(claims: SessionClaims) -> Self {
        Self {
            subject: claims.sub,
            issuer: claims.iss,
            key_class: claims.key_class,
            metering_key: claims.metering_key,
            deployment_id: claims.deployment_id,
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
