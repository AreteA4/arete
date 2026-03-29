/**
 * TanStack Start integration for Hyperstack Auth
 *
 * Drop-in API route handlers for TanStack Start.
 *
 * @example
 * ```typescript
 * // app/routes/api/hyperstack/sessions.ts
 * import { createTanStackSessionRoute, createTanStackJwksRoute } from 'hyperstack-typescript/ssr/tanstack-start';
 * import { json } from '@tanstack/react-start';
 *
 * export const APIRoute = createTanStackSessionRoute();
 *
 * // For JWKS at the same route with GET
 * export const GET = createTanStackJwksRoute();
 * ```
 */

import {
  type AuthHandlerConfig,
  mintSessionToken,
  generateJwks,
  type TokenResponse,
} from './handlers';

export { type AuthHandlerConfig, type TokenResponse };

export interface TanStackRequest {
  url: string;
  headers: Headers;
}

export interface TanStackResponse {
  json: (data: unknown, init?: { status?: number }) => Response;
}

export interface TanStackContext {
  request: TanStackRequest;
}

/**
 * Create a TanStack Start handler for POST /sessions
 * Returns a function compatible with TanStack Start's APIRoute
 */
export function createTanStackSessionRoute(config: AuthHandlerConfig = {}) {
  return async function POST({ request }: TanStackContext): Promise<Response> {
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
 * Create a TanStack Start handler for GET /.well-known/jwks.json
 */
export function createTanStackJwksRoute(config: AuthHandlerConfig = {}) {
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
 * Create a TanStack Start API route that handles both POST (sessions) and GET (JWKS)
 *
 * @example
 * ```typescript
 * // app/routes/api/hyperstack/auth.ts
 * import { createTanStackAuthRoute } from 'hyperstack-typescript/ssr/tanstack-start';
 *
 * export const APIRoute = createTanStackAuthRoute({
 *   ttlSeconds: 600,
 * });
 * ```
 */
export function createTanStackAuthRoute(config: AuthHandlerConfig = {}) {
  return {
    POST: createTanStackSessionRoute(config),
    GET: createTanStackJwksRoute(config),
  };
}

/**
 * Hook to access the Hyperstack token in TanStack Start loaders
 *
 * @example
 * ```typescript
 * // app/routes/dashboard.tsx
 * import { createFileRoute } from '@tanstack/react-start';
 * import { fetchHyperstackToken } from 'hyperstack-typescript/ssr/tanstack-start';
 *
 * export const Route = createFileRoute('/dashboard')({
 *   loader: async () => {
 *     const token = await fetchHyperstackToken('/api/hyperstack/sessions');
 *     // Use token for data fetching...
 *   },
 * });
 * ```
 */
export async function fetchHyperstackToken(
  endpoint: string = '/api/hyperstack/sessions'
): Promise<string> {
  const response = await fetch(endpoint, {
    method: 'POST',
  });

  if (!response.ok) {
    throw new Error(`Failed to fetch token: ${response.statusText}`);
  }

  const data = (await response.json()) as TokenResponse;
  return data.token;
}
