/**
 * Automations on a long-running Node.js server: any VM or container host
 * (Fly.io, Railway, Render, a Kubernetes deployment).
 *
 * ```ts
 * // server.ts
 * import { serve } from '@usearete/automations/node';
 * import definition from './automations';
 * import { runtime } from './automations/runtime';
 *
 * await serve(definition, runtime);
 * ```
 *
 * Unlike the serverless adapters it runs workers continuously, watches view
 * sources live (append sources included) and ticks schedules itself, so no
 * external cron is needed. It serves `POST /arete/webhook` and
 * `GET /arete/health` (the tick route also works), and stops cleanly on
 * SIGTERM and SIGINT.
 */

import { createServer, type IncomingMessage, type Server } from 'node:http';

import { type Automations, createAutomations } from './runtime/automations';
import type { AutomationsDefinition } from './runtime/define';
import type { Duration } from './runtime/time';
import { durationMs } from './runtime/time';
import { checkCluster, type HostEnv, type RuntimeFactory, sameText } from './hosts/host';

export interface ServeOptions {
  /** Defaults to `PORT`, else 8787. */
  readonly port?: number;
  /** Defaults to `0.0.0.0`. */
  readonly hostname?: string;
  /** Worker loops. Defaults to 2. */
  readonly concurrency?: number;
  /** How often schedules are ticked. Defaults to 15s. */
  readonly tickInterval?: Duration;
  /** Largest webhook body accepted. Defaults to 1 MiB. */
  readonly maxBodyBytes?: number;
  /** Defaults to `process.env`. */
  readonly env?: HostEnv;
  /** Stop on SIGTERM and SIGINT. Defaults to true. */
  readonly handleSignals?: boolean;
}

export interface RunningServer {
  readonly automations: Automations;
  readonly server: Server;
  readonly port: number;
  close(): Promise<void>;
}

class TooLarge extends Error {}

async function readBody(req: IncomingMessage, limit: number): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  let size = 0;
  for await (const chunk of req) {
    size += (chunk as Uint8Array).length;
    if (size > limit) throw new TooLarge('request body too large');
    chunks.push(chunk as Uint8Array);
  }
  return Buffer.concat(chunks);
}

function text(env: HostEnv, name: string): string | undefined {
  const value = env[name];
  return typeof value === 'string' && value !== '' ? value : undefined;
}

export async function serve(
  definition: AutomationsDefinition<any>,
  runtime: RuntimeFactory<HostEnv>,
  options: ServeOptions = {},
): Promise<RunningServer> {
  const env = options.env ?? (process.env as HostEnv);
  const config = await runtime(env);
  checkCluster(env, config);
  const automations = createAutomations(config);
  await automations.use(definition);
  automations.start({ concurrency: options.concurrency ?? 2 });

  let ticking: Promise<unknown> = Promise.resolve();
  const tickNow = () => {
    ticking = automations.tick({ poll: false, maxJobs: 0 }).catch((error: unknown) => config.onError?.(error, { phase: 'schedule' }));
  };
  tickNow();
  const ticker = setInterval(tickNow, durationMs(options.tickInterval ?? '15s'));

  const maxBody = options.maxBodyBytes ?? 1024 * 1024;
  const server = createServer(async (req, res) => {
    const send = (status: number, body: unknown) => {
      res.writeHead(status, { 'content-type': 'application/json' });
      res.end(JSON.stringify(body));
    };
    try {
      const route = new URL(req.url ?? '/', 'http://localhost').pathname.replace(/\/+$/, '').split('/').pop();
      if (route === 'webhook' && req.method === 'POST') {
        const secret = text(env, 'ARETE_WEBHOOK_SECRET');
        if (!secret) return send(503, { error: 'ARETE_WEBHOOK_SECRET is not set' });
        const result = await automations.acceptWebhook(await readBody(req, maxBody), req.headers, { secret });
        return send(result.status, result.body);
      }
      if (route === 'health' && req.method === 'GET') {
        return send(200, { ok: true, protocol: definition.protocol.name, deploymentId: config.client.deploymentId, cluster: checkCluster(env, config) });
      }
      if (route === 'tick' && (req.method === 'GET' || req.method === 'POST')) {
        const secret = text(env, 'CRON_SECRET');
        if (!secret || !sameText(req.headers.authorization ?? '', `Bearer ${secret}`)) return send(secret ? 401 : 503, { error: secret ? 'unauthorized' : 'CRON_SECRET is not set' });
        return send(200, await automations.tick({ maxJobs: 0 }));
      }
      return send(404, { error: 'not-found' });
    } catch (error) {
      if (error instanceof TooLarge) return send(413, { error: error.message });
      config.onError?.(error, { phase: 'http' });
      return send(500, { error: 'internal' });
    }
  });

  const port = options.port ?? Number(text(env, 'PORT') ?? 8787);
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, options.hostname ?? '0.0.0.0', () => resolve());
  });
  const bound = (server.address() as { port: number }).port;

  let closing: Promise<void> | null = null;
  const close = () =>
    (closing ??= (async () => {
      clearInterval(ticker);
      await new Promise<void>((resolve) => server.close(() => resolve()));
      await ticking;
      await automations.stop();
      await config.dispose?.();
    })());
  if (options.handleSignals ?? true) {
    const stop = () => void close().then(() => process.exit(0));
    process.once('SIGTERM', stop);
    process.once('SIGINT', stop);
  }
  return { automations, server, port: bound, close };
}

export type { RuntimeConfig, RuntimeFactory } from './hosts/host';
