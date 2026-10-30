/**
 * Helpers for running automations in your own server against an `a4 dev`
 * environment, the way `a4 dev up` runs them itself. Use them with
 * `[dev.automations] forward = "<your webhook route>"` in arete.toml, so the
 * environment's events reach your server and the rest of it (consent
 * routes, Privy, your UI) is exercised too. Node.js only: they read the
 * project's `.arete/dev` files.
 *
 * ```ts
 * import { devSimulator, devWalletsSigner, fileMemoryStore, routingSigner } from '@usearete/automations/dev';
 *
 * const automations = createAutomations({
 *   client,
 *   signer: routingSigner(client, [devWalletsSigner(client), privy]),
 *   simulator: devSimulator(client),
 *   store: fileMemoryStore(client, '.arete/dev/app-store.bin'),
 *   policy,
 * });
 * ```
 *
 * Each helper takes the protocol client and throws unless it targets a dev
 * environment, so a production build that still imports one fails at
 * startup rather than signing with plain key files. The environment is read
 * from the client's deployment (`clientCluster`); this is temporary, until
 * the SDK exposes the environment on the client.
 */

import { chmodSync, existsSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, extname, join, resolve } from 'node:path';
import { deserialize, serialize } from 'node:v8';

import type { ProtocolTransactionFormat } from '@usearete/sdk';

import { rpcSimulator, type RpcSimulatorOptions, type Simulator } from './policy/simulator';
import type { ConsentFields } from './consent/consent';
import { type AutomationClient, type ConsentApi, clientCluster } from './runtime/automations';
import type { ProtocolSigner } from './signer/signer';
import { type MemoryStoreState, memoryStore, memoryStoreState } from './store/memory';
import type { AutomationStore } from './store/store';
import { readDevWallets, walletsSigner } from './dev/signer';

/** What the helpers read from the client. */
export type DevClient = Pick<AutomationClient, 'definition' | 'deploymentId'>;

/**
 * Throws unless `client` targets a dev environment. Every helper here calls
 * it first.
 */
export function assertDevClient(client: DevClient, helper: string): void {
  if (clientCluster(client) !== 'dev') {
    throw new Error(
      `${helper} runs against a dev environment only, but the protocol client targets mainnet (deployment ${client.deploymentId}). ` +
        'Use it with the client `a4 dev` generates, and a production signer, simulator and store in production.',
    );
  }
}

/**
 * The project root: the nearest directory from `from` (default the working
 * directory) up that holds `.arete/dev` or `arete.toml`.
 */
export function findProjectRoot(from: string = process.cwd()): string {
  let dir = resolve(from);
  for (;;) {
    if (existsSync(join(dir, '.arete', 'dev')) || existsSync(join(dir, 'arete.toml'))) return dir;
    const parent = dirname(dir);
    if (parent === dir) {
      throw new Error(`no Arete project found from ${resolve(from)} (no arete.toml or .arete/dev); pass the path explicitly`);
    }
    dir = parent;
  }
}

/** A dev wallet signer: a {@link ProtocolSigner} that also signs messages. */
export interface DevWalletsSigner extends ProtocolSigner {
  /** The project's dev wallets, by file name and address, read now. */
  wallets(): { readonly name: string; readonly address: string }[];
  /** The address of a dev wallet given by name or address, or `undefined`. */
  resolve(wallet: string): string | undefined;
  /**
   * Sign `message` as the wallet would (`signMessage` in a wallet app), for
   * example a consent proof. Throws for a wallet that is not a dev wallet.
   */
  signMessage(wallet: string, message: Uint8Array): Uint8Array;
}

/**
 * Signs for every dev wallet of the project (the keypairs `a4 dev wallet`
 * writes to `.arete/dev/wallets/`; `dir` overrides the folder). Wallets
 * created while it runs are picked up at the next signature.
 */
