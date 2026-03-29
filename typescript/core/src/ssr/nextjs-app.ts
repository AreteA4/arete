/**
 * Next.js App Router integration for Hyperstack Auth
 *
 * Drop-in route handlers for Next.js App Router.
 *
 * @example
 * ```typescript
 * // app/api/hyperstack/sessions/route.ts
 * import { createNextJsSessionRoute, createNextJsJwksRoute } from 'hyperstack-typescript/ssr/nextjs-app';
 *
 * export const POST = createNextJsSessionRoute();
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
 *   ttlSeconds: 600,
 * });
 *
 * export const GET = createNextJsJwksRoute({
 *   signingKey: process.env.HYPERSTACK_SIGNING_KEY,
 * });
 * ```
 */

import type { NextRequest, NextResponse } from 'next/server';
import {
  type AuthHandlerConfig,
  mintSessionToken,
  generateJwks,
  type TokenResponse,
} from './handlers';

export { type AuthHandlerConfig, type TokenResponse };

/**
 * Create a Next.js App Router POST handler for /ws/sessions
 */
export function createNextJsSessionRoute(config: AuthHandlerConfig = {}) {
  return async function POST(request: NextRequest): Promise<Response> {
    // Get subject from header if provided (e.g., authenticated user)
    const subject = request.headers.get('x-hyperstack-subject') || 'anonymous';
    const scope = request.headers.get('x-hyperstack-scope') || 'read';
    const origin = request.headers.get('origin') || undefined;

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
export function createNextJsAuthRoute(config: AuthHandlerConfig = {}) {
  return {
    POST: createNextJsSessionRoute(config),
    GET: createNextJsJwksRoute(config),
  };
}
