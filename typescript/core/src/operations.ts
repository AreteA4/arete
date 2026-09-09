import {
  InstructionError,
  TransactionExecutionError,
  getTransactionFailureOutcome,
  normalizeTransactionError,
  parseInstructionError,
  type ErrorMetadata,
  type ProgramError,
  type TransactionFailureOutcome,
} from './instructions';
import type { SignerRegistry } from './signer-registry';
import type { TransactionTransport } from './transactions';
import { resolveTransactionBuildOptions } from './wallet/types';
import type {
  BuiltInstruction,
  SendOptions,
  TransactionInspectionOptions,
  TransactionInspectionResult,
  WalletAdapter,
  WalletExecutionContext,
} from './wallet/types';

export type OperationKind = 'instruction' | 'transaction' | 'flow';
export type NonEmptyReadonlyArray<T> = readonly [T, ...T[]];
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | readonly JsonValue[];
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export interface PreparedOperationDescription {
  readonly kind: OperationKind;
  readonly name: string;
  readonly artifacts: JsonValue;
  readonly transactions: readonly {
    readonly name: string;
    readonly requiredSignerAddresses: readonly string[];
    readonly errors: readonly ErrorMetadata[];
    readonly instructions: readonly {
      readonly programId: string;
      readonly keys: readonly {
        readonly pubkey: string;
        readonly isSigner: boolean;
        readonly isWritable: boolean;
      }[];
      readonly data: readonly number[];
    }[];
  }[];
}

export interface OperationInspection {
  /** JSON-safe description produced by {@link describePreparedOperation}. */
  readonly description: PreparedOperationDescription;
  /** Raw unsigned inspection returned by the adapter. */
  readonly transaction: TransactionInspectionResult;
  /** IDL-aware program error parsed from the inspection failure, if any. */
  readonly programError: ProgramError | null;
}

export interface OperationInspectionOptions {
  wallet?: WalletAdapter;
  inspect?: TransactionInspectionOptions;
  transactionTransport?: TransactionTransport;
}

export interface PreparedTransactionBody {
  readonly name: string;
  readonly instructions: NonEmptyReadonlyArray<BuiltInstruction>;
  readonly requiredSignerAddresses: readonly string[];
  /** Signer material created while preparing this transaction. */
  readonly signers?: readonly unknown[];
  readonly errors: readonly ErrorMetadata[];
}

export interface OperationPlan<TArtifacts = void> {
  readonly name: string;
  readonly artifacts: TArtifacts;
  readonly transactions: NonEmptyReadonlyArray<PreparedTransactionBody>;
}

interface PreparedOperationBase<TKind extends OperationKind, TArtifacts> {
  readonly kind: TKind;
  readonly name: string;
  readonly plan: OperationPlan<TArtifacts>;
  readonly artifacts: TArtifacts;
}

export interface PreparedInstruction<TArtifacts = void>
  extends PreparedOperationBase<'instruction', TArtifacts> {
  readonly instruction: BuiltInstruction;
  readonly transaction: PreparedTransactionBody;
}

export interface PreparedTransaction<TArtifacts = void>
  extends PreparedOperationBase<'transaction', TArtifacts> {
  readonly transaction: PreparedTransactionBody;
}

export interface PreparedFlow<TArtifacts = void>
  extends PreparedOperationBase<'flow', TArtifacts> {}

export type PreparedOperation<TArtifacts = unknown> =
  | PreparedInstruction<TArtifacts>
  | PreparedTransaction<TArtifacts>
  | PreparedFlow<TArtifacts>;

export interface CreatePreparedInstructionInput<TArtifacts> {
  name: string;
  instruction: BuiltInstruction;
  artifacts: TArtifacts;
  requiredSignerAddresses?: readonly string[];
  signers?: readonly unknown[];
  errors?: readonly ErrorMetadata[];
}

export type PreparedTransactionInstruction =
  | BuiltInstruction
  | PreparedInstruction<unknown>;

export type PreparedTransactionOperation =
  | PreparedInstruction<unknown>
  | PreparedTransaction<unknown>;

