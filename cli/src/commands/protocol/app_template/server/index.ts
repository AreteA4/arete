/**
 * The automations server: runs automations/index.ts with a Privy signer, a
 * store, the webhook route and the consent routes the app's backend calls.
 * Built from the token-raise example's server (`@usearete/automations`).
 *
 * In dev (`a4 dev run -- npm run server`), point `a4 dev up` at it with
 * `[dev.automations] forward = "http://localhost:8787/arete/webhook"` in
 * arete.toml. It then runs on `@usearete/automations/dev`: dev wallets sign
 * (Privy too, for any other wallet, when PRIVY_APP_ID is set), the store is
 * a file under .arete/dev/, transactions simulate on the fork, and
 * POST /dev/consent signs a consent with a dev wallet. Each dev helper
 * throws for a production client.
 *
 * In production (`npm run server`, after `a4 install --locked`): Privy
 * signs, Postgres keeps jobs and consent (`npm run server -- --migrate`
 * once), and RPC_URL simulates. See README.md.
 */
import { existsSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

import {
  type AutomationStore,
  clientCluster,
  createAutomations,
  definePolicy,
  type ProtocolSigner,
  rpcSimulator,
  SOL,
} from "@usearete/automations";
import { devSimulator, devWalletsSigner, fileMemoryStore, grantDevConsent, routingSigner } from "@usearete/automations/dev";
import { postgresStore } from "@usearete/automations/postgres";
import { type PrivyClientLike, privySigner } from "@usearete/automations/privy";
import { createProtocolClient } from "@usearete/sdk";

import definition from "../automations/index.js";
import protocol from "../generated/typescript/protocols/{{slug}}/{{slug}}.js";

// Settings from .env (see .env.example); variables already set win.
if (existsSync(".env")) process.loadEnvFile(".env");

function env(name: string, fallback?: string): string {
  const value = process.env[name] || fallback;
  if (value === undefined) throw new Error(`set ${name}`);
  return value;
}

/** Privy, when configured: `PRIVY_WALLETS` maps wallet addresses to Privy wallet ids. */
async function privyFromEnv(): Promise<ProtocolSigner | null> {
  if (!process.env.PRIVY_APP_ID) return null;
  // Loaded at run time, so the server needs @privy-io/node only with Privy.
  const privyModule = "@privy-io/node";
  const { PrivyClient } = (await import(privyModule)) as {
    PrivyClient: new (options: { appId: string; appSecret: string }) => PrivyClientLike;
  };
  const wallets = JSON.parse(env("PRIVY_WALLETS", "{}")) as Record<string, string>;
  return privySigner({
    privy: new PrivyClient({ appId: env("PRIVY_APP_ID"), appSecret: env("PRIVY_APP_SECRET") }),
    walletIdFor: async (wallet) => wallets[wallet] ?? null,
    authorizationPrivateKey: env("PRIVY_AUTHORIZATION_KEY"),
  });
}

async function main() {
  const client = createProtocolClient(protocol, { auth: { secretKey: env("ARETE_SECRET_KEY") } });
  // In dev mode the client learns the environment's deployment first; the
  // dev helpers and the policy name the deployment the jobs will target.
  await client.resolveDeployment();
  const dev = clientCluster(client) === "dev";
  const privy = await privyFromEnv();

  const devWallets = dev ? devWalletsSigner(client) : null;
  let store: AutomationStore;
  if (dev) {
    store = fileMemoryStore(client, ".arete/dev/app-store.bin");
  } else {
    const { default: pg } = await import("pg");
    const pool = new pg.Pool({ connectionString: env("DATABASE_URL") });
    const postgres = postgresStore(pool);
    if (process.argv.includes("--migrate")) {
      console.log(await postgres.migrate());
      await pool.end();
      return;
    }
    store = postgres;
  }
  if (!dev && !privy) throw new Error("set PRIVY_APP_ID, PRIVY_APP_SECRET and PRIVY_AUTHORIZATION_KEY: production signs through Privy");

  const automations = createAutomations({
    client,
    signer: devWallets ? routingSigner(client, privy ? [devWallets, privy] : [devWallets]) : privy!,
    policy: definePolicy({ ...definition.policy!, deployments: [client.deploymentId], clusters: [dev ? "dev" : "mainnet"] }),
    simulator: dev ? devSimulator(client) : rpcSimulator(env("RPC_URL")),
    store,
    consent: { domain: env("APP_DOMAIN", "localhost") },
    onOutcome: (outcome) => console.log(`[${outcome.state}] ${outcome.step} for ${outcome.wallet}: ${outcome.code}`),
    onError: (error, context) => console.error(`[${context.phase}]`, error),
  });
  await automations.use(definition);

  // A supporter's recurring tip on one jar: at most `amount` per tip, and
  // 30 tips' worth in 30 days.
  const consentFields = (wallet: string, instanceId: string, amount: string, expiresAt: number) => ({
    wallet,
    deploymentId: client.deploymentId,
    instanceId,
    steps: ["tip"],
    params: { amount },
    limits: {
      perExecution: { [SOL]: String(BigInt(amount) + 10_000n) },
      perWindow: { [SOL]: { amount: String((BigInt(amount) + 10_000n) * 30n), window: "30d" as const } },
    },
    expiresAt,
  });

  const readBody = async (req: IncomingMessage) => {
    const chunks: Buffer[] = [];
    let size = 0;
    for await (const chunk of req) {
      size += (chunk as Buffer).length;
      if (size > 64 * 1024) throw new Error("request body too large");
      chunks.push(chunk as Buffer);
    }
    return Buffer.concat(chunks);
  };
  const json = (res: ServerResponse, status: number, body: unknown) => {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(body, (_key, value) => (typeof value === "bigint" ? value.toString() : value)));
  };
  // The consent routes are for your app's backend, which has authenticated
  // the user and checked they own the wallet; here, behind a shared token.
  const appToken = env("APP_API_TOKEN");
  const fromApp = (req: IncomingMessage) => req.headers.authorization === `Bearer ${appToken}`;

  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? "/", "http://localhost");
      if (req.method === "POST" && url.pathname === "/arete/webhook") {
        const result = await automations.acceptWebhook(await readBody(req), req.headers, { secret: env("ARETE_WEBHOOK_SECRET") });
        return json(res, result.status, result.body);
      }
      if (req.method === "GET" && url.pathname === "/arete/health") return json(res, 200, { ok: true, dev });
      if (!fromApp(req)) return json(res, 401, { error: "unauthorized" });
      const body = req.method === "POST" ? (JSON.parse(String(await readBody(req))) as Record<string, string & number>) : {};
      // 1. The message the user signs to opt in, then 2. their signature.
      // The expiry is part of the signed message: the grant sends back the
      // one the message returned.
      if (req.method === "POST" && url.pathname === "/consent/message") {
        const expiresAt = Number(body.expiresAt ?? Date.now() + 30 * 86_400_000);
        const proof = await automations.consent.proofMessage(consentFields(body.wallet, body.instanceId, body.amount, expiresAt));
        return json(res, 200, { message: proof.message, nonce: proof.nonce, issuedAt: proof.issuedAt, expiresAt });
      }
      if (req.method === "POST" && url.pathname === "/consent/grant") {
        if (body.expiresAt === undefined) return json(res, 400, { error: "send the expiresAt /consent/message returned" });
        const fields = consentFields(body.wallet, body.instanceId, body.amount, Number(body.expiresAt));
        return json(res, 200, await automations.consent.grant({ ...fields, nonce: body.nonce, issuedAt: body.issuedAt, signature: body.signature }));
      }
      if (req.method === "DELETE" && url.pathname.startsWith("/consent/")) {
        return json(res, 200, { revoked: await automations.consent.revoke({ wallet: url.pathname.slice("/consent/".length) }) });
      }
      // Dev only: a dev wallet (name or address) signs its own consent.
      if (devWallets && req.method === "POST" && url.pathname === "/dev/consent") {
        const wallet = devWallets.resolve(body.wallet) ?? body.wallet;
        const expiresAt = Number(body.expiresAt ?? Date.now() + 30 * 86_400_000);
        return json(res, 200, await grantDevConsent(automations, devWallets, consentFields(wallet, body.instanceId, body.amount, expiresAt)));
      }
      return json(res, 404, { error: "not found" });
    } catch (error) {
      return json(res, 400, { error: (error as Error).message });
    }
  });

  automations.start({ concurrency: 2 });
  const port = Number(env("PORT", "8787"));
  server.listen(port, () => console.log(`automations server on :${port} (${dev ? "dev environment" : "mainnet"})`));
}

void main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
