/**
 * `arete-automations dev`: what `a4 dev up` runs for `[dev.automations]` in
 * arete.toml. It loads the project's automations definition and runs it
 * against the project's dev environment. `a4 dev up` runs it with
 * `ARETE_ENVIRONMENT=dev` and `ARETE_DEV_URL` (its edge), so the installed
 * SDKs call the environment:
 *
 * - the client of the definition's protocol module, in dev mode: the
 *   environment's deployment of the protocol, named by the edge;
 * - a signer for every dev wallet, and a policy for that deployment only;
 * - a simulator on the environment (the fork's state), through the edge;
 * - a memory store saved under `.arete/dev/automations/`, so jobs, consent
 *   and spending survive the restarts that reload the handlers;
 * - the environment's protocol events, forwarded as signed webhook
 *   deliveries (dev environments send no webhooks);
 * - view sources on the environment's stacks, through the edge;
 * - schedules, ticked on the environment's clock (so `a4 dev warp` moves
 *   them too).
 *
 * With `forward` instead of an entry it only forwards the signed deliveries
 * to the app's own webhook route. `a4 dev up` restarts the process when the
 * handlers or the generated client change.
 */

import { randomBytes, timingSafeEqual } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { Arete, createProtocolClient } from '@usearete/sdk';
import type { EnvironmentOption, ProtocolDefinition } from '@usearete/sdk';

import type { ConsentFields, ConsentLimits } from '../consent/consent';
import { encodeBase58 } from '../policy/encoding';
import { definePolicy } from '../policy/policy';
import { rpcSimulator } from '../policy/simulator';
import { type Automations, type AutomationOutcome, createAutomations } from '../runtime/automations';
import { type AutomationsDefinition, isAutomationsDefinition, type StackConnector } from '../runtime/define';
import { devWalletsSigner, fileMemoryStore } from '../dev';
import { type EventPump, type PumpClient, startEventPump } from './events';
import { chainNow, type ScheduleTicker, startScheduleTicker } from './schedules';
import { readDevWallets } from './signer';

/** What `a4 dev up` passes in the environment. */
export interface DevRunnerConfig {
  /** The project root (holds `.arete/dev`). */
  readonly root: string;
  /** The definition module, when running handlers. */
  readonly entry?: string;
  /** The app's webhook route, when forwarding instead. */
  readonly forward?: string;
  /** The account's API key (server-only). */
  readonly apiKey: string;
  /** Signs forwarded deliveries. */
  readonly webhookSecret: string;
  /** Loopback port of the control server (`a4 dev consent`); 0 picks one. */
  readonly port: number;
}

interface DevSessionFile {
  readonly apiUrl: string;
  readonly sessionEndpoint: string;
  readonly rpcUrl: string;
  readonly deploymentId?: string | null;
  readonly protocolSlug?: string | null;
}

export interface DevRunner {
  readonly port: number;
  stop(): Promise<void>;
}

type Log = (line: string) => void;

const sleep = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));

/** The config from `a4 dev up`'s environment variables. */
export function configFromEnv(env: NodeJS.ProcessEnv): DevRunnerConfig {
  const need = (name: string) => {
    const value = env[name];
    if (!value) throw new Error(`${name} is not set; a4 dev up runs this command`);
    return value;
  };
  const entry = env.ARETE_AUTOMATIONS_ENTRY || undefined;
  const forward = env.ARETE_AUTOMATIONS_FORWARD || undefined;
  if (!entry === !forward) throw new Error('set exactly one of ARETE_AUTOMATIONS_ENTRY and ARETE_AUTOMATIONS_FORWARD');
  return {
    root: need('ARETE_PROJECT_ROOT'),
    ...(entry ? { entry } : {}),
    ...(forward ? { forward } : {}),
    apiKey: need('ARETE_API_KEY'),
    webhookSecret: need('ARETE_WEBHOOK_SECRET'),
    port: Number(env.ARETE_AUTOMATIONS_PORT ?? '0'),
  };
}

