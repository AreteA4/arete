/**
 * What every host adapter shares: three routes over a definition and a
 * runtime factory, in fetch-standard `Request`/`Response`.
 *
 * - `POST …/webhook` accepts a signed protocol event (the endpoint's secret
 *   is `ARETE_WEBHOOK_SECRET`) and runs jobs in the background;
 * - `GET|POST …/tick` is the cron: due schedules, polled sources, then jobs.
 *   It needs `Authorization: Bearer <CRON_SECRET>`, the header Vercel's cron
 *   sends;
 * - `GET …/health` names the protocol, deployment and cluster.
 *
 * Routes match on the last path segment, so they work under any prefix a
 * host mounts them at. The factory builds the client, signer, store and
 * policy from the host's environment; the definition stays host-agnostic.
 */

import { verifyWebhook, WebhookVerificationError } from '@usearete/sdk';

import type { Cluster } from '../policy/policy';
import {
  type Automations,
  type AutomationsOptions,
  clientCluster,
  createAutomations,
  type TickResult,
} from '../runtime/automations';
import type { AutomationsDefinition } from '../runtime/define';

/** A host's environment: variables and, on Workers, bindings. */
export type HostEnv = Readonly<Record<string, unknown>>;

/** What a runtime factory returns: the runtime's options, and what to close after a request on hosts that do not keep state between requests. */
// `any`: the factory's client is typed for its protocol, which the host never reads.
export interface RuntimeConfig extends AutomationsOptions<any> {
  /** Closes what the factory opened (a database pool). Called after each invocation on Workers. */
  dispose?(): Promise<void> | void;
}

export type RuntimeFactory<Env = HostEnv> = (env: Env) => RuntimeConfig | Promise<RuntimeConfig>;

export interface HostOptions {
  /** Jobs one webhook delivery runs in the background. Defaults to 25. */
  readonly maxJobsPerWebhook?: number;
  /** Jobs one cron tick runs. Defaults to 100. */
  readonly maxJobsPerTick?: number;
}

/** Runs work after the response is sent (`ctx.waitUntil` on Workers, `waitUntil` on Vercel). */
export type WaitUntil = (promise: Promise<unknown>) => void;

interface Instance {
  readonly automations: Automations;
  readonly config: RuntimeConfig;
}

function text(env: HostEnv, name: string): string | undefined {
  const value = env[name];
  return typeof value === 'string' && value !== '' ? value : undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** Constant-time comparison of two strings' UTF-8 bytes. */
export function sameText(given: string, expected: string): boolean {
  const a = new TextEncoder().encode(given);
  const b = new TextEncoder().encode(expected);
  let difference = a.length ^ b.length;
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) difference |= (a[i] ?? 0) ^ (b[i] ?? 0);
  return difference === 0;
}

/**
 * Refuses a runtime whose client targets another cluster than
 * `ARETE_CLUSTER` says. A build from a working tree that still holds the
 * client `a4 dev` generated would otherwise deploy against the dev
 * environment; `a4 install --locked` restores the published one.
 */
export function checkCluster(env: HostEnv, config: Pick<AutomationsOptions, 'client' | 'cluster'>): Cluster {
  const cluster = config.cluster ?? clientCluster(config.client);
  const expected = text(env, 'ARETE_CLUSTER');
  if (expected !== undefined && expected !== cluster) {
    throw new Error(
      cluster === 'dev'
        ? `ARETE_CLUSTER is ${expected}, but the protocol client in this build targets the dev environment (deployment ${config.client.deploymentId}). Run \`a4 install --locked\` to restore the published client, then deploy again.`
        : `ARETE_CLUSTER is ${expected}, but the protocol client in this build targets ${cluster} (deployment ${config.client.deploymentId}).`,
    );
  }
  return cluster;
}

export interface Host<Env = HostEnv> {
  /** Answer one request on the three routes. */
  handle(request: Request, env: Env, waitUntil?: WaitUntil): Promise<Response>;
  /** One cron pass, for a host's scheduled handler. */
  tick(env: Env): Promise<TickResult>;
}

/**
 * The routes over `definition`. With `cache`, one runtime is built per
 * process and reused (Node, Vercel); without, one per invocation, whose
 * stack connections and runtime are closed after it (Workers, where a
 * connection cannot outlive its request).
 */
