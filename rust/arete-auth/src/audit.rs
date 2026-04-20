use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::SystemTime;

/// Security audit event severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditSeverity {
    /// Informational - normal operations
    Info,
    /// Warning - suspicious but not necessarily malicious
    Warning,
    /// Critical - potential security incident
    Critical,
}

impl std::fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditSeverity::Info => write!(f, "info"),
            AuditSeverity::Warning => write!(f, "warning"),
            AuditSeverity::Critical => write!(f, "critical"),
        }
    }
}

/// Security audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Authentication attempt (success or failure)
    AuthAttempt {
        success: bool,
        reason: Option<String>,
        error_code: Option<String>,
    },
    /// Token minted
    TokenMinted {
        key_id: String,
        key_class: String,
        ttl_seconds: u64,
    },
    /// Suspicious pattern detected
    SuspiciousPattern {
        pattern_type: String,
        details: String,
    },
    /// Rate limit exceeded
    RateLimitExceeded {
        limit_type: String,
        current_count: u32,
        limit: u32,
    },
    /// Origin validation failure
    OriginValidationFailed {
        expected: Option<String>,
        actual: Option<String>,
    },
    /// Key rotation event
    KeyRotation {
        old_key_id: Option<String>,
        new_key_id: String,
    },
}

/// Security audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAuditEvent {
    /// Unique event ID
    pub event_id: String,
    /// Timestamp when event occurred
    pub timestamp_ms: u64,
    /// Event severity
    pub severity: AuditSeverity,
    /// Event type with details
    pub event: AuditEvent,
    /// Client IP address
    pub client_ip: Option<String>,
    /// Client origin
    pub origin: Option<String>,
    /// User agent string
    pub user_agent: Option<String>,
    /// Request path
    pub path: Option<String>,
    /// Deployment ID if applicable
    pub deployment_id: Option<String>,
    /// Subject identifier if authenticated
    pub subject: Option<String>,
    /// Metering key if available
    pub metering_key: Option<String>,
}

impl SecurityAuditEvent {
    /// Create a new security audit event
    pub fn new(severity: AuditSeverity, event: AuditEvent) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            severity,
            event,
            client_ip: None,
            origin: None,
            user_agent: None,
            path: None,
            deployment_id: None,
            subject: None,
            metering_key: None,
        }
    }

    /// Add client IP address
    pub fn with_client_ip(mut self, ip: SocketAddr) -> Self {
        self.client_ip = Some(ip.ip().to_string());
        self
    }

    /// Add origin
    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    /// Add user agent
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Add request path
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Add deployment ID
    pub fn with_deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    /// Add subject
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Add metering key
    pub fn with_metering_key(mut self, metering_key: impl Into<String>) -> Self {
        self.metering_key = Some(metering_key.into());
        self
    }
}

/// Trait for security audit loggers
#[async_trait::async_trait]
pub trait SecurityAuditLogger: Send + Sync {
    /// Log a security audit event
    async fn log(&self, event: SecurityAuditEvent);
}

/// No-op audit logger for development/testing
pub struct NoOpAuditLogger;

#[async_trait::async_trait]
impl SecurityAuditLogger for NoOpAuditLogger {
    async fn log(&self, _event: SecurityAuditEvent) {
        // No-op
    }
}

/// Channel-based audit logger for async event streaming
pub struct ChannelAuditLogger {
    sender: tokio::sync::mpsc::UnboundedSender<SecurityAuditEvent>,
}

impl ChannelAuditLogger {
    /// Create a new channel audit logger
    pub fn new() -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<SecurityAuditEvent>,
    ) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }
}

#[async_trait::async_trait]
impl SecurityAuditLogger for ChannelAuditLogger {
    async fn log(&self, event: SecurityAuditEvent) {
        let _ = self.sender.send(event);
    }
}

/// Helper function to create an auth failure audit event
pub fn auth_failure_event(error_code: &crate::AuthErrorCode, reason: &str) -> SecurityAuditEvent {
    SecurityAuditEvent::new(
        AuditSeverity::Warning,
        AuditEvent::AuthAttempt {
            success: false,
            reason: Some(reason.to_string()),
            error_code: Some(error_code.to_string()),
        },
    )
}

/// Helper function to create an auth success audit event
pub fn auth_success_event(subject: &str) -> SecurityAuditEvent {
    SecurityAuditEvent::new(
        AuditSeverity::Info,
        AuditEvent::AuthAttempt {
            success: true,
            reason: None,
            error_code: None,
        },
    )
    .with_subject(subject)
}

/// Helper function to create a rate limit exceeded audit event
pub fn rate_limit_event(limit_type: &str, current: u32, limit: u32) -> SecurityAuditEvent {
    SecurityAuditEvent::new(
        AuditSeverity::Warning,
        AuditEvent::RateLimitExceeded {
            limit_type: limit_type.to_string(),
            current_count: current,
            limit,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_builder() {
        let event = SecurityAuditEvent::new(
            AuditSeverity::Warning,
            AuditEvent::AuthAttempt {
                success: false,
                reason: Some("Token expired".to_string()),
                error_code: Some("token-expired".to_string()),
            },
        )
        .with_client_ip("192.168.1.1:12345".parse().unwrap())
        .with_origin("https://example.com")
        .with_subject("user-123");

        assert_eq!(event.severity, AuditSeverity::Warning);
        assert_eq!(event.client_ip, Some("192.168.1.1".to_string()));
        assert_eq!(event.origin, Some("https://example.com".to_string()));
        assert_eq!(event.subject, Some("user-123".to_string()));
    }

    #[tokio::test]
    async fn test_channel_audit_logger() {
        let (logger, mut receiver) = ChannelAuditLogger::new();

        let event = auth_failure_event(&crate::AuthErrorCode::TokenExpired, "Token has expired");

        logger.log(event.clone()).await;

        let received = receiver.recv().await.expect("Should receive event");
        match received.event {
            AuditEvent::AuthAttempt { success, .. } => {
                assert!(!success);
            }
            _ => panic!("Expected AuthAttempt event"),
        }
    }
}
