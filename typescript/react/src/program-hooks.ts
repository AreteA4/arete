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
import type {
  MutationExecutionObserver,
  MutationExecutor,
  UseMutationOptions,
  UseMutationResult,
} from './hooks/use-mutation';
import { useInstructionMutation } from './hooks/use-mutation';
import {
  createProcessedSlotReconciliation,
  type ProcessedSlotClient,
} from './reconciliation';

type OperationOptionsArg<TClient> = TClient extends { execute(prepared: PreparedOperation, options?: infer TOptions): Promise<unknown> }
  ? NonNullable<TOptions>
  : Record<string, unknown>;

type RawOptionsArg<TClient> = TClient extends { transaction(instructions: readonly unknown[], options?: infer TOptions): Promise<unknown> }
  ? NonNullable<TOptions>
  : Record<string, unknown>;

type MutationHookFactory = <
  TParams,
  TResult,
  TOptions extends object,
  TPrepared extends PreparedOperation = PreparedOperation,
>(
  execute: MutationExecutor<TParams, TResult, TOptions, TPrepared>,
  options?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>
) => UseMutationResult<TParams, TResult, TOptions, TPrepared>;

type ConnectedOperationLike = {
  prepare(input: unknown): Promise<PreparedOperation>;
};

type ExecutionClient = {
  transaction(instructions: readonly unknown[], options?: unknown): Promise<unknown>;
  execute(prepared: PreparedOperation, options?: unknown): Promise<unknown>;
};

