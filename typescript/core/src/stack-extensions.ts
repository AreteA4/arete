import type { ChainClient } from './chain';
import type { Arete, ProgramInterface } from './client';
import type {
  FlowOperationNamespace,
  InstructionOperationNamespace,
  TransactionOperationNamespace,
} from './program-instructions';
import type { ProgramSdkDefinition, StackDefinition } from './types';
import type { WalletAdapter } from './wallet/types';

export const STACK_RUNTIME_EXTENSIONS = '__areteStackRuntimeExtensions' as const;
export const PROGRAM_OPERATION_EXTENSIONS = '__areteProgramOperationExtensions' as const;

type EmptyRecord = Record<string, never>;

/** Fixed arity, or `[required, total]` when a read has optional arguments. */
export type ReadArgumentCount = number | readonly [required: number, total: number];

type RequiredArgumentCount<
  TArgs extends readonly unknown[],
  TCount extends readonly unknown[] = [],
> = TArgs extends readonly [infer THead, ...infer TTail]
  ? undefined extends THead
    ? TCount['length']
    : RequiredArgumentCount<TTail, readonly [...TCount, unknown]>
  : TCount['length'];

type ReadArgumentCountFor<TRead> = TRead extends (...args: infer TArgs) => unknown
  ? number extends Required<TArgs>['length']
    ? ReadArgumentCount
    : RequiredArgumentCount<TArgs> extends Required<TArgs>['length']
      ? Required<TArgs>['length']
      : readonly [
          required: RequiredArgumentCount<TArgs>,
          total: Required<TArgs>['length'],
        ]
  : never;

export type ReadArgumentCounts<TRead = Record<string, unknown>> = {
  readonly [K in keyof TRead]: ReadArgumentCountFor<TRead[K]>;
};

type MaybeField<TKey extends string, TValue> = [TValue] extends [never]
  ? {}
  : { readonly [K in TKey]: TValue };

type Field<TValue, TKey extends PropertyKey> = TKey extends keyof TValue
  ? TValue[TKey]
  : never;

type DeepMerge<TBase, TExtension> =
  TBase extends Record<string, unknown>
    ? TExtension extends Record<string, unknown>
      ? Omit<TBase, keyof TExtension> & {
          readonly [K in keyof TExtension]: K extends keyof TBase
            ? DeepMerge<TBase[K], TExtension[K]>
            : TExtension[K];
        }
      : TExtension
    : TExtension;

type MergeField<TBase, TExtension> = [TBase] extends [never]
  ? TExtension
  : [TExtension] extends [never]
    ? TBase
    : DeepMerge<TBase, TExtension>;

type FactoryReturn<TValue, TKey extends PropertyKey> =
  TKey extends keyof TValue
    ? NonNullable<TValue[TKey]> extends (...args: any[]) => infer TResult
      ? TResult
      : never
    : never;

export interface ProgramOperations<
  TInstructions extends InstructionOperationNamespace | EmptyRecord = EmptyRecord,
  TTransactions extends TransactionOperationNamespace | EmptyRecord = EmptyRecord,
  TFlows extends FlowOperationNamespace | EmptyRecord = EmptyRecord,
> {
  readonly instructions?: TInstructions;
  readonly transactions?: TTransactions;
  readonly flows?: TFlows;
}

type AnyProgramOperations = ProgramOperations<
  InstructionOperationNamespace,
  TransactionOperationNamespace,
  FlowOperationNamespace
>;

type InstructionOperationsOf<TOperations> = TOperations extends ProgramOperations<
  infer TInstructions,
  any,
  any
> ? TInstructions : EmptyRecord;

type TransactionOperationsOf<TOperations> = TOperations extends ProgramOperations<
  any,
  infer TTransactions,
  any
> ? TTransactions : EmptyRecord;

type FlowOperationsOf<TOperations> = TOperations extends ProgramOperations<
  any,
  any,
  infer TFlows
> ? TFlows : EmptyRecord;

export interface ProgramOperationContext<
  TProgram extends ProgramSdkDefinition = ProgramSdkDefinition,
> {
  readonly chain: ChainClient;
  readonly wallet: WalletAdapter | undefined;
  readonly program: ProgramInterface<TProgram>;
}

export interface ProgramRuntimeExtensions<
  TOperations extends AnyProgramOperations = ProgramOperations,
  TProgram extends ProgramSdkDefinition = ProgramSdkDefinition,
> {
  readonly createOperations: (
    context: ProgramOperationContext<TProgram>
  ) => TOperations;
}

export interface ProgramRuntimeExtensionCarrier<
  TOperations extends AnyProgramOperations = ProgramOperations,
