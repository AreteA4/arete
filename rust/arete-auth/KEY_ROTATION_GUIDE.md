# Key Rotation Operational Guide

This guide explains how to perform zero-downtime key rotation for Arete websocket authentication using the `MultiKeyVerifier`.

## Overview

Key rotation is essential for:

- **Security**: Limiting the impact of compromised keys
- **Compliance**: Meeting security standards that require periodic rotation
- **Operational hygiene**: Regular key updates as a best practice

Arete supports **graceful key rotation** - meaning you can rotate keys without dropping active connections or requiring client re-authentication.

## Key Concepts

### Primary vs Secondary Keys

- **Primary Key**: The current key used for signing new tokens
- **Secondary Key**: A previous key still accepted during the grace period

### Grace Period

During rotation, tokens signed with the old key remain valid for a configurable period (default: 24 hours). This allows:

- Existing clients to continue operating
- Time for new tokens to propagate
- Gradual migration without downtime

## Rotation Workflow

### Standard Rotation Procedure

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Normal     │────>│  Rotation    │────>│   Normal     │
│  Operation   │     │   Period     │     │  Operation   │
│  (Key A)     │     │ (Keys A+B)   │     │  (Key B)     │
└──────────────┘     └──────────────┘     └──────────────┘
                              │
                              │ After grace period
                              ▼
                       ┌──────────────┐
                       │   Expire     │
                       │   Key A      │
                       └──────────────┘
```

## Implementation

### 1. Initial Setup

Start with a single primary key:

```rust
use arete_auth::{MultiKeyVerifier, RotationKey};
use arete_auth::SigningKey;

// Generate initial key pair
let signing_key = SigningKey::generate();
let verifying_key = signing_key.verifying_key();

// Create verifier with single primary key
let verifier = MultiKeyVerifier::from_single_key(
    verifying_key,
    verifying_key.key_id(),
    "arete-issuer",
    "arete-audience",
);

// Use with websocket plugin
let plugin = SignedSessionAuthPlugin::new_with_multi_key_verifier(verifier);
```

### 2. Rotation Process

#### Step 1: Generate New Key Pair

```rust
// Generate new key pair for rotation
let new_signing_key = SigningKey::generate();
let new_verifying_key = new_signing_key.verifying_key();
let new_kid = new_verifying_key.key_id();
```

#### Step 2: Add New Key as Primary

```rust
// The new key automatically becomes primary
// Old key is demoted to secondary with grace period
let new_rotation_key = RotationKey::primary(new_verifying_key, new_kid.clone());

// Add to verifier - this automatically demotes the old primary
verifier.add_key(new_rotation_key).await;
```

#### Step 3: Update Token Issuer

Switch your token issuer to use the new signing key:

```rust
// Before: signing with old key
let old_signer = TokenSigner::new(old_signing_key, "arete-issuer");

// After: signing with new key
let new_signer = TokenSigner::new(new_signing_key, "arete-issuer");
```

#### Step 4: Update JWKS (if applicable)

Add the new key to your JWKS endpoint:

```json
{
  "keys": [
    {
      "kty": "OKP",
      "use": "sig",
      "kid": "old-key-id",
      "alg": "EdDSA",
      "x": "base64-encoded-old-public-key"
    },
    {
      "kty": "OKP",
      "use": "sig",
      "kid": "new-key-id",
      "alg": "EdDSA",
      "x": "base64-encoded-new-public-key"
    }
  ]
}
```

#### Step 5: Monitor Grace Period

Track active keys during rotation:

```rust
// Check all active keys
let key_ids = verifier.key_ids().await;
println!("Active keys: {:?}", key_ids);

// Check which key is primary
let primary = verifier.primary_key_id().await;
println!("Primary key: {:?}", primary);
```

#### Step 6: Remove Old Key (After Grace Period)

After the grace period expires, remove the old key:

```rust
// Old key is automatically cleaned up after expiration
// Or manually remove it:
verifier.remove_key("old-key-id").await;
```

## Complete Example

```rust
use arete_auth::*;
use std::time::Duration;

