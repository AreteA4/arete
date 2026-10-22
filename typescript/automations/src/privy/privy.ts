/**
 * A signer over Privy server wallets, and the Privy policy that bounds it.
 *
 * The signer asks Privy to sign (never to sign and send) each prepared
 * transaction, authorized by your app's authorization key. The runtime
 * checks the returned bytes against the intent, as for any signer.
 *
 * The provider policy is the bound that holds if the backend running
 * automations is compromised, because Privy enforces it in its enclave.
 * Privy's Solana policies see top-level instructions only, so they bound
 * which programs a wallet may call and when, and the amounts of direct
 * transfers, but not what a protocol program moves by CPI.
 * `privyPolicyFor` returns the rules it could not express alongside the
 * policy; see docs/privy-policy-expressiveness.md.
 */

import type { ProtocolTransactionFormat } from '@usearete/sdk';

import type { NormalizedConsent } from '../consent/consent';
import { SOL } from '../consent/consent';
import { instructionName, NATIVE_MINTS, SYSTEM_PROGRAM, TOKEN_PROGRAMS } from '../policy/authority';
import { decodeTransaction } from '../policy/decode';
import { decodeBase64 } from '../policy/encoding';
import { RUNTIME_PROGRAMS, type SignerPolicy } from '../policy/policy';
import type { DelegationStatus, ProtocolSigner, SignTransactionsInput } from '../signer/signer';
import { SignerRefusal } from '../signer/signer';
import type { ConsentGrant } from '../store/store';

/** The parts of a `@privy-io/node` `PrivyClient` the signer uses. */
export interface PrivyClientLike {
  wallets(): {
    solana(): {
      signTransaction(
        walletId: string,
        input: {
          transaction: string;
          authorization_context?: { authorization_private_keys: string[] };
        },
      ): Promise<{ signedTransaction?: string; signed_transaction?: string }>;
    };
    get?(walletId: string): Promise<{ policy_ids?: readonly string[] | null }>;
  };
}

export interface PrivySignerOptions {
  readonly privy: PrivyClientLike;
  /** The Privy wallet id of a wallet address, or `null` when your app has no server access to it. */
  readonly walletIdFor: (wallet: string) => Promise<string | null>;
  /** Your app's authorization private key (the key quorum Privy lets sign for the wallet). */
  readonly authorizationPrivateKey: string;
  /**
   * Formats to offer. Defaults to v0 only: Privy's documentation covers
   * legacy and v0 transactions, and v1 message signing is not verified.
   */
  readonly transactionFormats?: readonly ProtocolTransactionFormat[];
  /**
   * Refuse, before asking Privy, transactions with top-level System or Token
   * instructions that `privyPolicyFor` cannot allow (see
   * {@link PRIVY_ALLOWED_INSTRUCTIONS}), with a reason that says so. On by
   * default; turn it off if your Privy policy allows more.
   */
  readonly refuseUnsupported?: boolean;
}

/**
 * The System and Token instructions a policy from `privyPolicyFor` allows.
 * Privy's Solana policies cannot express the others (for example Token
 * `syncNative`, which wrapping SOL needs), so its default deny refuses them.
 */
export const PRIVY_ALLOWED_INSTRUCTIONS: Readonly<Record<'system' | 'token', readonly string[]>> = {
  system: ['transfer'],
  token: ['transferChecked', 'closeAccount', 'initializeAccount3'],
};

/** The first top-level instruction a `privyPolicyFor` policy would refuse, or `null`. */
function unsupportedInstruction(transactionBase64: string): string | null {
  const bytes = decodeBase64(transactionBase64);
  if (!bytes) return null;
  const decoded = decodeTransaction(bytes);
  for (const instruction of decoded.instructions) {
    const program = decoded.staticKeys[instruction.programIndex];
    if (program === undefined) continue;
    const isSystem = program === SYSTEM_PROGRAM;
    const isToken = (TOKEN_PROGRAMS as readonly string[]).includes(program);
    if (!isSystem && !isToken) continue;
    const name = instructionName(program, instruction.data) ?? 'unknown';
    const allowed = PRIVY_ALLOWED_INSTRUCTIONS[isSystem ? 'system' : 'token'];
    if (!allowed.includes(name)) return `${isSystem ? 'System' : 'Token'} \`${name}\``;
  }
  return null;
}

/** Whether an error from Privy is its policy refusing, rather than a fault to retry. */
function isPolicyRefusal(error: unknown): boolean {
  const status = (error as { status?: unknown } | null)?.status;
  const message = String((error as { message?: unknown } | null)?.message ?? '');
  return status === 403 || /polic(y|ies)/i.test(message);
}

