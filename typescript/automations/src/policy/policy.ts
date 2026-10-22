/**
 * What a signer may sign for a user: the static policy.
 *
 * The rules, their names and the refusal shape match the Arete MCP server's
 * local signer, and both implementations run the shared vectors in
 * `test-vectors/signer-policy/`. The caps hold per transaction, measured by
 * simulating the exact prepared bytes. Wrapped SOL counts against
 * `maxSolPerTransaction`, as do the lamports of a token account closed to
 * someone else. Instructions that hand control of the signer's funds to
 * someone else (token approvals, authority changes, System assignments) are
 * always refused, and so is any transaction whose effect on the signer
 * could not be simulated.
 */

import { lamportsToSol, plainDecimal } from './amounts';
import { isNativeMint } from './authority';
import { asString, field, isObject, isPubkey } from './encoding';
import type { PreparedSummary } from './summary';

/** Clusters a policy allows when it does not list any. */
export const DEFAULT_CLUSTERS = ['localnet', 'devnet', 'testnet'] as const;
/** Every cluster name a policy may list. */
export const KNOWN_CLUSTERS = ['localnet', 'devnet', 'testnet', 'mainnet'] as const;
export type Cluster = (typeof KNOWN_CLUSTERS)[number];

/**
 * Programs the transaction compiler adds around a step's own instructions
 * (compute budget, memo, fee transfers, token accounts and wrapped SOL).
 * They are always allowed; what they move is bounded by the SOL and token
 * caps.
 */
export const RUNTIME_PROGRAMS = [
  'ComputeBudget111111111111111111111111111111',
  'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr',
  'Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo',
  '11111111111111111111111111111111',
  'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL',
  'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb',
] as const;

const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;

/** The fields of a policy, as an integrator writes them. */
export interface PolicyInput {
  /** Deployments the signer may act in. Required, at least one. */
  readonly deployments: readonly string[];
  /** The most SOL one transaction may move out of the signer, in SOL. */
  readonly maxSolPerTransaction: string | number;
  /** Programs a transaction may invoke besides the runtime programs. Omit to allow any. */
  readonly programs?: readonly string[] | null;
  /**
   * Base units per mint that one transaction may move out of the signer.
   * A mint without a cap may not leave at all, so without this table no
   * token may leave (wrapped SOL counts as SOL instead).
   */
  readonly maxTokenAmounts?: Readonly<Record<string, string | number | bigint>> | null;
  /** Defaults to localnet, devnet and testnet; mainnet only when listed. */
  readonly clusters?: readonly Cluster[] | null;
}

/** A validated policy. */
export interface SignerPolicy {
  readonly deployments: readonly string[];
  readonly maxLamportsPerTransaction: bigint;
  readonly programs: readonly string[] | null;
  readonly maxTokenAmounts: ReadonlyMap<string, bigint> | null;
  readonly clusters: readonly string[];
  /** Whether the input listed `clusters` (rather than the default). */
  readonly clustersListed: boolean;
}

/**
 * Why a step was not signed. The first six are the signing policy's rules;
 * the rest are the runtime's checks around the signer.
 */
export type RefusalRule =
  | 'deployments'
  | 'clusters'
  | 'programs'
  | 'authority'
  | 'maxSolPerTransaction'
  | 'maxTokenAmounts'
  /** The prepared transactions are not what the intent committed to. */
  | 'intent_mismatch'
  /** The signer and the deployment share no transaction format, or the prepare used another. */
  | 'transaction_format'
  /** The signer changed a message, another signature, or the signers. */
  | 'signer_modified'
  /** The wallet's signature slot does not hold its valid signature. */
  | 'signer_signature'
  /** The signer cannot sign for the wallet (no key, no session signer). */
  | 'signer_unavailable'
  /** The wallet provider's own policy refused to sign. */
  | 'provider_policy';

/** A policy refusal: the rule and why. */
export interface Refusal {
  readonly rule: RefusalRule;
  readonly reason: string;
}

export class PolicyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'PolicyError';
  }
}

const POLICY_FIELDS = new Set([
  'deployments',
  'maxSolPerTransaction',
  'programs',
  'maxTokenAmounts',
  'clusters',
]);

/** Parse SOL (a number or decimal string) into lamports. */
function solToLamports(value: unknown): bigint {
  let text: string;
  if (typeof value === 'string') text = value.trim();
  else if (typeof value === 'number' && Number.isFinite(value)) text = plainDecimal(value);
  else throw new PolicyError('maxSolPerTransaction must be a number of SOL');
  const point = text.indexOf('.');
  const whole = point < 0 ? text : text.slice(0, point);
  const fraction = point < 0 ? '' : text.slice(point + 1);
  if (
    (whole === '' && fraction === '') ||
    !/^\d*$/.test(whole) ||
    !/^\d*$/.test(fraction) ||
    fraction.length > 9
  ) {
    throw new PolicyError(
      'maxSolPerTransaction must be a non-negative number of SOL with at most 9 decimals',
    );
  }
  const wholeValue = whole === '' ? 0n : BigInt(whole);
  const lamports = wholeValue * 1_000_000_000n + BigInt(fraction.padEnd(9, '0'));
  if (wholeValue > U64_MAX || lamports > U64_MAX) {
    throw new PolicyError('maxSolPerTransaction is too large');
  }
  return lamports;
}

