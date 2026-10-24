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
import { encodeInstructionData, type ArgSchema } from './serializer';
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
  TAccountName extends string = string,
> {
  /** Instruction name, used in error messages. */
  name?: string;
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
   * Arguments that must be present. Option arguments are left out: an omitted
   * option encodes as `None`. When absent, missing arguments are reported by
   * the serializer instead.
   */
  requiredArgNames?: string[];
  /**
   * Build the instruction from already-resolved, ordered accounts.
   * Implemented by {@link createInstructionHandler}.
   */
  build(args: Record<string, unknown>, resolved: ResolvedAccount[]): BuiltInstruction;
  /** Phantom: merged params type (args + user-provided accounts). */
  readonly _params?: TParams;
  /** Phantom: typed error union. */
  readonly _error?: TError;
  /**
   * Phantom: the keys of `TParams` that are accounts rather than arguments.
   * `string` when the handler does not say, in which case no params key is
   * treated as an account at the type level.
   */
  readonly _accountNames?: TAccountName;
}

/**
 * Configuration accepted by {@link createInstructionHandler}.
 */
export interface InstructionHandlerConfig {
  /** Instruction name, used in error messages. */
  name?: string;
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
  TAccountName extends string = string,
>(config: InstructionHandlerConfig): InstructionHandler<TParams, TError, TAccountName> {
  const discriminator =
    config.discriminator instanceof Uint8Array
      ? config.discriminator
      : Uint8Array.from(config.discriminator);
  const argNames = config.args.map((a) => a.name);
  const requiredArgNames = config.args
    .filter((a) => !(typeof a.type === 'object' && 'option' in a.type))
    .map((a) => a.name);
  const errors = config.errors ?? [];

  return {
    ...(config.name !== undefined ? { name: config.name } : {}),
    programId: config.programId,
    accounts: config.accounts,
    errors,
    argNames,
    requiredArgNames,
    build(args: Record<string, unknown>, resolved: ResolvedAccount[]): BuiltInstruction {
      const data = encodeInstructionData(discriminator, args, config.args);
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
  wallet?: WalletAdapter | undefined;
  /** Explicit account-address overrides, including signer slots when needed. */
  accounts?: Record<string, string> | undefined;
  /**
   * Extra account metas appended after the instruction's declared accounts
   * (Anchor's `remainingAccounts`) — for routers, transfer hooks, and other
   * composition patterns the IDL cannot express.
   */
  remainingAccounts?: BuiltAccountMeta[] | undefined;
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
 * The params type a handler accepts: the `TParams` it was created with
 * (generated program modules pass their `<Instruction>Params` interface).
 * Handlers created without a params type accept any object.
 */
export type InstructionParams<THandler> =
  THandler extends InstructionHandler<infer TParams, any>
    ? unknown extends TParams
      ? Record<string, unknown>
      : TParams
    : never;

/**
 * Keys of option arguments: generated params declare them as `T | null`, and
 * an omitted option encodes as `None`. `unknown`/`any` typed keys are not
 * treated as options.
 */
type OptionParamKeys<TParams> = {
  [K in keyof TParams]-?: unknown extends TParams[K]
    ? never
    : null extends TParams[K]
      ? K
      : never;
}[keyof TParams];

type Simplify<T> = { [K in keyof T]: T[K] } & {};

/**
 * `TParams` with option arguments, and the accounts named in `TAccountKeys`
 * (supplied through `options.accounts`), made optional. Every other key keeps
 * its required/optional modifier.
 */
export type AcceptedInstructionParams<TParams, TAccountKeys extends PropertyKey = never> =
  Simplify<
    {
      [K in keyof TParams as K extends OptionParamKeys<TParams> | TAccountKeys
        ? never
        : K]: TParams[K];
    } & {
      [K in keyof TParams as K extends OptionParamKeys<TParams> | TAccountKeys
        ? K
        : never]?: TParams[K];
    }
  >;

/**
 * The params keys a handler declares as accounts (its third type parameter;
 * generated program modules pass the union of their account params). A
 * handler that does not declare them has none known, so no params key can be
 * supplied through `options.accounts` instead: an argument must always be in
 * params.
 */
type HandlerAccountNames<THandler> =
  THandler extends InstructionHandler<any, any, infer TAccountName>
    ? string extends TAccountName
      ? never
      : TAccountName
    : never;

/**
 * The accounts of `THandler` named by an `options.accounts` map, when its keys
 * are known literally. A wide `Record<string, string>` names no account in
 * particular, and keys that are not accounts of the handler (arguments
 * included) are never relaxed.
 */
type SuppliedAccountKeys<THandler, TAccountKeys extends string> = string extends TAccountKeys
  ? never
  : Extract<TAccountKeys, HandlerAccountNames<THandler>>;

/**
 * The params type {@link buildInstruction} checks a call against.
 *
 * An untyped value, whose type has a string index signature (such as
 * `Record<string, unknown>`), is accepted as is: generated operation wrappers
 * pass `params as unknown as Record<string, unknown>`. Anything else, object
 * literals included, is checked against the handler's
 * {@link InstructionParams}, with excess-property checks. Option arguments may
 * be omitted, and accounts of the handler passed through `options.accounts`
 * (`TAccountKeys`) need not be repeated in params. Arguments stay required.
 */
export type CheckedInstructionParams<THandler, TParams, TAccountKeys extends string = never> =
  string extends keyof TParams
    ? TParams
    : AcceptedInstructionParams<InstructionParams<THandler>, SuppliedAccountKeys<THandler, TAccountKeys>>;

/**
 * The `accounts` keys an options type guarantees are present. Only a required,
 * defined `accounts` map counts: an optional one may be absent at run time,
 * so its keys relax nothing.
 */
type GuaranteedAccountKeys<TOptions> = TOptions extends { accounts: infer TAccounts }
  ? undefined extends TAccounts
    ? never
    : Extract<keyof TAccounts, string>
  : never;

/**
 * `TOptions`, with keys that are not part of `TBase` rejected. Inferring the
 * options type skips excess-property checks, so this restores them.
 */
type ExactOptions<TOptions, TBase> = TOptions & {
  [K in Exclude<keyof TOptions, keyof TBase>]?: never;
};

/**
 * Thrown by {@link buildInstruction} when a params object does not match the
 * instruction: missing required arguments or accounts, keys that are neither
 * an argument nor an account, or account values that are not address strings.
 * Every problem is reported at once.
 */
export class InstructionParamsError extends Error {
  /** Instruction name, or a description of the handler when it has none. */
  readonly instruction: string;
  /** Required arguments that were not provided. */
  readonly missingArgs: readonly string[];
  /** Required accounts that were neither provided nor resolvable. */
  readonly missingAccounts: readonly string[];
  /** Keys that are neither an argument nor an account of the instruction. */
  readonly unknownKeys: readonly string[];
  /** Account keys whose value is not an address string. */
  readonly invalidAccounts: readonly string[];

  constructor(details: {
    instruction: string;
    missingArgs: readonly string[];
    missingAccounts: readonly string[];
    unknownKeys: readonly string[];
    invalidAccounts: readonly string[];
    message: string;
  }) {
    super(details.message);
    this.name = 'InstructionParamsError';
    this.instruction = details.instruction;
    this.missingArgs = details.missingArgs;
    this.missingAccounts = details.missingAccounts;
    this.unknownKeys = details.unknownKeys;
    this.invalidAccounts = details.invalidAccounts;
  }
}

function describeHandler(handler: InstructionHandler<any, any>): string {
  if (handler.name) return `instruction "${handler.name}"`;
  return handler.programId ? `instruction of program ${handler.programId}` : 'instruction';
}

function normalizeKey(key: string): string {
  return key.replace(/[_-]/g, '').toLowerCase();
}

/** A declared key that differs from `key` only in case or separators. */
function suggestKey(key: string, candidates: readonly string[]): string | undefined {
  const normalized = normalizeKey(key);
  return candidates.find((candidate) => normalizeKey(candidate) === normalized);
}

/**
 * Whether an account can be filled without the caller passing an address.
 * Mirrors the account resolver: known addresses, derivable PDAs, and signer
 * slots explicitly assigned to a connected wallet.
 */
function resolvesAutomatically(meta: AccountMeta, options: BuildOptions): boolean {
  switch (meta.category) {
    case 'known':
      return Boolean(meta.knownAddress);
    case 'pda':
      return Boolean(meta.pdaConfig);
    case 'signer':
      return meta.signerKind === 'wallet' && Boolean(options.wallet?.publicKey);
    default:
      return false;
  }
}

/**
 * Splits a merged params object into serialized args and account overrides,
 * and checks it against the instruction.
 *
 * Keys matching a declared argument name are args; keys matching a declared
 * account name (with a string value) are account address overrides. This
 * applies to signer slots too, allowing explicit signer addresses to fill or
 * override those slots. Unknown keys, non-string account values, missing
 * required args and missing caller-provided accounts are collected and thrown
 * together as an {@link InstructionParamsError}: a typo'd key silently dropped
 * here would otherwise change the built instruction. `options.accounts` on
 * {@link BuildOptions} is the explicit override map and wins over merged params.
 */
function splitParams(
  handler: InstructionHandler<any, any>,
  params: Record<string, unknown>,
  options: BuildOptions
): {
  args: Record<string, unknown>;
  accountOverrides: Record<string, string>;
  resolve?: Record<string, unknown>;
} {
  if (params === null || typeof params !== 'object' || Array.isArray(params)) {
    throw new Error(`Params for ${describeHandler(handler)} must be an object`);
  }

  const argNameSet = new Set(handler.argNames);
  const accountNames = handler.accounts.map((a) => a.name);
  const accountNameSet = new Set(accountNames);
  const args: Record<string, unknown> = {};
  const accountOverrides: Record<string, string> = {};
  let resolve: Record<string, unknown> | undefined;
  const unknownKeys: string[] = [];
  const invalidAccounts: string[] = [];

  for (const [key, value] of Object.entries(params)) {
    if (argNameSet.has(key)) {
      args[key] = value;
    } else if (key === 'resolve' && !accountNameSet.has(key)) {
      if (value === undefined) {
        continue;
      }
      if (value === null || Array.isArray(value) || typeof value !== 'object') {
        throw new Error(
          `Parameter "resolve" for ${describeHandler(handler)} must be an object when provided`
        );
      }
      resolve = value as Record<string, unknown>;
    } else if (accountNameSet.has(key)) {
      if (value === undefined) {
        // An optional account spelled out as `undefined` is simply omitted.
        continue;
      }
      if (typeof value !== 'string') {
        invalidAccounts.push(key);
        continue;
      }
      accountOverrides[key] = value;
    } else {
      unknownKeys.push(key);
    }
  }

  const missingArgs = (handler.requiredArgNames ?? []).filter(
    (name) => args[name] === undefined
  );
  const missingAccounts = handler.accounts
    .filter(
      (meta) =>
        !meta.isOptional &&
        accountOverrides[meta.name] === undefined &&
        options.accounts?.[meta.name] === undefined &&
        !invalidAccounts.includes(meta.name) &&
        !resolvesAutomatically(meta, options)
    )
    .map((meta) => meta.name);

  if (
    unknownKeys.length > 0 ||
    invalidAccounts.length > 0 ||
    missingArgs.length > 0 ||
    missingAccounts.length > 0
  ) {
    const instruction = describeHandler(handler);
    const declaredKeys = [...handler.argNames, ...accountNames];
    const problems: string[] = [];
    if (missingArgs.length > 0) {
      problems.push(`missing required args: ${missingArgs.join(', ')}`);
    }
    if (missingAccounts.length > 0) {
      problems.push(`missing required accounts: ${missingAccounts.join(', ')}`);
    }
    if (unknownKeys.length > 0) {
      const described = unknownKeys.map((key) => {
        const suggestion = suggestKey(key, declaredKeys);
        return suggestion ? `${key} (did you mean "${suggestion}"?)` : key;
      });
      problems.push(`unknown keys: ${described.join(', ')}`);
    }
    if (invalidAccounts.length > 0) {
      problems.push(`accounts that are not base58 address strings: ${invalidAccounts.join(', ')}`);
    }
    const requiredArgs = new Set(handler.requiredArgNames ?? handler.argNames);
    const expectedArgs = handler.argNames
      .map((name) => (requiredArgs.has(name) ? name : `${name}?`))
      .join(', ');
    const expectedAccounts = handler.accounts
      .map((meta) =>
        meta.isOptional || resolvesAutomatically(meta, options) ? `${meta.name}?` : meta.name
      )
      .join(', ');
    throw new InstructionParamsError({
      instruction,
      missingArgs,
      missingAccounts,
      unknownKeys,
      invalidAccounts,
      message:
        `Invalid params for ${instruction}: ${problems.join('; ')}. ` +
        `Expected args [${expectedArgs}] and accounts [${expectedAccounts}] ` +
        `(? = optional or resolved automatically)`,
    });
  }

  return { args, accountOverrides, resolve };
}

/**
 * Builds a {@link BuiltInstruction} from a handler and a merged params object.
 *
 * This is a pure function: it performs no network access. It is the unit of
 * composition for batching (`wallet.signAndSend([a, b, c])`).
 *
 * `params` is checked against the handler's params type, so a misspelled,
 * unknown or missing argument or account is a compile error. At run time the
 * same problems throw an {@link InstructionParamsError} naming the instruction.
 */
export function buildInstruction<
  THandler extends InstructionHandler<any, any>,
  TParams = InstructionParams<THandler>,
  TOptions extends BuildOptions = BuildOptions,
>(
  handler: THandler,
  params: CheckedInstructionParams<THandler, TParams, GuaranteedAccountKeys<TOptions>>,
  buildOptions?: ExactOptions<TOptions, BuildOptions>
): BuiltInstruction {
  const options: BuildOptions = buildOptions ?? {};
  const { args, accountOverrides, resolve } = splitParams(
    handler,
    params as unknown as Record<string, unknown>,
    options
  );

  const resolutionOptions: AccountResolutionOptions = {
    accounts: { ...accountOverrides, ...options.accounts },
    resolve,
    wallet: options.wallet,
    programId: handler.programId,
  };

  let instruction: BuiltInstruction;
  try {
    const resolution = resolveAccounts(handler.accounts, args, resolutionOptions);
    validateAccountResolution(resolution);
    instruction = handler.build(args, resolution.accounts);
  } catch (error) {
    // Name the instruction in resolver and serializer errors too.
    if (error instanceof Error && !error.message.startsWith('instruction ')) {
      error.message = `${describeHandler(handler)}: ${error.message}`;
    }
    throw error;
  }
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
export async function executeInstruction<
  THandler extends InstructionHandler<any, any>,
  TParams = InstructionParams<THandler>,
  TOptions extends ExecuteOptions = ExecuteOptions,
>(
  handler: THandler,
  params: CheckedInstructionParams<THandler, TParams, GuaranteedAccountKeys<TOptions>>,
  executeOptions?: ExactOptions<TOptions, ExecuteOptions>
): Promise<ExecutionResult> {
  const options: ExecuteOptions = executeOptions ?? {};
  let instruction: BuiltInstruction;
  try {
    instruction = buildInstruction(
      handler as InstructionHandler<any, any>,
      params as unknown as Record<string, unknown>,
      options as BuildOptions
    );
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
    execute: async <
      THandler extends InstructionHandler<any, any>,
      TParams = InstructionParams<THandler>,
      TOptions extends Omit<ExecuteOptions, 'wallet'> = Omit<ExecuteOptions, 'wallet'>,
    >(
      handler: THandler,
      params: CheckedInstructionParams<THandler, TParams, GuaranteedAccountKeys<TOptions>>,
      options?: ExactOptions<TOptions, Omit<ExecuteOptions, 'wallet'>>
    ) => {
      const executeOptions: ExecuteOptions = { ...options, wallet };
      return executeInstruction(
        handler,
        params as unknown as Record<string, unknown>,
        executeOptions
      );
    },
    build: <
      THandler extends InstructionHandler<any, any>,
      TParams = InstructionParams<THandler>,
      TOptions extends Omit<BuildOptions, 'wallet'> = Omit<BuildOptions, 'wallet'>,
    >(
      handler: THandler,
      params: CheckedInstructionParams<THandler, TParams, GuaranteedAccountKeys<TOptions>>,
      options?: ExactOptions<TOptions, Omit<BuildOptions, 'wallet'>>
    ) => {
      const buildOptions: BuildOptions = { ...options, wallet };
      return buildInstruction(handler, params as unknown as Record<string, unknown>, buildOptions);
    },
  };
}
