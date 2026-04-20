/**
 * TanStack Start integration for Arete Auth
 *
 * Drop-in API route handlers for TanStack Start.
 * `resolveSession` must derive the subject from verified server-side auth.
 * Never trust caller-supplied headers for identity or scope.
 *
 * @example
 * ```typescript
 * // app/routes/api/arete/sessions.ts
 * import { createTanStackSessionRoute, createTanStackJwksRoute } from '@usearete/sdk/ssr/tanstack-start';
 * import { json } from '@tanstack/react-start';
 *
 * export const APIRoute = createTanStackSessionRoute({
 *   resolveSession: async ({ request }) => {
 *     const user = await getAuthenticatedUser(request);
 *     if (!user) return null;
 *     return { subject: user.id };
 *   },
 * });
 *
 * // For JWKS at the same route with GET
 * export const GET = createTanStackJwksRoute();
 * ```
 */

import {
  type AuthHandlerConfig,
  mintSessionToken,
  generateJwks,
  type ResolvedSession,
  type TokenResponse,
} from './handlers';

export { type AuthHandlerConfig, type ResolvedSession, type TokenResponse };

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

export interface TanStackSessionRouteConfig extends AuthHandlerConfig {
  resolveSession?: (context: TanStackContext) => Promise<ResolvedSession | null> | ResolvedSession | null;
}

/**
 * Create a TanStack Start handler for POST /sessions
 * Returns a function compatible with TanStack Start's APIRoute
 */
export function createTanStackSessionRoute(config: TanStackSessionRouteConfig = {}) {
  return async function POST({ request }: TanStackContext): Promise<Response> {
    const origin = request.headers.get('origin') || undefined;

    try {
      if (!config.resolveSession) {
        return new Response(
          JSON.stringify({
            error: 'createTanStackSessionRoute requires resolveSession to derive the subject from authenticated server-side state',
          }),
          {
            status: 500,
            headers: {
              'Content-Type': 'application/json',
            },
          }
        );
      }

      const session = await config.resolveSession({ request });
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
 * // app/routes/api/arete/auth.ts
 * import { createTanStackAuthRoute } from '@usearete/sdk/ssr/tanstack-start';
 *
 * export const APIRoute = createTanStackAuthRoute({
 *   resolveSession: async ({ request }) => {
 *     const user = await getAuthenticatedUser(request);
 *     if (!user) return null;
 *     return { subject: user.id };
 *   },
 *   ttlSeconds: 600,
 * });
 * ```
 */
export function createTanStackAuthRoute(config: TanStackSessionRouteConfig = {}) {
  return {
    POST: createTanStackSessionRoute(config),
    GET: createTanStackJwksRoute(config),
  };
}

/**
 * Hook to access the Arete token in TanStack Start loaders
 *
 * @example
 * ```typescript
 * // app/routes/dashboard.tsx
 * import { createFileRoute } from '@tanstack/react-start';
 * import { fetchAreteToken } from '@usearete/sdk/ssr/tanstack-start';
 *
 * export const Route = createFileRoute('/dashboard')({
 *   loader: async () => {
 *     const token = await fetchAreteToken('/api/arete/sessions');
 *     // Use token for data fetching...
 *   },
 * });
 * ```
 */
export async function fetchAreteToken(
  endpoint: string = '/api/arete/sessions'
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
