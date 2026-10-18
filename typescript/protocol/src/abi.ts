/**
 * The `arete.protocol-handler/v1` handler ABI and `arete.protocol-ctx/v1`.
 *
 * A handler is reviewed code that runs on a trusted protocol host. It sees
 * fixed inputs (`PrepareInputV1`) and reaches data only through the closed
 * `ProtocolContextV1`; it has no network, filesystem, clock, randomness,
 * environment or secrets. It returns portable instructions, and trusted
 * platform code adds signers, fees, glue and the memo afterwards.
 */

import { validatePreparedOperation } from "./contracts/handler.js";
import { encodeBase64, isPubkey } from "./contracts/shape.js";
import {
  PROTOCOL_HANDLER_ABI_V1,
  type AccountReadResultV1,
  type FeePlacement,
  type PreparedOperationV1,
} from "./contracts/types.js";
import {
  MAX_INSTRUCTION_ACCOUNTS,
  MAX_INSTRUCTION_DATA_BYTES,
  MAX_OPERATION_TRANSACTIONS,
  MAX_TRANSACTION_INSTRUCTIONS,
} from "./contracts/handler.js";

/** What a handler returns: the wire object of `arete.protocol-handler/v1`. */
export type HostedPreparedOperationV1 = PreparedOperationV1;

export interface InstanceInputV1 {
  id: string;
  state: string;
  /** Bound instance accounts, name → pubkey. */
  accounts: Record<string, string>;
  /** Bound role wallets, role → pubkey. */
  roles: Record<string, string>;
}

export interface ParticipantInputV1 {
  id: string;
  wallet: string;
  state: string;
}

/** A declared read: the raw account plus, for `account` reads, the decoded fields. */
export interface ReadInputV1 {
  account: AccountReadResultV1;
  decoded: unknown | null;
}

export interface FeeContextV1 {
  gross: string;
  fee: string;
  net: string;
  /** `"native"` or a mint address. */
  asset: string;
  placement: FeePlacement;
}

/** What the platform hands a handler for one invocation. Every value is fixed before dispatch. */
export interface PrepareInputV1<TArgs = unknown, TParams = unknown> {
  abi: typeof PROTOCOL_HANDLER_ABI_V1;
  step: string;
  args: TArgs;
  /** The exact resource values of the deployment revision. */
  resources: Record<string, unknown>;
  params: TParams;
  instance: InstanceInputV1 | null;
  participant: ParticipantInputV1 | null;
  reads: Record<string, ReadInputV1>;
  wallet: string;
  executionId: string;
  /** Ephemeral keypairs allocated for this intent, name → pubkey. */
  ephemeral: Record<string, string>;
  /** Declared programs, alias → program id. */
  programs: Record<string, string>;
  fee: FeeContextV1 | null;
  /** Unix seconds, fixed for the whole invocation. */
  now: number;
  /** The platform keys of the deployment the invocation runs under. */
  deployment: DeploymentInputV1;
}

/** The deployment an invocation runs under; the same values are the `deployment.*` expression root. */
export interface DeploymentInputV1 {
  /** The deployment id (a UUID). */
  id: string;
  /** The deployment's attestation pubkey. */
  attestation: string;
  /** The Squads proposer pubkey for the calling integrator account; `null` when no step uses `squadsProposer`. */
  squadsProposer: string | null;
}

/** A raw account as the platform returns it; decoding runs in the handler. */
export interface RawAccountV1 {
  address: string;
  /** Decimal lamports. */
  lamports: string;
  owner: string;
  data: Uint8Array;
  /** Decimal slot the account was read at. */
  slot: string;
}

/** Decodes raw account bytes, usually with a generated program SDK bundled with the handler. */
export type AccountDecoder<T> =
  | ((data: Uint8Array, account: RawAccountV1) => T)
  | { decode(data: Uint8Array, account: RawAccountV1): T };

export interface TokenAccountV1 {
  address: string;
  mint: string;
  owner: string;
  /** Decimal base units. */
  amount: string;
  /** Decimal slot the account was read at. */
  slot: string;
}

/**
 * The closed `arete.protocol-ctx/v1` surface. Each call travels to the
 * platform, which resolves it under the caller's metered claims and records
 * it (method, canonical arguments, result) as the provider snapshot.
 */
