/**
 * Automations on Cloudflare Workers.
 *
 * ```ts
 * // worker.ts
 * import { toWorker } from '@usearete/automations/cloudflare';
 * import definition from './automations';
 * import { runtime } from './automations/runtime';
 *
 * export default toWorker(definition, runtime);
 * ```
 *
 * `fetch` serves `POST /arete/webhook` (any prefix works), `/arete/tick` and
 * `/arete/health`; `scheduled` (a cron trigger in wrangler.jsonc) ticks.
 * The runtime is built for each invocation from the Worker's `env` and
 * disposed afterwards, since a database connection cannot outlive the
 * request that opened it. Needs the `nodejs_compat` flag.
 */

import type { AutomationsDefinition } from './runtime/define';
import { createHost, type HostOptions, type RuntimeFactory } from './hosts/host';

/** The parts of a Worker's `ExecutionContext` the adapter uses. */
export interface ExecutionContextLike {
  waitUntil(promise: Promise<unknown>): void;
}

export interface WorkerHandlers<Env> {
  fetch(request: Request, env: Env, ctx: ExecutionContextLike): Promise<Response>;
  scheduled(controller: unknown, env: Env, ctx: ExecutionContextLike): Promise<void>;
}

export function toWorker<Env extends object>(
  definition: AutomationsDefinition<any>,
  runtime: RuntimeFactory<Env>,
  options: HostOptions = {},
): WorkerHandlers<Env> {
  const host = createHost(definition, runtime, { ...options, cache: false });
  return {
    fetch: (request, env, ctx) => host.handle(request, env, (promise) => ctx.waitUntil(promise)),
    async scheduled(_controller, env, ctx) {
      ctx.waitUntil(host.tick(env));
    },
  };
}

export type { HostOptions, RuntimeConfig, RuntimeFactory } from './hosts/host';