function isPreparedInstruction(
  instruction: PreparedTransactionInstruction
): instruction is PreparedInstruction<unknown> {
  return 'kind' in instruction
    && instruction.kind === 'instruction'
    && 'instruction' in instruction;
}

interface CreatePreparedTransactionBaseInput<TArtifacts> {
  name: string;
  artifacts: TArtifacts;
  requiredSignerAddresses?: readonly string[];
  signers?: readonly unknown[];
  errors?: readonly ErrorMetadata[];
}

export type CreatePreparedTransactionInput<TArtifacts> =
  CreatePreparedTransactionBaseInput<TArtifacts> & (
    | {
      instructions: readonly PreparedTransactionInstruction[];
      operations?: never;
    }
    | {
      operations: readonly PreparedTransactionOperation[];
      instructions?: never;
    }
  );

export interface CreatePreparedFlowInput<TArtifacts> {
  name: string;
  transactions: readonly PreparedTransactionBody[];
  artifacts: TArtifacts;
}

function nonEmpty<T>(values: readonly T[], label: string): NonEmptyReadonlyArray<T> {
  if (values.length === 0) {
    throw new Error(`${label} must contain at least one item`);
  }
  return [...values] as unknown as NonEmptyReadonlyArray<T>;
}

function dedupe(values: readonly string[]): string[] {
  return [...new Set(values)];
}

function inferSignerAddresses(instructions: readonly BuiltInstruction[]): string[] {
  return dedupe(
    instructions.flatMap((instruction) =>
      instruction.keys.filter((key) => key.isSigner).map((key) => key.pubkey)
    )
  );
}

export function createPreparedTransactionBody(input: {
  name: string;
  instructions: readonly BuiltInstruction[];
  requiredSignerAddresses?: readonly string[];
  signers?: readonly unknown[];
  errors?: readonly ErrorMetadata[];
}): PreparedTransactionBody {
  const instructions = nonEmpty(input.instructions, `Transaction '${input.name}'`);
  return {
    name: input.name,
    instructions,
    requiredSignerAddresses: dedupe(
      input.requiredSignerAddresses ?? inferSignerAddresses(instructions)
    ),
    signers: [...new Set(input.signers ?? [])],
    errors: [...(input.errors ?? [])],
  };
}

function createPlan<TArtifacts>(
  name: string,
  artifacts: TArtifacts,
  transactions: readonly PreparedTransactionBody[]
): OperationPlan<TArtifacts> {
  return {
    name,
    artifacts,
    transactions: nonEmpty(transactions, `Flow '${name}'`),
  };
}

export function createPreparedInstruction<TArtifacts>(
  input: CreatePreparedInstructionInput<TArtifacts>
): PreparedInstruction<TArtifacts> {
  const transaction = createPreparedTransactionBody({
    name: input.name,
    instructions: [input.instruction],
    requiredSignerAddresses: input.requiredSignerAddresses,
    signers: input.signers,
    errors: input.errors,
  });
  return {
    kind: 'instruction',
    name: input.name,
    instruction: input.instruction,
    transaction,
    plan: createPlan(input.name, input.artifacts, [transaction]),
    artifacts: input.artifacts,
  };
}

export function createPreparedTransaction<TArtifacts>(
  input: CreatePreparedTransactionInput<TArtifacts>
): PreparedTransaction<TArtifacts> {
  const hasInstructions = input.instructions !== undefined;
  const hasOperations = input.operations !== undefined;
  if (hasInstructions === hasOperations) {
    throw new Error(
      `Transaction '${input.name}' must provide exactly one of instructions or operations`
    );
  }
  const transactionParts = hasOperations
    ? input.operations.map((operation) => operation.transaction)
    : input.instructions.map((instruction) =>
      isPreparedInstruction(instruction)
        ? instruction.transaction
        : createPreparedTransactionBody({
          name: input.name,
          instructions: [instruction],
        })
    );
  const transaction = createPreparedTransactionBody({
    name: input.name,
    instructions: transactionParts.flatMap((part) => part.instructions),
    requiredSignerAddresses: input.requiredSignerAddresses
      ?? transactionParts.flatMap((part) => part.requiredSignerAddresses),
    signers: [
      ...transactionParts.flatMap((part) => part.signers),
      ...(input.signers ?? []),
    ],
    errors: input.errors ?? transactionParts.flatMap((part) => part.errors),
  });
  return {
    kind: 'transaction',
    name: input.name,
    transaction,
    plan: createPlan(input.name, input.artifacts, [transaction]),
    artifacts: input.artifacts,
  };
}

