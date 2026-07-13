import type {
  PreparedFlow,
  PreparedInstruction,
  PreparedTransaction,
} from './operations';

export interface InstructionOperation<TInput = unknown, TArtifacts = void> {
  readonly kind: 'instruction';
  prepare(input: TInput): Promise<PreparedInstruction<TArtifacts>>;
}

export interface TransactionOperation<TInput = unknown, TArtifacts = void> {
  readonly kind: 'transaction';
  prepare(input: TInput): Promise<PreparedTransaction<TArtifacts>>;
}

export interface FlowOperation<TInput = unknown, TArtifacts = void> {
  readonly kind: 'flow';
  prepare(input: TInput): Promise<PreparedFlow<TArtifacts>>;
}

export type AnyOperation =
  | InstructionOperation<any, any>
  | TransactionOperation<any, any>
  | FlowOperation<any, any>;

export type OperationNamespace<TOperation extends AnyOperation = AnyOperation> = {
  readonly [key: string]: TOperation | OperationNamespace<TOperation>;
};

export type InstructionOperationNamespace = OperationNamespace<InstructionOperation<any, any>>;
export type TransactionOperationNamespace = OperationNamespace<TransactionOperation<any, any>>;
export type FlowOperationNamespace = OperationNamespace<FlowOperation<any, any>>;

type MaybePromise<T> = T | Promise<T>;

export function instructionOperation<TInput, TArtifacts>(
  prepare: (input: TInput) => MaybePromise<PreparedInstruction<TArtifacts>>
): InstructionOperation<TInput, TArtifacts> {
  return {
    kind: 'instruction',
    prepare: async (input) => prepare(input),
  };
}

export function transactionOperation<TInput, TArtifacts>(
  prepare: (input: TInput) => MaybePromise<PreparedTransaction<TArtifacts>>
): TransactionOperation<TInput, TArtifacts> {
  return {
    kind: 'transaction',
    prepare: async (input) => prepare(input),
  };
}

export function flowOperation<TInput, TArtifacts>(
  prepare: (input: TInput) => MaybePromise<PreparedFlow<TArtifacts>>
): FlowOperation<TInput, TArtifacts> {
  return {
    kind: 'flow',
    prepare: async (input) => prepare(input),
  };
}