async function readSession(root: string, log: Log): Promise<DevSessionFile & { deploymentId: string }> {
  const path = join(root, '.arete', 'dev', 'session.json');
  let waiting = false;
  for (;;) {
    try {
      const session = JSON.parse(readFileSync(path, 'utf8')) as DevSessionFile;
      if (session.deploymentId) return session as DevSessionFile & { deploymentId: string };
    } catch {
      // Not written yet.
    }
    if (!waiting) log('waiting for the protocol to deploy to the dev environment');
    waiting = true;
    await sleep(2000);
  }
}

type TsImport = (specifier: string, parent: string) => Promise<Record<string, unknown>>;

/**
 * tsx's `tsImport`, from the project that holds `entry`: resolved from the
 * entry's directory, since a package manager that keeps each package's
 * dependencies apart (pnpm) does not let this package see the project's
 * tsx. `undefined` without tsx: Node's own type stripping (22.18+, 23.6+)
 * is the fallback.
 */
export async function projectTsImport(entry: string): Promise<TsImport | undefined> {
  let manifestPath: string;
  try {
    manifestPath = createRequire(entry).resolve('tsx/package.json');
  } catch {
    return undefined;
  }
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as {
    exports?: Record<string, { import?: string | { default?: string } }>;
  };
  const target = manifest.exports?.['./esm/api']?.import;
  const file = typeof target === 'string' ? target : target?.default;
  if (!file) return undefined;
  const api = (await import(pathToFileURL(join(dirname(manifestPath), file)).href)) as { tsImport?: TsImport };
  return api.tsImport;
}

/** Import the definition module; TypeScript through the project's tsx when it is installed. */
export async function loadDefinition(entry: string): Promise<AutomationsDefinition> {
  const url = pathToFileURL(entry).href;
  let module: Record<string, unknown>;
  if (/\.[cm]?tsx?$/.test(entry)) {
    const tsImport = await projectTsImport(entry);
    try {
      module = tsImport ? await tsImport(url, import.meta.url) : ((await import(url)) as Record<string, unknown>);
    } catch (error) {
      if (!tsImport) {
        throw new Error(`could not load ${entry}: install tsx (npm install --save-dev tsx) to run TypeScript (${(error as Error).message})`);
      }
      throw error;
    }
  } else {
    module = (await import(url)) as Record<string, unknown>;
  }
  // In a project without `"type": "module"` tsx loads the entry as
  // CommonJS, so its default export arrives one level down.
  const nested = (module.default as { default?: unknown } | undefined)?.default;
  const definition = [module.default, nested, module.automations].find(isAutomationsDefinition);
  if (!definition) {
    throw new Error(`${entry} must export its defineAutomations(...) as the default export`);
  }
  return definition;
}

/**
 * Stacks connect through the installed stack SDK in dev mode: the edge
 * streams the environment's deployment of each stack. A stack the
 * environment lacks is refused (`devenv-stack-not-loaded`), never read from
 * production.
 */
export function devStackConnector(): StackConnector {
  return async (stack) => {
    const client = await Arete.connect(stack, { environment: { kind: 'dev' } });
    // A stack the environment refused still returns a client (its other
    // surfaces work); a source needs the stack itself, so surface the refusal.
    await client.connect();
    return client as never;
  };
}

function readJson<T>(path: string, fallback: T): T {
  try {
    return JSON.parse(readFileSync(path, 'utf8')) as T;
  } catch {
    return fallback;
  }
}

function describe(outcome: AutomationOutcome): string {
  const where = outcome.instanceId ? ` on ${outcome.instanceId}` : '';
  const args = outcome.args && Object.keys(outcome.args).length > 0 ? ` ${JSON.stringify(outcome.args)}` : '';
  const reason = outcome.reason ? ` (${outcome.reason})` : '';
  const signatures = outcome.signatures.filter(Boolean);
  return `${outcome.state}: ${outcome.step}${args} for ${outcome.wallet}${where}: ${outcome.code}${reason}${signatures.length ? ` ${signatures.join(', ')}` : ''}`;
}

