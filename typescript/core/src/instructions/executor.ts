import type {
  WalletAdapter,
  BuiltInstruction,
  BuiltAccountMeta,
  SendOptions,
  ConfirmationLevel,
} from '../wallet/types';
import {
  resolveAccounts,
  validateAccountResolution,
  type AccountMeta,
  type AccountResolutionOptions,
  type ResolvedAccount,
} from './account-resolver';
import { serializeInstructionData, type ArgSchema } from './serializer';
import {
  TransactionExecutionError,
  normalizeTransactionError,
  type ErrorMetadata,
} from './error-parser';

/**
 * Resolved accounts map passed to a handler's build function.
 * Keys are account names, values are base58 addresses.
 */
export type ResolvedAccounts = Record<string, string>;

// Re-export the boundary instruction type for convenience.
export type { BuiltInstruction } from '../wallet/types';

/**
 * Instruction handler consumed by the core executor.
 *
 * Handlers are normally produced by {@link createInstructionHandler} (either
 * hand-written or code-generated), so `build` is implemented generically and
 * callers never deal with serialization directly.
 *
 * The phantom `_params` / `_error` fields carry compile-time type information
 * for the typed client surface; they are never populated at runtime.
 */
export interface InstructionHandler<
  TParams = Record<string, unknown>,
  TError = unknown,
> {
  /** Program ID for this instruction (base58). Used for PDA derivation. */
  programId?: string;
  /** Ordered account metadata used by the core SDK for resolution. */
  accounts: AccountMeta[];
  /** Error definitions used for error parsing. */
  errors: ErrorMetadata[];
  /**
   * Names of the instruction's serialized arguments. Everything in the merged
   * params object that is NOT in this list is treated as a user-provided
   * account address override.
   */
  argNames: string[];
  /**
   * Build the instruction from already-resolved, ordered accounts.
   * Implemented by {@link createInstructionHandler}.
   */
  build(args: Record<string, unknown>, resolved: ResolvedAccount[]): BuiltInstruction;
  /** Phantom: merged params type (args + user-provided accounts). */
  readonly _params?: TParams;
  /** Phantom: typed error union. */
  readonly _error?: TError;
}

/**
 * Configuration accepted by {@link createInstructionHandler}.
 */
export interface InstructionHandlerConfig {
  /** Program ID (base58). */
  programId: string;
  /** Instruction discriminator bytes (8 for Anchor, 1 for Steel, etc.). */
  discriminator: Uint8Array | number[];
  /** Ordered account metadata. */
  accounts: AccountMeta[];
  /** Ordered argument schema for Borsh serialization. */
  args: ArgSchema[];
  /** Error definitions from the IDL. */
  errors?: ErrorMetadata[];
}

/**
 * Creates a data-driven instruction handler.
 *
 * The returned handler implements `build()` generically: it serializes args
 * via the schema-driven serializer and constructs the account key list from
 * the resolved, ordered accounts. No imperative per-instruction code is
 * required, which keeps generated SDKs tiny and puts all serialization logic
 * in one tested place.
 */
export function createInstructionHandler<
  TParams = Record<string, unknown>,
  TError = unknown,
>(config: InstructionHandlerConfig): InstructionHandler<TParams, TError> {
  const discriminator =
    config.discriminator instanceof Uint8Array
      ? config.discriminator
      : Uint8Array.from(config.discriminator);
  const argNames = config.args.map((a) => a.name);
  const errors = config.errors ?? [];

  return {
    programId: config.programId,
    accounts: config.accounts,
    errors,
    argNames,
    build(args: Record<string, unknown>, resolved: ResolvedAccount[]): BuiltInstruction {
      const data = serializeInstructionData(discriminator, args, config.args);
      return {
        programId: config.programId,
        keys: resolved.map((r) => ({
          pubkey: r.address,
          isSigner: r.isSigner,
          isWritable: r.isWritable,
        })),
        data,
      };
    },
  };
}

/**
 * Options for building an instruction (no network access).
 */
export interface BuildOptions {
  /** Wallet, used only for accounts explicitly marked with signerKind: 'wallet'. */
  wallet?: WalletAdapter;
  /** Explicit account-address overrides, including signer slots when needed. */
  accounts?: Record<string, string>;
  /**
   * Extra account metas appended after the instruction's declared accounts
   * (Anchor's `remainingAccounts`) — for routers, transfer hooks, and other
   * composition patterns the IDL cannot express.
   */
  remainingAccounts?: BuiltAccountMeta[];
}

/**
 * Options for executing (building + sending) an instruction.
 */
export interface ExecuteOptions extends BuildOptions {
  /** Wallet adapter that signs and broadcasts the transaction. */
  wallet?: WalletAdapter;
  /** Confirmation level forwarded to the adapter. */
  confirmationLevel?: ConfirmationLevel;
  /** Additional options forwarded verbatim to the wallet adapter. */
  send?: SendOptions;
}

/**
 * Result of a successful instruction execution.
 */
