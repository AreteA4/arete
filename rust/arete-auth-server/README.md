# Arete Auth Server

A reference authentication server for self-hosted Arete deployments. This server provides token minting and JWKS endpoints for WebSocket session authentication.

## Overview

The auth server implements the Arete authentication protocol:

- **Token Minting**: `POST /ws/sessions` - Issues short-lived Ed25519-signed session tokens
- **JWKS Endpoint**: `GET /.well-known/jwks.json` - Serves public keys for token verification
- **Health Check**: `GET /health` - Service health status

## Quick Start

### 1. Generate Signing Keys

```bash
# Generate base64-encoded Ed25519 signing + verifying keys on first boot
mkdir -p .arete-auth
```

By default the reference server stores keys at:

- `./.arete-auth/signing.key`
- `./.arete-auth/verifying.key`

If those files do not exist, the server generates them automatically on startup.

### 2. Configure Environment

```bash
# Optional (shown with defaults)
export HOST="0.0.0.0"
export PORT="8080"
export ISSUER="arete-auth"
export DEFAULT_AUDIENCE="arete"
export DEFAULT_TTL_SECONDS="300"
export SIGNING_KEY_PATH="./.arete-auth/signing.key"
export VERIFYING_KEY_PATH="./.arete-auth/verifying.key"

# Simple key store for the reference server
export SECRET_KEYS="a4-sk_dev_secret"
export PUBLISHABLE_KEYS="a4-pub_dev_public"

# Session limits embedded in minted tokens
export MAX_CONNECTIONS_PER_SUBJECT="10"
export MAX_SUBSCRIPTIONS_PER_CONNECTION="100"

# Enable real in-memory token mint rate limiting
export ENABLE_RATE_LIMIT="true"
export RATE_LIMIT_PER_MINUTE="60"
```

### 3. Run the Server

```bash
cargo run --bin arete-auth-server
```

The server will start on `0.0.0.0:8080` by default.

## API Endpoints

### POST /ws/sessions

Mint a new WebSocket session token.

**Request Headers:**
- `Authorization: Bearer <api_key>` (required)
- `Content-Type: application/json`

**Request Body:**
```json
{
  "websocket_url": "wss://demo.stack.example.com",
  "deployment_id": "optional-deployment-id",
  "scope": "read",
  "ttl_seconds": 300,
  "origin": "https://your-domain.com"
}
```

**Response:**
```json
{
  "token": "eyJhbGc...",
  "expires_at": 1712345678,
  "token_type": "Bearer"
}
```

The token is a JWT with the following claims:
- `iss` - Issuer (configured issuer)
- `sub` - Subject (derived from API key)
- `aud` - Audience (deployment ID, derived from `websocket_url` unless explicitly provided)
- `exp` - Expiration time
- `jti` - Unique token ID
- `scope` - Granted permissions
- `metering_key` - For usage attribution
- `key_class` - "secret" or "publishable"
- `limits` - Resource limits

### GET /.well-known/jwks.json

Retrieve the JSON Web Key Set for token verification.

**Response:**
```json
{
  "keys": [
    {
      "kty": "OKP",
      "crv": "Ed25519",
      "kid": "a1b2c3d4",
      "use": "sig",
      "alg": "EdDSA",
      "x": "base64url-encoded-public-key"
    }
  ]
}
```

### GET /health

Health check endpoint.

**Response:**
```json
{
  "status": "healthy",
  "version": "0.5.10"
}
```

## Key Classes

The auth server supports two types of API keys:

### Secret Keys (a4-sk_..., legacy hsk_...)
- Long-lived, high-trust keys
- For server-side use only
- Can mint tokens for any deployment
- Maximum TTL: 1 hour

### Publishable Keys (a4-pub_..., legacy hspk_...)
- Safe for browser/client use
- Can be exposed in frontend code
- Constrained by origin allowlist
- Maximum TTL: 5 minutes
- Lower resource limits

## Integration with Arete Server

To use this auth server with your Arete server:

1. **Configure the Arete server** to verify tokens using the JWKS endpoint:

```rust
use arete_auth::{AsyncVerifier, KeyLoader};
use arete_server::websocket::auth::SignedSessionAuthPlugin;

let verifier = AsyncVerifier::with_jwks_url(
    "http://auth-server:8080/.well-known/jwks.json",
    "arete-auth",
    "arete",
);

let auth_plugin = SignedSessionAuthPlugin::new_with_async_verifier(verifier);
```

2. **Configure your SDK client** to fetch tokens from the auth server:

```typescript
const client = createClient({
  websocketUrl: 'ws://your-server:8080',
  auth: {
    tokenEndpoint: 'http://auth-server:8080/ws/sessions',
    publishableKey: 'hspk_your_publishable_key',
  },
});
```

## Token Revocation

**Token revocation is intentionally not implemented.**

Since tokens are short-lived (5 minutes for publishable keys, 1 hour for secret keys), the complexity of distributed revocation isn't worth the benefit. Instead:

- **Prevent abuse at the minting layer**: Block new token issuance via rate limiting
- **Use short TTLs**: Tokens naturally expire quickly
- **Rotate keys if needed**: Via JWKS endpoint updates

If you need immediate revocation for compliance or security reasons, implement a custom `RevocationChecker` trait from `arete-auth`.

## Production Considerations

### Security
- Never commit signing keys to version control
- Use environment variables or a secrets manager
- Rotate keys regularly (support for key rotation via JWKS `kid`)
- Use HTTPS in production for all endpoints
- Set appropriate origin allowlists for publishable keys

### Scaling
- The current reference server uses an env-backed in-memory key store
- Run multiple instances behind a load balancer
- Add a shared database or external key service before using it in multi-instance production setups
- The built-in mint limiter is process-local; use Redis or another shared limiter for multi-instance setups
- Cache JWKS responses at the edge (CDN)

### Monitoring
- Monitor token minting rates
- Track failed authentication attempts
- Set up alerts for unusual patterns
- Log all token revocations

## Custom Issuer Implementation

You can replace this reference auth server with your own implementation as long as it:

1. Issues tokens with the required claims (iss, sub, aud, exp, jti, scope, metering_key)
2. Uses Ed25519 (EdDSA) for signing
3. Exposes a JWKS endpoint with the public key
4. Follows the Arete token contract

See the [Arete Auth Plan](../../auth-plan.md) for the full specification.

## TypeScript SSR Handlers

For Next.js, Vite, or TanStack Start applications, use the provided TypeScript handlers instead of running a separate auth server:

```typescript
// Next.js App Router
import { createNextJsSessionRoute } from '@usearete/sdk/ssr/nextjs-app';

export const POST = createNextJsSessionRoute({
  signingKey: process.env.ARETE_SIGNING_KEY,
  resolveSession: async () => {
    const user = await getAuthenticatedUser();
    if (!user) return null;
    return { subject: user.id };
  },
});
```

See the [TypeScript SSR documentation](../typescript/core/src/ssr/) for more details.

## License

This project is licensed under the same terms as the Arete project.