/** Forward-only mode needs a client for listing instances and their events, not a protocol module. */
function bareClient(
  session: DevSessionFile & { deploymentId: string },
  apiKey: string,
  newClient: (definition: ProtocolDefinition, apiKey: string, environment?: EnvironmentOption) => unknown,
): PumpClient {
  // The environment's deployment by id, with the account's key: this
  // client is the runner's own, not an installed SDK.
  const definition = {
    kind: 'protocol',
    name: session.protocolSlug ?? 'protocol',
    deployment: {
      deploymentId: session.deploymentId,
      access: 'private',
      apiUrl: `${session.apiUrl.replace(/\/$/, '')}/api`,
      sessionEndpoint: session.sessionEndpoint,
    },
    steps: {},
    events: {},
  } as unknown as ProtocolDefinition;
  return newClient(definition, apiKey, 'production') as PumpClient;
}

/** Test seams. */
export interface DevRunnerOverrides {
  /**
   * Builds the protocol client (default `createProtocolClient` with the API
   * key; the environment switch decides where it goes unless given).
   */
  readonly createClient?: (definition: ProtocolDefinition, apiKey: string, environment?: EnvironmentOption) => unknown;
}

/** How long the runner waits for the edge to name the deployment `a4 dev up` just made. */
const DEPLOYMENT_WAIT_MS = 15_000;