> {
  readonly [PROGRAM_OPERATION_EXTENSIONS]?: ProgramRuntimeExtensions<TOperations, any>;
}

export type ProgramOperationsOf<TProgram> =
  TProgram extends ProgramRuntimeExtensionCarrier<infer TOperations>
    ? TOperations
    : ProgramOperations;

type MergeProgramOperations<
  TBase extends AnyProgramOperations,
  TExtension extends AnyProgramOperations,
> = ProgramOperations<
  Extract<
    MergeField<InstructionOperationsOf<TBase>, InstructionOperationsOf<TExtension>>,
    InstructionOperationNamespace | EmptyRecord
  >,
  Extract<
    MergeField<TransactionOperationsOf<TBase>, TransactionOperationsOf<TExtension>>,
    TransactionOperationNamespace | EmptyRecord
  >,
  Extract<
    MergeField<FlowOperationsOf<TBase>, FlowOperationsOf<TExtension>>,
    FlowOperationNamespace | EmptyRecord
  >
>;

export type ExtendedProgramDefinition<
  TBase extends ProgramSdkDefinition,
  TAddresses = never,
  TConstants = never,
  TDefaults = never,
  TOperations extends AnyProgramOperations = ProgramOperations,
  TMath = never,
> = Omit<TBase, 'definitionHash'>
  & MaybeField<'addresses', TAddresses>
  & MaybeField<'constants', TConstants>
  & MaybeField<'defaults', TDefaults>
  & MaybeField<'math', TMath>
  & ProgramRuntimeExtensionCarrier<
      MergeProgramOperations<ProgramOperationsOf<TBase>, TOperations>
    >;

export interface ProgramExtensionInput<
  TAddresses = never,
  TConstants = never,
  TDefaults = never,
  TOperations extends AnyProgramOperations = ProgramOperations,
  TProgram extends ProgramSdkDefinition = ProgramSdkDefinition,
  TMath = never,
> {
  readonly pdas?: Record<string, unknown>;
  readonly accounts?: Record<string, unknown>;
  readonly queries?: Record<string, unknown>;
  readonly raw?: Record<string, unknown>;
  readonly addresses?: TAddresses;
  readonly constants?: TConstants;
  readonly defaults?: TDefaults;
  readonly math?: TMath;
  readonly createOperations?: (
    context: ProgramOperationContext<TProgram>
  ) => TOperations;
}

export function defineProgramExtensions<TBase extends ProgramSdkDefinition>() {
  return <
    TAddresses = never,
    TConstants = never,
    TDefaults = never,
    TOperations extends AnyProgramOperations = ProgramOperations,
    TMath = never,
  >(
    extensions: Omit<
      ProgramExtensionInput<any, any, any, any, any, any>,
      'addresses' | 'constants' | 'defaults' | 'math' | 'createOperations'
    > & {
      readonly addresses?: TAddresses;
      readonly constants?: TConstants;
      readonly defaults?: TDefaults;
      readonly math?: TMath;
      readonly createOperations?: (
        context: ProgramOperationContext<
          ExtendedProgramDefinition<
            TBase,
            TAddresses,
            TConstants,
            TDefaults,
            ProgramOperations,
            TMath
          >
        >
      ) => TOperations;
    }
  ): ProgramExtensionInput<
    TAddresses,
    TConstants,
    TDefaults,
    TOperations,
    ExtendedProgramDefinition<TBase, TAddresses, TConstants, TDefaults, ProgramOperations, TMath>,
    TMath
  > => extensions;
}

function mergeNamespace(base: unknown, extension: unknown): unknown {
  if (
    base && typeof base === 'object' && !Array.isArray(base)
    && extension && typeof extension === 'object' && !Array.isArray(extension)
  ) {
    const merged: Record<string, unknown> = { ...(base as Record<string, unknown>) };
    for (const [key, value] of Object.entries(extension as Record<string, unknown>)) {
      merged[key] = key in merged ? mergeNamespace(merged[key], value) : value;
    }
    return merged;
  }
  return extension;
}

function mergeOperations(
  base: AnyProgramOperations | undefined,
  extension: AnyProgramOperations | undefined
): AnyProgramOperations {
  return {
    instructions: mergeNamespace(base?.instructions, extension?.instructions) as InstructionOperationNamespace,
    transactions: mergeNamespace(base?.transactions, extension?.transactions) as TransactionOperationNamespace,
    flows: mergeNamespace(base?.flows, extension?.flows) as FlowOperationNamespace,
  };
}

