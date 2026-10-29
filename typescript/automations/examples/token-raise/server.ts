// Automatic refunds for a token raise: the definition in automations.ts, with
// a signer, a store, the webhook route and the consent routes your app calls.
//
// Against a dev environment (a client `a4 dev` generated, and
// `[dev.automations] forward = "http://localhost:8787/arete/webhook"` in
// arete.toml), it runs on the helpers of `@usearete/automations/dev`: dev
// wallets sign (Privy too, for other wallets, with SIGNER=privy), the store
// is a file under .arete/dev/, transactions simulate on the fork, and
// POST /dev/consent signs a consent with a dev wallet.
//
// In production (the published client, after `a4 install --locked`):
//   SIGNER=keypair DEV_WALLET=~/.config/solana/dev.json ...
//   SIGNER=privy PRIVY_APP_ID=... PRIVY_APP_SECRET=... PRIVY_AUTHORIZATION_KEY=... PRIVY_WALLETS='{"<address>":"<wallet id>"}'
//
// See README.md for the whole setup.

import { readFileSync } from 'node:fs';
import { createServer, type IncomingMessage } from 'node:http';
import { homedir } from 'node:os';

import { createProtocolClient } from '@usearete/sdk';
import pg from 'pg';

import {
  type AutomationOutcome,
  type AutomationStore,
  clientCluster,
  createAutomations,
  definePolicy,
  type ProtocolSigner,
  rpcSimulator,
  SOL,
} from '@usearete/automations';
import { devSimulator, devWalletsSigner, fileMemoryStore, grantDevConsent, routingSigner } from '@usearete/automations/dev';
import { keypairSigner } from '@usearete/automations/keypair';
import { postgresStore } from '@usearete/automations/postgres';
import { type PrivyClientLike, privySigner } from '@usearete/automations/privy';

import definition from './automations';
import { TOKEN_RAISE_PROTOCOL } from './token-raise';

function env(name: string, fallback?: string): string {
  const value = process.env[name] ?? fallback;
  if (value === undefined) throw new Error(`set ${name}`);
  return value;
}

async function signerFromEnv(): Promise<ProtocolSigner> {
  if (env('SIGNER', 'keypair') === 'keypair') {
    const path = env('DEV_WALLET').replace(/^~(?=\/)/, homedir());
    return keypairSigner(JSON.parse(readFileSync(path, 'utf8')) as number[]);
  }
  // Loaded at run time so the example needs @privy-io/node only with SIGNER=privy.
  const privyModule = '@privy-io/node';
  const { PrivyClient } = (await import(privyModule)) as {
    PrivyClient: new (options: { appId: string; appSecret: string }) => PrivyClientLike;
  };
  const wallets = JSON.parse(env('PRIVY_WALLETS', '{}')) as Record<string, string>;
  return privySigner({
    privy: new PrivyClient({ appId: env('PRIVY_APP_ID'), appSecret: env('PRIVY_APP_SECRET') }),
    walletIdFor: async (wallet) => wallets[wallet] ?? null,
    authorizationPrivateKey: env('PRIVY_AUTHORIZATION_KEY'),
  });
}

async function privyConfigured(): Promise<ProtocolSigner[]> {
  return process.env.SIGNER === 'privy' ? [await signerFromEnv()] : [];
}

