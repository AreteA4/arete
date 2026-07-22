import {
  Arete,
  withPrograms,
  type ConnectedArete,
  type ConnectOptions,
  type ProgramInterface,
  type StackWithAttachedPrograms,
  type TransactionOptions,
} from './client';
import { createChainClient, type ChainClient } from './chain';
import { AreteError, type AuthConfig, type StackDefinition, type ProgramSdkDefinition } from './types';
import type { StorageAdapter } from './storage/adapter';
import type { WalletAdapter, BuiltInstruction } from './wallet/types';
import type { ExecutionResult } from './instructions';
import type {
  OperationExecutionOptions,
  OperationReceiptFor,
  PreparedOperation,
} from './operations';
import { createSignerRegistry, type SignerRegistry } from './signer-registry';
import type { TransactionTransport } from './transactions';

/**
 * A session composes multiple stack and standalone-program SDK clients
 * behind one wallet and one shared endpoint configuration.
 *
 * Each member gets its own client (connection + store); a stack is itself a
 * composition of programs + views, and a standalone program member reuses the
 * exact same machinery as a stack with no views, connected HTTP-only.
 */
export interface SessionDefinition {
  readonly stacks?: Record<string, StackDefinition>;
  readonly programs?: Record<string, ProgramSdkDefinition>;
}

type SessionStackPrograms<TDef extends SessionDefinition> = Partial<{
  [K in keyof NonNullable<TDef['stacks']>]: Record<string, ProgramSdkDefinition> | undefined;
}>;

type EffectiveStackPrograms<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
  K extends keyof NonNullable<TDef['stacks']>,
> = NonNullable<TDef['stacks']>[K] extends StackDefinition
  ? K extends keyof TStackPrograms
    ? StackWithAttachedPrograms<
        NonNullable<TDef['stacks']>[K],
        TStackPrograms[K]
      >['programs']
    : NonNullable<NonNullable<TDef['stacks']>[K]['programs']>
  : never;

type StackProgramKeys<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
> = {
  [K in keyof NonNullable<TDef['stacks']>]: keyof EffectiveStackPrograms<TDef, TStackPrograms, K>;
}[keyof NonNullable<TDef['stacks']>];

/** Per-member connection overrides (a subset of {@link ConnectOptions}). */
export interface SessionMemberOptions<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined = undefined,
> {
  url?: string;
  httpUrl?: string;
  transport?: 'ws' | 'http';
  auth?: AuthConfig;
  storage?: StorageAdapter;
  autoConnect?: boolean;
  autoReconnect?: boolean;
  programs?: TPrograms;
}

export interface SessionOptions<
  TDef extends SessionDefinition = SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
> {
  /** One wallet governs execution across every member. */
  wallet?: WalletAdapter;
  /** Canonical chain reader shared by the session when explicitly provided. */
  chain?: ChainClient;
  /** Transaction transport override; defaults to the execution host's authenticated client. */
  transactions?: TransactionTransport;
  auth?: AuthConfig;
  fetch?: typeof fetch;
  /** Default transport for all members ('ws' unless overridden). */
  transport?: 'ws' | 'http';
  /** Shared fallback endpoints used when a member defines none of its own. */
  endpoints?: { http?: string; ws?: string };
  /** Default execution settings shared by transaction/plan helpers on the session. */
  execution?: OperationExecutionOptions<any>;
  /** Signers available to every transaction executed through this session. */
  signerRegistry?: SignerRegistry<any>;
  /** Per-member overrides, keyed by the member's key in the definition. */
  stacks?: {
    [K in keyof NonNullable<TDef['stacks']>]?: SessionMemberOptions<
      K extends keyof TStackPrograms ? TStackPrograms[K] : undefined
    >;
  };
  programs?: Record<string, SessionMemberOptions>;
}

type SessionStacks<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
> = {
  readonly [K in keyof NonNullable<TDef['stacks']>]: NonNullable<TDef['stacks']>[K] extends StackDefinition
      ? K extends keyof TStackPrograms
        ? ConnectedArete<
          StackWithAttachedPrograms<
            NonNullable<TDef['stacks']>[K],
            TStackPrograms[K]
          >,
          NonNullable<TDef['stacks']>[K]
        >
        : ConnectedArete<
          NonNullable<TDef['stacks']>[K],
          NonNullable<TDef['stacks']>[K]
        >
    : never;
};

type StackProgramInterfaceForKey<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
  P extends PropertyKey,
> = {
  [K in keyof NonNullable<TDef['stacks']>]: P extends keyof SessionStacks<
    TDef,
    TStackPrograms
  >[K]['programs']
    ? SessionStacks<TDef, TStackPrograms>[K]['programs'][P]
    : never;
}[keyof NonNullable<TDef['stacks']>];