export function extendProgram<
  TBase extends ProgramSdkDefinition,
  TExtension extends ProgramExtensionInput<any, any, any, any, any, any>,
>(
  program: TBase,
  extensions: TExtension
): ExtendedProgramDefinition<
  TBase,
  MergeField<Field<TBase, 'addresses'>, Field<TExtension, 'addresses'>>,
  MergeField<Field<TBase, 'constants'>, Field<TExtension, 'constants'>>,
  MergeField<Field<TBase, 'defaults'>, Field<TExtension, 'defaults'>>,
  Extract<FactoryReturn<TExtension, 'createOperations'>, AnyProgramOperations>,
  MergeField<Field<TBase, 'math'>, Field<TExtension, 'math'>>
> {
  const base = program as Record<PropertyKey, unknown> & ProgramRuntimeExtensionCarrier;
  const extended = { ...program } as Record<PropertyKey, unknown>;
  delete extended.definitionHash;

  for (const key of ['pdas', 'accounts', 'queries', 'addresses', 'constants', 'defaults', 'math'] as const) {
    const extensionValue = extensions[key];
    if (extensionValue !== undefined) {
      extended[key] = mergeNamespace(base[key], extensionValue);
    }
  }
  if (extensions.raw !== undefined) {
    extended.rawInstructions = mergeNamespace(base.rawInstructions, extensions.raw);
  }

  const baseFactory = base[PROGRAM_OPERATION_EXTENSIONS]?.createOperations;
  const extensionFactory = extensions.createOperations;
  if (baseFactory || extensionFactory) {
    Object.defineProperty(extended, PROGRAM_OPERATION_EXTENSIONS, {
      value: {
        createOperations(
          context: ProgramOperationContext<
            ExtendedProgramDefinition<
              TBase,
              MergeField<Field<TBase, 'addresses'>, Field<TExtension, 'addresses'>>,
              MergeField<Field<TBase, 'constants'>, Field<TExtension, 'constants'>>,
              MergeField<Field<TBase, 'defaults'>, Field<TExtension, 'defaults'>>,
              ProgramOperations,
              MergeField<Field<TBase, 'math'>, Field<TExtension, 'math'>>
            >
          >
        ) {
          const baseOperations = baseFactory?.(context);
          if (baseOperations) {
            const connectedProgram = context.program as unknown as {
              instructions: InstructionOperationNamespace;
              transactions: TransactionOperationNamespace;
              flows: FlowOperationNamespace;
            };
            connectedProgram.instructions = mergeNamespace(
              connectedProgram.instructions,
              baseOperations.instructions
            ) as InstructionOperationNamespace;
            connectedProgram.transactions = mergeNamespace(
              connectedProgram.transactions,
              baseOperations.transactions
            ) as TransactionOperationNamespace;
            connectedProgram.flows = mergeNamespace(
              connectedProgram.flows,
              baseOperations.flows
            ) as FlowOperationNamespace;
          }
          return mergeOperations(baseOperations, extensionFactory?.(context));
        },
      },
      enumerable: false,
      configurable: false,
      writable: false,
    });
  }

  return extended as ExtendedProgramDefinition<
    TBase,
    MergeField<Field<TBase, 'addresses'>, Field<TExtension, 'addresses'>>,
    MergeField<Field<TBase, 'constants'>, Field<TExtension, 'constants'>>,
    MergeField<Field<TBase, 'defaults'>, Field<TExtension, 'defaults'>>,
    Extract<FactoryReturn<TExtension, 'createOperations'>, AnyProgramOperations>,
    MergeField<Field<TBase, 'math'>, Field<TExtension, 'math'>>
  >;
}

export function extendPrograms<
  TPrograms extends Record<string, ProgramSdkDefinition>,
  TExtensions extends Partial<{
    [K in keyof TPrograms]: ProgramExtensionInput<any, any, any, any, any, any>;
  }>,
>(
  programs: TPrograms,
  extensions: TExtensions
): {
  readonly [K in keyof TPrograms]: K extends keyof TExtensions
    ? NonNullable<TExtensions[K]> extends ProgramExtensionInput<
        infer TAddresses,
        infer THelpers,
        infer TTypes,
        infer TOperations,
        any,
        infer TMath
      >
      ? ExtendedProgramDefinition<TPrograms[K], TAddresses, THelpers, TTypes, TOperations, TMath>
      : TPrograms[K]
    : TPrograms[K];
} {
  const merged: Record<string, ProgramSdkDefinition> = { ...programs };
  for (const [name, program] of Object.entries(programs)) {
    const extension = extensions[name as keyof TExtensions];
    merged[name] = extension
      ? extendProgram(program, extension as ProgramExtensionInput<any, any, any, any, any, any>)
      : program;
  }
  return merged as any;
}

