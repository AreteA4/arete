# hyperstack-auth

Authentication and authorization utilities for Hyperstack, providing JWT token handling, claims validation, and Ed25519-based key management for secure WebSocket and HTTP authentication.

## Overview

`hyperstack-auth` provides a robust, production-ready authentication system designed specifically for Hyperstack deployments. It uses **Ed25519 (EdDSA)** for asymmetric cryptographic signing, offering superior security compared to traditional HMAC-based approaches.

### Key Features

- **Ed25519 Signatures**: Asymmetric signing using the EdDSA algorithm for enhanced security
- **JWT Token Support**: Full JWT implementation with customizable session claims
- **Key Rotation**: JWKS (JSON Web Key Set) support for seamless key rotation
- **Origin Binding**: Optional defense-in-depth with origin validation
- **IP Binding**: Client IP validation for high-security scenarios
- **Resource Limits**: Built-in metering and rate limiting support
- **Token Revocation**: Support for token revocation lists
- **Security Auditing**: Structured audit logging for authentication events
- **Multi-key Verification**: Verify tokens against multiple signing keys

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hyperstack-auth = "0.2"
```

### Feature Flags

- `jwks` (default): Enables JWKS fetching from URLs via `reqwest`

```toml
[dependencies]
hyperstack-auth = { version = "0.2", default-features = false }  # Without JWKS
```

## Usage

### Basic Token Signing and Verification

```rust
use hyperstack_auth::{SigningKey, TokenSigner, TokenVerifier, SessionClaims};

// Generate a new Ed25519 key pair
let signing_key = SigningKey::generate();
let verifying_key = signing_key.verifying_key();

// Create a signer with your issuer
let signer = TokenSigner::new(signing_key, "my-service");

// Build session claims
let claims = SessionClaims::builder("my-service", "user-123", "api-gateway")
    .with_ttl(300)  // 5 minute session
    .with_scope("read write")
    .with_metering_key("meter-456")
    .build();

// Sign the token
let token = signer.sign(claims).unwrap();

// Verify the token
let verifier = TokenVerifier::new(verifying_key, "my-service", "api-gateway");
let auth_context = verifier.verify(&token, None, None).unwrap();

println!("Authenticated user: {}", auth_context.subject);
```

### Key Management

```rust
use hyperstack_auth::{SigningKey, VerifyingKey, KeyLoader};
use std::path::Path;

// Generate and save keys to files
let (signing_key, verifying_key) = KeyLoader::generate_and_save_keys(
    "signing.key",
    "verifying.key"
).unwrap();

// Load keys from files
let signing_key = KeyLoader::signing_key_from_file("signing.key").unwrap();
let verifying_key = KeyLoader::verifying_key_from_file("verifying.key").unwrap();

// Load from environment variables (base64-encoded)
std::env::set_var("SIGNING_KEY", "base64-encoded-key-here");
let signing_key = KeyLoader::signing_key_from_env("SIGNING_KEY").unwrap();
```

### Origin and IP Binding

Add defense-in-depth by binding tokens to specific origins or client IPs:

```rust
use hyperstack_auth::{SessionClaims, TokenSigner, TokenVerifier, SigningKey};

let signing_key = SigningKey::generate();

// Create origin-bound token
let claims = SessionClaims::builder("issuer", "user-123", "audience")
    .with_origin("https://example.com")
    .with_client_ip("192.168.1.1")
    .build();

let signer = TokenSigner::new(signing_key.clone(), "issuer");
let token = signer.sign(claims).unwrap();

// Verify with origin validation
let verifier = TokenVerifier::new(
    signing_key.verifying_key(),
    "issuer",
    "audience"
)
.with_origin_validation()
.with_client_ip_validation();

// Must provide matching origin and IP
let context = verifier.verify(
    &token,
    Some("https://example.com"),
    Some("192.168.1.1")
).unwrap();
```

### Resource Limits and Metering

```rust
use hyperstack_auth::{SessionClaims, Limits, KeyClass};

let limits = Limits {
    max_connections: Some(10),
    max_subscriptions: Some(100),
    max_snapshot_rows: Some(1000),
    max_messages_per_minute: Some(1000),
    max_bytes_per_minute: Some(10_000_000),
};

let claims = SessionClaims::builder("issuer", "user-123", "audience")
    .with_limits(limits)
    .with_key_class(KeyClass::Publishable)  // or KeyClass::Secret
    .with_plan("pro")
    .build();
```

### JWKS Key Rotation

```rust
use hyperstack_auth::token::JwksVerifier;

// Fetch JWKS from a URL
let jwks = JwksVerifier::fetch_jwks("https://auth.example.com/.well-known/jwks.json")
    .await
    .unwrap();

// Create verifier with JWKS support
let verifier = JwksVerifier::new(jwks, "auth.example.com", "my-api");

// Tokens signed with any key in the JWKS can be verified
let context = verifier.verify(token, None, None).unwrap();
```

### Multi-key Verification

Verify tokens against multiple keys (useful for zero-downtime rotation):

```rust
use hyperstack_auth::{MultiKeyVerifier, RotationKey};

let verifier = MultiKeyVerifier::builder("issuer", "audience")
    .with_key(RotationKey::Primary(primary_key))
    .with_key(RotationKey::Secondary(secondary_key))
    .build();

let context = verifier.verify(token, None, None).unwrap();
```

### Security Audit Logging

```rust
use hyperstack_auth::{SecurityAuditLogger, AuditEvent, AuditSeverity};

// Log authentication events
let logger = ChannelAuditLogger::new(tx);
logger.log_event(AuditEvent::AuthSuccess {
    subject: "user-123".to_string(),
    jti: "token-id".to_string(),
    ip: Some("192.168.1.1".to_string()),
}).await;
```

## Security Considerations

1. **Key Storage**: Store signing keys securely (e.g., in environment variables, AWS KMS, or HashiCorp Vault)
2. **Token TTL**: Use short-lived tokens (default: 5 minutes) with refresh mechanisms
3. **Origin Binding**: Enable origin validation for browser-based clients
4. **Rate Limiting**: Implement rate limiting using the built-in metering support

## License

This project is licensed under the terms specified in the [LICENSE](LICENSE) file.

## See Also

- [KEY_ROTATION_GUIDE.md](KEY_ROTATION_GUIDE.md) - Guide for key rotation procedures
- [SECURITY_AUDIT_LOGGING.md](SECURITY_AUDIT_LOGGING.md) - Security audit logging documentation