export async function runDev(
  config: DevRunnerConfig,
  log: Log = (line) => console.log(line),
  overrides: DevRunnerOverrides = {},
): Promise<DevRunner> {
  const newClient =
    overrides.createClient ??
    ((definition: ProtocolDefinition, apiKey: string, environment?: EnvironmentOption) =>
      createProtocolClient(definition, { auth: { secretKey: apiKey }, ...(environment ? { environment } : {}) }));
  const devDir = join(config.root, '.arete', 'dev');
  const stateDir = join(devDir, 'automations');
  mkdirSync(stateDir, { recursive: true, mode: 0o700 });
  // A mode given at creation does not change what already exists.
  chmodSync(stateDir, 0o700);
  const session = await readSession(config.root, log);

  // Events before the first run are history, not triggers.
  const sincePath = join(stateDir, 'since');
  if (!existsSync(sincePath)) writeFileSync(sincePath, String(Date.now()));
  const recorded = Number(readFileSync(sincePath, 'utf8').trim());
  const since = Number.isFinite(recorded) ? recorded : Date.now();
  const cursorsPath = join(stateDir, 'cursors.json');
  const cursors = new Map(Object.entries(readJson<Record<string, string>>(cursorsPath, {})));
  const saveCursors = () => writeFileSync(cursorsPath, JSON.stringify(Object.fromEntries(cursors), null, 2));

  let automations: Automations | null = null;
  let ticker: ScheduleTicker | null = null;
  let pumpClient: PumpClient;
  let saveStore = () => {};
  let deliver: (body: string, headers: Record<string, string>) => Promise<void>;

  if (config.entry) {
    const entry = resolve(config.root, config.entry);
    const definition = await loadDefinition(entry);
    const create = () => newClient(definition.protocol, config.apiKey) as ReturnType<typeof createProtocolClient>;
    let client = create();
    // `a4 dev up` sets the switch; only an override gets here in production.
    if (client.environment !== 'dev') {
      throw new Error(
        `the protocol client of ${entry} runs in production (ARETE_ENVIRONMENT is ${process.env.ARETE_ENVIRONMENT || 'unset'}), so it would target mainnet. ` +
          'a4 dev runs automations against the dev environment only: leave ARETE_ENVIRONMENT to `a4 dev up`.',
      );
    }
    // The client calls the environment's deployment, which the edge names.
    // Right after a redeploy the edge may still name the previous one for a
    // few seconds; a4 dev up restarts the runner on each new deployment.
    for (let waited = 0; ; waited += 1000) {
      const deployment = await client.resolveDeployment();
      if (deployment === session.deploymentId) break;
      if (waited >= DEPLOYMENT_WAIT_MS) {
        throw new Error(`the dev environment's deployment is ${deployment}, but a4 dev up deployed ${session.deploymentId}`);
      }
      await sleep(1000);
      client = create();
    }

    // One store per deployment: a redeploy to a new deployment starts afresh
    // and keeps the previous state beside it.
    const store = fileMemoryStore(client, join(stateDir, 'store.bin'), {
      saveIntervalMs: 0,
      onArchive: (archived) =>
        log(`the deployment changed; earlier consent and jobs are kept in ${basename(archived)}`),
    });
    saveStore = () => store.save();

    const walletsDir = join(devDir, 'wallets');
    const rpc = rpcSimulator(undefined, { environment: { kind: 'dev' } });
    const running = createAutomations({
      client,
      signer: devWalletsSigner(client, walletsDir),
      policy: definePolicy({
        maxSolPerTransaction: '1',
        ...definition.policy,
        deployments: [client.deploymentId],
        clusters: ['dev'],
      }),
      simulator: rpc,
      store,
      consent: { domain: 'localhost' },
      onOutcome: (outcome) => log(describe(outcome)),
      onError: (error, context) => log(`error (${context.phase}${context.jobId ? ` ${context.jobId}` : ''}): ${(error as Error)?.message ?? error}`),
    });
    await running.use(definition as AutomationsDefinition<any>, { connect: devStackConnector() });
    running.start({ pollInterval: '1s' });
    ticker = startScheduleTicker(running, () => chainNow(rpc), log);
    automations = running;
    pumpClient = client as unknown as PumpClient;
    deliver = async (body, headers) => {
      const result = await running.acceptWebhook(body, headers, { secret: config.webhookSecret });
      if (result.status !== 200) throw new Error(`the delivery was refused: ${JSON.stringify(result.body)}`);
    };
  } else {
    const url = config.forward!;
    pumpClient = bareClient(session, config.apiKey, newClient);
    deliver = async (body, headers) => {
      const response = await fetch(url, { method: 'POST', headers, body });
      if (!response.ok) throw new Error(`${url} answered HTTP ${response.status}`);
    };
  }

  const pump: EventPump = startEventPump({
    client: pumpClient,
    secret: config.webhookSecret,
    cursors,
    since,
    deliver: async (delivery, event) => {
      await deliver(delivery.body, delivery.headers);
      if (!automations) log(`forwarded ${event.event} on ${event.instanceId}`);
    },
    onError: (error, context) =>
      log(`error (${context.phase}${context.instanceId ? ` ${context.instanceId}` : ''}): ${(error as Error)?.message ?? error}`),
  });

  const persist = setInterval(() => {
    try {
      saveStore();
      saveCursors();
    } catch (error) {
      log(`could not save the automations state: ${(error as Error).message}`);
    }
  }, 1000);

  // `a4 dev consent` reads the token from runner.json (mode 0600); a web page
  // that reaches the loopback port cannot.
  const token = randomBytes(32).toString('hex');
  const server = await startControl(config.port, token, automations, pumpClient.deploymentId, join(devDir, 'wallets'), log);
  const port = (server.address() as { port: number }).port;
  writeFileSync(join(stateDir, 'runner.json'), JSON.stringify({ pid: process.pid, port, token }), { mode: 0o600 });
  chmodSync(join(stateDir, 'runner.json'), 0o600);
  log(
    automations
      ? `running ${config.entry} against deployment ${session.deploymentId}`
      : `forwarding events of deployment ${session.deploymentId} to ${config.forward}`,
  );

  return {
    port,
    async stop() {
      // `a4 dev up` kills the runner 5s after SIGTERM: save first, let the
      // workers wind down for at most 3s (saving as they go), save again.
      pump.stop();
      server.closeAllConnections();
      server.close();
      const save = () => {
        try {
          saveStore();
          saveCursors();
        } catch (error) {
          log(`could not save the automations state: ${(error as Error).message}`);
        }
      };
      save();
      await Promise.race([Promise.all([ticker?.stop(), automations?.stop()]), sleep(3000)]);
      clearInterval(persist);
      save();
    },
  };
}

