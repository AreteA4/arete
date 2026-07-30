# WebSocket Rate Limiting and Error Handling

This document describes the enhanced rate limiting and error handling system for Arete WebSocket connections.

## Overview

The WebSocket server provides comprehensive rate limiting and structured error handling to protect against abuse and provide clear feedback to clients.

## Rate Limiting

### Architecture

Rate limiting is implemented at multiple levels:

1. **Handshake Rate Limiting** - Per-IP rate limiting on connection attempts
2. **Connection Rate Limiting** - Per-subject and per-metering-key limits
3. **Operational Rate Limiting** - Per-connection limits on subscriptions, messages, and snapshots

### Rate Limiter Configuration

The `RateLimiterConfig` struct allows fine-grained control over rate limits:

```rust
use arete_server::{WebSocketRateLimiter, RateLimiterConfig, RateLimitWindow};
use std::time::Duration;

let config = RateLimiterConfig {
    // Handshake attempts per IP
    handshake_per_ip: RateLimitWindow::new(60, Duration::from_secs(60))
        .with_burst(10),
    
    // Connection attempts per subject
    connections_per_subject: RateLimitWindow::new(30, Duration::from_secs(60))
        .with_burst(5),
    
    // Connection attempts per metering key
    connections_per_metering_key: RateLimitWindow::new(100, Duration::from_secs(60))
        .with_burst(20),
    
    // Subscription rate per connection
    subscriptions_per_connection: RateLimitWindow::new(120, Duration::from_secs(60))
        .with_burst(10),
    
    // Message rate per connection
    messages_per_connection: RateLimitWindow::new(1000, Duration::from_secs(60))
        .with_burst(100),
    
    // Snapshot requests per connection
    snapshots_per_connection: RateLimitWindow::new(30, Duration::from_secs(60))
        .with_burst(5),
    
    enabled: true,
};

let rate_limiter = Arc::new(WebSocketRateLimiter::new(config));
```

### Environment Variables

All rate limits can be configured via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `ARETE_RATE_LIMIT_HANDSHAKE_PER_IP_MAX` | Max handshake attempts per IP per window | 60 |
| `ARETE_RATE_LIMIT_HANDSHAKE_PER_IP_WINDOW_SECS` | Handshake rate limit window | 60 |
| `ARETE_RATE_LIMIT_CONNECTIONS_PER_SUBJECT_MAX` | Max connections per subject per window | 30 |
| `ARETE_RATE_LIMIT_CONNECTIONS_PER_SUBJECT_WINDOW_SECS` | Subject connection window | 60 |
| `ARETE_RATE_LIMIT_CONNECTIONS_PER_METERING_KEY_MAX` | Max connections per metering key | 100 |
| `ARETE_RATE_LIMIT_CONNECTIONS_PER_METERING_KEY_WINDOW_SECS` | Metering key window | 60 |
| `ARETE_RATE_LIMIT_SUBSCRIPTIONS_PER_CONNECTION_MAX` | Max subscriptions per connection | 120 |
| `ARETE_RATE_LIMIT_SUBSCRIPTIONS_PER_CONNECTION_WINDOW_SECS` | Subscription window | 60 |
| `ARETE_RATE_LIMIT_MESSAGES_PER_CONNECTION_MAX` | Max messages per connection | 1000 |
| `ARETE_RATE_LIMIT_MESSAGES_PER_CONNECTION_WINDOW_SECS` | Message window | 60 |
| `ARETE_RATE_LIMIT_SNAPSHOTS_PER_CONNECTION_MAX` | Max snapshot requests per connection | 30 |
| `ARETE_RATE_LIMIT_SNAPSHOTS_PER_CONNECTION_WINDOW_SECS` | Snapshot window | 60 |
| `ARETE_RATE_LIMITING_ENABLED` | Enable/disable rate limiting | true |

### Using the Rate Limiter

Attach the rate limiter to your server:

```rust
use arete_server::{Server, WebSocketRateLimiter, RateLimiterConfig};
use std::sync::Arc;

let rate_limiter = Arc::new(WebSocketRateLimiter::new(
    RateLimiterConfig::from_env()
));

Server::builder()
    .websocket()
    .bind("[::]:8877".parse()?)
    .websocket_auth_plugin(Arc::new(auth_plugin))
    .websocket_rate_limit_config(rate_limit_config)
    .start()
    .await
```

### Rate Limiting Algorithm

The rate limiter uses a sliding window algorithm with burst capacity:

- **Base Limit**: The standard number of requests allowed in the window
- **Burst**: Additional requests allowed temporarily (useful for handling traffic spikes)
- **Window**: The time period over which requests are counted

