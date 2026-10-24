/**
 * The files `arete-automations add <target>` writes. Each host gets a thin
 * entry over two files the project owns: the definition (`a4 dev up` runs
 * it) and `runtime.ts`, which builds the client, signer, store and policy
 * from the host's environment. Neither knows which host runs it.
 */

export type Target = 'cloudflare' | 'vercel' | 'node';
export type SignerKind = 'keypair' | 'privy';

export const TARGETS: readonly Target[] = ['cloudflare', 'vercel', 'node'];

export interface TemplateInput {
  /** The definition module relative to the project root, without extension (`automations/index`). */
  readonly entry: string;
  /** `runtime.ts` beside the definition, relative to the root, without extension. */
  readonly runtime: string;
  /** The project's npm package name. */
  readonly packageName: string;
  readonly signer: SignerKind;
}

/** `from` → `to`, both relative to the root, as an import specifier. */
export function importPath(from: string, to: string): string {
  const fromParts = from.split('/').slice(0, -1);
  const toParts = to.split('/');
  let common = 0;
  while (common < fromParts.length && common < toParts.length - 1 && fromParts[common] === toParts[common]) common += 1;
  const up = fromParts.length - common;
  const rest = toParts.slice(common).join('/');
  return up === 0 ? `./${rest}` : `${'../'.repeat(up)}${rest}`;
}

const SIGNERS: Record<SignerKind, { imports: string; body: string; env: string[] }> = {
  keypair: {
    imports: `import { keypairSigner } from '@usearete/automations/keypair';`,
    body: `/**
 * One wallet's key, as a Solana CLI keypair (a JSON array of 64 numbers), in
 * SIGNER_KEYPAIR. It signs for that wallet only: enough for staging, or for
 * steps your own wallet runs. To sign for your users, delegate their wallets
 * to a provider: \`arete-automations add <target> --signer privy\`.
 */
function signer(env: Env): ProtocolSigner {
  return keypairSigner(JSON.parse(need(env, 'SIGNER_KEYPAIR')) as number[]);
}`,
    env: ['SIGNER_KEYPAIR'],
  },
  privy: {
    imports: `import { PrivyClient } from '@privy-io/node';
import { privySigner } from '@usearete/automations/privy';`,
    body: `/**
 * Your users' Privy wallets, through the authorization key you registered
 * as a signer on them. \`walletIdFor\` maps a wallet address to its Privy
 * wallet id; replace the PRIVY_WALLETS lookup with your user records.
 */
function signer(env: Env): ProtocolSigner {
  const wallets = JSON.parse(text(env, 'PRIVY_WALLETS') ?? '{}') as Record<string, string>;
  return privySigner({
    privy: new PrivyClient({ appId: need(env, 'PRIVY_APP_ID'), appSecret: need(env, 'PRIVY_APP_SECRET') }),
    walletIdFor: async (address) => wallets[address] ?? null,
    authorizationPrivateKey: need(env, 'PRIVY_AUTHORIZATION_KEY'),
  });
}`,
    env: ['PRIVY_APP_ID', 'PRIVY_APP_SECRET', 'PRIVY_AUTHORIZATION_KEY'],
  },
};

/** The secrets and variables a deployment needs, for the next steps. */
export function requiredEnv(signer: SignerKind): string[] {
  return ['ARETE_SECRET_KEY', 'ARETE_WEBHOOK_SECRET', 'CRON_SECRET', 'DATABASE_URL', 'RPC_URL', ...SIGNERS[signer].env];
}