export function createPreparedFlow<TArtifacts>(
  input: CreatePreparedFlowInput<TArtifacts>
): PreparedFlow<TArtifacts> {
  const transactions = input.transactions.map((transaction) =>
    createPreparedTransactionBody(transaction)
  );
  return {
    kind: 'flow',
    name: input.name,
    plan: createPlan(input.name, input.artifacts, transactions),
    artifacts: input.artifacts,
  };
}

export function prependTransactionInstructions(
  transaction: PreparedTransactionBody,
  instructions: readonly BuiltInstruction[]
): PreparedTransactionBody {
  return createPreparedTransactionBody({
    ...transaction,
    instructions: [...instructions, ...transaction.instructions],
    requiredSignerAddresses: [
      ...inferSignerAddresses(instructions),
      ...transaction.requiredSignerAddresses,
    ],
  });
}

export function appendTransactionInstructions(
  transaction: PreparedTransactionBody,
  instructions: readonly BuiltInstruction[]
): PreparedTransactionBody {
  return createPreparedTransactionBody({
    ...transaction,
    instructions: [...transaction.instructions, ...instructions],
    requiredSignerAddresses: [
      ...transaction.requiredSignerAddresses,
      ...inferSignerAddresses(instructions),
    ],
  });
}

export function appendFlowTransactions<TArtifacts>(
  flow: PreparedFlow<TArtifacts>,
  transactions: readonly PreparedTransactionBody[]
): PreparedFlow<TArtifacts> {
  return createPreparedFlow({
    name: flow.name,
    artifacts: flow.artifacts,
    transactions: [...flow.plan.transactions, ...transactions],
  });
}

export function prependFlowTransactionInstructions<TArtifacts>(
  flow: PreparedFlow<TArtifacts>,
  transactionIndex: number,
  instructions: readonly BuiltInstruction[]
): PreparedFlow<TArtifacts> {
  const transaction = flow.plan.transactions[transactionIndex];
  if (!transaction) {
    throw new Error(
      `Flow '${flow.name}' has no transaction at index ${transactionIndex}`
    );
  }
  const transactions = [...flow.plan.transactions];
  transactions[transactionIndex] = prependTransactionInstructions(transaction, instructions);
  return createPreparedFlow({
    name: flow.name,
    artifacts: flow.artifacts,
    transactions,
  });
}

export interface OperationTransactionReceipt {
  readonly transactionIndex: number;
  readonly transactionName: string;
  readonly signature: string;
  readonly slot?: number;
}

export interface SingleTransactionOperationReceipt<TArtifacts> {
  readonly kind: 'instruction' | 'transaction';
  readonly operationName: string;
  readonly artifacts: TArtifacts;
  readonly signatures: NonEmptyReadonlyArray<string>;
  readonly transaction: OperationTransactionReceipt;
  readonly callbackErrors?: readonly OperationCallbackError[];
}

export interface FlowOperationReceipt<TArtifacts> {
  readonly kind: 'flow';
  readonly operationName: string;
  readonly artifacts: TArtifacts;
  readonly signatures: NonEmptyReadonlyArray<string>;
  readonly transactions: NonEmptyReadonlyArray<OperationTransactionReceipt>;
  readonly callbackErrors?: readonly OperationCallbackError[];
}

export type OperationReceiptFor<TPrepared extends PreparedOperation> =
  TPrepared extends PreparedFlow<infer TArtifacts>
    ? FlowOperationReceipt<TArtifacts>
    : TPrepared extends PreparedInstruction<infer TArtifacts>
      ? SingleTransactionOperationReceipt<TArtifacts>
      : TPrepared extends PreparedTransaction<infer TArtifacts>
        ? SingleTransactionOperationReceipt<TArtifacts>
        : never;