async fn perform_key_rotation() {
    // 1. Setup initial state
    let old_key = SigningKey::generate();
    let old_verifying = old_key.verifying_key();
    let old_signer = TokenSigner::new(old_key, "issuer");
    
    let verifier = MultiKeyVerifier::from_single_key(
        old_verifying.clone(),
        old_verifying.key_id(),
        "issuer",
        "audience",
    );
    
    // 2. Generate new key
    let new_key = SigningKey::generate();
    let new_verifying = new_key.verifying_key();
    let new_kid = new_verifying.key_id();
    
    // 3. Start rotation - add new key as primary
    // Old key automatically becomes secondary with 24hr grace period
    let rotation_key = RotationKey::primary(new_verifying, new_kid);
    verifier.add_key(rotation_key).await;
    
    println!("🔄 Key rotation started");
    println!("   Old key: {}", old_verifying.key_id());
    println!("   New key: {}", new_kid);
    
    // 4. Switch to new signer
    let new_signer = TokenSigner::new(new_key, "issuer");
    
    // 5. Both old and new tokens work during grace period
    let old_claims = SessionClaims::builder("issuer", "user1", "audience")
        .with_scope("read")
        .build();
    let old_token = old_signer.sign(old_claims).unwrap();
    
    let new_claims = SessionClaims::builder("issuer", "user2", "audience")
        .with_scope("read")
        .build();
    let new_token = new_signer.sign(new_claims).unwrap();
    
    // Both verify successfully
    assert!(verifier.verify(&old_token, None, None).await.is_ok());
    assert!(verifier.verify(&new_token, None, None).await.is_ok());
    
    // 6. After grace period, clean up
    tokio::time::sleep(Duration::from_secs(86400)).await;
    verifier.remove_key(&old_verifying.key_id()).await;
    
    println!("✅ Key rotation complete");
}
```

## Automation

### Scheduled Rotation

Automate rotation with a scheduled job:

```rust
use tokio::time::{interval, Duration};

async fn scheduled_rotation(
    verifier: MultiKeyVerifier,
    rotation_interval: Duration,
) {
    let mut ticker = interval(rotation_interval);
    
    loop {
        ticker.tick().await;
        
        // Generate new key
        let new_key = SigningKey::generate();
        let new_verifying = new_key.verifying_key();
        
        // Add as primary
        let rotation_key = RotationKey::primary(new_verifying, new_verifying.key_id());
        verifier.add_key(rotation_key).await;
        
        // Log rotation event
        log::info!(
            "Scheduled key rotation complete. New primary: {}",
            new_verifying.key_id()
        );
    }
}
```

### Emergency Rotation

For compromised key scenarios:

```rust
async fn emergency_rotation(
    verifier: MultiKeyVerifier,
    compromised_key_id: &str,
) {
    // 1. Generate new key immediately
    let new_key = SigningKey::generate();
    let new_verifying = new_key.verifying_key();
    
    // 2. Add new key
    let rotation_key = RotationKey::primary(new_verifying, new_verifying.key_id());
    verifier.add_key(rotation_key).await;
    
    // 3. Immediately revoke compromised key (skip grace period)
    verifier.remove_key(compromised_key_id).await;
    
    // 4. Alert security team
    send_security_alert(format!(
        "Emergency key rotation performed. Revoked key: {}",
        compromised_key_id
    )).await;
    
    // 5. Force token refresh
    force_all_clients_to_refresh().await;
}
```

## Monitoring

### Key Metrics

Monitor these metrics during rotation:

```rust
// Key count
let key_count = verifier.key_ids().await.len();
metrics::gauge!("auth.keys.total", key_count as f64);

// Token verification by key
// (Track which keys are being used)

// Failed verifications
// (May indicate old tokens still being used after grace period)
```

### Alerts

Set up alerts for:

```rust
// Alert if more than 2 keys active (indicates stuck rotation)
if key_count > 2 {
    alert("Multiple active keys detected - rotation may be stuck");
}