type PromotedSessionPrograms<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
> = {
  readonly [P in StackProgramKeys<TDef, TStackPrograms>]: StackProgramInterfaceForKey<
    TDef,
    TStackPrograms,
    P
  >;
};

type ExplicitSessionPrograms<TDef extends SessionDefinition> = {
  readonly [K in keyof NonNullable<TDef['programs']>]: NonNullable<TDef['programs']>[K] extends ProgramSdkDefinition
    ? ProgramInterface<NonNullable<TDef['programs']>[K]>
    : never;
};

type SessionPrograms<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
> = Omit<PromotedSessionPrograms<TDef, TStackPrograms>, keyof NonNullable<TDef['programs']>>
  & ExplicitSessionPrograms<TDef>;

export interface Session<
  TDef extends SessionDefinition = SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
> {
  readonly stacks: SessionStacks<TDef, TStackPrograms>;
  readonly programs: SessionPrograms<TDef, TStackPrograms>;
  readonly wallet: WalletAdapter | undefined;
  readonly signerRegistry: SignerRegistry<any>;
  readonly chain: ChainClient;
  readonly transactions: TransactionTransport;
  transaction(
    instructions: readonly BuiltInstruction[],
    options?: TransactionOptions
  ): Promise<ExecutionResult>;
  execute<TPrepared extends PreparedOperation, TSigner = unknown>(
    prepared: TPrepared,
    options?: OperationExecutionOptions<TSigner, TPrepared>
  ): Promise<OperationReceiptFor<TPrepared>>;
  setWallet(wallet: WalletAdapter | undefined): void;
  close(): void;
}

type SessionConnectionMemberOptions = Pick<
  SessionMemberOptions<Record<string, ProgramSdkDefinition>>,
  'url' | 'httpUrl' | 'transport' | 'auth' | 'storage' | 'autoConnect' | 'autoReconnect'
>;

type SessionConnectionOptions = Pick<
  SessionOptions,
  'wallet' | 'auth' | 'fetch' | 'transport' | 'endpoints'
>;

function resolveMemberConnectOptions(
  stack: StackDefinition,
  member: SessionConnectionMemberOptions | undefined,
  options: SessionConnectionOptions | undefined,
  forceHttpOnly: boolean
): ConnectOptions {
  const transport = member?.transport ?? options?.transport ?? (forceHttpOnly ? 'http' : 'ws');
  // Resolution order: per-member override → the stack's own endpoints →
  // session-wide fallback endpoints.
  const url = member?.url ?? (stack.endpoints.ws || options?.endpoints?.ws);
  const httpUrl = member?.httpUrl ?? stack.endpoints.http ?? options?.endpoints?.http;
  return {
    url,
    httpUrl,
    transport,
    auth: member?.auth ?? options?.auth,
    storage: member?.storage,
    autoConnect: member?.autoConnect,
    autoReconnect: member?.autoReconnect,
    wallet: options?.wallet,
    fetch: options?.fetch,
  };
}

function programAsStack(name: string, program: ProgramSdkDefinition): StackDefinition {
  return {
    name,
    endpoints: { ws: '' },
    views: {},
    programs: { [name]: program },
  };
}