export function createHost<Env extends object = HostEnv>(
  definition: AutomationsDefinition<any>,
  runtime: RuntimeFactory<Env>,
  options: HostOptions & { readonly cache: boolean },
): Host<Env> {
  const maxJobsPerWebhook = options.maxJobsPerWebhook ?? 25;
  const maxJobsPerTick = options.maxJobsPerTick ?? 100;
  let cached: Promise<Instance> | null = null;

  /** Close what an instance opened: the stacks its definition connected, then the factory's resources. */
  const release = async (instance: { automations?: Automations; config: RuntimeConfig }) => {
    try {
      await instance.automations?.stop();
    } finally {
      await instance.config.dispose?.();
    }
  };

  const build = async (env: Env): Promise<Instance> => {
    const config = await runtime(env);
    let automations: Automations | undefined;
    try {
      checkCluster(env as HostEnv, config);
      automations = createAutomations(config);
      await automations.use(definition, { sources: 'poll' });
      return { automations, config };
    } catch (error) {
      await release({ automations, config }).catch(() => {});
      throw error;
    }
  };

  /** Run `work` on an instance; a per-invocation instance is disposed once `work` and its background work end. */
  const withInstance = async <T>(env: Env, work: (instance: Instance, background: (promise: Promise<unknown>) => void) => Promise<T>, waitUntil?: WaitUntil) => {
    if (options.cache) {
      cached ??= build(env).catch((error: unknown) => {
        cached = null;
        throw error;
      });
      const instance = await cached;
      return work(instance, (promise) => (waitUntil ? waitUntil(promise) : void promise.catch(() => {})));
    }
    const instance = await build(env);
    const pending: Promise<unknown>[] = [];
    try {
      return await work(instance, (promise) => void pending.push(promise));
    } finally {
      const done = Promise.allSettled(pending).then(() => release(instance));
      if (waitUntil) waitUntil(done);
      else await done;
    }
  };

  const report = (instance: Instance, phase: string) => (error: unknown) => {
    try {
      instance.config.onError?.(error, { phase });
    } catch {
      // A failing error hook must not fail the request.
    }
  };

  const tick = (env: Env, waitUntil?: WaitUntil) =>
    withInstance(env, (instance) => instance.automations.tick({ maxJobs: maxJobsPerTick }), waitUntil);

  return {
    tick: (env) => tick(env),
    async handle(request, env, waitUntil) {
      const route = new URL(request.url).pathname.replace(/\/+$/, '').split('/').pop();
      const vars = env as HostEnv;
      try {
        if (route === 'webhook' && request.method === 'POST') {
          const secret = text(vars, 'ARETE_WEBHOOK_SECRET');
          if (!secret) return json(503, { error: 'ARETE_WEBHOOK_SECRET is not set' });
          const body = new Uint8Array(await request.arrayBuffer());
          // A forged request is refused before any runtime (or database
          // connection) is built for it.
          try {
            verifyWebhook(secret, body, request.headers as never);
          } catch (error) {
            if (!(error instanceof WebhookVerificationError)) throw error;
            // The status and body acceptWebhook gives a refused delivery.
            const status = error.code === 'invalid-signature' || error.code === 'timestamp-out-of-tolerance' ? 401 : 400;
            return json(status, { error: error.code });
          }
          return await withInstance(
            env,
            async (instance, background) => {
              const result = await instance.automations.acceptWebhook(body, request.headers, { secret });
              if (result.status === 200 && result.body.accepted === true) {
                background(instance.automations.drain({ maxJobs: maxJobsPerWebhook }).catch(report(instance, 'worker')));
              }
              return json(result.status, result.body);
            },
            waitUntil,
          );
        }
        if (route === 'tick' && (request.method === 'GET' || request.method === 'POST')) {
          const secret = text(vars, 'CRON_SECRET');
          if (!secret) return json(503, { error: 'CRON_SECRET is not set' });
          if (!sameText(request.headers.get('authorization') ?? '', `Bearer ${secret}`)) return json(401, { error: 'unauthorized' });
          return json(200, await tick(env, waitUntil));
        }
        if (route === 'health' && request.method === 'GET') {
          return await withInstance(
            env,
            async ({ config }) =>
              json(200, {
                ok: true,
                protocol: definition.protocol.name,
                deploymentId: config.client.deploymentId,
                cluster: config.cluster ?? clientCluster(config.client),
              }),
            waitUntil,
          );
        }
        return json(404, { error: 'not-found', routes: ['POST …/webhook', 'GET|POST …/tick', 'GET …/health'] });
      } catch (error) {
        console.error('arete automations:', error);
        return json(500, { error: 'internal', message: (error as Error)?.message ?? String(error) });
      }
    },
  };
}