export interface ProtocolContextV1 {
  solana: {
    account<T = RawAccountV1>(
      pubkey: string,
      decoder?: AccountDecoder<T>,
      opts?: { minContextSlot?: number },
    ): Promise<T | null>;
    accounts<T = RawAccountV1>(pubkeys: string[], decoder?: AccountDecoder<T>): Promise<(T | null)[]>;
    /** `owner` must be the wallet or a bound instance account; mints must come from resources or params. */
    tokenAccountsByOwner(owner: string, mints: string[]): Promise<TokenAccountV1[]>;
  };
  stacks: {
    /** Advisory: never use a stack view for a money decision; use the Solana methods. */
    view<T = unknown>(stack: string, view: string, key: string): Promise<T | null>;
  };
}

export interface ProtocolHandlerV1<TArgs = unknown, TParams = unknown> {
  prepare(input: PrepareInputV1<TArgs, TParams>, ctx: ProtocolContextV1): Promise<HostedPreparedOperationV1>;
}

/** The function form of a handler, as a bundle exports it. */
export type ProtocolHandlerFnV1<TArgs = unknown, TParams = unknown> = (
  input: PrepareInputV1<TArgs, TParams>,
  ctx: ProtocolContextV1,
) => Promise<HostedPreparedOperationV1>;

export type ProtocolHandler<TArgs = unknown, TParams = unknown> =
  | ProtocolHandlerV1<TArgs, TParams>
  | ProtocolHandlerFnV1<TArgs, TParams>;

/** Call a handler in either form. */
export function invokeHandler<TArgs, TParams>(
  handler: ProtocolHandler<TArgs, TParams>,
  input: PrepareInputV1<TArgs, TParams>,
  ctx: ProtocolContextV1,
): Promise<HostedPreparedOperationV1> {
  return typeof handler === "function" ? handler(input, ctx) : handler.prepare(input, ctx);
}

export function decodeAccount<T>(decoder: AccountDecoder<T>, account: RawAccountV1): T {
  return typeof decoder === "function" ? decoder(account.data, account) : decoder.decode(account.data, account);
}

// ---------------------------------------------------------------------------
// wirePreparedOperation
// ---------------------------------------------------------------------------

/** Structurally the core SDK's `BuiltAccountMeta`. */
export interface BuiltAccountMetaLike {
  pubkey: string;
  isSigner: boolean;
  isWritable: boolean;
}

/** Structurally the core SDK's `BuiltInstruction`. */
export interface BuiltInstructionLike {
  programId: string;
  keys: readonly BuiltAccountMetaLike[];
  data: Uint8Array;
}

/** Structurally the core SDK's `PreparedTransactionBody`, minus signer material. */
export interface WireTransactionInput {
  name: string;
  instructions: readonly BuiltInstructionLike[];
  /** Always rejected: handlers never return signer material. */
  signers?: readonly unknown[];
  [key: string]: unknown;
}

export type WireOperationInput =
  | readonly WireTransactionInput[]
  | { transactions: readonly WireTransactionInput[] }
  | { plan: { transactions: readonly WireTransactionInput[] } };

/** What `wirePreparedOperation` checks a handler's output against. */
export interface WireStepV1 {
  /**
   * The step's declared `build.programs` aliases. Each must appear in
   * `input.programs`. Omit to allow every program in `input.programs`.
   */
  programs?: readonly string[];
  /** The invocation input: program ids, the wallet and the ephemeral pubkeys. */
  input: Pick<PrepareInputV1, "programs" | "wallet" | "ephemeral">;
  /** Extra signer pubkeys the step declares (for example a bound role wallet). */
  signers?: readonly string[];
}

export const WIRE_ERROR_CODES = [
  "signers-not-allowed",
  "empty-operation",
  "empty-transaction",
  "undeclared-program",
  "undeclared-signer",
  "too-many-transactions",
  "too-many-instructions",
  "too-many-accounts",
  "instruction-data-too-large",
  "invalid-instruction",
] as const;

export type WireErrorCode = (typeof WIRE_ERROR_CODES)[number];

/** A handler output that breaks the step's declarations or the ABI bounds. */
export class WireOperationError extends Error {
  readonly code: WireErrorCode;
  readonly path: string;

  constructor(code: WireErrorCode, path: string, message: string) {
    super(message);
    this.name = "WireOperationError";
    this.code = code;
    this.path = path;
  }
}

function wireError(code: WireErrorCode, path: string, message: string): never {
  throw new WireOperationError(code, path, message);
}

function transactionsOf(operation: WireOperationInput): readonly WireTransactionInput[] {
  if (Array.isArray(operation)) return operation;
  const value = operation as { transactions?: unknown; plan?: { transactions?: unknown } };
  if (Array.isArray(value.transactions)) return value.transactions as WireTransactionInput[];
  if (value.plan && Array.isArray(value.plan.transactions)) {
    return value.plan.transactions as WireTransactionInput[];
  }
  return wireError("empty-operation", "/transactions", "expected a list of transactions");
}