/** A {@link ProtocolSigner} over Privy server wallets. */
export function privySigner(options: PrivySignerOptions): ProtocolSigner {
  const { privy, walletIdFor, authorizationPrivateKey } = options;
  if (!authorizationPrivateKey) throw new TypeError('privySigner needs an authorization private key');
  const walletId = async (wallet: string) => {
    const id = await walletIdFor(wallet);
    if (!id) throw new SignerRefusal(`no Privy server access to ${wallet}`, 'signer_unavailable');
    return id;
  };
  return {
    kind: 'privy',
    supportedTransactionFormats: options.transactionFormats ?? ['v0'],
    async canSign(wallet) {
      return (await walletIdFor(wallet)) !== null;
    },
    async delegationStatus(wallet): Promise<DelegationStatus> {
      const id = await walletIdFor(wallet);
      if (!id) return { delegated: false, providerPolicy: false, detail: 'no Privy wallet with server access' };
      const get = privy.wallets().get;
      if (!get) return { delegated: true, providerPolicy: false, detail: 'the Privy client cannot read wallet policies' };
      const details = await get.call(privy.wallets(), id);
      return { delegated: true, providerPolicy: (details.policy_ids?.length ?? 0) > 0 };
    },
    async signTransactions({ wallet, transactions }: SignTransactionsInput) {
      if (options.refuseUnsupported !== false) {
        for (const transaction of transactions) {
          const unsupported = unsupportedInstruction(transaction.transactionBase64);
          if (unsupported) {
            throw new SignerRefusal(
              `transaction ${transaction.index} has a top-level ${unsupported} instruction, which Privy policies cannot allow; this step cannot be signed through Privy`,
            );
          }
        }
      }
      const id = await walletId(wallet);
      const signed: string[] = [];
      for (const transaction of transactions) {
        let result: { signedTransaction?: string; signed_transaction?: string };
        try {
          result = await privy.wallets().solana().signTransaction(id, {
            transaction: transaction.transactionBase64,
            authorization_context: { authorization_private_keys: [authorizationPrivateKey] },
          });
        } catch (error) {
          if (isPolicyRefusal(error)) {
            throw new SignerRefusal(`Privy refused to sign transaction ${transaction.index}: ${String((error as Error).message)}`);
          }
          throw error;
        }
        const bytes = result.signedTransaction ?? result.signed_transaction;
        if (typeof bytes !== 'string') throw new Error('Privy returned no signed transaction');
        signed.push(bytes);
      }
      return signed;
    },
  };
}

// ---------------------------------------------------------------------------
// Provider policy
// ---------------------------------------------------------------------------

export interface PrivyPolicyCondition {
  readonly field_source: string;
  readonly field: string;
  readonly operator: 'eq' | 'neq' | 'lt' | 'lte' | 'gt' | 'gte' | 'in';
  readonly value: string | readonly string[];
}

export interface PrivyPolicyRule {
  readonly name: string;
  readonly method: string;
  readonly conditions: readonly PrivyPolicyCondition[];
  readonly action: 'ALLOW' | 'DENY';
}

export interface PrivyPolicy {
  readonly version: '1.0';
  readonly name: string;
  readonly chain_type: 'solana';
  readonly rules: readonly PrivyPolicyRule[];
}

const CHECKED_BY_INSTRUCTION = new Set<string>([SYSTEM_PROGRAM, ...TOKEN_PROGRAMS]);
/** The most values Privy's `in` operator accepts. */
const PRIVY_IN_LIMIT = 100;

function minimum(...values: Array<bigint | undefined>): bigint | undefined {
  return values.reduce<bigint | undefined>((low, value) => (value === undefined ? low : low === undefined || value < low ? value : low), undefined);
}

/**
 * The Privy policy for a wallet's consent under a signing policy, and the
 * rules Privy cannot enforce (which the runtime still enforces, but which a
 * compromised backend could skip).
 *
 * Every rule is for `signTransaction` only and holds until the consent
 * expires. Programs from `policy.programs` (plus compute budget, memo and
 * associated token account programs) are allowed. System transfers and
 * token `TransferChecked` are allowed up to the smaller of the policy's and
 * the consent's per-run caps, per instruction; token accounts may be
 * closed only to the wallet itself. Everything else is denied, including
 * key export.
 */
