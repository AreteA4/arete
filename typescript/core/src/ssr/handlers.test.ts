import { describe, it, expect } from 'vitest';
import crypto from 'node:crypto';
import * as ed25519 from '@noble/ed25519';
import {
  mintSessionToken,
  generateJwks,
  type AuthHandlerConfig,
} from './handlers';

describe('SSR Auth Handlers', () => {
  // Generate a test Ed25519 keypair
  const testSeed = crypto.randomBytes(32);
  const testConfig: AuthHandlerConfig = {
    signingKey: testSeed.toString('base64'),
    issuer: 'test-issuer',
    audience: 'test-audience',
    ttlSeconds: 300,
  };

  describe('mintSessionToken', () => {
    it('should mint a valid Ed25519-signed token', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read');

      expect(result.token).toBeDefined();
      expect(result.expires_at).toBeGreaterThan(Math.floor(Date.now() / 1000));
      expect(result.token.split('.')).toHaveLength(3); // JWT format: header.payload.signature
    });

    it('should include correct claims in token', async () => {
      const result = await mintSessionToken(testConfig, 'user-123', 'write');
      
      // Decode the JWT payload (middle part)
      const parts = result.token.split('.');
      const payload = JSON.parse(
        Buffer.from(parts[1]!, 'base64url').toString('utf-8')
      );

      expect(payload.iss).toBe('test-issuer');
      expect(payload.aud).toBe('test-audience');
      expect(payload.sub).toBe('user-123');
      expect(payload.scope).toBe('write');
      expect(payload.key_class).toBe('secret');
      expect(payload.metering_key).toBe('meter:user-123');
      expect(payload.jti).toBeDefined();
      expect(payload.iat).toBeDefined();
      expect(payload.exp).toBeDefined();
      expect(payload.nbf).toBeDefined();
    });

    it('should have a valid Ed25519 signature', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read');
      
      const parts = result.token.split('.');
      const signingInput = `${parts[0]}.${parts[1]}`;
      const signature = Buffer.from(parts[2]!, 'base64url');
      
      // Derive public key from private key
      const publicKey = await ed25519.getPublicKeyAsync(testSeed);
      
      // Verify the signature
      const messageBytes = new TextEncoder().encode(signingInput);
      const isValid = await ed25519.verifyAsync(signature, messageBytes, publicKey);
      
      expect(isValid).toBe(true);
    });

    it('should include custom limits in claims', async () => {
      const configWithLimits: AuthHandlerConfig = {
        ...testConfig,
        limits: {
          max_connections: 5,
          max_subscriptions: 50,
          max_snapshot_rows: 500,
        },
      };

      const result = await mintSessionToken(configWithLimits, 'test-user', 'read');

      const parts = result.token.split('.');
      const payload = JSON.parse(
        Buffer.from(parts[1]!, 'base64url').toString('utf-8')
      );

      // Custom limits replace defaults entirely
      expect(payload.limits).toEqual({
        max_connections: 5,
        max_subscriptions: 50,
        max_snapshot_rows: 500,
      });
    });

    it('should use default limits when not specified', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read');

      const parts = result.token.split('.');
      const payload = JSON.parse(
        Buffer.from(parts[1]!, 'base64url').toString('utf-8')
      );

      expect(payload.limits).toEqual({
        max_connections: 10,
        max_subscriptions: 100,
        max_snapshot_rows: 1000,
        max_messages_per_minute: 10000,
        max_bytes_per_minute: 104857600,
      });
    });

    it('should include origin when provided', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read', 'https://example.com');
      
      const parts = result.token.split('.');
      const payload = JSON.parse(
        Buffer.from(parts[1]!, 'base64url').toString('utf-8')
      );

      expect(payload.origin).toBe('https://example.com');
    });

    it('should throw error when signing key is missing', async () => {
      const configWithoutKey: AuthHandlerConfig = {
        signingKey: undefined,
      };

      await expect(mintSessionToken(configWithoutKey)).rejects.toThrow('HYPERSTACK_SIGNING_KEY not set');
    });

    it('should throw error when signing key has wrong length', async () => {
      const configWithBadKey: AuthHandlerConfig = {
        signingKey: Buffer.from('short').toString('base64'),
      };

      await expect(mintSessionToken(configWithBadKey)).rejects.toThrow('Invalid signing key length');
    });
  });

  describe('generateJwks', () => {
    it('should generate valid JWKS from signing key', async () => {
      const jwks = await generateJwks(testConfig);

      expect(jwks.keys).toHaveLength(1);
      
      const key = jwks.keys[0]!;
      expect(key.kty).toBe('OKP');
      expect(key.crv).toBe('Ed25519');
      expect(key.use).toBe('sig');
      expect(key.alg).toBe('EdDSA');
      expect(key.kid).toBeDefined();
      expect(key.x).toBeDefined();
      
      // Verify the public key is valid base64url
      const publicKeyBytes = Buffer.from(key.x, 'base64url');
      expect(publicKeyBytes).toHaveLength(32);
    });

    it('should derive same public key as used for signing', async () => {
      const jwks = await generateJwks(testConfig);
      
      // Derive public key directly from seed
      const expectedPublicKey = await ed25519.getPublicKeyAsync(testSeed);
      
      // JWKS public key should match
      const jwksPublicKey = Buffer.from(jwks.keys[0]!.x, 'base64url');
      expect(new Uint8Array(jwksPublicKey)).toEqual(expectedPublicKey);
    });

    it('should use custom key ID when provided', async () => {
      const configWithKid: AuthHandlerConfig = {
        ...testConfig,
        keyId: 'my-custom-key-id',
      };

      const jwks = await generateJwks(configWithKid);
      expect(jwks.keys[0]!.kid).toBe('my-custom-key-id');
    });

    it('should return empty keys array when no key is configured', async () => {
      const emptyConfig: AuthHandlerConfig = {};
      const jwks = await generateJwks(emptyConfig);
      expect(jwks.keys).toHaveLength(0);
    });

    it('should use provided public key instead of deriving', async () => {
      // Generate a different keypair
      const differentSeed = crypto.randomBytes(32);
      const differentPublicKey = await ed25519.getPublicKeyAsync(differentSeed);
      
      const configWithPublicKey: AuthHandlerConfig = {
        signingKey: testSeed.toString('base64'),
        publicKey: Buffer.from(differentPublicKey).toString('base64'),
      };

      const jwks = await generateJwks(configWithPublicKey);
      
      // Should use the provided public key, not derived one
      expect(jwks.keys[0]!.x).toBe(Buffer.from(differentPublicKey).toString('base64url'));
    });

    it('should throw error for invalid public key length', async () => {
      const configWithBadPublicKey: AuthHandlerConfig = {
        publicKey: Buffer.from('short').toString('base64'),
      };

      await expect(generateJwks(configWithBadPublicKey)).rejects.toThrow('Invalid public key length');
    });
  });

  describe('JWT format', () => {
    it('should have correct JWT header', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read');
      
      const parts = result.token.split('.');
      const header = JSON.parse(
        Buffer.from(parts[0]!, 'base64url').toString('utf-8')
      );

      expect(header.alg).toBe('EdDSA');
      expect(header.typ).toBe('JWT');
      expect(header.kid).toBeDefined();
    });

    it('should have unique jti for each token', async () => {
      const result1 = await mintSessionToken(testConfig, 'test-user', 'read');
      const result2 = await mintSessionToken(testConfig, 'test-user', 'read');
      
      const parts1 = result1.token.split('.');
      const parts2 = result2.token.split('.');
      
      const payload1 = JSON.parse(Buffer.from(parts1[1]!, 'base64url').toString('utf-8'));
      const payload2 = JSON.parse(Buffer.from(parts2[1]!, 'base64url').toString('utf-8'));
      
      expect(payload1.jti).not.toBe(payload2.jti);
    });

    it('should have matching kid in header and JWKS', async () => {
      const result = await mintSessionToken(testConfig, 'test-user', 'read');
      const jwks = await generateJwks(testConfig);
      
      const parts = result.token.split('.');
      const header = JSON.parse(Buffer.from(parts[0]!, 'base64url').toString('utf-8'));
      
      expect(header.kid).toBe(jwks.keys[0]!.kid);
    });
  });
});