export function runtimeTs(input: TemplateInput): string {
  const signer = SIGNERS[input.signer];
  return `// How the automations run in production: the client, signer, store and
// policy, built from the host's environment. The host entry (worker.ts,
// api/arete/[route].ts or server.ts) calls \`runtime(env)\`; \`a4 dev up\` runs
// the definition with dev wallets and a local store instead, so nothing here
// runs in development.
//
// Environment:
//   ARETE_SECRET_KEY   a secret key of the account that deployed the protocol
//   DATABASE_URL       Postgres for jobs, consent and spending (on Workers, a
//                      Hyperdrive binding named HYPERDRIVE instead)
//   RPC_URL            a mainnet RPC; every transaction is simulated before signing
//   ARETE_CLUSTER      \`mainnet\` to let the policy sign on mainnet (default: dev only)
//   APP_DOMAIN         the host name your users see in the consent message
${signer.env.map((name) => `//   ${name}`).join('\n')}

import { createProtocolClient } from '@usearete/sdk';
import { definePolicy, type ProtocolSigner, rpcSimulator, type RuntimeConfig } from '@usearete/automations';
import { postgresStore } from '@usearete/automations/postgres';
${signer.imports}
import pg from 'pg';

import definition from '${importPath(input.runtime, input.entry)}';

type Env = Readonly<Record<string, unknown>>;

function text(env: Env, name: string): string | undefined {
  const value = env[name];
  return typeof value === 'string' && value !== '' ? value : undefined;
}

function need(env: Env, name: string): string {
  const value = text(env, name);
  if (value === undefined) throw new Error(\`set \${name}\`);
  return value;
}

${signer.body}

export function runtime(env: Env): RuntimeConfig {
  // The published client (\`a4 install --locked\`) targets mainnet; the one
  // \`a4 dev up\` generates targets the dev environment. The host refuses to
  // start when ARETE_CLUSTER and the client disagree.
  const client = createProtocolClient(definition.protocol, { auth: { secretKey: need(env, 'ARETE_SECRET_KEY') } });
  const hyperdrive = env.HYPERDRIVE as { connectionString?: string } | undefined;
  // Production signs only under the definition's own caps: no default here.
  if (!definition.policy) throw new Error('the automations definition needs a policy (programs and maxSolPerTransaction)');
  const pool = new pg.Pool({ connectionString: hyperdrive?.connectionString ?? need(env, 'DATABASE_URL'), max: 5 });
  return {
    client,
    signer: signer(env),
    policy: definePolicy({
      ...definition.policy,
      deployments: [client.deploymentId],
      clusters: [text(env, 'ARETE_CLUSTER') === 'mainnet' ? 'mainnet' : 'dev'],
    }),
    simulator: rpcSimulator(need(env, 'RPC_URL')),
    // Run \`npx arete-automations migrate\` once per release that changes the schema.
    store: postgresStore(pool),
    consent: { domain: text(env, 'APP_DOMAIN') ?? 'localhost' },
    onOutcome: (outcome) =>
      console.log(\`[\${outcome.state}] \${outcome.step} for \${outcome.wallet}: \${outcome.code}\${outcome.reason ? \` (\${outcome.reason})\` : ''}\`),
    onError: (error, context) => console.error(\`[\${context.phase}]\`, error),
    dispose: () => pool.end(),
  };
}
`;
}

export interface TargetFile {
  readonly path: string;
  readonly content: string;
}

export interface TargetTemplate {
  readonly files: readonly TargetFile[];
  readonly dependencies: readonly string[];
  readonly devDependencies: readonly string[];
  /** Where the webhook route is served, with `<host>` for the deployment's address. */
  readonly webhookUrl: string;
  /** How to store one secret (`{name}`), and how to deploy. */
  readonly secretCommand: string;
  readonly deployCommand: string;
  readonly notes: readonly string[];
}

function workerName(packageName: string): string {
  const base = packageName.replace(/^@[^/]+\//, '').replace(/[^a-z0-9-]/g, '-').replace(/-+/g, '-').replace(/^-|-$/g, '');
  return `${base || 'arete'}-automations`.slice(0, 63);
}

export function targetTemplate(target: Target, input: TemplateInput): TargetTemplate {
  const signerDeps = input.signer === 'privy' ? ['@privy-io/node'] : [];
  const base = ['@usearete/automations', '@usearete/sdk', 'pg', ...signerDeps];
  switch (target) {
    case 'cloudflare':
      return {
        files: [
          {
            path: 'worker.ts',
            content: `// The automations as a Cloudflare Worker: \`POST /arete/webhook\` takes the
// protocol's events, the cron trigger ticks schedules and retries, and
// \`GET /arete/health\` reports the deployment. See ${input.runtime}.ts for the
// environment it reads.
import { toWorker } from '@usearete/automations/cloudflare';

import definition from '${importPath('worker.ts', input.entry)}';
import { runtime } from '${importPath('worker.ts', input.runtime)}';

export default toWorker(definition, runtime);
`,
          },
          {
            path: 'wrangler.jsonc',
            content: `{
  "$schema": "node_modules/wrangler/config-schema.json",
  "name": "${workerName(input.packageName)}",
  "main": "worker.ts",
  "compatibility_date": "2026-10-01",
  // pg and the Solana libraries need Node.js APIs.
  "compatibility_flags": ["nodejs_compat"],
  // Ticks schedules, polls view sources and retries jobs.
  "triggers": { "crons": ["* * * * *"] },
  "vars": {
    // "mainnet" lets the policy sign on mainnet; the worker refuses to start
    // if the bundled protocol client targets the dev environment.
    "ARETE_CLUSTER": "mainnet",
    "APP_DOMAIN": "app.example.com"
  },
  // Postgres through Hyperdrive (recommended): create one with
  //   npx wrangler hyperdrive create ${workerName(input.packageName)} --connection-string="postgres://..."
  // and put its id here. Without it, set the DATABASE_URL secret.
  // "hyperdrive": [{ "binding": "HYPERDRIVE", "id": "<hyperdrive id>" }],
  "observability": { "enabled": true }
}
`,
          },
        ],
        dependencies: base,
        devDependencies: ['wrangler'],
        webhookUrl: 'https://<worker>.workers.dev/arete/webhook',
        secretCommand: 'npx wrangler secret put {name}',
        deployCommand: 'npx wrangler deploy',
        notes: [
          'The worker builds the runtime for each request and closes the database pool after it; Hyperdrive keeps the connections warm.',
          'View sources are polled once a minute by the cron trigger; append-view sources need the node target.',
        ],
      };
    case 'vercel':
      return {
        files: [
          {
            path: 'api/arete/[route].ts',
            content: `// The automations as a Vercel Function: \`POST /api/arete/webhook\` takes the
// protocol's events, the cron in vercel.json calls \`/api/arete/tick\` (with
// CRON_SECRET as its bearer token) for schedules and retries, and
// \`GET /api/arete/health\` reports the deployment. See ${input.runtime}.ts for
// the environment it reads.
import { waitUntil } from '@vercel/functions';
import { toVercel } from '@usearete/automations/vercel';

import definition from '${importPath('api/arete/[route].ts', input.entry)}';
import { runtime } from '${importPath('api/arete/[route].ts', input.runtime)}';

export const { GET, POST } = toVercel(definition, runtime, { waitUntil });
`,
          },
          {
            path: 'vercel.json',
            content: `${JSON.stringify(
              {
                $schema: 'https://openapi.vercel.sh/vercel.json',
                crons: [{ path: '/api/arete/tick', schedule: '* * * * *' }],
              },
              null,
              2,
            )}\n`,
          },
        ],
        dependencies: [...base, '@vercel/functions'],
        devDependencies: [],
        webhookUrl: 'https://<project>.vercel.app/api/arete/webhook',
        secretCommand: 'npx vercel env add {name} production',
        deployCommand: 'npx vercel deploy --prod',
        notes: [
          'Set ARETE_CLUSTER=mainnet and APP_DOMAIN as environment variables too.',
          'Per-minute crons need a Pro plan; on Hobby the cron runs once a day, so webhooks start jobs but retries and schedules wait for it.',
          'Use a pooled Postgres URL (Neon, Supabase or another pooler): each function instance opens its own pool.',
        ],
      };
    case 'node':
      return {
        files: [
          {
            path: 'server.ts',
            content: `// The automations as a long-running server, for any VM or container host
// (Fly.io, Railway, Render, Kubernetes): \`POST /arete/webhook\` takes the
// protocol's events; workers, schedules and view sources run in-process, so
// no external cron is needed. See ${input.runtime}.ts for the environment it reads.
import { serve } from '@usearete/automations/node';

import definition from '${importPath('server.ts', input.entry)}';
import { runtime } from '${importPath('server.ts', input.runtime)}';

const { port } = await serve(definition, runtime);
console.log(\`automations listening on :\${port}\`);
`,
          },
          {
            path: 'Dockerfile',
            content: `FROM node:22-slim
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
ENV NODE_ENV=production PORT=8787
EXPOSE 8787
CMD ["npx", "tsx", "server.ts"]
`,
          },
          {
            path: '.dockerignore',
            content: `node_modules
.arete
.git
`,
          },
        ],
        dependencies: [...base, 'tsx'],
        devDependencies: [],
        webhookUrl: 'https://<your host>/arete/webhook',
        secretCommand: 'set {name} in your host (fly secrets set, railway variables, ...)',
        deployCommand: 'deploy the Dockerfile (fly deploy, railway up, ...)',
        notes: [
          'Set ARETE_CLUSTER=mainnet and APP_DOMAIN as environment variables too.',
          'Run one instance, or several: jobs are claimed once and one step per wallet runs at a time.',
        ],
      };
  }
}