export function privyPolicyFor(
  policy: SignerPolicy,
  consent: Pick<NormalizedConsent | ConsentGrant, 'wallet' | 'expiresAt' | 'limits'>,
  options: { readonly name?: string } = {},
): { policy: PrivyPolicy; unenforced: string[] } {
  if (policy.programs === null) {
    throw new TypeError(
      'privyPolicyFor needs a programs allow-list: without one, Privy would have to allow every program',
    );
  }
  const expires = String(Math.floor(consent.expiresAt / 1000));
  const until: PrivyPolicyCondition = {
    field_source: 'system',
    field: 'current_unix_timestamp',
    operator: 'lt',
    value: expires,
  };
  const rule = (name: string, conditions: PrivyPolicyCondition[]): PrivyPolicyRule => ({
    name,
    method: 'signTransaction',
    conditions: [...conditions, until],
    action: 'ALLOW',
  });
  const programs = [...new Set([...RUNTIME_PROGRAMS, ...policy.programs])].filter((program) => !CHECKED_BY_INSTRUCTION.has(program));
  const perRun = consent.limits.perExecution;
  // A consent without a SOL limit lets no SOL leave, as the consent check reads it.
  const solCap = minimum(policy.maxLamportsPerTransaction, perRun[SOL] === undefined ? 0n : BigInt(perRun[SOL]));
  const rules: PrivyPolicyRule[] = [
    { name: 'Never export the key', method: 'exportPrivateKey', conditions: [], action: 'DENY' },
    // Privy's `in` takes at most 100 values: one rule per hundred programs.
    ...Array.from({ length: Math.ceil(programs.length / PRIVY_IN_LIMIT) }, (_, chunk) =>
      rule(chunk === 0 ? 'Programs the steps may call' : `Programs the steps may call (${chunk + 1})`, [
        {
          field_source: 'solana_program_instruction',
          field: 'programId',
          operator: 'in',
          value: programs.slice(chunk * PRIVY_IN_LIMIT, (chunk + 1) * PRIVY_IN_LIMIT),
        },
      ]),
    ),
    rule('SOL transfers up to the per-run cap', [
      { field_source: 'solana_system_program_instruction', field: 'instructionName', operator: 'eq', value: 'Transfer' },
      { field_source: 'solana_system_program_instruction', field: 'Transfer.lamports', operator: 'lte', value: String(solCap ?? 0n) },
    ]),
    rule('Token accounts closed back to the wallet', [
      { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'CloseAccount' },
      { field_source: 'solana_token_program_instruction', field: 'CloseAccount.destination', operator: 'eq', value: consent.wallet },
    ]),
    rule('Token accounts opened for the wallet', [
      { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'InitializeAccount3' },
    ]),
  ];
  const mints = new Set([...(policy.maxTokenAmounts?.keys() ?? []), ...Object.keys(perRun).filter((key) => key !== SOL)]);
  for (const mint of [...mints].sort()) {
    if ((NATIVE_MINTS as readonly string[]).includes(mint)) continue;
    const cap = minimum(policy.maxTokenAmounts?.get(mint), perRun[mint] === undefined ? undefined : BigInt(perRun[mint]));
    // A mint missing from either side may not leave: the stricter side wins.
    if (cap === undefined || !policy.maxTokenAmounts?.has(mint) || perRun[mint] === undefined) continue;
    rules.push(
      rule(`Transfers of ${mint} up to the per-run cap`, [
        { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'TransferChecked' },
        { field_source: 'solana_token_program_instruction', field: 'TransferChecked.mint', operator: 'eq', value: mint },
        { field_source: 'solana_token_program_instruction', field: 'TransferChecked.amount', operator: 'lte', value: cap.toString() },
      ]),
    );
  }
  const unenforced = [
    'deployments and clusters: Privy does not know which deployment or cluster a transaction is for',
    'totals: the transfer caps apply to each instruction, so several capped transfers in one transaction or run can add up to more than maxSolPerTransaction, maxTokenAmounts or the consent per-run limits',
    'amounts a protocol program moves by CPI: Privy checks top-level instructions only, so maxSolPerTransaction, maxTokenAmounts and the consent per-run limits bind direct transfers only',
    'authority refusals in inner instructions (approvals or authority changes a program makes by CPI)',
    'transactions whose effect cannot be simulated: Privy policies do not read simulations',
    'consent per-window limits: Privy has no cumulative limits on Solana',
    'wrapped SOL spent, and lamports of token accounts closed by CPI',
    'steps with other top-level System or Token instructions (for example creating an account or syncNative to wrap SOL): Privy refuses them, and privySigner refuses them before asking',
  ];
  return {
    policy: {
      version: '1.0',
      name: options.name ?? `Arete automations for ${consent.wallet}`.slice(0, 50),
      chain_type: 'solana',
      rules,
    },
    unenforced,
  };
}