async function main() {
  const client = createProtocolClient(TOKEN_RAISE_PROTOCOL, { auth: { secretKey: env('ARETE_SECRET_KEY') } });
  // A client `a4 dev` generated targets the dev environment; the published
  // one (`a4 install --locked`) targets mainnet, which the policy allows only
  // with ALLOW_MAINNET=1. (Temporary: this reads the environment from the
  // client's deployment until the SDK exposes it on the client.)
  const dev = clientCluster(client) === 'dev';

  // Dev: the project's dev wallets first, then Privy for any other wallet;
  // a file store; simulation on the fork. Each dev helper refuses a client
  // that targets production.
  const devWallets = dev ? devWalletsSigner(client) : null;
  let store: AutomationStore;
  if (dev) {
    store = fileMemoryStore(client, '.arete/dev/token-raise-store.bin');
  } else {
    const pool = new pg.Pool({ connectionString: env('DATABASE_URL') });
    const postgres = postgresStore(pool);
    store = postgres;
    if (process.argv.includes('--migrate')) {
      console.log(await postgres.migrate());
      await pool.end();
      return;
    }
  }

  const notify = (outcome: AutomationOutcome) =>
    console.log(`[${outcome.state}] ${outcome.step} for ${outcome.wallet}: ${outcome.code}${outcome.reason ? ` (${outcome.reason})` : ''}`);

  const automations = createAutomations({
    client,
    signer: devWallets ? routingSigner(client, [devWallets, ...(await privyConfigured())]) : await signerFromEnv(),
    policy: definePolicy({
      ...definition.policy!,
      deployments: [client.deploymentId],
      clusters: env('ALLOW_MAINNET', '') === '1' ? ['dev', 'mainnet'] : ['dev'],
    }),
    simulator: dev ? devSimulator(client) : rpcSimulator(env('RPC_URL')),
    store,
    consent: { domain: env('APP_DOMAIN', 'localhost') },
    onOutcome: notify,
    onError: (error, context) => console.error(`[${context.phase}]`, error),
  });

  await automations.use(definition);

  const MAX_BODY = 64 * 1024;
  class TooLarge extends Error {}
  const readBody = async (req: IncomingMessage) => {
    const chunks: Buffer[] = [];
    let size = 0;
    for await (const chunk of req) {
      size += (chunk as Buffer).length;
      if (size > MAX_BODY) {
        req.destroy();
        throw new TooLarge('request body too large');
      }
      chunks.push(chunk as Buffer);
    }
    return Buffer.concat(chunks);
  };
  // The consent routes are for your app's backend, which has already
  // authenticated the user and checked that they own the wallet. In a real
  // app they sit behind your session middleware; here, behind a shared token.
  const appToken = env('APP_API_TOKEN');
  const fromApp = (req: IncomingMessage) => req.headers.authorization === `Bearer ${appToken}`;
  const json = (res: import('node:http').ServerResponse, status: number, body: unknown) => {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(body, (_key, value) => (typeof value === 'bigint' ? value.toString() : value)));
  };
  const consentFields = (wallet: string, expiresAt: number) => ({
    wallet,
    deploymentId: client.deploymentId,
    steps: ['refund'],
    limits: { perExecution: { [SOL]: '10000' }, perWindow: { [SOL]: { amount: '100000', window: '30d' as const } } },
    expiresAt,
  });

  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', 'http://localhost');
      if (req.method === 'POST' && url.pathname === '/arete/webhook') {
        const result = await automations.acceptWebhook(await readBody(req), req.headers, {
          secret: env('ARETE_WEBHOOK_SECRET'),
        });
        return json(res, result.status, result.body);
      }
      if ((url.pathname.startsWith('/consent/') || url.pathname.startsWith('/dev/')) && !fromApp(req)) {
        return json(res, 401, { error: 'unauthorized' });
      }
      // Dev only: a dev wallet (by name or address) signs its own consent,
      // as `a4 dev consent` does when `a4 dev up` runs the automations.
      if (devWallets && req.method === 'POST' && url.pathname === '/dev/consent') {
        const { wallet, expiresAt } = JSON.parse(String(await readBody(req))) as { wallet: string; expiresAt?: number };
        const address = devWallets.resolve(wallet) ?? wallet;
        return json(res, 200, await grantDevConsent(automations, devWallets, consentFields(address, expiresAt ?? Date.now() + 30 * 86_400_000)));
      }
      // 1. The app asks for the message its user signs to opt in.
      if (req.method === 'POST' && url.pathname === '/consent/message') {
        const { wallet, expiresAt } = JSON.parse(String(await readBody(req))) as { wallet: string; expiresAt: number };
        const proof = await automations.consent.proofMessage(consentFields(wallet, expiresAt));
        return json(res, 200, { message: proof.message, nonce: proof.nonce, issuedAt: proof.issuedAt });
      }
      // 2. The app sends the wallet's signature back.
      if (req.method === 'POST' && url.pathname === '/consent/grant') {
        const body = JSON.parse(String(await readBody(req))) as {
          wallet: string;
          expiresAt: number;
          nonce: string;
          issuedAt: number;
          signature: string;
        };
        const { grant, warnings } = await automations.consent.grant({
          ...consentFields(body.wallet, body.expiresAt),
          nonce: body.nonce,
          issuedAt: body.issuedAt,
          signature: body.signature,
        });
        return json(res, 200, { grant, warnings });
      }
      // Opting out needs no wallet signature, but only the app (for its signed-in user) may ask.
      if (req.method === 'DELETE' && url.pathname.startsWith('/consent/')) {
        const revoked = await automations.consent.revoke({ wallet: url.pathname.slice('/consent/'.length) });
        return json(res, 200, { revoked });
      }
      return json(res, 404, { error: 'not found' });
    } catch (error) {
      if (error instanceof TooLarge) return json(res, 413, { error: error.message });
      return json(res, 400, { error: (error as Error).message });
    }
  });

  automations.start({ concurrency: 2 });
  server.listen(Number(env('PORT', '8787')), () => console.log(`listening on :${env('PORT', '8787')}`));
}

void main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
