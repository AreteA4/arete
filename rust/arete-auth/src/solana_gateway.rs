use thiserror::Error;

use crate::{
    AuthContext, KeyClass, Limits, TargetKind, SCOPE_READ, SCOPE_TRANSACTION_INSPECT,
    SCOPE_TRANSACTION_SEND, SOLANA_GATEWAY_AUDIENCE,
};

/// One exact permission understood by the Solana gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolanaGatewayScope {
    Read,
    TransactionInspect,
    TransactionSend,
}

impl SolanaGatewayScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => SCOPE_READ,
            Self::TransactionInspect => SCOPE_TRANSACTION_INSPECT,
            Self::TransactionSend => SCOPE_TRANSACTION_SEND,
        }
    }
}

impl std::fmt::Display for SolanaGatewayScope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Authorization for one regional Solana gateway binding.
#[derive(Debug, Clone)]
pub struct SolanaGatewayAuthorization {
    pub subject: String,
    pub issuer: String,
    pub key_class: KeyClass,
    pub metering_key: String,
    pub target_id: String,
    pub limits: Limits,
    pub plan: Option<String>,
    pub expires_at: u64,
    pub jti: String,
}

impl SolanaGatewayAuthorization {
    /// Validate only the stable audience and exact gateway target claims.
    pub fn validate_target(
        context: &AuthContext,
        expected_target_id: &str,
    ) -> Result<(), SolanaGatewayAuthorizationError> {
        if context.audience != SOLANA_GATEWAY_AUDIENCE {
            return Err(SolanaGatewayAuthorizationError::InvalidAudience {
                actual: context.audience.clone(),
            });
        }

        match context.target_kind {
            Some(TargetKind::SolanaGatewayBinding) => {}
            Some(actual) => {
                return Err(SolanaGatewayAuthorizationError::InvalidTargetKind { actual });
            }
            None => return Err(SolanaGatewayAuthorizationError::MissingClaim("targetKind")),
        }

        let target_id = context
            .target_id
            .as_deref()
            .filter(|target_id| !target_id.is_empty())
            .ok_or(SolanaGatewayAuthorizationError::MissingClaim("targetId"))?;
        if target_id != expected_target_id {
            return Err(SolanaGatewayAuthorizationError::TargetIdMismatch {
                expected: expected_target_id.to_string(),
                actual: target_id.to_string(),
            });
        }
        Ok(())
    }

    /// Validate verified claims against an exact gateway binding and permission.
    pub fn try_from_context(
        context: &AuthContext,
        expected_target_id: &str,
        required_scope: SolanaGatewayScope,
    ) -> Result<Self, SolanaGatewayAuthorizationError> {
        Self::validate_target(context, expected_target_id)?;
        let target_id = context
            .target_id
            .as_deref()
            .expect("validated gateway target ID");

        if !context.has_scope(required_scope.as_str()) {
            return Err(SolanaGatewayAuthorizationError::MissingScope {
                required: required_scope,
            });
        }

        Ok(Self {
            subject: context.subject.clone(),
            issuer: context.issuer.clone(),
            key_class: context.key_class,
            metering_key: context.metering_key.clone(),
            target_id: target_id.to_string(),
            limits: context.limits.clone(),
            plan: context.plan.clone(),
            expires_at: context.expires_at,
            jti: context.jti.clone(),
        })
    }
}

/// Failure to authorize verified claims for a Solana gateway binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SolanaGatewayAuthorizationError {
    #[error("invalid Solana gateway audience: {actual}")]
    InvalidAudience { actual: String },
    #[error("missing required Solana gateway claim: {0}")]
    MissingClaim(&'static str),
    #[error("invalid Solana gateway target kind: {actual:?}")]
    InvalidTargetKind { actual: TargetKind },
    #[error("Solana gateway target mismatch: expected {expected}, got {actual}")]
    TargetIdMismatch { expected: String, actual: String },
    #[error("Solana gateway authorization requires the {required} scope")]
    MissingScope { required: SolanaGatewayScope },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionClaims, SigningKey, TokenSigner, TokenVerifier};

    const TARGET_ID: &str = "gateway-us-east-1";

    fn claims(scope: &str) -> SessionClaims {
        SessionClaims::solana_gateway_builder("issuer", "user:1", TARGET_ID)
            .with_scope(scope)
            .with_metering_key("api_key:42")
            .build()
    }

    fn context(scope: &str) -> AuthContext {
        AuthContext::from_claims(claims(scope))
    }

    #[test]
    fn signed_gateway_claims_validate_for_each_exact_scope() {
        let signing_key = SigningKey::generate();
        let token = TokenSigner::new(signing_key.clone(), "issuer")
            .sign(claims(
                "read transaction:inspect transaction:send transaction:send-extra",
            ))
            .unwrap();
        let context = TokenVerifier::new(
            signing_key.verifying_key(),
            "issuer",
            SOLANA_GATEWAY_AUDIENCE,
        )
        .verify(&token, None, None)
        .unwrap();

        for scope in [
            SolanaGatewayScope::Read,
            SolanaGatewayScope::TransactionInspect,
            SolanaGatewayScope::TransactionSend,
        ] {
            let authorization =
                SolanaGatewayAuthorization::try_from_context(&context, TARGET_ID, scope).unwrap();
            assert_eq!(authorization.target_id, TARGET_ID);
            assert_eq!(authorization.metering_key, "api_key:42");
        }
    }

    #[test]
    fn similarly_named_scope_does_not_authorize_send() {
        let error = SolanaGatewayAuthorization::try_from_context(
            &context("transaction:send-extra"),
            TARGET_ID,
            SolanaGatewayScope::TransactionSend,
        )
        .unwrap_err();

        assert_eq!(
            error,
            SolanaGatewayAuthorizationError::MissingScope {
                required: SolanaGatewayScope::TransactionSend
            }
        );
    }

    #[test]
    fn wrong_audience_kind_or_target_is_rejected() {
        let mut wrong_audience = context(SCOPE_READ);
        wrong_audience.audience = "deployment-1".into();
        assert!(matches!(
            SolanaGatewayAuthorization::try_from_context(
                &wrong_audience,
                TARGET_ID,
                SolanaGatewayScope::Read
            ),
            Err(SolanaGatewayAuthorizationError::InvalidAudience { .. })
        ));

        let deployment = AuthContext::from_claims(
            SessionClaims::builder("issuer", "user:1", SOLANA_GATEWAY_AUDIENCE)
                .with_target(TargetKind::Deployment, TARGET_ID)
                .build(),
        );
        assert!(matches!(
            SolanaGatewayAuthorization::try_from_context(
                &deployment,
                TARGET_ID,
                SolanaGatewayScope::Read
            ),
            Err(SolanaGatewayAuthorizationError::InvalidTargetKind {
                actual: TargetKind::Deployment
            })
        ));

        assert!(matches!(
            SolanaGatewayAuthorization::try_from_context(
                &context(SCOPE_READ),
                "gateway-eu-west-1",
                SolanaGatewayScope::Read
            ),
            Err(SolanaGatewayAuthorizationError::TargetIdMismatch { .. })
        ));
    }
}