export interface OperationExecutionEvent<TPrepared extends PreparedOperation = PreparedOperation> {
  readonly operation: TPrepared;
  readonly transaction: PreparedTransactionBody;
  readonly transactionIndex: number;
}

export interface OperationExecutionSuccessEvent<
  TPrepared extends PreparedOperation = PreparedOperation,
> extends OperationExecutionEvent<TPrepared> {
  readonly receipt: OperationTransactionReceipt;
}

export type OperationCallbackPhase = 'transaction-start' | 'transaction-success';

export interface OperationExecutionOptions<
  TSigner = unknown,
  TPrepared extends PreparedOperation = PreparedOperation,
> {
  wallet?: WalletAdapter;
  transactionTransport?: TransactionTransport;
  send?: SendOptions;
  signers?: readonly TSigner[];
  signerRegistry?: SignerRegistry<TSigner>;
  availableSignerAddresses?: readonly string[];
  onTransactionStart?: (
    event: OperationExecutionEvent<TPrepared>
  ) => void | Promise<void>;
  onTransactionSuccess?: (
    event: OperationExecutionSuccessEvent<TPrepared>
  ) => void | Promise<void>;
  /** Receives observer failures without changing the transaction outcome. */
  onCallbackError?: (
    error: OperationCallbackError<TPrepared>
  ) => void | Promise<void>;
}

export interface OperationExecutionHost<TSigner = unknown> {
  readonly wallet?: WalletAdapter;
  readonly publicKey?: string;
  transaction(
    instructions: readonly BuiltInstruction[],
    options?: {
      wallet?: WalletAdapter;
      send?: SendOptions;
      errors?: ErrorMetadata[];
      signers?: readonly TSigner[];
      transactionTransport?: TransactionTransport;
    }
  ): Promise<{ signature: string; slot?: number }>;
}

function inferSignerAddress(value: unknown): string | null {
  if (typeof value === 'string' && value.length > 0) {
    return value;
  }
  if (!value || typeof value !== 'object') {
    return null;
  }
  const candidate = value as {
    address?: unknown;
    publicKey?: unknown;
  };
  if (typeof candidate.address === 'string' && candidate.address.length > 0) {
    return candidate.address;
  }
  if (typeof candidate.publicKey === 'string' && candidate.publicKey.length > 0) {
    return candidate.publicKey;
  }
  const publicKey = candidate.publicKey as { toBase58?: () => string } | undefined;
  return typeof publicKey?.toBase58 === 'function' ? publicKey.toBase58() : null;
}

function validateTransactionSigners<
  TSigner,
  TPrepared extends PreparedOperation,
>(
  transaction: PreparedTransactionBody,
  host: OperationExecutionHost<TSigner>,
  options: OperationExecutionOptions<TSigner, TPrepared>,
  signers: readonly unknown[]
) {
  const signerAddresses = signers.map(inferSignerAddress);
  const available = new Set(options.availableSignerAddresses ?? []);
  for (const address of options.signerRegistry?.addresses() ?? []) {
    available.add(address);
  }
  const wallet = options.wallet ?? host.wallet;
  for (const address of wallet?.signerAddresses ?? []) {
    available.add(address);
  }
  const walletAddress = wallet?.publicKey ?? host.publicKey;
  if (walletAddress) {
    available.add(walletAddress);
  }
  for (const address of signerAddresses) {
    if (address) {
      available.add(address);
    }
  }
  const missing = transaction.requiredSignerAddresses.filter(
    (address) => !available.has(address)
  );
  if (missing.length > 0) {
    throw new Error(
      `Missing signer(s) for ${transaction.name}: ${missing.join(', ')}`
    );
  }
}

export class OperationCallbackError<
  TPrepared extends PreparedOperation = PreparedOperation,