export function getProgramRuntimeExtensions<TProgram extends ProgramSdkDefinition>(
  program: TProgram
): ProgramRuntimeExtensions<ProgramOperationsOf<TProgram>, TProgram> | undefined {
  return (program as ProgramRuntimeExtensionCarrier<ProgramOperationsOf<TProgram>>)[
    PROGRAM_OPERATION_EXTENSIONS
  ] as ProgramRuntimeExtensions<ProgramOperationsOf<TProgram>, TProgram> | undefined;
}

export interface StackRuntimeExtensions<
  TRead = EmptyRecord,
  TFlows extends FlowOperationNamespace | EmptyRecord = EmptyRecord,
  TClient = unknown,
> {
  readonly readArgCounts?: ReadArgumentCounts<TRead>;
  readonly createRead?: (client: TClient) => TRead;
  readonly createFlows?: (client: TClient) => TFlows;
}

export interface StackRuntimeExtensionCarrier<
  TRead = EmptyRecord,
  TFlows extends FlowOperationNamespace | EmptyRecord = EmptyRecord,
> {
  readonly [STACK_RUNTIME_EXTENSIONS]?: StackRuntimeExtensions<TRead, TFlows>;
}

export type ExtendedStackDefinition<
  TBase extends StackDefinition,
  TAddresses = never,
  TConstants = never,
  TDefaults = never,
  TMath = never,
  TRead = never,
  TFlows extends FlowOperationNamespace | EmptyRecord = never,
> = TBase
  & MaybeField<'addresses', TAddresses>
  & MaybeField<'constants', TConstants>
  & MaybeField<'defaults', TDefaults>
  & MaybeField<'math', TMath>
  & StackRuntimeExtensionCarrier<TRead, TFlows>;

export type StackConnectedExtensions<TStack> =
  MaybeField<'addresses', Field<TStack, 'addresses'>>
  & MaybeField<'constants', Field<TStack, 'constants'>>
  & MaybeField<'defaults', Field<TStack, 'defaults'>>
  & MaybeField<'math', Field<TStack, 'math'>>
  & (TStack extends StackRuntimeExtensionCarrier<infer TRead, infer TFlows>
    ? MaybeField<'read', TRead> & MaybeField<'flows', TFlows>
    : {});

export type ConnectedStackClient<TClient extends object, TStack> =
  TClient & StackConnectedExtensions<TStack>;

type StackReadOf<TStack> = TStack extends StackRuntimeExtensionCarrier<infer TRead, any>
  ? TRead
  : never;

type StackFlowsOf<TStack> = TStack extends StackRuntimeExtensionCarrier<any, infer TFlows>
  ? TFlows
  : never;

export interface StackExtensionInput<
  TAddresses = never,
  TConstants = never,
  TDefaults = never,
  TMath = never,
  TRead = never,
  TFlows extends FlowOperationNamespace | EmptyRecord = never,
  TClient = unknown,
> {
  readonly addresses?: TAddresses;
  readonly constants?: TConstants;
  readonly defaults?: TDefaults;
  readonly math?: TMath;
  readonly readArgCounts?: ReadArgumentCounts<TRead>;
  readonly createRead?: (client: TClient) => TRead;
  readonly createFlows?: (client: TClient) => TFlows;
}

type ReadArgumentCountRequirement<TExtension> =
  TExtension extends { readonly createRead: (...args: any[]) => infer TRead }
    ? { readonly readArgCounts: ReadArgumentCounts<TRead> }
    : {};

export type StackExtensionClient<
  TBase extends StackDefinition,
  TAddresses = never,
  TConstants = never,
  TDefaults = never,
  TMath = never,
> = Arete<TBase>
  & MaybeField<'addresses', TAddresses>
  & MaybeField<'constants', TConstants>
  & MaybeField<'defaults', TDefaults>
  & MaybeField<'math', TMath>;

export function defineStackExtensions<TBase extends StackDefinition>() {
  return <
    const TExtension extends StackExtensionInput<
      any,
      any,
      any,
      any,
      any,
      FlowOperationNamespace | EmptyRecord,
      any
    >,
  >(
    extensions: TExtension & ReadArgumentCountRequirement<TExtension> & {
      readonly createRead?: (
        client: StackExtensionClient<TBase, any, any, any, any>
      ) => unknown;
      readonly createFlows?: (
        client: StackExtensionClient<TBase, any, any, any, any>
      ) => FlowOperationNamespace | EmptyRecord;
    }
  ): TExtension => extensions;
}

