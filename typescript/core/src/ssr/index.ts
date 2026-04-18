/**
 * Arete SSR - Drop-in Auth Endpoints
 *
 * These modules provide drop-in API route handlers for popular React frameworks.
 * Each handler can mint Ed25519-signed session tokens for WebSocket authentication.
 *
 * Quick Start:
 * ```bash
 * # Generate an Ed25519 signing key (32 bytes)
 * node -e "console.log(require('crypto').randomBytes(32).toString('base64'))"
 *
 * # Add to .env
 * ARETE_SIGNING_KEY=your-base64-key-here
 * ```
 *
 * Usage:
 *
 * **Next.js App Router:**
 * ```typescript
 * // app/api/arete/sessions/route.ts
 * import { createNextJsSessionRoute, createNextJsJwksRoute } from '@usearete/sdk/ssr/nextjs-app';
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
 * **Vite:**
 * ```typescript
 * // server.ts
 * import { createViteAuthMiddleware } from '@usearete/sdk/ssr/vite';
 *
 * app.use('/api/arete', createViteAuthMiddleware({
 *   basePath: '/api/arete',
 *   resolveSession: async (req) => {
 *     const user = await getAuthenticatedUser(req);
 *     if (!user) return null;
 *     return { subject: user.id };
 *   },
 * }));
 * ```
 *
 * **TanStack Start:**
 * ```typescript
 * // app/routes/api/arete/sessions.ts
 * import { createTanStackSessionRoute } from '@usearete/sdk/ssr/tanstack-start';
 *
 * export const APIRoute = createTanStackSessionRoute({
 *   resolveSession: async ({ request }) => {
 *     const user = await getAuthenticatedUser(request);
 *     if (!user) return null;
 *     return { subject: user.id };
 *   },
 * });
 * ```
 *
 * **Framework-agnostic:**
 * ```typescript
 * import { handleSessionRequest, handleJwksRequest } from '@usearete/sdk/ssr/handlers';
 *
  * // Use with any framework
 * export async function POST(request: Request) {
 *   const user = await getAuthenticatedUser(request);
 *   if (!user) return new Response('Unauthorized', { status: 401 });
 *   return handleSessionRequest({}, user.id);
 * }
 * ```
 */

// Re-export handlers for framework-agnostic usage
export {
  type AuthHandlerConfig,
  type ResolvedSession,
  type SessionClaims,
  type TokenResponse,
  type JwksResponse,
  mintSessionToken,
  generateJwks,
  handleSessionRequest,
  handleJwksRequest,
  handleHealthRequest,
} from './handlers';

// Re-export utilities
export { base64url } from './utils';