> extends Error {
  readonly phase: OperationCallbackPhase;
  readonly operation: TPrepared;
  readonly transaction: PreparedTransactionBody;
  readonly transactionIndex: number;
  readonly receipt?: OperationTransactionReceipt;
  readonly cause: unknown;

  constructor(input: {
    phase: OperationCallbackPhase;
    operation: TPrepared;
    transaction: PreparedTransactionBody;
    transactionIndex: number;
    receipt?: OperationTransactionReceipt;
    cause: unknown;
  }) {
    super(
      `Operation '${input.operation.name}' ${input.phase} callback failed for transaction ${input.transactionIndex + 1} (${input.transaction.name})`
    );
    this.name = 'OperationCallbackError';
    this.phase = input.phase;
    this.operation = input.operation;
    this.transaction = input.transaction;
    this.transactionIndex = input.transactionIndex;
    this.receipt = input.receipt;
    this.cause = input.cause;
  }
}

export class OperationExecutionError<
  TPrepared extends PreparedOperation = PreparedOperation,
> extends Error {
  readonly operation: TPrepared;
  readonly failedTransaction: PreparedTransactionBody;
  readonly failedTransactionIndex: number;
  readonly completedReceipts: readonly OperationTransactionReceipt[];
  readonly callbackErrors: readonly OperationCallbackError<TPrepared>[];
  readonly outcome: TransactionFailureOutcome;
  readonly signature?: string;
  readonly slot?: number;
  readonly cause: unknown;

  constructor(input: {
    operation: TPrepared;
    failedTransaction: PreparedTransactionBody;
    failedTransactionIndex: number;
    completedReceipts: readonly OperationTransactionReceipt[];
    callbackErrors?: readonly OperationCallbackError<TPrepared>[];
    outcome?: TransactionFailureOutcome;
    cause: unknown;
  }) {
    const context = `Operation '${input.operation.name}' failed at transaction ${input.failedTransactionIndex + 1} (${input.failedTransaction.name})`;
    const detail = input.cause instanceof Error
      ? input.cause.message
      : typeof input.cause === 'string' ? input.cause : '';
    super(detail ? `${context}: ${detail}` : context);
    this.name = 'OperationExecutionError';
    this.operation = input.operation;
    this.failedTransaction = input.failedTransaction;
    this.failedTransactionIndex = input.failedTransactionIndex;
    this.completedReceipts = [...input.completedReceipts];
    this.callbackErrors = [...(input.callbackErrors ?? [])];
    this.outcome = getTransactionFailureOutcome(input.cause) ?? input.outcome ?? {
      status: 'not-submitted',
      phase: 'send',
      cause: input.cause,
    };
    this.signature = 'signature' in this.outcome ? this.outcome.signature : undefined;
    this.slot = 'slot' in this.outcome ? this.outcome.slot : undefined;
    this.cause = input.cause;
  }
}

async function runOperationCallback<TPrepared extends PreparedOperation>(
  input: {
    phase: OperationCallbackPhase;
    operation: TPrepared;
    transaction: PreparedTransactionBody;
    transactionIndex: number;
    receipt?: OperationTransactionReceipt;
    callback?: () => void | Promise<void>;
  },
  callbackErrors: OperationCallbackError<TPrepared>[],
  onCallbackError?: (error: OperationCallbackError<TPrepared>) => void | Promise<void>
): Promise<void> {
  if (!input.callback) {
    return;
  }
  try {
    await input.callback();
  } catch (cause) {
    const error = new OperationCallbackError({
      phase: input.phase,
      operation: input.operation,
      transaction: input.transaction,
      transactionIndex: input.transactionIndex,
      receipt: input.receipt,
      cause,
    });
    callbackErrors.push(error);
    try {
      await onCallbackError?.(error);
    } catch {
      // The callback-error observer is also observational and must not alter execution.
    }
  }
}

export async function executePreparedOperation<
  TPrepared extends PreparedOperation,
  TSigner = unknown,
