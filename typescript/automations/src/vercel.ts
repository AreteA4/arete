/**
 * Automations on Vercel Functions.
 *
 * ```ts
 * // api/arete/[route].ts
 * import { waitUntil } from '@vercel/functions';
 * import { toVercel } from '@usearete/automations/vercel';
 * import definition from '../../automations';
 * import { runtime } from '../../automations/runtime';
 *
 * export const { GET, POST } = toVercel(definition, runtime, { waitUntil });
 * ```
 *
 * Serves `POST /api/arete/webhook`, `/api/arete/tick` (point a cron in
 * vercel.json at it; Vercel sends `CRON_SECRET` as the bearer token) and
 * `/api/arete/health`. The runtime is built once per instance from
 * `process.env` and reused while the instance is warm.
 */

import type { AutomationsDefinition } from './runtime/define';
import { createHost, type HostEnv, type HostOptions, type RuntimeFactory, type WaitUntil } from './hosts/host';

export interface VercelOptions extends HostOptions {
  /** `waitUntil` from `@vercel/functions`, so jobs run after the response. Without it a webhook's jobs wait for the next tick. */
  readonly waitUntil?: WaitUntil;
  /** Defaults to `process.env`. */
  readonly env?: HostEnv;
}

export interface VercelHandlers {
  GET(request: Request): Promise<Response>;
  POST(request: Request): Promise<Response>;
}

export function toVercel(
  definition: AutomationsDefinition<any>,
  runtime: RuntimeFactory<HostEnv>,
  options: VercelOptions = {},
): VercelHandlers {
  const host = createHost(definition, runtime, { ...options, cache: true });
  const env = () => options.env ?? ((globalThis as { process?: { env: HostEnv } }).process?.env ?? {});
  const handle = (request: Request) => host.handle(request, env(), options.waitUntil);
  return { GET: handle, POST: handle };
}

export type { HostOptions, RuntimeConfig, RuntimeFactory } from './hosts/host';