// Alert if old key still in use after grace period
if verifier.key_ids().await.contains(&old_key_id) && grace_period_expired {
    alert("Old key still active after grace period");
}
```

## Best Practices

### 1. Rotation Schedule

- **Standard**: Rotate keys every 90 days
- **High-security**: Rotate every 30 days
- **Emergency**: Rotate immediately on suspected compromise

### 2. Grace Period

- **Default**: 24 hours
- **High-traffic systems**: 48-72 hours
- **Emergency rotation**: 0 hours (immediate revocation)

### 3. Testing

Always test rotation in staging:

```rust
#[tokio::test]
async fn test_key_rotation() {
    // Simulate full rotation
    let verifier = setup_test_verifier();
    
    // Issue token with old key
    let old_token = issue_token_with_old_key();
    
    // Rotate
    perform_rotation(&verifier).await;
    
    // Verify old token still works
    assert!(verifier.verify(&old_token, None, None).await.is_ok());
    
    // Issue token with new key
    let new_token = issue_token_with_new_key();
    
    // Verify new token works
    assert!(verifier.verify(&new_token, None, None).await.is_ok());
}
```

### 4. Documentation

Keep a rotation log:

```markdown
## Key Rotation Log

| Date | Old Key ID | New Key ID | Reason | Performed By |
|------|-----------|-----------|---------|-------------|
| 2024-03-28 | a1b2c3... | d4e5f6... | Scheduled | ops-team |
| 2024-03-15 | x9y8z7... | q1w2e3... | Security incident | security-team |
```

### 5. Backup Keys

Keep offline backups of old keys for 30 days:

```bash
# Export key to encrypted backup
gpg --encrypt --recipient security@example.com old-signing-key.pem > backup-2024-03-28.gpg

# Store securely
aws s3 cp backup-2024-03-28.gpg s3://secure-backups/arete-keys/
```

## Troubleshooting

### Old Tokens Failing After Rotation

**Symptoms:** Clients with old tokens can't connect after rotation.

**Diagnosis:**
```rust
// Check if old key is still present
let keys = verifier.key_ids().await;
if !keys.contains(&old_key_id) {
    println!("Old key was removed too early");
}
```

**Solution:** Increase grace period or re-add old key temporarily.

### High Verification Latency

**Symptoms:** Slower token verification during rotation.

**Cause:** Verifying against multiple keys.

**Solution:**
- Primary key is checked first (fast path)
- Monitor `verification_latency_us` metric
- Consider shorter grace periods

### Key ID Mismatch

**Symptoms:** Tokens failing with `KeyNotFound`.

**Diagnosis:**
```rust
// Decode token header to check kid
let parts: Vec<&str> = token.split('.').collect();
let header = base64_decode(parts[0]);
println!("Token kid: {}", header.kid);

// Check JWKS
println!("Available keys: {:?}", verifier.key_ids().await);
```

**Solution:** Ensure token issuer and verifier use same key IDs.

## Platform-Specific Notes

### Self-Hosted

You control the full rotation process:

```rust
// Direct access to verifier
let verifier = MultiKeyVerifier::new(...);
verifier.add_key(new_key).await;
```

### Arete Cloud

Key rotation is managed automatically:

- Platform rotates keys every 90 days
- Grace period: 24 hours
- JWKS endpoint always includes active keys

No action required - keys are transparently rotated.

## Migration from Single-Key

If you're currently using a single key:

```rust
// Before: Single key
let verifier = TokenVerifier::new(key, issuer, audience);
let plugin = SignedSessionAuthPlugin::new(verifier);

// After: Multi-key (enables future rotation)
let verifier = MultiKeyVerifier::from_single_key(key, kid, issuer, audience);
let plugin = SignedSessionAuthPlugin::new_with_multi_key_verifier(verifier);
```

The migration is backward-compatible - existing tokens continue to work.

## Summary

Key rotation with Arete is:

- ✅ **Zero-downtime**: Grace period allows gradual migration
- ✅ **Automatic cleanup**: Expired keys are removed automatically
- ✅ **Observable**: Full audit trail of rotation events
- ✅ **Flexible**: Supports scheduled and emergency rotations

Follow this guide to maintain secure, compliant authentication for your Arete deployment.