>(
  host: OperationExecutionHost<TSigner>,
  operation: TPrepared,
  options: OperationExecutionOptions<TSigner, TPrepared> = {}
): Promise<OperationReceiptFor<TPrepared>> {
  const receipts: OperationTransactionReceipt[] = [];
  const callbackErrors: OperationCallbackError<TPrepared>[] = [];
  for (const [transactionIndex, transaction] of operation.plan.transactions.entries()) {
    const signers = [
      ...new Set([
        ...(transaction.signers ?? []),
        ...(options.signerRegistry?.values() ?? []),
        ...(options.signers ?? []),
      ]),
    ];
    try {
      validateTransactionSigners(transaction, host, options, signers);
    } catch (cause) {
      throw new OperationExecutionError({
        operation,
        failedTransaction: transaction,
        failedTransactionIndex: transactionIndex,
        completedReceipts: receipts,
        callbackErrors,
        outcome: {
          status: 'not-submitted',
          phase: 'build',
          cause,
        },
        cause,
      });
    }

    const startEvent = { operation, transaction, transactionIndex };
    await runOperationCallback({
      phase: 'transaction-start',
      ...startEvent,
      callback: options.onTransactionStart
        ? () => options.onTransactionStart!(startEvent)
        : undefined,
    }, callbackErrors, options.onCallbackError);

    let result: { signature: string; slot?: number };
    try {
      result = await host.transaction(transaction.instructions, {
        wallet: options.wallet,
        send: options.send,
        errors: [...transaction.errors],
        signers: signers.length > 0 ? signers as readonly TSigner[] : undefined,
        transactionTransport: options.transactionTransport,
      });
    } catch (cause) {
      const normalized = normalizeTransactionError(cause, transaction.errors);
      throw new OperationExecutionError({
        operation,
        failedTransaction: transaction,
        failedTransactionIndex: transactionIndex,
        completedReceipts: receipts,
        callbackErrors,
        outcome: getTransactionFailureOutcome(normalized) ?? undefined,
        cause: normalized,
      });
    }

    const receipt: OperationTransactionReceipt = {
      transactionIndex,
      transactionName: transaction.name,
      signature: result.signature,
      slot: result.slot,
    };
    receipts.push(receipt);
    const successEvent = { operation, transaction, transactionIndex, receipt };
    await runOperationCallback({
      phase: 'transaction-success',
      ...successEvent,
      callback: options.onTransactionSuccess
        ? () => options.onTransactionSuccess!(successEvent)
        : undefined,
    }, callbackErrors, options.onCallbackError);
  }

  const callbackErrorResult = callbackErrors.length > 0
    ? { callbackErrors: [...callbackErrors] }
    : {};

  if (operation.kind === 'flow') {
    return {
      kind: 'flow',
      operationName: operation.name,
      artifacts: operation.artifacts,
      signatures: nonEmpty(
        receipts.map((receipt) => receipt.signature),
        `Operation '${operation.name}' signatures`
      ),
      transactions: nonEmpty(receipts, `Operation '${operation.name}' receipts`),
      ...callbackErrorResult,
    } as OperationReceiptFor<TPrepared>;
  }
  return {
    kind: operation.kind,
    operationName: operation.name,
    artifacts: operation.artifacts,
    signatures: [receipts[0]!.signature],
    transaction: receipts[0]!,
    ...callbackErrorResult,
  } as unknown as OperationReceiptFor<TPrepared>;
}

/**
 * Recursively unwrap operation context while retaining structured transaction outcomes.
 */
export function unwrapOperationExecutionError(
  error: unknown
): InstructionError | TransactionFailureOutcome | unknown {
  if (error instanceof InstructionError) {
    return error;
  }
  if (error instanceof TransactionExecutionError) {
    return error.outcome;
  }
  if (error instanceof OperationExecutionError) {
    const underlying = unwrapOperationExecutionError(error.cause);
    return underlying instanceof InstructionError ? underlying : error.outcome;
  }
  return error;
}