/**
 * Turn Arete core-SDK instructions into the `arete.protocol-handler/v1`
 * wire object, rejecting anything the platform would reject: signer
 * material, empty flows, programs the step did not declare, signers other
 * than the wallet, the step's ephemeral keys and declared extra signers, and
 * breaches of the 8 transaction / 64 instruction / 256 account / 10 KiB
 * bounds. The result is then checked with `validatePreparedOperation`.
 */
export function wirePreparedOperation(
  operation: WireOperationInput,
  step: WireStepV1,
): HostedPreparedOperationV1 {
  const transactions = transactionsOf(operation);
  if (transactions.length === 0) wireError("empty-operation", "/transactions", "an operation has at least one transaction");
  if (transactions.length > MAX_OPERATION_TRANSACTIONS) {
    wireError("too-many-transactions", "/transactions", `at most ${MAX_OPERATION_TRANSACTIONS} transactions are allowed`);
  }
  const programs = new Set<string>();
  const declared = step.programs ?? Object.keys(step.input.programs);
  for (const alias of declared) {
    const id = Object.prototype.hasOwnProperty.call(step.input.programs, alias)
      ? step.input.programs[alias]
      : undefined;
    if (id === undefined) {
      wireError("undeclared-program", "", `program '${alias}' is not in the invocation's programs`);
    }
    programs.add(id);
  }
  const signers = new Set<string>([step.input.wallet, ...Object.values(step.input.ephemeral), ...(step.signers ?? [])]);

  const wire: HostedPreparedOperationV1 = {
    abi: PROTOCOL_HANDLER_ABI_V1,
    transactions: transactions.map((transaction, i) => {
      const path = `/transactions/${i}`;
      if (transaction === null || typeof transaction !== "object") {
        wireError("invalid-instruction", path, "expected a transaction object");
      }
      if (Object.prototype.hasOwnProperty.call(transaction, "signers")) {
        wireError("signers-not-allowed", `${path}/signers`, "handlers never return signer material");
      }
      const instructions = transaction.instructions;
      if (!Array.isArray(instructions) || instructions.length === 0) {
        wireError("empty-transaction", `${path}/instructions`, "a transaction has at least one instruction");
      }
      if (instructions.length > MAX_TRANSACTION_INSTRUCTIONS) {
        wireError(
          "too-many-instructions",
          `${path}/instructions`,
          `at most ${MAX_TRANSACTION_INSTRUCTIONS} instructions per transaction are allowed`,
        );
      }
      return {
        name: transaction.name,
        instructions: instructions.map((instruction: BuiltInstructionLike, j: number) => {
          const ixPath = `${path}/instructions/${j}`;
          if (instruction === null || typeof instruction !== "object" || !(instruction.data instanceof Uint8Array)) {
            wireError("invalid-instruction", ixPath, "expected { programId, keys, data: Uint8Array }");
          }
          if (Object.prototype.hasOwnProperty.call(instruction, "signers")) {
            wireError("signers-not-allowed", `${ixPath}/signers`, "handlers never return signer material");
          }
          if (!programs.has(instruction.programId)) {
            wireError("undeclared-program", `${ixPath}/programId`, `program ${instruction.programId} is not declared by the step`);
          }
          if (!Array.isArray(instruction.keys)) wireError("invalid-instruction", `${ixPath}/keys`, "expected an array");
          if (instruction.keys.length > MAX_INSTRUCTION_ACCOUNTS) {
            wireError("too-many-accounts", `${ixPath}/keys`, `at most ${MAX_INSTRUCTION_ACCOUNTS} account metas are allowed`);
          }
          if (instruction.data.length > MAX_INSTRUCTION_DATA_BYTES) {
            wireError("instruction-data-too-large", `${ixPath}/dataBase64`, `instruction data is at most ${MAX_INSTRUCTION_DATA_BYTES} bytes`);
          }
          return {
            programId: instruction.programId,
            keys: instruction.keys.map((key, k) => {
              if (key.isSigner && !signers.has(key.pubkey)) {
                wireError(
                  "undeclared-signer",
                  `${ixPath}/keys/${k}`,
                  `signer ${key.pubkey} is neither the wallet nor a declared ephemeral or role key`,
                );
              }
              return { pubkey: key.pubkey, isSigner: key.isSigner, isWritable: key.isWritable };
            }),
            dataBase64: encodeBase64(instruction.data),
          };
        }),
      };
    }),
  };
  // The shape validator reports anything else (names, pubkeys) as ProtocolContractError.
  return validatePreparedOperation(wire);
}

/** True when `value` is a base58 32-byte public key. */
export { isPubkey };
