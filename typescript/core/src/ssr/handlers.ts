/**
 * Arete Auth Server - Drop-in Endpoint Handlers
 *
 * These are framework-agnostic API route handlers that users can mount however they like.
 * They handle token minting and JWKS serving directly using Ed25519 signing.
 *
 * @example
 * ```typescript
 * // app/api/arete/sessions/route.ts (Next.js App Router)
 * import { handleSessionRequest, handleJwksRequest } from '@usearete/sdk/ssr/handlers';
 *
 * export async function POST(request: Request) {
 *   const user = await getAuthenticatedUser(request);
 *   if (!user) return new Response('Unauthorized', { status: 401 });
 *   return handleSessionRequest({}, user.id);
 * }
 *
 * export async function GET() {
 *   return handleJwksRequest();
 * }
 * ```
 */

import * as ed25519 from '@noble/ed25519';
import { base64url } from './utils.js';

export interface AuthHandlerConfig {
  /**
   * Ed25519 signing key seed (base64-encoded, 32 bytes).
   * Set ARETE_SIGNING_KEY env var OR pass here.
   * Generate with: node -e "console.log(require('crypto').randomBytes(32).toString('base64'))"
   */
  signingKey?: string;

  /**
   * Optional: Pre-derived public key (base64-encoded, 32 bytes).
   * If not provided, will be derived from the signing key.
   */
  publicKey?: string;

  /**
   * Token issuer (defaults to ARETE_ISSUER env var or 'arete')
   */
  issuer?: string;

  /**
   * Token audience (defaults to ARETE_AUDIENCE env var)
   */
  audience?: string;

  /**
   * Token TTL in seconds (defaults to 300 = 5 minutes)
   */
  ttlSeconds?: number;

  /**
   * Key ID for JWKS (defaults to 'key-1')
   */
  keyId?: string;

  /**
   * Custom limits for tokens
   */
  limits?: {
    max_connections?: number;
    max_subscriptions?: number;
    max_snapshot_rows?: number;
  };
}

export interface SessionClaims {
  iss: string;
  sub: string;
  aud: string;
  iat: number;
  nbf: number;
  exp: number;
  jti: string;
  scope: string;
  metering_key: string;
  key_class: 'secret' | 'publishable';
  limits?: {
    max_connections?: number;
    max_subscriptions?: number;
    max_snapshot_rows?: number;
    max_messages_per_minute?: number;
    max_bytes_per_minute?: number;
  };
  /**
   * Origin binding for browser tokens (optional defense-in-depth)
   */
  origin?: string;
}

export interface TokenResponse {
  token: string;
  expires_at: number;
}

/**
 * Authenticated session data resolved by framework adapters.
 * Always derive this from verified server-side auth, never request headers.
 */
export interface ResolvedSession {
  subject: string;
  scope?: string;
}

export interface JwksKey {
  kty: 'OKP';
  crv: 'Ed25519';
  kid: string;
  use: 'sig';
  alg: 'EdDSA';
  x: string;
}

export interface JwksResponse {
  keys: JwksKey[];
}

/**
 * Decode base64 to Uint8Array
 */
function decodeBase64(base64: string): Uint8Array {
  const binary = Buffer.from(base64, 'base64');
  return new Uint8Array(binary);
}

/**
 * Encode Uint8Array to base64url
 */
function encodeBase64url(bytes: Uint8Array): string {
  return base64url.encode(bytes);
}

/**
 * Generate a key ID from public key bytes
 */