export function devWalletsSigner(client: DevClient, dir?: string): DevWalletsSigner {
  assertDevClient(client, 'devWalletsSigner');
  const walletsDir = dir ?? join(findProjectRoot(), '.arete', 'dev', 'wallets');
  const lookup = (wallet: string) => {
    const wallets = readDevWallets(walletsDir);
    return wallets.byAddress.get(wallet) ?? wallets.byName.get(wallet);
  };
  return {
    ...walletsSigner(walletsDir),
    wallets() {
      return [...readDevWallets(walletsDir).byName].map(([name, signer]) => ({ name, address: signer.publicKey }));
    },
    resolve(wallet) {
      return lookup(wallet)?.publicKey;
    },
    signMessage(wallet, message) {
      const signer = lookup(wallet);
      if (!signer) throw new Error(`${wallet} is not a dev wallet of this project (a4 dev wallet <name> creates one)`);
      return signer.signMessage(message);
    },
  };
}

/**
 * Grant a consent signed by a dev wallet (given by name or address), as the
 * user would sign it in their wallet app: builds the proof message, signs
 * it with the wallet's key file and stores the grant.
 */
export async function grantDevConsent(
  automations: { readonly consent: Pick<ConsentApi, 'proofMessage' | 'grant'> },
  signer: DevWalletsSigner,
  fields: ConsentFields,
): ReturnType<ConsentApi['grant']> {
  const wallet = signer.resolve(fields.wallet);
  if (!wallet) throw new Error(`${fields.wallet} is not a dev wallet of this project (a4 dev wallet <name> creates one)`);
  const resolved = { ...fields, wallet };
  const proof = await automations.consent.proofMessage(resolved);
  return automations.consent.grant({
    ...resolved,
    nonce: proof.nonce,
    issuedAt: proof.issuedAt,
    signature: signer.signMessage(wallet, proof.bytes),
  });
}

/**
 * One signer over several: for each wallet, the first signer whose
 * `canSign` accepts it signs. With `[devWalletsSigner(client), privy]`,
 * dev wallets sign with their key files and every other wallet through
 * Privy. It offers the transaction formats every signer supports.
 */
export function routingSigner(client: DevClient, signers: readonly ProtocolSigner[]): ProtocolSigner {
  assertDevClient(client, 'routingSigner');
  if (signers.length === 0) throw new TypeError('routingSigner needs at least one signer');
  const formats = signers[0]!.supportedTransactionFormats.filter((format: ProtocolTransactionFormat) =>
    signers.every((signer) => signer.supportedTransactionFormats.includes(format)),
  );
  if (formats.length === 0) {
    throw new TypeError(`routingSigner: ${signers.map((signer) => signer.kind).join(', ')} share no transaction format`);
  }
  const route = async (wallet: string) => {
    for (const signer of signers) if (await signer.canSign(wallet)) return signer;
    return undefined;
  };
  return {
    kind: signers.map((signer) => signer.kind).join('|'),
    supportedTransactionFormats: formats,
    async canSign(wallet) {
      return (await route(wallet)) !== undefined;
    },
    async delegationStatus(wallet) {
      const signer = await route(wallet);
      if (!signer) return { delegated: false, providerPolicy: false, detail: 'no signer can sign for this wallet' };
      return signer.delegationStatus ? signer.delegationStatus(wallet) : { delegated: true, providerPolicy: false };
    },
    async signTransactions(input) {
      const signer = await route(input.wallet);
      if (!signer) throw new Error(`no signer can sign for ${input.wallet}`);
      return signer.signTransactions(input);
    },
  };
}

export interface FileMemoryStoreOptions {
  /** How often changes are written, in milliseconds. Defaults to 1000; 0 writes only on `save()`. */
  readonly saveIntervalMs?: number;
  /** Called when the saved state belonged to another deployment and was moved aside. */
  readonly onArchive?: (archivedPath: string, deployment: string) => void;
  /** Called when a periodic save fails (default: logged with `console.error`); the next one retries. */
  readonly onSaveError?: (error: unknown) => void;
}