export interface ExecutionResult {
  /** Transaction signature. */
  signature: string;
  /** Slot in which the transaction landed, if the adapter reports it. */
  slot?: number;
}

/**
 * Splits a merged params object into serialized args and account overrides.
 *
 * Keys matching a declared argument name are args; keys matching a declared
 * account name (with a string value) are account address overrides. This
 * applies to signer slots too, allowing explicit signer addresses to fill or
 * override those slots. Anything else throws — a typo'd key silently dropped
 * here would otherwise change the built instruction. `options.accounts` on
 * {@link BuildOptions} is the explicit override map and wins over merged params.
 */
function splitParams(
  handler: InstructionHandler<any, any>,
  params: Record<string, unknown>
): {
  args: Record<string, unknown>;
  accountOverrides: Record<string, string>;
  resolve?: Record<string, unknown>;
} {
  const argNameSet = new Set(handler.argNames);
  const accountNameSet = new Set(handler.accounts.map((a) => a.name));
  const args: Record<string, unknown> = {};
  const accountOverrides: Record<string, string> = {};
  let resolve: Record<string, unknown> | undefined;

  for (const [key, value] of Object.entries(params)) {
    if (argNameSet.has(key)) {
      args[key] = value;
    } else if (key === 'resolve' && !accountNameSet.has(key)) {
      if (value === undefined) {
        continue;
      }
      if (value === null || Array.isArray(value) || typeof value !== 'object') {
        throw new Error('Parameter "resolve" must be an object when provided');
      }
      resolve = value as Record<string, unknown>;
    } else if (accountNameSet.has(key)) {
      if (typeof value !== 'string') {
        // Non-string values are not valid account addresses.
        throw new Error(
          `Parameter "${key}" is not a known argument and is not a base58 account address`
        );
      }
      accountOverrides[key] = value;
    } else {
      throw new Error(
        `Unknown parameter "${key}". Expected one of args [${[...argNameSet].join(', ')}] ` +
          `or accounts [${[...accountNameSet].join(', ')}]`
      );
    }
  }

  return { args, accountOverrides, resolve };
}

/**
 * Builds a {@link BuiltInstruction} from a handler and a merged params object.
 *
 * This is a pure function: it performs no network access. It is the unit of
 * composition for batching (`wallet.signAndSend([a, b, c])`).
 */
export function buildInstruction(
  handler: InstructionHandler<any, any>,
  params: Record<string, unknown>,
  options: BuildOptions = {}
): BuiltInstruction {
  const { args, accountOverrides, resolve } = splitParams(handler, params);

  const resolutionOptions: AccountResolutionOptions = {
    accounts: { ...accountOverrides, ...options.accounts },
    resolve,
    wallet: options.wallet,
    programId: handler.programId,
  };

  const resolution = resolveAccounts(handler.accounts, args, resolutionOptions);
  validateAccountResolution(resolution);

  const instruction = handler.build(args, resolution.accounts);
  if (options.remainingAccounts?.length) {
    instruction.keys.push(...options.remainingAccounts);
  }
  return instruction;
}

/**
 * Builds, signs, and sends an instruction via the wallet adapter.
 *
 * The core SDK does not touch RPC: the adapter owns blockhash, compilation,
 * signing, sending, and confirmation. On failure, program errors are parsed
 * against the handler's IDL error definitions and surfaced as an
 * {@link InstructionError}.
 */
export async function executeInstruction(
  handler: InstructionHandler<any, any>,
  params: Record<string, unknown>,
  options: ExecuteOptions = {}
): Promise<ExecutionResult> {
  let instruction: BuiltInstruction;
  try {
    instruction = buildInstruction(handler, params, options);
  } catch (cause) {
    throw new TransactionExecutionError({
      status: 'not-submitted',
      phase: 'build',
      cause,
    });
  }

  if (!options.wallet) {
    const cause = new Error('Wallet required to sign and send transaction');
    throw new TransactionExecutionError({
      status: 'not-submitted',
      phase: 'wallet',
      cause,
    });
  }

  const sendOptions: SendOptions = {
    ...options.send,
  };
  if (options.confirmationLevel !== undefined) {
    sendOptions.confirmationLevel = options.confirmationLevel;
  }

  try {
    const result = await options.wallet.signAndSend([instruction], sendOptions);
    return { signature: result.signature, slot: result.slot };
  } catch (err) {
    throw normalizeTransactionError(err, handler.errors);
  }
}

/**
 * Creates an instruction executor bound to a specific wallet.
 */
export function createInstructionExecutor(wallet: WalletAdapter) {
  return {
    execute: async (
      handler: InstructionHandler,
      params: Record<string, unknown>,
      options?: Omit<ExecuteOptions, 'wallet'>
    ) => {
      return executeInstruction(handler, params, { ...options, wallet });
    },
    build: (
      handler: InstructionHandler,
      params: Record<string, unknown>,
      options?: Omit<BuildOptions, 'wallet'>
    ) => {
      return buildInstruction(handler, params, { ...options, wallet });
    },
  };
}