function tokenAmount(value: unknown): bigint {
  let amount: bigint | undefined;
  if (typeof value === 'bigint' && value >= 0n) amount = value;
  else if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) {
    amount = BigInt(value);
  } else if (typeof value === 'string' && /^\d+$/.test(value)) amount = BigInt(value);
  if (amount === undefined || amount > U128_MAX) {
    throw new PolicyError('maxTokenAmounts values must be non-negative integers in base units');
  }
  return amount;
}

function stringList(value: unknown, name: string): string[] {
  if (!Array.isArray(value) || !value.every((item) => typeof item === 'string')) {
    throw new PolicyError(`policy.${name} must be a list of strings`);
  }
  return [...value];
}

/** Validate a policy. Throws {@link PolicyError} naming the field at fault. */
export function definePolicy(input: PolicyInput): SignerPolicy {
  if (!isObject(input)) throw new PolicyError('a policy must be an object');
  const unknown = Object.keys(input).filter((key) => !POLICY_FIELDS.has(key));
  if (unknown.length > 0) throw new PolicyError(`policy has unknown field '${unknown[0]}'`);
  if (input.deployments === undefined) throw new PolicyError('policy.deployments is required');
  const deployments = stringList(input.deployments, 'deployments');
  if (input.maxSolPerTransaction === undefined) {
    throw new PolicyError('policy.maxSolPerTransaction is required');
  }
  if (deployments.length === 0) {
    throw new PolicyError('policy.deployments must list at least one deployment id');
  }
  const maxLamportsPerTransaction = solToLamports(input.maxSolPerTransaction);
  let programs: string[] | null = null;
  if (input.programs !== undefined && input.programs !== null) {
    programs = stringList(input.programs, 'programs');
    const bad = programs.find((program) => !isPubkey(program));
    if (bad !== undefined) throw new PolicyError(`policy.programs: '${bad}' is not a program id`);
  }
  let maxTokenAmounts: Map<string, bigint> | null = null;
  if (input.maxTokenAmounts !== undefined && input.maxTokenAmounts !== null) {
    if (!isObject(input.maxTokenAmounts)) {
      throw new PolicyError('policy.maxTokenAmounts must map mints to amounts');
    }
    maxTokenAmounts = new Map();
    for (const mint of Object.keys(input.maxTokenAmounts).sort()) {
      if (!isPubkey(mint)) {
        throw new PolicyError(`policy.maxTokenAmounts: '${mint}' is not a mint address`);
      }
      maxTokenAmounts.set(mint, tokenAmount(input.maxTokenAmounts[mint]));
    }
  }
  const clustersListed = input.clusters !== undefined && input.clusters !== null;
  let clusters: string[] = [...DEFAULT_CLUSTERS];
  if (clustersListed) {
    clusters = stringList(input.clusters, 'clusters');
    const bad = clusters.find((cluster) => !(KNOWN_CLUSTERS as readonly string[]).includes(cluster));
    if (bad !== undefined) {
      throw new PolicyError(`policy.clusters: '${bad}' is not one of ${KNOWN_CLUSTERS.join(', ')}`);
    }
  }
  return { deployments, maxLamportsPerTransaction, programs, maxTokenAmounts, clusters, clustersListed };
}

/** Whether the deployment and cluster are allowed, before preparing. `null` when they are. */
export function checkTarget(
  policy: SignerPolicy,
  deploymentId: string,
  cluster: string,
): Refusal | null {
  if (!policy.deployments.includes(deploymentId)) {
    return {
      rule: 'deployments',
      reason: `deployment ${deploymentId} is not in the signer's deployments allow-list`,
    };
  }
  if (!policy.clusters.includes(cluster)) {
    return {
      rule: 'clusters',
      reason: `the target cluster is ${cluster}, which the signer's clusters (${policy.clusters.join(', ')}) do not include${cluster === 'mainnet' ? '; mainnet must be listed explicitly' : ''}`,
    };
  }
  return null;
}

export interface CheckPreparedInput {
  /** The deployment the step was prepared in. */
  readonly deploymentId: string;
  /** The prepare result; its intent's deployment must match. */
  readonly prepared: unknown;
  /** The summary of the prepared step for the signer. */
  readonly summary: PreparedSummary;
  /** Whether the summary came from a simulation at all. */
  readonly simulates: boolean;
}