export function extendStack<
  TBase extends StackDefinition,
  TExtension extends StackExtensionInput<any, any, any, any, any, any, any>,
>(
  stack: TBase,
  extensions: TExtension & ReadArgumentCountRequirement<TExtension>
): ExtendedStackDefinition<
  TBase,
  MergeField<Field<TBase, 'addresses'>, Field<TExtension, 'addresses'>>,
  MergeField<Field<TBase, 'constants'>, Field<TExtension, 'constants'>>,
  MergeField<Field<TBase, 'defaults'>, Field<TExtension, 'defaults'>>,
  MergeField<Field<TBase, 'math'>, Field<TExtension, 'math'>>,
  MergeField<StackReadOf<TBase>, FactoryReturn<TExtension, 'createRead'>>,
  Extract<
    MergeField<StackFlowsOf<TBase>, FactoryReturn<TExtension, 'createFlows'>>,
    FlowOperationNamespace | EmptyRecord
  >
> {
  const base = stack as Record<PropertyKey, unknown> & StackRuntimeExtensionCarrier;
  const extended = { ...stack } as Record<PropertyKey, unknown>;
  for (const key of ['addresses', 'constants', 'defaults', 'math'] as const) {
    if (extensions[key] !== undefined) {
      extended[key] = mergeNamespace(base[key], extensions[key]);
    }
  }
  const baseRuntime = base[STACK_RUNTIME_EXTENSIONS];
  if (baseRuntime || extensions.createRead || extensions.createFlows) {
    Object.defineProperty(extended, STACK_RUNTIME_EXTENSIONS, {
      value: {
        readArgCounts: baseRuntime?.readArgCounts && extensions.readArgCounts
          ? mergeNamespace(
              baseRuntime.readArgCounts,
              extensions.readArgCounts
            ) as ReadArgumentCounts
          : extensions.readArgCounts ?? baseRuntime?.readArgCounts,
        createRead: baseRuntime?.createRead && extensions.createRead
          ? (client: unknown) => mergeNamespace(
              baseRuntime.createRead!(client),
              extensions.createRead!(client)
            )
          : extensions.createRead ?? baseRuntime?.createRead,
        createFlows: baseRuntime?.createFlows && extensions.createFlows
          ? (client: unknown) => mergeNamespace(
              baseRuntime.createFlows!(client),
              extensions.createFlows!(client)
            )
          : extensions.createFlows ?? baseRuntime?.createFlows,
      },
      enumerable: false,
      configurable: false,
      writable: false,
    });
  }
  return extended as ExtendedStackDefinition<
    TBase,
    MergeField<Field<TBase, 'addresses'>, Field<TExtension, 'addresses'>>,
    MergeField<Field<TBase, 'constants'>, Field<TExtension, 'constants'>>,
    MergeField<Field<TBase, 'defaults'>, Field<TExtension, 'defaults'>>,
    MergeField<Field<TBase, 'math'>, Field<TExtension, 'math'>>,
    MergeField<StackReadOf<TBase>, FactoryReturn<TExtension, 'createRead'>>,
    Extract<
      MergeField<StackFlowsOf<TBase>, FactoryReturn<TExtension, 'createFlows'>>,
      FlowOperationNamespace | EmptyRecord
    >
  >;
}

export function getStackRuntimeExtensions<TStack extends StackDefinition>(
  stack: TStack
): StackRuntimeExtensions<StackReadOf<TStack>> | undefined {
  return (stack as StackRuntimeExtensionCarrier<StackReadOf<TStack>>)[STACK_RUNTIME_EXTENSIONS];
}

function defineClientField(target: object, key: string, value: unknown) {
  if (value === undefined || key in target) {
    return;
  }
  Object.defineProperty(target, key, {
    value,
    enumerable: true,
    configurable: true,
    writable: false,
  });
}

export function applyConnectedStackExtensions<TClient extends object, TStack extends StackDefinition>(
  client: TClient,
  stack: TStack
): ConnectedStackClient<TClient, TStack> {
  const extendedStack = stack as Record<string, unknown> & StackRuntimeExtensionCarrier;
  defineClientField(client, 'addresses', extendedStack.addresses);
  defineClientField(client, 'constants', extendedStack.constants);
  defineClientField(client, 'defaults', extendedStack.defaults);
  defineClientField(client, 'math', extendedStack.math);

  const runtime = getStackRuntimeExtensions(stack);
  defineClientField(client, 'flows', runtime?.createFlows?.(client));
  defineClientField(client, 'read', runtime?.createRead?.(client));
  return client as ConnectedStackClient<TClient, TStack>;
}