When the limit is exceeded, the client receives a `429 Too Many Requests` response with a `Retry-After` header indicating when to retry.

## Error Handling

### Enhanced AuthDeny Structure

Authentication failures now return structured error information:

```rust
pub struct AuthDeny {
    pub reason: String,                    // Human-readable error message
    pub code: AuthErrorCode,               // Machine-readable error code
    pub details: AuthErrorDetails,         // Structured error details
    pub retry_policy: RetryPolicy,         // Retry guidance
    pub http_status: u16,                  // Equivalent HTTP status code
    pub reset_at: Option<SystemTime>,      // When the error condition resets
}

pub struct AuthErrorDetails {
    pub field: Option<String>,             // Field that caused the error
    pub context: Option<String>,           // Additional context
    pub suggested_action: Option<String>,  // What the client should do
    pub docs_url: Option<String>,          // Link to documentation
}
```

### Retry Policies

The system provides several retry policies:

```rust
pub enum RetryPolicy {
    /// Do not retry this request
    NoRetry,
    /// Retry immediately (for transient errors)
    RetryImmediately,
    /// Retry after a specific duration
    RetryAfter(Duration),
    /// Retry with exponential backoff
    RetryWithBackoff {
        initial: Duration,
        max: Duration,
    },
    /// Refresh the token before retrying
    RetryWithFreshToken,
}
```

### Error Response Format

When a WebSocket handshake is rejected, the server returns an HTTP response with the following JSON body:

```json
{
  "error": "token-expired",
  "message": "Token has expired",
  "code": "token-expired",
  "retryable": true,
  "retry_after": null,
  "suggested_action": "Refresh your authentication token",
  "docs_url": "https://docs.arete.run/auth/errors#token-expired"
}
```

### Error Codes

#### Authentication Errors (4xx)

| Code | HTTP Status | Retry Policy | Description |
|------|-------------|--------------|-------------|
| `token-missing` | 401 | NoRetry | No authentication token provided |
| `token-expired` | 401 | RetryWithFreshToken | Token has expired |
| `token-invalid-signature` | 401 | RetryWithFreshToken | Token signature verification failed |
| `token-invalid-format` | 400 | RetryWithFreshToken | Token is malformed |
| `token-invalid-issuer` | 401 | RetryWithFreshToken | Token issuer doesn't match |
| `token-invalid-audience` | 401 | RetryWithFreshToken | Token audience doesn't match |
| `token-missing-claim` | 400 | RetryWithFreshToken | Required claim is missing |
| `token-key-not-found` | 401 | RetryWithFreshToken | Signing key not found |
| `origin-mismatch` | 403 | NoRetry | Origin header doesn't match token |
| `origin-required` | 403 | NoRetry | Origin header is required but missing |

#### Rate Limiting Errors (429)

| Code | HTTP Status | Retry Policy | Description |
|------|-------------|--------------|-------------|
| `rate-limit-exceeded` | 429 | RetryWithBackoff | General rate limit exceeded |
| `connection-limit-exceeded` | 429 | NoRetry | Maximum connections reached |
| `subscription-limit-exceeded` | 429 | RetryWithBackoff | Subscription rate limit exceeded |
| `snapshot-limit-exceeded` | 429 | RetryWithBackoff | Snapshot request rate limit exceeded |
| `egress-limit-exceeded` | 429 | RetryWithBackoff | Egress bandwidth limit exceeded |

#### Server Errors (5xx)

| Code | HTTP Status | Retry Policy | Description |
|------|-------------|--------------|-------------|
| `internal-error` | 500 | RetryWithBackoff | Internal server error |

### Helper Methods

Create common error responses:

```rust
use arete_server::{AuthDeny, AuthErrorCode, RetryPolicy};
use std::time::Duration;

// Rate limit error with retry information
let deny = AuthDeny::rate_limited(
    Duration::from_secs(30),
    "websocket connections"
);

// Connection limit error
let deny = AuthDeny::connection_limit_exceeded(
    "user-123",
    5,  // current connections
    5   // max connections
);

// Custom error with details
let deny = AuthDeny::new(
    AuthErrorCode::TokenExpired,
    "Token has expired"
)
.with_field("exp")
.with_context("Token expired 5 minutes ago")
.with_suggested_action("Refresh your authentication token")
.with_docs_url("https://docs.example.com/errors#token-expired");
```

## Client-Side Handling

### TypeScript SDK

The TypeScript SDK handles rate limiting and errors automatically:

```typescript
import { Arete } from '@usearete/sdk';
import { APP_STREAM_STACK } from './generated/app-stack';

const client = await Arete.connect(APP_STREAM_STACK, {
  url: 'wss://...',
  auth: {
    getToken: async () => {
      // Your token fetching logic
    }
  }
});

client.onConnectionStateChange((state) => {
  console.log('Connection state:', state);
});

client.onSocketIssue((issue) => {
  console.error('WebSocket issue:', issue);
});

// The SDK automatically:
// - Retries with fresh tokens on auth errors
// - Backoffs on rate limit errors
// - Parses error codes from WebSocket close frames
```

### Handling Error Responses

When the WebSocket handshake fails, the client receives an HTTP response. Here's how to handle it:

```typescript
async function connectWithErrorHandling() {
  try {
    await client.connect();
  } catch (error) {
    if (error instanceof AreteError) {
      switch (error.code) {
        case 'TOKEN_EXPIRED':
        case 'TOKEN_INVALID_SIGNATURE':
          // Refresh token and retry
          await refreshToken();
          await client.connect();
          break;
          
        case 'RATE_LIMIT_EXCEEDED':
          // Wait and retry with backoff
          const retryAfter = error.details?.retryAfter || 60;
          await sleep(retryAfter * 1000);
          await client.connect();
          break;
          
        case 'CONNECTION_LIMIT_EXCEEDED':
          // Don't retry - requires user action
          showError('Maximum connections reached. Close other connections and try again.');
          break;
          
        case 'ORIGIN_MISMATCH':
          // Don't retry - security issue
          showError('Origin validation failed. Check your configuration.');
          break;
          
        default:
          if (error.retryable) {
            // Use exponential backoff
            await retryWithBackoff(client.connect);
          } else {
            showError(error.message);
          }
      }
    }
  }
}
```

## Best Practices

### Server Configuration

1. **Set appropriate limits based on your use case**:
   - Public APIs: Stricter limits to prevent abuse
   - Internal APIs: More permissive limits for better UX

2. **Monitor rate limit metrics**:
   - Track rate limit hits to identify legitimate traffic vs abuse
   - Adjust limits based on actual usage patterns

3. **Use burst capacity for traffic spikes**:
   - Configure burst to handle legitimate spikes
   - Base limit should cover normal sustained traffic

### Error Handling

1. **Always check retryable flag**: Don't retry non-retryable errors
2. **Respect Retry-After headers**: Wait the specified time before retrying
3. **Implement exponential backoff**: For retryable errors without specific delays
4. **Log error details**: Include error codes and context for debugging

### Security

1. **Don't expose internal details**: Error messages should be helpful but not leak implementation details
2. **Log security events**: Failed auth attempts should be logged for monitoring
3. **Use origin validation**: Enable origin checks for browser-based clients
4. **Rotate signing keys**: Implement key rotation for production deployments

## Migration Guide

### From v0.1.x

If you're upgrading from an earlier version:

1. Update your auth plugin initialization to use the new error types
2. Add rate limiter configuration (optional - defaults are provided)
3. Update client error handling to check for structured error responses

### Example Migration

**Before:**
```rust
let plugin = SignedSessionAuthPlugin::new(verifier);
```

**After:**
```rust
let plugin = SignedSessionAuthPlugin::new(verifier)
    .with_audit_logger(audit_logger)
    .with_metrics(metrics);

let rate_limiter = Arc::new(WebSocketRateLimiter::new(
    RateLimiterConfig::from_env()
));

let manager = ClientManager::new()
    .with_rate_limiter(rate_limiter);
```

## Testing

### Testing Rate Limits

```rust
#[tokio::test]
async fn test_rate_limiting() {
    let config = RateLimiterConfig {
        handshake_per_ip: RateLimitWindow::new(5, Duration::from_secs(60)),
        ..RateLimiterConfig::disabled()
    };
    let limiter = WebSocketRateLimiter::new(config);
    
    let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    
    // First 5 should succeed
    for _ in 0..5 {
        let result = limiter.check_handshake(addr).await;
        assert!(matches!(result, RateLimitResult::Allowed { .. }));
    }
    
    // 6th should be denied
    let result = limiter.check_handshake(addr).await;
    assert!(matches!(result, RateLimitResult::Denied { .. }));
}
```

### Testing Error Handling

```rust
#[tokio::test]
async fn test_auth_error_response() {
    let deny = AuthDeny::new(
        AuthErrorCode::TokenExpired,
        "Token expired"
    );
    
    let response = deny.to_error_response();
    assert_eq!(response.code, "token-expired");
    assert!(response.retryable);
}
```