function composeCallbacks<T>(
  observerCallback: ((value: T) => void) | undefined,
  callerCallback: ((value: T) => void | Promise<void>) | undefined,
): ((value: T) => Promise<void>) | undefined {
  if (!observerCallback && !callerCallback) return undefined;
  return async (value) => {
    const errors: unknown[] = [];
    for (const callback of [observerCallback, callerCallback]) {
      if (!callback) continue;
      try {
        await callback(value);
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length > 0) throw errors[0];
  };
}

/**
 * Wrap the mutation factory so every generated program hook reconciles against
 * the stream by default: after confirmation, wait until the stack has
 * processed the transaction's slot. Callers opt out per hook or per submit
 * with `reconcile: false`, replace it with a function, or tweak it with a
 * `{ refresh, timeoutMs }` shorthand object.
 */
function withDefaultReconciliation(
  createMutationHook: MutationHookFactory,
  client: (ExecutionClient & Partial<ProcessedSlotClient>) | null,
): MutationHookFactory {
  const defaultReconcile = client && typeof client.waitForProcessedSlot === 'function'
    ? createProcessedSlotReconciliation(client as ProcessedSlotClient)
    : undefined;
  if (!defaultReconcile) {
    return createMutationHook;
  }
  const factory: MutationHookFactory = (execute, options) => {
    const callerReconcile = options?.reconcile;
    let reconcile = callerReconcile;
    if (callerReconcile !== false && typeof callerReconcile !== 'function') {
      reconcile = (callerReconcile
        ? defaultReconcile.withOverrides(callerReconcile)
        : defaultReconcile) as typeof callerReconcile;
    }
    return createMutationHook(execute, { ...options, reconcile } as typeof options);
  };
  return factory;
}

export type RawInstructionHookFor<THandler, TClient> = THandler extends TypedInstruction<infer P, infer E>
  ? {
      useMutation: (
        options?: Partial<UseMutationOptions<RawOptionsArg<TClient>, E>>
      ) => UseMutationResult<P, E, RawOptionsArg<TClient>>;
      execute: (input: P, options?: RawOptionsArg<TClient>) => Promise<E>;
      build: THandler['build'];
    }
  : never;

export type OperationHookFor<TOperation, TClient> = TOperation extends {
  prepare(input: infer P): Promise<infer TPrepared extends PreparedOperation>;
}
  ? {
      useMutation: (
        options?: Partial<UseMutationOptions<
          OperationOptionsArg<TClient>,
          OperationReceiptFor<TPrepared>,
          TPrepared
        >>
      ) => UseMutationResult<
        P,
        OperationReceiptFor<TPrepared>,
        OperationOptionsArg<TClient>,
        TPrepared
      >;
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
  const execute = async (
    input: TParams,
    options?: RawOptionsArg<TClient>,
    observer?: MutationExecutionObserver
  ) => {
    if (!client) {
      throw new Error('Arete client is not connected');
    }
    const built = instruction.build(input);
    observer?.onAwaitingWallet();
    return client.transaction([built], options as TransactionOptions);
  };

  return {
    execute,
    build: instruction.build,
    useMutation: (options) => createMutationHook(
      execute as MutationExecutor<TParams, TResult, RawOptionsArg<TClient>>,
      options
    ),
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
  type TPrepared = Awaited<ReturnType<TOperation['prepare']>>;
  const execute = async (
    input: Parameters<TOperation['prepare']>[0],
    options?: OperationOptionsArg<TClient>,
    observer?: MutationExecutionObserver<TPrepared>
  ) => {
    if (!client) {
      throw new Error('Arete client is not connected');
    }
    const prepared = await operation.prepare(input) as TPrepared;
    observer?.onPrepared(prepared);
    observer?.onAwaitingWallet();
    const executionOptions = options as OperationExecutionOptions<unknown, TPrepared> | undefined;
    return client.execute(prepared, {
      ...(executionOptions ?? {}),
      onTransactionStart: composeCallbacks(
        observer ? (event) => observer.onTransactionStart(event) : undefined,
        executionOptions?.onTransactionStart,
      ),
      onTransactionSuccess: composeCallbacks(
        observer ? (event) => observer.onTransactionSuccess(event) : undefined,
        executionOptions?.onTransactionSuccess,
      ),
      onCallbackError: composeCallbacks(
        observer ? (error) => observer.onCallbackError(error) : undefined,
        executionOptions?.onCallbackError,
      ),
    } satisfies OperationExecutionOptions<unknown, TPrepared>);
  };

  return {
    prepare: operation.prepare.bind(operation),
    execute,
    useMutation: (options) => createMutationHook(
      execute as MutationExecutor<any, any, OperationOptionsArg<TClient>, TPrepared>,
      options
    ),
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

const NOT_CONNECTED_ERROR = 'Arete client is not connected';

function useDisconnectedMutation() {
  return useInstructionMutation(async () => {
    throw new Error(NOT_CONNECTED_ERROR);
  });
}

/**
 * Placeholder program hooks used before the client connects. Every namespace
 * path resolves (so `arete.programs.ore.transactions.mining.deploy.useMutation()`
 * is safe to call unconditionally) and every mutation hook keeps React state,
 * but preparing, executing, or submitting throws "not connected" — matching
 * what the real wrappers do. This lets components render their full tree
 * while the stack is offline instead of gating on `arete.client`.
 */
export function buildDisconnectedProgramHooks(): Record<string, never> {
  const notConnected = () => {
    throw new Error(NOT_CONNECTED_ERROR);
  };
  const leaf = Object.assign(notConnected, {
    prepare: notConnected,
    execute: notConnected,
    send: notConnected,
    resolve: notConnected,
    plan: notConnected,
    build: notConnected,
    stage: notConnected,
    useMutation: () => useDisconnectedMutation(),
  });
  // eslint-disable-next-line prefer-const
  let proxy: Record<string, never>;
  proxy = new Proxy(leaf as unknown as Record<string, never>, {
    get(target, property) {
      if (typeof property === 'string' && !(property in target)) {
        return proxy;
      }
      return Reflect.get(target, property);
    },
  }) as Record<string, never>;
  return proxy;
}

export function buildProgramHookInterfaces<TClient extends ExecutionClient>(
  programs: ProgramsInterface<Record<string, ProgramSdkDefinition>> | undefined,
  client: TClient | null,
  createMutationHook: MutationHookFactory,
  options: { defaultReconciliation?: boolean } = {},
): Record<string, ProgramHookInterface<ProgramSdkDefinition, TClient>> {
  const wrappedPrograms: Record<string, ProgramHookInterface<ProgramSdkDefinition, TClient>> = {};
  if (!programs) {
    return wrappedPrograms;
  }

  const mutationHookFactory = options.defaultReconciliation === false
    ? createMutationHook
    : withDefaultReconciliation(
        createMutationHook,
        client as ExecutionClient & Partial<ProcessedSlotClient>,
      );

  for (const [name, program] of Object.entries(programs)) {
    const raw: Record<string, unknown> = {};
    for (const [instructionName, instruction] of Object.entries(program.raw)) {
      raw[instructionName] = wrapRawInstruction(
        instruction as TypedInstruction<Record<string, unknown>, unknown>,
        client,
        mutationHookFactory,
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
      math: program.math,
      read: program.read,
      raw: raw as ProgramHookInterface<ProgramSdkDefinition, TClient>['raw'],
      instructions: wrapMaybeOperationNamespace(program.instructions, client, mutationHookFactory) as ProgramHookInterface<ProgramSdkDefinition, TClient>['instructions'],
      transactions: wrapMaybeOperationNamespace(program.transactions, client, mutationHookFactory) as ProgramHookInterface<ProgramSdkDefinition, TClient>['transactions'],
      flows: wrapMaybeOperationNamespace(program.flows, client, mutationHookFactory) as ProgramHookInterface<ProgramSdkDefinition, TClient>['flows'],
    };
  }

  return wrappedPrograms;
}
