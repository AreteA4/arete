use thiserror::Error;

use crate::{AuthContext, KeyClass, Limits, TargetKind, PROGRAM_READ_AUDIENCE, SCOPE_READ};

/// Authorization for one exact program read, derived from verified claims.
#[derive(Debug, Clone)]
pub struct ProgramReadAuthorization {
    /// Subject used for user attribution.
    pub subject: String,
    /// Issuer of the verified token.
    pub issuer: String,
    /// API key class used for policy and attribution.
    pub key_class: KeyClass,
    /// Opaque API-key or anonymous metering identity.
    pub metering_key: String,
    /// Authorized public program-read binding ID.
    pub target_id: String,
    /// Authorized Solana program ID.
    pub program_id: String,
    /// Authorized immutable program release hash.
    pub program_release_hash: String,
    /// Resource limits carried by the token.
    pub limits: Limits,
    /// Plan or access tier carried by the token.
    pub plan: Option<String>,
    /// Token expiration time.
    pub expires_at: u64,
    /// JWT ID for audit correlation.
    pub jti: String,
}

impl ProgramReadAuthorization {
    /// Convert a verified context into authorization for the requested resource.
    pub fn try_from_context(
        context: &AuthContext,
        expected_target_id: &str,
        expected_program_id: &str,
        expected_program_release_hash: &str,
    ) -> Result<Self, ProgramReadAuthorizationError> {
        if context.audience != PROGRAM_READ_AUDIENCE {
            return Err(ProgramReadAuthorizationError::InvalidAudience {
                actual: context.audience.clone(),
            });
        }

        match context.target_kind {
            Some(TargetKind::ProgramReadBinding) => {}
            Some(actual) => {
                return Err(ProgramReadAuthorizationError::InvalidTargetKind { actual });
            }
            None => return Err(ProgramReadAuthorizationError::MissingClaim("targetKind")),
        }

        let target_id = required_claim(context.target_id.as_deref(), "targetId")?;
        if target_id != expected_target_id {
            return Err(ProgramReadAuthorizationError::TargetIdMismatch {
                expected: expected_target_id.to_string(),
                actual: target_id.to_string(),
            });
        }

        let program_id = required_claim(context.program_id.as_deref(), "programId")?;
        if program_id != expected_program_id {
            return Err(ProgramReadAuthorizationError::ProgramIdMismatch {
                expected: expected_program_id.to_string(),
                actual: program_id.to_string(),
            });
        }

        let program_release_hash = required_claim(
            context.program_release_hash.as_deref(),
            "programReleaseHash",
        )?;
        if program_release_hash != expected_program_release_hash {
            return Err(ProgramReadAuthorizationError::ProgramReleaseHashMismatch {
                expected: expected_program_release_hash.to_string(),
                actual: program_release_hash.to_string(),
            });
        }

        if !context.has_scope(SCOPE_READ) {
            return Err(ProgramReadAuthorizationError::MissingReadScope);
        }

        Ok(Self {
            subject: context.subject.clone(),
            issuer: context.issuer.clone(),
            key_class: context.key_class,
            metering_key: context.metering_key.clone(),
            target_id: target_id.to_string(),
            program_id: program_id.to_string(),
            program_release_hash: program_release_hash.to_string(),
            limits: context.limits.clone(),
            plan: context.plan.clone(),
            expires_at: context.expires_at,
            jti: context.jti.clone(),
        })
    }
}

impl<'a> TryFrom<(&'a AuthContext, &'a str, &'a str, &'a str)> for ProgramReadAuthorization {
    type Error = ProgramReadAuthorizationError;

    fn try_from(
        (context, target_id, program_id, program_release_hash): (
            &'a AuthContext,
            &'a str,
            &'a str,
            &'a str,
        ),
    ) -> Result<Self, Self::Error> {
        Self::try_from_context(context, target_id, program_id, program_release_hash)
    }
}

fn required_claim<'a>(
    value: Option<&'a str>,
    name: &'static str,
) -> Result<&'a str, ProgramReadAuthorizationError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or(ProgramReadAuthorizationError::MissingClaim(name))
}

