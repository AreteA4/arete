# Security Audit Logging

Arete provides comprehensive security audit logging for websocket authentication. This helps you monitor security events, detect suspicious activity, and maintain compliance.

## Overview

The security audit logging system captures:

- Authentication attempts (success and failure)
- Token minting events
- Rate limit violations
- Origin validation failures
- Key rotation events
- Suspicious patterns

## Quick Start

### Basic Setup

```rust
use arete_server::websocket::auth::{
    SignedSessionAuthPlugin, ChannelAuditLogger, SecurityAuditEvent,
};
use std::sync::Arc;

// Create a channel-based audit logger
let (audit_logger, mut receiver) = ChannelAuditLogger::new();

// Create plugin with audit logging
let plugin = SignedSessionAuthPlugin::new(verifier)
    .with_audit_logger(Arc::new(audit_logger));
```

### Processing Audit Events

```rust
use tokio::task;

// Spawn a task to process audit events
task::spawn(async move {
    while let Some(event) = receiver.recv().await {
        // Log to your security information and event management (SIEM) system
        println!("Security Event: {:?}", event);
        
        // Or send to external service
        send_to_siem(event).await;
    }
});
```

## Audit Event Types

### AuthAttempt

Captured for every authentication attempt:

```rust
AuditEvent::AuthAttempt {
    success: bool,
    reason: Option<String>,
    error_code: Option<String>,
}
```

**Use cases:**
- Monitor failed login attempts
- Detect brute force attacks
- Track successful authentications
- Investigate security incidents

### TokenMinted

Captured when a new session token is issued:

```rust
AuditEvent::TokenMinted {
    key_id: String,
    key_class: String,
    ttl_seconds: u64,
}
```

**Use cases:**
- Track token issuance rates
- Monitor publishable key usage
- Detect unusual token minting patterns

### RateLimitExceeded

Captured when rate limits are hit:

```rust
AuditEvent::RateLimitExceeded {
    limit_type: String,
    current_count: u32,
    limit: u32,
}
```

**Use cases:**
- Identify abusive clients
- Tune rate limit thresholds
- Investigate DoS attempts

### OriginValidationFailed

Captured when origin validation fails:

```rust
AuditEvent::OriginValidationFailed {
    expected: Option<String>,
    actual: Option<String>,
}
```

**Use cases:**
- Detect stolen publishable keys being used from unauthorized domains
- Monitor cross-origin request attempts
- Enforce CORS policies

### KeyRotation

Captured during key rotation events:

```rust
AuditEvent::KeyRotation {
    old_key_id: Option<String>,
    new_key_id: String,
}
```

**Use cases:**
- Track key rotation compliance
- Audit key lifecycle management
- Investigate signing issues

### SuspiciousPattern

Captured for anomalous behavior:

```rust
AuditEvent::SuspiciousPattern {
    pattern_type: String,
    details: String,
}
```

**Use cases:**
- Custom anomaly detection
- Bot detection
- Abuse prevention

## Event Metadata

Each audit event includes rich metadata:

```rust
SecurityAuditEvent {
    event_id: String,           // Unique event UUID
    timestamp_ms: u64,          // Unix timestamp in milliseconds
    severity: AuditSeverity,    // Info, Warning, or Critical
    event: AuditEvent,          // Event-specific data
    client_ip: Option<String>,  // Client IP address
    origin: Option<String>,     // Request origin
    user_agent: Option<String>, // User agent string
    path: Option<String>,       // Request path
    deployment_id: Option<String>,
    subject: Option<String>,    // Authenticated subject
    metering_key: Option<String>,
}
```

## Severity Levels

- **Info**: Normal operations, successful authentications
- **Warning**: Suspicious but not malicious (e.g., failed auth with wrong password)
- **Critical**: Potential security incidents (e.g., origin mismatch, rate limit exceeded)

## Custom Audit Loggers

Implement the `SecurityAuditLogger` trait for custom handling:

```rust
use async_trait::async_trait;
use arete_auth::{SecurityAuditLogger, SecurityAuditEvent};

struct MyAuditLogger {
    sender: kafka::Producer,
}

#[async_trait]
impl SecurityAuditLogger for MyAuditLogger {
    async fn log(&self, event: SecurityAuditEvent) {
        // Send to Kafka for stream processing
        self.sender.send("security-events", event).await;
        
        // Also log to console for debugging
        println!("[{}] {:?}", event.severity, event.event);
    }
}
```

## Best Practices

### 1. Log Retention

Store audit logs for at least 90 days to support security investigations:

```rust
// Example: Writing to rotating files
use std::fs::OpenOptions;
use std::io::Write;

struct FileAuditLogger {
    file: std::sync::Mutex<std::fs::File>,
}

#[async_trait]
impl SecurityAuditLogger for FileAuditLogger {
    async fn log(&self, event: SecurityAuditEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{}", json).unwrap();
    }
}
```

### 2. Alerting

Set up alerts for critical events:

```rust
while let Some(event) = receiver.recv().await {
    if event.severity == AuditSeverity::Critical {
        // Send Slack/PagerDuty alert
        send_alert(&event).await;
    }
    
    // Always store the event
    store_event(event).await;
}
```

### 3. Correlation

Use the `event_id` to correlate related events:

```rust
// All events from the same connection share context
let event = auth_success_event(&subject)
    .with_client_ip(remote_addr)
    .with_path("/ws");
```

### 4. Privacy

Be careful with PII in audit logs:

```rust
// Don't log sensitive data
let event = SecurityAuditEvent::new(
    AuditSeverity::Info,
    AuditEvent::AuthAttempt {
        success: true,
        reason: None,
        error_code: None,
    },
)
.with_subject(hash_subject_id(&user_id))  // Hash PII
.with_metering_key(&metering_key);        // Use opaque identifiers
```

## Integration Examples

### With Prometheus

```rust
use prometheus::{Counter, Registry};

struct PrometheusAuditLogger {
    auth_attempts: Counter,
    auth_failures: Counter,
}

#[async_trait]
impl SecurityAuditLogger for PrometheusAuditLogger {
    async fn log(&self, event: SecurityAuditEvent) {
        match event.event {
            AuditEvent::AuthAttempt { success, .. } => {
                self.auth_attempts.inc();
                if !success {
                    self.auth_failures.inc();
                }
            }
            _ => {}
        }
    }
}
```

### With Datadog

```rust
struct DatadogAuditLogger {
    client: datadog::Client,
}

#[async_trait]
impl SecurityAuditLogger for DatadogAuditLogger {
    async fn log(&self, event: SecurityAuditEvent) {
        self.client.send_log(
            &event.event_id,
            serde_json::json!({
                "service": "arete-auth",
                "severity": event.severity.to_string(),
                "event": event.event,
                "client_ip": event.client_ip,
            })
        ).await;
    }
}
```

## Testing

Use `NoOpAuditLogger` in tests to skip logging:

```rust
#[cfg(test)]
mod tests {
    use arete_auth::NoOpAuditLogger;
    
    #[tokio::test]
    async fn test_auth() {
        let plugin = SignedSessionAuthPlugin::new(verifier)
            .with_audit_logger(Arc::new(NoOpAuditLogger));
        
        // Test code...
    }
}
```

## Troubleshooting

### Missing Events

If audit events aren't being logged:

1. Check that `with_audit_logger()` was called on the plugin
2. Ensure the logger is not being dropped prematurely
3. Verify the channel receiver is still active

### Performance Impact

Audit logging is designed to be low-overhead:

- Events are sent asynchronously (non-blocking)
- Use `ChannelAuditLogger` to buffer events
- Consider batching writes to external systems

```rust
// Use bounded channel to prevent memory exhaustion
let (tx, rx) = tokio::sync::mpsc::channel(1000);
let logger = ChannelAuditLogger::from_sender(tx);
```

## Compliance

Audit logging helps meet compliance requirements:

- **SOC 2**: Track access to systems
- **GDPR**: Monitor data access patterns
- **PCI DSS**: Log authentication events
- **HIPAA**: Track PHI access attempts

Consult your compliance team for specific retention and access requirements.