function parseBigInt(value: unknown): bigint | null {
  if (typeof value !== 'string' || !/^-?\d+$/.test(value)) return null;
  return BigInt(value);
}

/** How a JSON value prints, the way the reference implementation formats it in reasons. */
function display(value: unknown): string {
  return value === undefined ? 'null' : JSON.stringify(value);
}

/**
 * Check a prepared step against the policy: the intent's deployment, every
 * transaction's programs, the instructions that would hand over control of
 * the signer's funds, and the simulated SOL and token outflows of the
 * signer, per transaction. `null` when it may be signed.
 */
export function checkPrepared(policy: SignerPolicy, input: CheckPreparedInput): Refusal | null {
  const { deploymentId, prepared, summary, simulates } = input;
  const intentDeployment =
    asString(field(field(field(prepared, 'intent'), 'deployment'), 'deploymentId')) ??
    deploymentId;
  if (intentDeployment !== deploymentId || !policy.deployments.includes(intentDeployment)) {
    return {
      rule: 'deployments',
      reason: `the prepared intent is for deployment ${intentDeployment}, which is not allowed`,
    };
  }
  for (const transaction of summary.transactions) {
    const index = display(transaction.index);
    if (policy.programs !== null) {
      for (const program of transaction.programs) {
        const id = program.programId;
        if (!(RUNTIME_PROGRAMS as readonly string[]).includes(id) && !policy.programs.includes(id)) {
          return {
            rule: 'programs',
            reason: `transaction ${index} invokes ${id}, which is not in the signer's programs allow-list`,
          };
        }
      }
    }
    const finding = transaction.unsafeInstructions[0];
    if (finding !== undefined) {
      return {
        rule: 'authority',
        reason: `transaction ${index} has ${finding.inner ? 'an inner' : 'a'} \`${finding.instruction}\` instruction of ${finding.programId} that ${finding.reason}; the signer never signs those`,
      };
    }
    const effect = transaction.signerEffect;
    if (effect.known !== true) {
      let why = '';
      if ((effect.omittedAccounts ?? 0) > 0) {
        why = ' (it writes more accounts than the simulation reports on)';
      } else if (
        (effect.closedAccountLamportsOut ?? null) === null &&
        typeof effect.solLamports === 'string'
      ) {
        why = ' (it closes a token account the simulation could not measure)';
      } else if (!simulates) {
        why = ' (no simulator is configured)';
      }
      return {
        rule: 'maxSolPerTransaction',
        reason: `the signer's balance change in transaction ${index} could not be simulated${why}, so the caps cannot be checked`,
      };
    }
    const lamports = parseBigInt(effect.solLamports) ?? -(1n << 126n);
    const tokens = effect.tokens ?? [];
    // Wrapped SOL is SOL: its delta joins the lamports, so wrapping is not
    // an outflow and spending wrapped SOL is.
    const wrapped = tokens
      .filter((token) => isNativeMint(token.mint))
      .reduce((sum, token) => sum + (parseBigInt(token.amount) ?? 0n), 0n);
    const closed = parseBigInt(effect.closedAccountLamportsOut) ?? 0n;
    let outflow = closed - lamports - wrapped;
    if (outflow < 0n) outflow = 0n;
    if (outflow > policy.maxLamportsPerTransaction) {
      return {
        rule: 'maxSolPerTransaction',
        reason: `transaction ${index} moves ${lamportsToSol(outflow)} SOL out of the signer (wrapped SOL and closed token accounts included), over the ${lamportsToSol(policy.maxLamportsPerTransaction)} SOL cap`,
      };
    }
    for (const token of tokens) {
      if (isNativeMint(token.mint)) continue;
      const amount = parseBigInt(token.amount) ?? 0n;
      if (amount >= 0n) continue;
      const tokenOutflow = -amount;
      const mint = token.mint;
      if (policy.maxTokenAmounts === null) {
        return {
          rule: 'maxTokenAmounts',
          reason: `transaction ${index} moves ${tokenOutflow} base units of ${mint} out of the signer, and the policy sets no maxTokenAmounts, so no token may leave`,
        };
      }
      const cap = policy.maxTokenAmounts.get(mint);
      if (cap === undefined) {
        return {
          rule: 'maxTokenAmounts',
          reason: `transaction ${index} moves ${tokenOutflow} base units of ${mint} out of the signer, and that mint has no cap`,
        };
      }
      if (tokenOutflow > cap) {
        return {
          rule: 'maxTokenAmounts',
          reason: `transaction ${index} moves ${tokenOutflow} base units of ${mint} out of the signer, over the cap of ${cap}`,
        };
      }
    }
  }
  return null;
}
