/**
 * Next.js App Router integration for Hyperstack Auth
 *
 * Drop-in route handlers for Next.js App Router.
 * `resolveSession` must derive the subject from verified server-side auth.
 * Never trust caller-supplied headers for identity or scope.
 *
 * @example
 * ```typescript
 * // app/api/hyperstack/sessions/route.ts
 * import { createNextJsSessionRoute, createNextJsJwksRoute } from 'hyperstack-typescript/ssr/nextjs-app';
 *
 * async function getAuthenticatedUser() {
 *   // Return your verified server-side user/session here.
 * }
 *
 * export const POST = createNextJsSessionRoute({
 *   resolveSession: async () => {
 *     const user = await getAuthenticatedUser();
 *     if (!user) return null;
 *     return { subject: user.id };
 *   },
 * });
 * export const GET = createNextJsJwksRoute();
 * ```
 *
 * @example
 * ```typescript
 * // app/api/hyperstack/sessions/route.ts (with custom config)
 * import { createNextJsSessionRoute, createNextJsJwksRoute } from 'hyperstack-typescript/ssr/nextjs-app';
 *
 * export const POST = createNextJsSessionRoute({
 *   signingKey: process.env.HYPERSTACK_SIGNING_KEY,
 *   resolveSession: async () => {
 *     const user = await getAuthenticatedUser();
 *     if (!user) return null;
 *     return { subject: user.id, scope: 'read' };
 *   },
 *   ttlSeconds: 600,
 * });
 *
 * export const GET = createNextJsJwksRoute({
 *   signingKey: process.env.HYPERSTACK_SIGNING_KEY,
 * });
 * ```
 */

import type { NextRequest } from 'next/server';
import {
  type AuthHandlerConfig,
  mintSessionToken,
  generateJwks,
  type ResolvedSession,
  type TokenResponse,
} from './handlers';

export { type AuthHandlerConfig, type ResolvedSession, type TokenResponse };

export interface NextJsSessionRouteConfig extends AuthHandlerConfig {
  resolveSession?: (request: NextRequest) => Promise<ResolvedSession | null> | ResolvedSession | null;
}

/**
 * Create a Next.js App Router POST handler for /ws/sessions
 */
export function createNextJsSessionRoute(config: NextJsSessionRouteConfig = {}) {
  return async function POST(request: NextRequest): Promise<Response> {
    const origin = request.headers.get('origin') || undefined;

    try {
      if (!config.resolveSession) {
        return new Response(
          JSON.stringify({
            error: 'createNextJsSessionRoute requires resolveSession to derive the subject from authenticated server-side state',
          }),
          {
            status: 500,
            headers: {
              'Content-Type': 'application/json',
            },
          }
        );
      }

      const session = await config.resolveSession(request);
      if (!session) {
        return new Response(
          JSON.stringify({
            error: 'Unauthorized',
          }),
          {
            status: 401,
            headers: {
              'Content-Type': 'application/json',
            },
          }
        );
      }

      const tokenData = await mintSessionToken(config, session.subject, session.scope || 'read', origin);
      
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
  };
}

/**
 * Create a Next.js App Router GET handler for /.well-known/jwks.json
 */
export function createNextJsJwksRoute(config: AuthHandlerConfig = {}) {
  return async function GET(): Promise<Response> {
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
  };
}

/**
 * Create a combined route handler that supports both POST (sessions) and GET (JWKS)
 * Mount at a single route like /api/hyperstack/auth
 */
export function createNextJsAuthRoute(config: NextJsSessionRouteConfig = {}) {
  return {
    POST: createNextJsSessionRoute(config),
    GET: createNextJsJwksRoute(config),
  };
}