/** A memory store saved to a file. */
export type FileMemoryStore = AutomationStore & {
  /** Write the state now, if it changed. */
  save(): void;
  /** Stop the periodic save and write the state one last time (call it on shutdown). */
  close(): void;
};

/**
 * A memory store kept in a file (owner-only), so jobs, consent and spending
 * survive restarts of a dev server. State saved for another deployment is
 * moved aside (`<name>-<deployment><ext>`) and the store starts empty. Not
 * for production: use a database store there.
 */
export function fileMemoryStore(client: DevClient, path: string, options: FileMemoryStoreOptions = {}): FileMemoryStore {
  assertDevClient(client, 'fileMemoryStore');
  const file = resolve(path);
  let state: MemoryStoreState | undefined;
  if (existsSync(file)) {
    // It may predate the owner-only mode; it holds consent and jobs.
    chmodSync(file, 0o600);
    state = deserialize(readFileSync(file)) as MemoryStoreState;
    if (state.deployment !== null && state.deployment !== client.deploymentId) {
      const ext = extname(file);
      const archived = join(dirname(file), `${basename(file, ext)}-${state.deployment}${ext}`);
      renameSync(file, archived);
      options.onArchive?.(archived, state.deployment);
      state = undefined;
    }
  }
  const store = memoryStore(state ? { state } : {});
  let saved = state ? serialize(state) : Buffer.alloc(0);
  const save = () => {
    const next = serialize(memoryStoreState(store));
    if (next.equals(saved)) return;
    writeFileSync(`${file}.tmp`, next, { mode: 0o600 });
    chmodSync(`${file}.tmp`, 0o600);
    renameSync(`${file}.tmp`, file);
    saved = next;
  };
  const intervalMs = options.saveIntervalMs ?? 1000;
  // A failing periodic save (a full disk, a removed folder) must not stop
  // the server: it is reported, and the next tick retries.
  const onSaveError = options.onSaveError ?? ((error: unknown) => console.error(`fileMemoryStore: could not save ${file}:`, error));
  const timer =
    intervalMs > 0
      ? setInterval(() => {
          try {
            save();
          } catch (error) {
            onSaveError(error);
          }
        }, intervalMs)
      : undefined;
  timer?.unref();
  return Object.assign(store, {
    save,
    close() {
      if (timer) clearInterval(timer);
      save();
    },
  });
}

/** What `a4 dev up` records about the running dev environment. */
export interface DevSession {
  readonly rpcUrl: string;
  readonly deploymentId?: string | null;
}

/** `.arete/dev/session.json` of the project at `root` (default: found from the working directory). */
export function readDevSession(root?: string): DevSession {
  const path = join(root ?? findProjectRoot(), '.arete', 'dev', 'session.json');
  if (!existsSync(path) || !statSync(path).isFile()) {
    throw new Error(`${path} does not exist: start the dev environment with \`a4 dev up\``);
  }
  return JSON.parse(readFileSync(path, 'utf8')) as DevSession;
}

export interface DevSimulatorOptions extends RpcSimulatorOptions {
  /** The project root. Defaults to the one found from the working directory. */
  readonly root?: string;
}

/**
 * Simulates on the dev environment's fork, through `a4 dev up`'s local RPC.
 * The RPC is read from `.arete/dev/session.json` at each call, so it follows
 * `a4 dev up` restarts. (Temporary: once the SDK exposes the dev
 * environment, it is taken from the client.)
 */
export function devSimulator(client: DevClient, options: DevSimulatorOptions = {}): Simulator {
  assertDevClient(client, 'devSimulator');
  const root = options.root ?? findProjectRoot();
  return {
    async call(method, params) {
      const session = readDevSession(root);
      if (session.deploymentId && session.deploymentId !== client.deploymentId) {
        throw new Error(
          `the dev environment runs deployment ${session.deploymentId}, but the client names ${client.deploymentId}; run \`a4 dev sdk\` to regenerate it`,
        );
      }
      return rpcSimulator(session.rpcUrl, options).call(method, params);
    },
  };
}
