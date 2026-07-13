import type {
  OperationExecutionOptions,
  OperationReceiptFor,
  PreparedOperation,
  ProgramInterface,
  ProgramSdkDefinition,
  ProgramsInterface,
  TransactionOptions,
  TypedInstruction,
} from '@usearete/sdk';
import type { MutationExecutor, UseMutationResult } from './hooks/use-mutation';

type OperationOptionsArg<TClient> = TClient extends { execute(prepared: PreparedOperation, options?: infer TOptions): Promise<unknown> }
  ? NonNullable<TOptions>
  : Record<string, never>;

type RawOptionsArg<TClient> = TClient extends { transaction(instructions: readonly unknown[], options?: infer TOptions): Promise<unknown> }
  ? NonNullable<TOptions>
  : Record<string, never>;

type MutationHookFactory = <TParams, TResult, TOptions extends object>(
  execute: MutationExecutor<TParams, TResult, TOptions>
) => UseMutationResult<TParams, TResult, TOptions>;

type ConnectedOperationLike = {
  prepare(input: unknown): Promise<PreparedOperation>;
};

type ExecutionClient = {
  transaction(instructions: readonly unknown[], options?: unknown): Promise<unknown>;
  execute(prepared: PreparedOperation, options?: unknown): Promise<unknown>;
};

export type RawInstructionHookFor<THandler, TClient> = THandler extends TypedInstruction<infer P, infer E>
  ? {
      useMutation: () => UseMutationResult<P, E, RawOptionsArg<TClient>>;
      execute: (input: P, options?: RawOptionsArg<TClient>) => Promise<E>;
      build: THandler['build'];
    }
  : never;

export type OperationHookFor<TOperation, TClient> = TOperation extends {
  prepare(input: infer P): Promise<infer TPrepared extends PreparedOperation>;
}
  ? {
      useMutation: () => UseMutationResult<P, OperationReceiptFor<TPrepared>, OperationOptionsArg<TClient>>;
      execute: (input: P, options?: OperationOptionsArg<TClient>) => Promise<OperationReceiptFor<TPrepared>>;
      prepare: TOperation['prepare'];
    }
  : never;

export type OperationHookNamespace<TNamespace, TClient> = {
  [K in keyof TNamespace]: TNamespace[K] extends ConnectedOperationLike
    ? OperationHookFor<TNamespace[K], TClient>
    : TNamespace[K] extends Record<string, unknown>
      ? OperationHookNamespace<TNamespace[K], TClient>
      : never;
};

export type ProgramHookInterface<
  TProgram extends ProgramSdkDefinition,
  TClient,
> = Omit<
  ProgramInterface<TProgram>,
  'raw' | 'instructions' | 'transactions' | 'flows'
> & {
  raw: {
    [K in keyof ProgramInterface<TProgram>['raw']]: RawInstructionHookFor<
      ProgramInterface<TProgram>['raw'][K],
      TClient
    >;
  };
  instructions: OperationHookNamespace<ProgramInterface<TProgram>['instructions'], TClient>;
  transactions: OperationHookNamespace<ProgramInterface<TProgram>['transactions'], TClient>;
  flows: OperationHookNamespace<ProgramInterface<TProgram>['flows'], TClient>;
};

export type BuildProgramInterface<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined,
  TClient,
> = TPrograms extends Record<string, ProgramSdkDefinition>
  ? { [K in keyof TPrograms]: ProgramHookInterface<TPrograms[K], TClient> }
  : Record<string, never>;

function wrapRawInstruction<TParams, TResult, TClient extends ExecutionClient>(
  instruction: TypedInstruction<TParams, TResult>,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
): RawInstructionHookFor<TypedInstruction<TParams, TResult>, TClient> {
  const execute = async (input: TParams, options?: RawOptionsArg<TClient>) => {
    if (!client) {
      throw new Error('Arete client is not connected');
    }
    return client.transaction([instruction.build(input)], options as TransactionOptions);
  };

  return {
    execute,
    build: instruction.build,
    useMutation: () => createMutationHook(execute as MutationExecutor<TParams, TResult, RawOptionsArg<TClient>>),
  } as RawInstructionHookFor<TypedInstruction<TParams, TResult>, TClient>;
}

function isConnectedOperation(value: unknown): value is ConnectedOperationLike {
  return Boolean(
    value && typeof value === 'object' && typeof (value as ConnectedOperationLike).prepare === 'function'
  );
}

function wrapOperation<TOperation extends ConnectedOperationLike, TClient extends ExecutionClient>(
  operation: TOperation,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
): OperationHookFor<TOperation, TClient> {
  const execute = async (input: Parameters<TOperation['prepare']>[0], options?: OperationOptionsArg<TClient>) => {
    if (!client) {
      throw new Error('Arete client is not connected');
    }
    const prepared = await operation.prepare(input);
    return client.execute(prepared, options as OperationExecutionOptions);
  };

  return {
    prepare: operation.prepare.bind(operation),
    execute,
    useMutation: () => createMutationHook(execute as MutationExecutor<any, any, OperationOptionsArg<TClient>>),
  } as OperationHookFor<TOperation, TClient>;
}

function wrapOperationNamespace<TNamespace extends Record<string, unknown>, TClient extends ExecutionClient>(
  namespace: TNamespace,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
): OperationHookNamespace<TNamespace, TClient> {
  const wrapped: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(namespace)) {
    if (isConnectedOperation(value)) {
      wrapped[name] = wrapOperation(value, client, createMutationHook);
      continue;
    }
    if (value && typeof value === 'object') {
      wrapped[name] = wrapOperationNamespace(value as Record<string, unknown>, client, createMutationHook);
    }
  }
  return wrapped as OperationHookNamespace<TNamespace, TClient>;
}

function wrapMaybeOperationNamespace<TClient extends ExecutionClient>(
  namespace: Record<string, unknown> | undefined,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
) {
  return namespace
    ? wrapOperationNamespace(namespace, client, createMutationHook)
    : {};
}

export function buildProgramHookInterfaces<TClient extends ExecutionClient>(
  programs: ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
): Record<string, ProgramHookInterface<ProgramSdkDefinition, TClient>> {
  const wrappedPrograms: Record<string, ProgramHookInterface<ProgramSdkDefinition, TClient>> = {};
  if (!programs) {
    return wrappedPrograms;
  }

  for (const [name, program] of Object.entries(programs)) {
    const raw: Record<string, unknown> = {};
    for (const [instructionName, instruction] of Object.entries(program.raw)) {
      raw[instructionName] = wrapRawInstruction(
        instruction as TypedInstruction<Record<string, unknown>, unknown>,
        client,
        createMutationHook,
      );
    }

    wrappedPrograms[name] = {
      name: program.name,
      programId: program.programId,
      schemas: program.schemas,
      pdas: program.pdas,
      accounts: program.accounts,
      queries: program.queries,
      addresses: program.addresses,
      constants: program.constants,
      defaults: program.defaults,
      raw: raw as ProgramHookInterface<ProgramSdkDefinition, TClient>['raw'],
      instructions: wrapMaybeOperationNamespace(program.instructions, client, createMutationHook) as ProgramHookInterface<ProgramSdkDefinition, TClient>['instructions'],
      transactions: wrapMaybeOperationNamespace(program.transactions, client, createMutationHook) as ProgramHookInterface<ProgramSdkDefinition, TClient>['transactions'],
      flows: wrapMaybeOperationNamespace(program.flows, client, createMutationHook) as ProgramHookInterface<ProgramSdkDefinition, TClient>['flows'],
    };
  }

  return wrappedPrograms;
}