export async function createSession<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
>(
  definition: TDef,
  options?: SessionOptions<TDef, TStackPrograms>
): Promise<Session<TDef, TStackPrograms>> {
  const stackEntries = Object.entries(definition.stacks ?? {});
  const programEntries = Object.entries(definition.programs ?? {});
  if (stackEntries.length === 0 && programEntries.length === 0) {
    throw new AreteError('createSession requires at least one stack or program member', 'INVALID_CONFIG');
  }

  let wallet = options?.wallet;
  const signerRegistry = options?.signerRegistry ?? createSignerRegistry();

  const connectedStacks = await Promise.all(
    stackEntries.map(async ([key, stack]) => {
      const memberOptions = options?.stacks?.[key as keyof NonNullable<TDef['stacks']>];
      const effectiveStack = withPrograms(
        stack,
        memberOptions?.programs as Record<string, ProgramSdkDefinition> | undefined
      );
      const connectOptions = resolveMemberConnectOptions(effectiveStack, memberOptions, options, false);
      const client = await Arete.connect(effectiveStack, connectOptions);
      return [key, client] as const;
    })
  );

  const connectedPrograms = await Promise.all(
    programEntries.map(async ([key, program]) => {
      const syntheticStack = programAsStack(key, program);
      const connectOptions = resolveMemberConnectOptions(syntheticStack, options?.programs?.[key], options, true);
      const client = await Arete.connect(syntheticStack, connectOptions);
      return [key, client] as const;
    })
  );

  const memberClients = [
    ...connectedStacks.map(([, client]) => client),
    ...connectedPrograms.map(([, client]) => client),
  ];
  const executionHost = memberClients[0]!;
  const transactions = options?.transactions ?? executionHost.transactions;

  const stacks = Object.fromEntries(connectedStacks) as SessionStacks<TDef, TStackPrograms>;
  const explicitProgramKeys = new Set(connectedPrograms.map(([key]) => key));
  const programOwners = new Map<string, { stack: string; programId?: string }>();
  const connectedProgramEntries = connectedPrograms.map(([key, client]) => [
    key,
    (client.programs as Record<string, ProgramInterface<ProgramSdkDefinition>>)[key],
  ] as const);
  const promotedPrograms = Object.fromEntries(connectedProgramEntries) as Record<
    string,
    ProgramInterface<ProgramSdkDefinition>
  >;

  for (const [stackKey, client] of connectedStacks) {
    for (const [programKey, program] of Object.entries(
      client.programs as Record<string, ProgramInterface<ProgramSdkDefinition>>
    )) {
      if (explicitProgramKeys.has(programKey)) {
        continue;
      }
      const existingOwner = programOwners.get(programKey);
      if (existingOwner) {
        console.warn(
          `Program '${programKey}' is bundled by stacks '${existingOwner.stack}'` +
          ` (${existingOwner.programId ?? 'unknown program ID'}) and '${stackKey}'` +
          ` (${program.programId ?? 'unknown program ID'}); session.programs.${programKey}` +
          ` uses '${existingOwner.stack}' because it was connected first`
        );
        continue;
      }
      promotedPrograms[programKey] = program;
      programOwners.set(programKey, { stack: stackKey, programId: program.programId });
    }
  }

  const programs = promotedPrograms as SessionPrograms<TDef, TStackPrograms>;

  const chain =
    options?.chain ?? (options?.endpoints?.http !== undefined
      ? createChainClient(
          options.endpoints.http,
          (options?.fetch ?? globalThis.fetch?.bind(globalThis)) as typeof fetch
        )
      : executionHost.chain);

  const session: Session<TDef, TStackPrograms> = {
    stacks,
    programs,
    get wallet() {
      return wallet;
    },
    signerRegistry,
    chain,
    transactions,
    transaction(instructions, transactionOptions) {
      const defaults = options?.execution;
      const configuredSigners = transactionOptions?.signers ?? defaults?.signers;
      const signers = [...new Set([...signerRegistry.values(), ...(configuredSigners ?? [])])];
      return executionHost.transaction(instructions, {
        wallet: transactionOptions?.wallet ?? defaults?.wallet,
        transactionTransport: transactionOptions?.transactionTransport ?? transactions,
        send: defaults?.send || transactionOptions?.send
          ? { ...(defaults?.send ?? {}), ...(transactionOptions?.send ?? {}) }
          : undefined,
        errors: transactionOptions?.errors,
        signers: signers.length > 0 ? signers : undefined,
      });
    },
    execute<TPrepared extends PreparedOperation, TSigner = unknown>(
      prepared: TPrepared,
      executionOptions?: OperationExecutionOptions<TSigner, TPrepared>
    ) {
      const defaults = options?.execution as OperationExecutionOptions<TSigner, TPrepared> | undefined;
      const configuredSigners = executionOptions?.signers ?? defaults?.signers;
      return executionHost.execute(prepared, {
        wallet: executionOptions?.wallet ?? defaults?.wallet,
        transactionTransport:
          executionOptions?.transactionTransport ?? defaults?.transactionTransport ?? transactions,
        send: defaults?.send || executionOptions?.send
          ? { ...(defaults?.send ?? {}), ...(executionOptions?.send ?? {}) }
          : undefined,
        signers: configuredSigners,
        signerRegistry,
        availableSignerAddresses:
          executionOptions?.availableSignerAddresses ?? defaults?.availableSignerAddresses,
        onTransactionStart:
          executionOptions?.onTransactionStart ?? defaults?.onTransactionStart,
        onTransactionSuccess:
          executionOptions?.onTransactionSuccess ?? defaults?.onTransactionSuccess,
        onCallbackError:
          executionOptions?.onCallbackError ?? defaults?.onCallbackError,
      });
    },
    setWallet(nextWallet) {
      wallet = nextWallet;
      for (const client of memberClients) {
        client.setWallet(nextWallet);
      }
    },
    close() {
      for (const client of memberClients) {
        client.disconnect();
      }
    },
  };

  return session;
}