// ---------------------------------------------------------------------------
// The control server: loopback only, for `a4 dev consent`.
// ---------------------------------------------------------------------------

interface ConsentRequest {
  readonly wallet: string;
  readonly steps: readonly string[];
  readonly limits?: unknown;
  readonly params?: Record<string, unknown>;
  readonly instanceId?: string | null;
  readonly expiresAt?: number;
  readonly revoke?: boolean;
}

async function startControl(
  port: number,
  token: string,
  automations: Automations | null,
  deploymentId: string,
  walletsDir: string,
  log: Log,
): Promise<Server> {
  const json = (res: ServerResponse, status: number, body: unknown) => {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(body, (_key, value) => (typeof value === 'bigint' ? value.toString() : value)));
  };
  const readBody = async (req: IncomingMessage) => {
    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    return JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}') as ConsentRequest;
  };
  const server = createServer(async (req, res) => {
    try {
      if (req.method === 'GET' && req.url === '/health') return json(res, 200, { ok: true, handlers: automations !== null });
      if (req.method !== 'POST' || req.url !== '/consent') return json(res, 404, { error: 'not-found' });
      // Only `a4 dev consent`: no browser origin, JSON only, and the token.
      if (req.headers.origin !== undefined) return json(res, 403, { error: 'forbidden' });
      if (!req.headers['content-type']?.startsWith('application/json')) return json(res, 415, { error: 'json-only' });
      const expected = Buffer.from(`Bearer ${token}`);
      const given = Buffer.from(req.headers.authorization ?? '');
      if (given.length !== expected.length || !timingSafeEqual(given, expected)) {
        return json(res, 401, { error: 'unauthorized' });
      }
      if (!automations) return json(res, 409, { error: 'forwarding only: consent lives in your app' });
      const request = await readBody(req);
      const wallets = readDevWallets(walletsDir);
      const signer = wallets.byName.get(request.wallet) ?? wallets.byAddress.get(request.wallet);
      if (!signer) return json(res, 400, { error: `${request.wallet} is not a dev wallet (a4 dev wallet <name> creates one)` });
      const wallet = signer.publicKey;
      if (request.revoke) {
        const revoked = await automations.consent.revoke({
          wallet,
          ...(request.steps.length ? { steps: request.steps } : {}),
          ...(request.instanceId ? { instanceId: request.instanceId } : {}),
        });
        log(`revoked ${revoked} consent grant(s) of ${wallet}`);
        return json(res, 200, { wallet, revoked });
      }
      const fields: ConsentFields = {
        wallet,
        deploymentId,
        steps: request.steps,
        limits: (request.limits ?? {}) as ConsentLimits,
        ...(request.params ? { params: request.params } : {}),
        ...(request.instanceId ? { instanceId: request.instanceId } : {}),
        expiresAt: request.expiresAt ?? Date.now() + 30 * 86_400_000,
      };
      const proof = await automations.consent.proofMessage(fields);
      // The dev wallet signs the message as the user's wallet would.
      const signature = encodeBase58(signer.signMessage(proof.bytes));
      const { grant, warnings } = await automations.consent.grant({
        ...fields,
        nonce: proof.nonce,
        issuedAt: proof.issuedAt,
        signature,
      });
      log(`${wallet} consented to ${request.steps.join(', ')}`);
      return json(res, 200, { grant, warnings, message: proof.message });
    } catch (error) {
      json(res, 400, { error: (error as Error).message });
    }
  });
  await new Promise<void>((done, fail) => {
    server.once('error', fail);
    server.listen(port, '127.0.0.1', () => done());
  });
  return server;
}
