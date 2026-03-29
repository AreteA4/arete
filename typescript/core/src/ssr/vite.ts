/**
 * Vite SSR integration for Hyperstack Auth
 *
 * Express/Connect middleware that mounts auth endpoints.
 *
 * @example
 * ```typescript
 * // server.ts
 * import express from 'express';
 * import { createViteAuthMiddleware } from 'hyperstack-typescript/ssr/vite';
 *
 * const app = express();
 *
 * // Mount auth endpoints at /api/hyperstack
 * app.use('/api/hyperstack', createViteAuthMiddleware());
 *
 * // Or mount at root
 * app.use(createViteAuthMiddleware({
 *   basePath: '/auth',
 * }));
 * ```
 */

import type { Request, Response } from 'express';
import {
  type AuthHandlerConfig,
  mintSessionToken,
  generateJwks,
  handleHealthRequest,
  type TokenResponse,
} from './handlers';

export { type AuthHandlerConfig, type TokenResponse };

export interface ViteAuthMiddlewareOptions extends AuthHandlerConfig {
  /**
   * Base path for auth endpoints
   * @default '/'
   */
  basePath?: string;
}

/**
 * Create Express middleware that mounts Hyperstack auth endpoints
 */
export function createViteAuthMiddleware(options: ViteAuthMiddlewareOptions = {}) {
  const { basePath = '/', ...config } = options;

  // Note: In production, you'd use express.Router(), but for Vite SSR
  // we just return a middleware function that checks the path
  return async function middleware(req: Request, res: Response, next: () => void) {
    const pathname = req.path;

    // POST /{basePath}/sessions - Mint token
    if (req.method === 'POST' && pathname === `${basePath}/sessions`) {
      const subject = (req.headers['x-hyperstack-subject'] as string) || 'anonymous';
      const scope = (req.headers['x-hyperstack-scope'] as string) || 'read';
      const origin = req.headers.origin as string | undefined;

      try {
        const tokenData = await mintSessionToken(config, subject, scope, origin);
        res.json(tokenData);
        return;
      } catch (error) {
        res.status(500).json({
          error: error instanceof Error ? error.message : 'Failed to mint token',
        });
        return;
      }
    }

    // GET /{basePath}/.well-known/jwks.json - JWKS
    if (req.method === 'GET' && pathname === `${basePath}/.well-known/jwks.json`) {
      try {
        const jwks = await generateJwks(config);
        res.json(jwks);
        return;
      } catch (error) {
        res.status(500).json({
          error: error instanceof Error ? error.message : 'Failed to generate JWKS',
        });
        return;
      }
    }

    // GET /{basePath}/health - Health check
    if (req.method === 'GET' && pathname === `${basePath}/health`) {
      const response = handleHealthRequest();
      res.status(response.status).json(await response.json());
      return;
    }

    // Not an auth route, pass to next middleware
    next();
  };
}

/**
 * Create a Vite plugin that injects the auth endpoints
 * This is for use with Vite's configureServer hook
 *
 * @example
 * ```typescript
 * // vite.config.ts
 * import { defineConfig } from 'vite';
 * import { createViteAuthPlugin } from 'hyperstack-typescript/ssr/vite';
 *
 * export default defineConfig({
 *   plugins: [
 *     createViteAuthPlugin({
 *       basePath: '/api/hyperstack',
 *     }),
 *   ],
 * });
 * ```
 */
export function createViteAuthPlugin(options: ViteAuthMiddlewareOptions = {}) {
  return {
    name: 'hyperstack-auth',
    configureServer(server: { middlewares: { use: (path: string, middleware: unknown) => void } }) {
      server.middlewares.use(options.basePath || '/api/hyperstack', createViteAuthMiddleware(options));
    },
  };
}

/**
 * Helper to inject token into HTML for client-side hydration
 */
export function injectHyperstackToken(
  html: string,
  token: string | undefined
): string {
  if (!token) return html;

  const tokenScript = `
    <script>
      window.__HYPERSTACK_TOKEN__ = ${JSON.stringify(token)};
    </script>
  `;

  if (html.includes('</head>')) {
    return html.replace('</head>', `${tokenScript}</head>`);
  }

  return html.replace('<body>', `<body>${tokenScript}`);
}