/// Failure to authorize a verified token for an exact program read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProgramReadAuthorizationError {
    #[error("invalid program-read audience: {actual}")]
    InvalidAudience { actual: String },
    #[error("missing required program-read claim: {0}")]
    MissingClaim(&'static str),
    #[error("invalid program-read target kind: {actual:?}")]
    InvalidTargetKind { actual: TargetKind },
    #[error("program-read target mismatch: expected {expected}, got {actual}")]
    TargetIdMismatch { expected: String, actual: String },
    #[error("program mismatch: expected {expected}, got {actual}")]
    ProgramIdMismatch { expected: String, actual: String },
    #[error("program release mismatch: expected {expected}, got {actual}")]
    ProgramReleaseHashMismatch { expected: String, actual: String },
    #[error("program-read authorization requires the read scope")]
    MissingReadScope,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionClaims, SigningKey, TokenSigner, TokenVerifier};

    const TARGET_ID: &str = "binding-1";
    const PROGRAM_ID: &str = "program-1";
    const RELEASE_HASH: &str = "arete:h1:program-release:sha256:release-1";

    fn claims() -> SessionClaims {
        SessionClaims::program_read_builder("issuer", "user:1", TARGET_ID, PROGRAM_ID, RELEASE_HASH)
            .with_metering_key("api_key:42")
            .with_limits(Limits {
                max_http_requests_per_minute: Some(120),
                max_http_batch_addresses: Some(50),
                ..Limits::default()
            })
            .build()
    }

    fn context() -> AuthContext {
        AuthContext::from_claims(claims())
    }

    fn authorize(
        context: &AuthContext,
    ) -> Result<ProgramReadAuthorization, ProgramReadAuthorizationError> {
        ProgramReadAuthorization::try_from_context(context, TARGET_ID, PROGRAM_ID, RELEASE_HASH)
    }

    #[test]
    fn typed_program_read_authorizes_exact_resource() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();
        let token = TokenSigner::new(signing_key, "issuer")
            .sign(claims())
            .unwrap();
        let context = TokenVerifier::new(verifying_key, "issuer", PROGRAM_READ_AUDIENCE)
            .verify(&token, None, None)
            .unwrap();
        let authorization = authorize(&context).unwrap();

        assert_eq!(authorization.subject, "user:1");
        assert_eq!(authorization.metering_key, "api_key:42");
        assert_eq!(authorization.target_id, TARGET_ID);
        assert_eq!(authorization.program_id, PROGRAM_ID);
        assert_eq!(authorization.program_release_hash, RELEASE_HASH);
        assert_eq!(authorization.limits.max_http_batch_addresses, Some(50));
    }

    #[test]
    fn missing_or_wrong_target_rejects() {
        let mut missing_kind = context();
        missing_kind.target_kind = None;
        assert!(matches!(
            authorize(&missing_kind),
            Err(ProgramReadAuthorizationError::MissingClaim("targetKind"))
        ));

        let mut missing = context();
        missing.target_id = None;
        assert!(matches!(
            authorize(&missing),
            Err(ProgramReadAuthorizationError::MissingClaim("targetId"))
        ));

        let mut wrong = context();
        wrong.target_id = Some("binding-2".into());
        assert!(matches!(
            authorize(&wrong),
            Err(ProgramReadAuthorizationError::TargetIdMismatch { .. })
        ));
    }

    #[test]
    fn wrong_audience_rejects() {
        let mut context = context();
        context.audience = "deployment-1".into();
        assert!(matches!(
            authorize(&context),
            Err(ProgramReadAuthorizationError::InvalidAudience { .. })
        ));
    }

    #[test]
    fn missing_read_scope_rejects() {
        let mut context = context();
        context.scope = "transaction:inspect".into();
        assert!(matches!(
            authorize(&context),
            Err(ProgramReadAuthorizationError::MissingReadScope)
        ));
    }

    #[test]
    fn missing_or_wrong_program_rejects() {
        let mut missing = context();
        missing.program_id = None;
        assert!(matches!(
            authorize(&missing),
            Err(ProgramReadAuthorizationError::MissingClaim("programId"))
        ));

        let mut wrong = context();
        wrong.program_id = Some("program-2".into());
        assert!(matches!(
            authorize(&wrong),
            Err(ProgramReadAuthorizationError::ProgramIdMismatch { .. })
        ));
    }

    #[test]
    fn missing_or_wrong_release_rejects() {
        let mut missing = context();
        missing.program_release_hash = None;
        assert!(matches!(
            authorize(&missing),
            Err(ProgramReadAuthorizationError::MissingClaim(
                "programReleaseHash"
            ))
        ));

        let mut wrong = context();
        wrong.program_release_hash = Some("release-2".into());
        assert!(matches!(
            authorize(&wrong),
            Err(ProgramReadAuthorizationError::ProgramReleaseHashMismatch { .. })
        ));
    }

    #[test]
    fn deployment_token_cannot_authorize_program_read() {
        let context = AuthContext::from_claims(
            SessionClaims::builder("issuer", "subject", PROGRAM_READ_AUDIENCE)
                .with_target(TargetKind::Deployment, TARGET_ID)
                .with_program_id(PROGRAM_ID)
                .with_program_release_hash(RELEASE_HASH)
                .build(),
        );

        assert!(matches!(
            authorize(&context),
            Err(ProgramReadAuthorizationError::InvalidTargetKind {
                actual: TargetKind::Deployment
            })
        ));
    }
}
