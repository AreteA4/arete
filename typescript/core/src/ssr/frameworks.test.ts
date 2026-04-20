import { describe, it, expect, vi } from 'vitest';
import crypto from 'node:crypto';
import type { AuthHandlerConfig } from './handlers';
import { createNextJsSessionRoute } from './nextjs-app';
import { createTanStackSessionRoute } from './tanstack-start';
import { createViteAuthMiddleware, createViteAuthPlugin } from './vite';

const testSeed = crypto.randomBytes(32);
const testConfig: AuthHandlerConfig = {
  signingKey: testSeed.toString('base64'),
  issuer: 'test-issuer',
  audience: 'test-audience',
  ttlSeconds: 300,
};

function decodePayload(token: string) {
  const [, payload] = token.split('.');
  return JSON.parse(Buffer.from(payload!, 'base64url').toString('utf-8'));
}

describe('SSR framework adapters', () => {
  it('nextjs route derives subject and scope from resolveSession', async () => {
    const handler = createNextJsSessionRoute({
      ...testConfig,
      resolveSession: async () => ({
        subject: 'trusted-user',
        scope: 'write',
      }),
    });

    const response = await handler({
      headers: new Headers({
        origin: 'https://example.com',
        'x-arete-subject': 'attacker',
        'x-arete-scope': 'admin',
      }),
    } as any);

    expect(response.status).toBe(200);

    const data = await response.json() as { token: string };
    const payload = decodePayload(data.token);
    expect(payload.sub).toBe('trusted-user');
    expect(payload.scope).toBe('write');
  });

  it('nextjs route fails closed when resolveSession is missing', async () => {
    const handler = createNextJsSessionRoute(testConfig);

    const response = await handler({
      headers: new Headers(),
    } as any);

    expect(response.status).toBe(500);
  });

  it('tanstack route rejects unauthenticated requests', async () => {
    const handler = createTanStackSessionRoute({
      ...testConfig,
      resolveSession: async () => null,
    });

    const response = await handler({
      request: {
        url: '/api/arete/sessions',
        headers: new Headers(),
      },
    });

    expect(response.status).toBe(401);
    await expect(response.json()).resolves.toEqual({ error: 'Unauthorized' });
  });

  it('vite middleware derives subject and scope from resolveSession', async () => {
    const middleware = createViteAuthMiddleware({
      ...testConfig,
      basePath: '/api/arete',
      resolveSession: async () => ({
        subject: 'trusted-user',
        scope: 'write',
      }),
    });

    const sent: { status?: number; body?: unknown } = {};
    const res = {
      json(data: unknown) {
        sent.body = data;
        return res;
      },
      status(code: number) {
        sent.status = code;
        return res;
      },
    };
    const next = vi.fn();

    await middleware(
      {
        method: 'POST',
        path: '/api/arete/sessions',
        headers: {
          origin: 'https://example.com',
          'x-arete-subject': 'attacker',
          'x-arete-scope': 'admin',
        },
      } as any,
      res as any,
      next
    );

    expect(next).not.toHaveBeenCalled();
    expect(sent.status).toBeUndefined();

    const payload = decodePayload((sent.body as { token: string }).token);
    expect(payload.sub).toBe('trusted-user');
    expect(payload.scope).toBe('write');
  });

  it('vite middleware handles root auth routes when basePath is omitted', async () => {
    const middleware = createViteAuthMiddleware({
      ...testConfig,
      resolveSession: async () => ({
        subject: 'trusted-user',
      }),
    });

    const sent: { status?: number; body?: unknown } = {};
    const res = {
      json(data: unknown) {
        sent.body = data;
        return res;
      },
      status(code: number) {
        sent.status = code;
        return res;
      },
    };
    const next = vi.fn();

    await middleware(
      {
        method: 'POST',
        path: '/sessions',
        headers: {
          origin: 'https://example.com',
        },
      } as any,
      res as any,
      next
    );

    expect(next).not.toHaveBeenCalled();
    expect(sent.status).toBeUndefined();

    const payload = decodePayload((sent.body as { token: string }).token);
    expect(payload.sub).toBe('trusted-user');
  });

  it('vite plugin mounts middleware with an empty internal basePath', async () => {
    const plugin = createViteAuthPlugin({
      ...testConfig,
      basePath: '/api/arete',
      resolveSession: async () => ({
        subject: 'trusted-user',
      }),
    });

    let mountPath: string | undefined;
    let mountedMiddleware: ((req: any, res: any, next: () => void) => Promise<void>) | undefined;

    plugin.configureServer({
      middlewares: {
        use(path: string, middleware: unknown) {
          mountPath = path;
          mountedMiddleware = middleware as typeof mountedMiddleware;
        },
      },
    });

    expect(mountPath).toBe('/api/arete');
    expect(mountedMiddleware).toBeTypeOf('function');

    const sent: { status?: number; body?: unknown } = {};
    const res = {
      json(data: unknown) {
        sent.body = data;
        return res;
      },
      status(code: number) {
        sent.status = code;
        return res;
      },
    };
    const next = vi.fn();

    await mountedMiddleware!(
      {
        method: 'POST',
        path: '/sessions',
        headers: {
          origin: 'https://example.com',
        },
      },
      res as any,
      next
    );

    expect(next).not.toHaveBeenCalled();
    expect(sent.status).toBeUndefined();

    const payload = decodePayload((sent.body as { token: string }).token);
    expect(payload.sub).toBe('trusted-user');
  });
});