function convertToJsonValue(
  value: unknown,
  ancestors: Set<object>
): JsonValue | undefined {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : null;
  }
  if (typeof value === 'bigint') {
    return value.toString();
  }
  if (
    value === undefined
    || typeof value === 'function'
    || typeof value === 'symbol'
  ) {
    return undefined;
  }
  if (typeof value !== 'object') {
    return String(value);
  }
  if (ancestors.has(value)) {
    throw new Error('Cannot convert a circular value to JSON');
  }

  ancestors.add(value);
  try {
    if (ArrayBuffer.isView(value)) {
      return Array.from(
        new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
      );
    }
    if (value instanceof ArrayBuffer) {
      return Array.from(new Uint8Array(value));
    }
    if (Array.isArray(value)) {
      return value.map((entry) => convertToJsonValue(entry, ancestors) ?? null);
    }
    if (value instanceof Set) {
      return [...value].map((entry) => convertToJsonValue(entry, ancestors) ?? null);
    }
    if (value instanceof Map) {
      const result: Record<string, JsonValue> = {};
      for (const [key, entry] of value) {
        const converted = convertToJsonValue(entry, ancestors);
        if (converted !== undefined) {
          result[String(key)] = converted;
        }
      }
      return result;
    }

    const toJSON = (value as { toJSON?: () => unknown }).toJSON;
    if (typeof toJSON === 'function') {
      return convertToJsonValue(toJSON.call(value), ancestors) ?? null;
    }

    const result: Record<string, JsonValue> = {};
    for (const [key, entry] of Object.entries(value)) {
      const converted = convertToJsonValue(entry, ancestors);
      if (converted !== undefined) {
        result[key] = converted;
      }
    }
    return result;
  } finally {
    ancestors.delete(value);
  }
}

export function toJsonValue(value: unknown): JsonValue {
  return convertToJsonValue(value, new Set()) ?? null;
}

export function describePreparedOperation(
  operation: PreparedOperation
): PreparedOperationDescription {
  return {
    kind: operation.kind,
    name: operation.name,
    artifacts: toJsonValue(operation.artifacts),
    transactions: operation.plan.transactions.map((transaction) => ({
      name: transaction.name,
      requiredSignerAddresses: transaction.requiredSignerAddresses,
      errors: transaction.errors,
      instructions: transaction.instructions.map((instruction) => ({
        programId: instruction.programId,
        keys: instruction.keys,
        data: Array.from(instruction.data),
      })),
    })),
  };
}

/**
 * Inspect one prepared instruction/transaction without signing or submission.
 * Multi-transaction flows are intentionally unsupported.
 */
export async function inspectPreparedOperation(
  wallet: WalletAdapter | undefined,
  operation: PreparedOperation,
  options?: TransactionInspectionOptions,
  context?: WalletExecutionContext
): Promise<OperationInspection> {
  if (operation.plan.transactions.length !== 1) {
    throw new Error(
      `Cannot inspect operation '${operation.name}': multi-transaction operation inspection is not supported`
    );
  }
  if (operation.kind === 'flow') {
    throw new Error(`Cannot inspect flow '${operation.name}': flow inspection is not supported`);
  }
  if (!wallet?.inspectTransaction) {
    throw new Error('Wallet adapter does not support unsigned transaction inspection');
  }
  resolveTransactionBuildOptions(options, wallet);

  const transaction = operation.plan.transactions[0]!;
  const description = describePreparedOperation(operation);
  const inspection = context
    ? await wallet.inspectTransaction(transaction.instructions, options, context)
    : await wallet.inspectTransaction(transaction.instructions, options);
  return {
    description,
    transaction: inspection,
    programError: parseInstructionError(inspection.error ?? inspection, transaction.errors),
  };
}

export function formatPreparedOperation(operation: PreparedOperation): string {
  const lines = [
    `${operation.kind}: ${operation.name}`,
    `Transactions: ${operation.plan.transactions.length}`,
  ];
  for (const [index, transaction] of operation.plan.transactions.entries()) {
    lines.push(
      `  ${index + 1}. ${transaction.name} (${transaction.instructions.length} instruction${transaction.instructions.length === 1 ? '' : 's'})`
    );
    if (transaction.requiredSignerAddresses.length > 0) {
      lines.push(`    Signers: ${transaction.requiredSignerAddresses.join(', ')}`);
    }
  }
  return lines.join('\n');
}