function deriveKeyId(publicKey: Uint8Array): string {
  // Use first 8 bytes of public key as hex for kid
  return Array.from(publicKey.slice(0, 8))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

/**
 * Create JWT header for Ed25519
 */
function createJwtHeader(keyId: string): string {
  const header = {
    alg: 'EdDSA',
    typ: 'JWT',
    kid: keyId,
  };
  return encodeBase64url(new TextEncoder().encode(JSON.stringify(header)));
}

/**
 * Create JWT payload from claims
 */
function createJwtPayload(claims: SessionClaims): string {
  return encodeBase64url(new TextEncoder().encode(JSON.stringify(claims)));
}

/**
 * Sign data with Ed25519
 */
async function signEd25519(
  data: string,
  privateKey: Uint8Array
): Promise<Uint8Array> {
  const messageBytes = new TextEncoder().encode(data);
  return await ed25519.signAsync(messageBytes, privateKey);
}

/**
 * Mint a session token using Ed25519 signing
 * `subject` must come from verified server-side auth or trusted service code.
 */
export async function mintSessionToken(
  config: AuthHandlerConfig,
  subject: string = 'anonymous',
  scope: string = 'read',
  origin?: string
): Promise<TokenResponse> {
  const signingKeyBase64 = config.signingKey || process.env.ARETE_SIGNING_KEY;
  if (!signingKeyBase64) {
    throw new Error(
      'ARETE_SIGNING_KEY not set. Generate with: node -e "console.log(require(\'crypto\').randomBytes(32).toString(\'base64\'))"'
    );
  }

  const privateKeyBytes = decodeBase64(signingKeyBase64);
  if (privateKeyBytes.length !== 32) {
    throw new Error(
      `Invalid signing key length: expected 32 bytes, got ${privateKeyBytes.length}. ` +
      'Ed25519 signing key must be 32 bytes (base64-encoded).'
    );
  }

  // Derive public key from private key
  const publicKeyBytes = await ed25519.getPublicKeyAsync(privateKeyBytes);
  const keyId = config.keyId || deriveKeyId(publicKeyBytes);

  const issuer = config.issuer || process.env.ARETE_ISSUER || 'arete';
  const audience = config.audience || process.env.ARETE_AUDIENCE || 'arete';
  const ttlSeconds = config.ttlSeconds || 300;

  const now = Math.floor(Date.now() / 1000);
  const expiresAt = now + ttlSeconds;

  const claims: SessionClaims = {
    iss: issuer,
    sub: subject,
    aud: audience,
    iat: now,
    nbf: now,
    exp: expiresAt,
    jti: crypto.randomUUID(),
    scope,
    metering_key: `meter:${subject}`,
    key_class: 'secret',
    limits: config.limits || {
      max_connections: 10,
      max_subscriptions: 100,
      max_snapshot_rows: 1000,
      max_messages_per_minute: 10000,
      max_bytes_per_minute: 100 * 1024 * 1024,
    },
  };

  // Add origin binding if provided
  if (origin) {
    claims.origin = origin;
  }

  // Create JWT
  const header = createJwtHeader(keyId);
  const payload = createJwtPayload(claims);
  const signingInput = `${header}.${payload}`;
  const signature = await signEd25519(signingInput, privateKeyBytes);
  const signatureBase64 = encodeBase64url(signature);

  const token = `${signingInput}.${signatureBase64}`;

  return {
    token,
    expires_at: expiresAt,
  };
}

/**
 * Generate JWKS response from signing key
 */
export async function generateJwks(config: AuthHandlerConfig): Promise<JwksResponse> {
  const signingKeyBase64 = config.signingKey || process.env.ARETE_SIGNING_KEY;
  const publicKeyBase64 = config.publicKey || process.env.ARETE_PUBLIC_KEY;

  if (!signingKeyBase64 && !publicKeyBase64) {
    return { keys: [] };
  }

  let publicKeyBytes: Uint8Array;

  if (publicKeyBase64) {
    // Use provided public key
    publicKeyBytes = decodeBase64(publicKeyBase64);
    if (publicKeyBytes.length !== 32) {
      throw new Error(
        `Invalid public key length: expected 32 bytes, got ${publicKeyBytes.length}`
      );
    }
  } else {
    // Derive public key from private key
    const privateKeyBytes = decodeBase64(signingKeyBase64!);
    if (privateKeyBytes.length !== 32) {
      throw new Error(
        `Invalid signing key length: expected 32 bytes, got ${privateKeyBytes.length}`
      );
    }
    publicKeyBytes = await ed25519.getPublicKeyAsync(privateKeyBytes);
  }

  const keyId = config.keyId ||
    (publicKeyBase64 ? 'key-1' : deriveKeyId(publicKeyBytes));

  return {
    keys: [
      {
        kty: 'OKP',
        crv: 'Ed25519',
        kid: keyId,
        use: 'sig',
        alg: 'EdDSA',
        x: encodeBase64url(publicKeyBytes),
      },
    ],
  };
}

/**
 * Framework-agnostic request handler for token minting
 * Returns a Response object that can be used with any framework
 * The `subject` must be derived from verified server-side auth.
 */
export async function handleSessionRequest(
  config: AuthHandlerConfig = {},
  subject: string = 'anonymous',
  scope: string = 'read',
  origin?: string
): Promise<Response> {
  try {
    const tokenData = await mintSessionToken(config, subject, scope, origin);

    return new Response(JSON.stringify(tokenData), {
      status: 200,
      headers: {
        'Content-Type': 'application/json',
      },
    });
  } catch (error) {
    return new Response(
      JSON.stringify({
        error: error instanceof Error ? error.message : 'Failed to mint token',
      }),
      {
        status: 500,
        headers: {
          'Content-Type': 'application/json',
        },
      }
    );
  }
}

/**
 * Framework-agnostic request handler for JWKS endpoint
 */
export async function handleJwksRequest(config: AuthHandlerConfig = {}): Promise<Response> {
  try {
    const jwks = await generateJwks(config);

    return new Response(JSON.stringify(jwks), {
      status: 200,
      headers: {
        'Content-Type': 'application/json',
      },
    });
  } catch (error) {
    return new Response(
      JSON.stringify({
        error: error instanceof Error ? error.message : 'Failed to generate JWKS',
      }),
      {
        status: 500,
        headers: {
          'Content-Type': 'application/json',
        },
      }
    );
  }
}

/**
 * Framework-agnostic health check handler
 */
export function handleHealthRequest(): Response {
  return new Response(
    JSON.stringify({
      status: 'healthy',
      version: '0.5.10',
    }),
    {
      status: 200,
      headers: {
        'Content-Type': 'application/json',
      },
    }
  );
}
