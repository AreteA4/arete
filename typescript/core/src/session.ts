import { resolveAuthConfig } from './auth-keys';
import { shareApiCredential } from './api-credential';
import { defaultPlatformHttpClient, type PlatformHttpClient } from './platform-http';
import {
  Arete,
  validateProgramReadDescriptor,
  withPrograms,
  type ConnectedArete,
  type ConnectOptions,
  type ProgramInterface,
  type StackWithAttachedPrograms,
  type TransactionOptions,
} from './client';
import { createChainClient, type ChainClient } from './chain';
import { getProgramReadDescriptor } from './program-sdk';
import {
  ambiguousSessionProgram,
  compareProgramIdentity,
  sessionProgramKeyConflict,
  sessionProtocolProgramKeyConflict,
  warnUnprovenSessionProgram,
  warnUnprovenStackPrograms,
} from './program-identity';
import { createHostedSolanaGatewayTransports } from './solana-gateway';
import { isSameStackSdk, sessionStackKeyConflict } from './stack-identity';
import {
  createProtocolClientWithMembers,
  type ProtocolClient,
  type ProtocolClientOptions,
} from './protocol/client';
import type {
  AnyProtocolDefinition,
  ProtocolKeyClass,
  ProtocolProgramsOf,
  ProtocolStacksOf,
  ProtocolTypesOf,
} from './protocol/types';
import {
  AreteError,
  type AuthConfig,
  type HostedSolanaGatewayBindings,
  type ProgramReadDescriptor,
  type ProgramReadDescriptors,
  type ProgramReadOverride,
  type StackDefinition,
  type ProgramSdkDefinition,
} from './types';
import type { StorageAdapter } from './storage/adapter';
import { mergeSendOptions } from './wallet/types';
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
export interface SessionDefinition<
  TPrograms extends Record<string, ProgramSdkDefinition> = Record<string, ProgramSdkDefinition>,
> {
  readonly mode?: 'composition';
  readonly stacks?: Record<string, StackDefinition>;
  /**
   * Standalone program SDKs, available at `session.programs.<key>`. Under a key
   * a stack also provides, the same program SDK (the same object or the same
   * `packageReleaseHash`) is served from the stack's connected instance, unless
   * this definition's `programReads.<key>` resolves to a descriptor other than
   * the one the stack reads through. That program, or one with the same
   * `programSpecHash` that cannot be proven identical, takes
   * `session.programs.<key>` with its own read configuration, with a warning,
   * while `session.stacks.<stack>.programs.<key>` keeps the stack's; anything
   * else throws `PROGRAM_KEY_CONFLICT`.
   */
  readonly programs?: TPrograms;
  /** Hosted chain and transaction capabilities shared by a generated composition. */
  readonly gateway?: HostedSolanaGatewayBindings;
  /** @deprecated Generated program cartridges carry defaults; use only for complete overrides. */
  readonly programReads?: ProgramReadDescriptors<TPrograms>;
  /**
   * Generated protocol definitions, available at `session.protocols.<key>`.
   * Each protocol's hoisted stacks join `session.stacks` and its programs join
   * `session.programs`, connected once: a stack or program the session already
   * has (the same object or the same `packageReleaseHash`) is shared, and a
   * different one under the same key throws `STACK_KEY_CONFLICT` or
   * `PROGRAM_KEY_CONFLICT`.
   */
  readonly protocols?: Record<string, AnyProtocolDefinition>;
}

/** Per-protocol client options inside a session; auth and fetch are the session's. */
export type SessionProtocolOptions = Omit<ProtocolClientOptions, 'auth' | 'fetch' | 'keyClass'>;

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
  /** Override applied to every program owned by this member. */
  programRead?: ProgramReadOverride;
  /** Per-program overrides for this member. */
  programReads?: Readonly<Record<string, ProgramReadOverride>>;
}

export interface SessionOptions<
  TDef extends SessionDefinition = SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
  TAuth extends AuthConfig = AuthConfig,
  TProtocolKeyClass extends ProtocolKeyClass | undefined = undefined,
> {
  /** One wallet governs execution across every member. */
  wallet?: WalletAdapter;
  /** Canonical chain reader shared by the session when explicitly provided. */
  chain?: ChainClient;
  /** Transaction transport override; defaults to the execution host's authenticated client. */
  transactions?: TransactionTransport;
  auth?: TAuth;
  /**
   * The key class `session.protocols` are typed as when it cannot be read
   * from `auth` (secret when `auth.secretKey` is given, else publishable).
   */
  protocolKeyClass?: TProtocolKeyClass;
  /** Per-protocol client options, keyed by the protocol's key in the definition. */
  protocols?: {
    [K in keyof NonNullable<TDef['protocols']>]?: SessionProtocolOptions;
  };
  fetch?: typeof fetch;
  /** Default transport for all members ('ws' unless overridden). */
  transport?: 'ws' | 'http';
  /** Shared fallback endpoints used when a member defines none of its own. */
  endpoints?: { http?: string; ws?: string };
  /** Arete API origin for the session's platform client. Defaults to `https://api.arete.run`. */
  apiUrl?: string;
  /** The session's Arete API client; built from `auth` when omitted. */
  platform?: PlatformHttpClient;
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
  /** Override applied to every program in the session. */
  programRead?: ProgramReadOverride;
  /** Session-wide per-program overrides, keyed by program key. */
  programReads?: Readonly<Record<string, ProgramReadOverride>>;
}

export type CompositionSessionOptions<
  TDef extends SessionDefinition = SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
  TAuth extends AuthConfig = AuthConfig,
  TProtocolKeyClass extends ProtocolKeyClass | undefined = undefined,
> = Omit<
  SessionOptions<TDef, TStackPrograms, TAuth, TProtocolKeyClass>,
  'chain' | 'transactions' | 'endpoints'
>
  & (TDef extends { readonly gateway: HostedSolanaGatewayBindings }
    ? {
        /** Explicit override for the generated hosted chain transport. */
        chain?: ChainClient;
        /** Explicit override for the generated hosted transaction transport. */
        transactions?: TransactionTransport;
        endpoints?: never;
      }
    : {
        /** Chain reads never inherit a live member's HTTP endpoint. */
        chain: ChainClient;
        /** Transactions never inherit a live member's HTTP endpoint. */
        transactions: TransactionTransport;
        endpoints?: never;
      });

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
> = TDef extends { readonly mode: 'composition' }
  ? ExplicitSessionPrograms<TDef>
  : Omit<PromotedSessionPrograms<TDef, TStackPrograms>, keyof NonNullable<TDef['programs']>>
    & ExplicitSessionPrograms<TDef>;

type SessionProtocolDefinitions<TDef extends SessionDefinition> = TDef extends {
  readonly protocols: infer TProtocols extends Record<string, AnyProtocolDefinition>;
}
  ? TProtocols
  : {};

type HasSessionProtocols<TDef extends SessionDefinition> = TDef extends {
  readonly protocols: Record<string, AnyProtocolDefinition>;
}
  ? true
  : false;

type ProtocolHoistedStackKeys<TDef extends SessionDefinition> = {
  [P in keyof SessionProtocolDefinitions<TDef>]: keyof ProtocolStacksOf<SessionProtocolDefinitions<TDef>[P]>;
}[keyof SessionProtocolDefinitions<TDef>];

type ProtocolHoistedStack<TDef extends SessionDefinition, K extends PropertyKey> = {
  [P in keyof SessionProtocolDefinitions<TDef>]: K extends keyof ProtocolStacksOf<SessionProtocolDefinitions<TDef>[P]>
    ? ProtocolStacksOf<SessionProtocolDefinitions<TDef>[P]>[K]
    : never;
}[keyof SessionProtocolDefinitions<TDef>];

/** Stacks hoisted from protocols that the definition does not list itself. */
type PromotedSessionStacks<TDef extends SessionDefinition> = {
  readonly [K in Exclude<ProtocolHoistedStackKeys<TDef>, keyof NonNullable<TDef['stacks']>>]:
    ProtocolHoistedStack<TDef, K> extends infer TStack extends StackDefinition
      ? ConnectedArete<TStack, TStack>
      : never;
};

type AllSessionStacks<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
> = HasSessionProtocols<TDef> extends true
  ? SessionStacks<TDef, TStackPrograms> & PromotedSessionStacks<TDef>
  : SessionStacks<TDef, TStackPrograms>;

type ProtocolHoistedProgramKeys<TDef extends SessionDefinition> = {
  [P in keyof SessionProtocolDefinitions<TDef>]: keyof ProtocolProgramsOf<SessionProtocolDefinitions<TDef>[P]>;
}[keyof SessionProtocolDefinitions<TDef>];

type ProtocolHoistedProgram<TDef extends SessionDefinition, K extends PropertyKey> = {
  [P in keyof SessionProtocolDefinitions<TDef>]: K extends keyof ProtocolProgramsOf<SessionProtocolDefinitions<TDef>[P]>
    ? ProtocolProgramsOf<SessionProtocolDefinitions<TDef>[P]>[K]
    : never;
}[keyof SessionProtocolDefinitions<TDef>];

type PromotedStackProgramKeys<TDef extends SessionDefinition> = {
  [K in keyof PromotedSessionStacks<TDef>]: keyof PromotedSessionStacks<TDef>[K]['programs'];
}[keyof PromotedSessionStacks<TDef>];

type PromotedStackProgram<TDef extends SessionDefinition, P extends PropertyKey> = {
  [K in keyof PromotedSessionStacks<TDef>]: P extends keyof PromotedSessionStacks<TDef>[K]['programs']
    ? PromotedSessionStacks<TDef>[K]['programs'][P]
    : never;
}[keyof PromotedSessionStacks<TDef>];

/** Programs hoisted from protocols (and their stacks) the session does not already type. */
type PromotedProtocolPrograms<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
> = {
  readonly [P in Exclude<
    ProtocolHoistedProgramKeys<TDef> | PromotedStackProgramKeys<TDef>,
    keyof SessionPrograms<TDef, TStackPrograms>
  >]: P extends PromotedStackProgramKeys<TDef>
    ? PromotedStackProgram<TDef, P>
    : ProtocolHoistedProgram<TDef, P> extends infer TProgram extends ProgramSdkDefinition
      ? ProgramInterface<TProgram>
      : never;
};

type AllSessionPrograms<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef>,
> = HasSessionProtocols<TDef> extends true
  ? SessionPrograms<TDef, TStackPrograms> & PromotedProtocolPrograms<TDef, TStackPrograms>
  : SessionPrograms<TDef, TStackPrograms>;

/** The key class `session.protocols` are typed as for the given session options. */
export type SessionProtocolKeyClass<TAuth, TProtocolKeyClass> =
  TProtocolKeyClass extends ProtocolKeyClass
    ? TProtocolKeyClass
    : TAuth extends { readonly secretKey: unknown }
      ? 'secret'
      : 'publishable';

/** `session.protocols`: one client per protocol, wired to the session's connected members. */
export type SessionProtocols<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
  TKeyClass extends ProtocolKeyClass = 'publishable',
> = {
  readonly [P in keyof SessionProtocolDefinitions<TDef>]: ProtocolClient<
    ProtocolTypesOf<SessionProtocolDefinitions<TDef>[P]>,
    TKeyClass,
    {
      readonly [K in keyof ProtocolStacksOf<SessionProtocolDefinitions<TDef>[P]>]:
        K extends keyof AllSessionStacks<TDef, TStackPrograms>
          ? AllSessionStacks<TDef, TStackPrograms>[K]
          : never;
    },
    {
      readonly [K in keyof ProtocolProgramsOf<SessionProtocolDefinitions<TDef>[P]>]:
        K extends keyof AllSessionPrograms<TDef, TStackPrograms>
          ? AllSessionPrograms<TDef, TStackPrograms>[K]
          : never;
    }
  >;
};

export interface Session<
  TDef extends SessionDefinition = SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
  TProtocolKeyClass extends ProtocolKeyClass = 'publishable',
> {
  readonly stacks: AllSessionStacks<TDef, TStackPrograms>;
  readonly programs: AllSessionPrograms<TDef, TStackPrograms>;
  /** Protocol clients, keyed like `definition.protocols`; empty without protocols. */
  readonly protocols: SessionProtocols<TDef, TStackPrograms, TProtocolKeyClass>;
  readonly wallet: WalletAdapter | undefined;
  readonly signerRegistry: SignerRegistry<any>;
  readonly chain: ChainClient;
  readonly transactions: TransactionTransport;
  /**
   * The session's authenticated Arete API client, shared with every member
   * (`context.platform` in SDK extensions); `undefined` unless the auth
   * carries an API credential (a secret key, or `tokenKind: 'api'`).
   */
  readonly platform: PlatformHttpClient | undefined;
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
  | 'url'
  | 'httpUrl'
  | 'transport'
  | 'auth'
  | 'storage'
  | 'autoConnect'
  | 'autoReconnect'
  | 'programRead'
  | 'programReads'
>;

type SessionConnectionOptions = Pick<
  SessionOptions,
  | 'wallet'
  | 'apiUrl'
  | 'platform'
  | 'auth'
  | 'fetch'
  | 'transport'
  | 'endpoints'
  | 'programRead'
  | 'programReads'
  | 'chain'
  | 'transactions'
>;

function mergeProgramReadOverrides(
  ...layers: readonly (ProgramReadOverride | undefined)[]
): ProgramReadOverride | undefined {
  return layers.reduce<ProgramReadOverride | undefined>(
    (selected, layer) => layer ?? selected,
    undefined
  );
}

function resolveSessionProgramReads(
  stack: StackDefinition,
  member: SessionConnectionMemberOptions | undefined,
  options: SessionConnectionOptions | undefined
): Readonly<Record<string, ProgramReadOverride>> | undefined {
  const reads = Object.fromEntries(
    Object.keys(stack.programs ?? {}).flatMap((name) => {
      const override = mergeProgramReadOverrides(
        options?.programRead,
        options?.programReads?.[name],
        member?.programRead,
        member?.programReads?.[name]
      );
      return override ? [[name, override] as const] : [];
    })
  );
  return Object.keys(reads).length > 0 ? reads : undefined;
}

/**
 * The program read descriptor a session member connected for `stack` reads
 * `programKey` through, resolved the way `Arete.connect` resolves it: session
 * and member overrides, then the stack's `programReads`, then the descriptor
 * the program carries.
 */
function memberProgramRead(
  stack: StackDefinition,
  programKey: string,
  member: SessionConnectionMemberOptions | undefined,
  options: SessionConnectionOptions | undefined
): ProgramReadDescriptor | undefined {
  return resolveSessionProgramReads(stack, member, options)?.[programKey]
    ?? stack.programReads?.[programKey]
    ?? getProgramReadDescriptor(stack.programs?.[programKey]);
}

/** Structural equality of two program read descriptors (plain JSON data). */
function sameProgramRead(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (
    left === null || right === null
    || typeof left !== 'object' || typeof right !== 'object'
    || Array.isArray(left) !== Array.isArray(right)
  ) {
    return false;
  }
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord).filter((key) => leftRecord[key] !== undefined);
  const rightKeys = Object.keys(rightRecord).filter((key) => rightRecord[key] !== undefined);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key) =>
      Object.prototype.hasOwnProperty.call(rightRecord, key)
      && sameProgramRead(leftRecord[key], rightRecord[key])
    );
}

function resolveMemberConnectOptions(
  stack: StackDefinition,
  member: SessionConnectionMemberOptions | undefined,
  options: SessionConnectionOptions | undefined,
  forceHttpOnly: boolean
): ConnectOptions {
  const transport = member?.transport ?? options?.transport ?? (forceHttpOnly ? 'http' : 'ws');
  // Stack endpoints remain on the stack definition. Only explicit session/member
  // HTTP options are forwarded as ConnectOptions.httpUrl for local Program Reads.
  const url = member?.url ?? (stack.endpoints.ws || options?.endpoints?.ws);
  const httpUrl = member?.httpUrl ?? options?.endpoints?.http;
  return {
    url,
    httpUrl,
    transport,
    auth: member?.auth ?? options?.auth,
    // A member with its own auth builds its own API client.
    ...(member?.auth ? { apiUrl: options?.apiUrl } : { apiUrl: options?.apiUrl, platform: options?.platform }),
    storage: member?.storage,
    autoConnect: member?.autoConnect,
    autoReconnect: member?.autoReconnect,
    wallet: options?.wallet,
    fetch: options?.fetch,
    programReads: resolveSessionProgramReads(stack, member, options),
    chain: options?.chain,
    transactions: options?.transactions,
  };
}

function validateCompositionProgramReads(
  stack: StackDefinition,
  member: SessionConnectionMemberOptions | undefined,
  options: SessionConnectionOptions | undefined
): void {
  const overrides = resolveSessionProgramReads(stack, member, options);
  for (const [name, program] of Object.entries(stack.programs ?? {})) {
    const descriptor = overrides?.[name]
      ?? stack.programReads?.[name]
      ?? getProgramReadDescriptor(program);
    if (descriptor) validateProgramReadDescriptor(name, descriptor);
    if (Object.keys(program.accounts ?? {}).length === 0) continue;
    const binding = descriptor?.transport.kind === 'hosted-binding'
      ? descriptor.transport.binding
      : undefined;
    if (
      !descriptor?.release
      || !binding
      || !binding.endpoint?.trim()
      || !binding.programReadBindingId?.trim()
      || binding.auth?.targetKind !== 'program-read-binding'
      || binding.auth.targetId !== binding.programReadBindingId
      || !binding.auth.sessionEndpoint?.trim()
    ) {
      throw new AreteError(
        `Composition session program '${name}' requires a complete hosted-binding descriptor or override`,
        'INVALID_CONFIG'
      );
    }
  }
}

function programAsStack(
  name: string,
  program: ProgramSdkDefinition,
  descriptor: ProgramReadDescriptor | undefined
): StackDefinition {
  return {
    name,
    endpoints: { ws: '' },
    views: {},
    programs: { [name]: program },
    ...(descriptor ? { programReads: { [name]: descriptor } } : {}),
    ...(program.gateway ? { gateway: program.gateway } : {}),
  };
}

function unavailableInSession<T extends object>(what: string): T {
  return new Proxy({} as T, {
    get(_target, property) {
      if (property === 'then') return undefined;
      throw new AreteError(
        `This session has no ${what}: it has no stack or program members. Pass options.${what} `
          + 'or add a stack or program to the session.',
        'INVALID_CONFIG'
      );
    },
  });
}

export async function createSession<
  TDef extends SessionDefinition,
  TStackPrograms extends SessionStackPrograms<TDef> = {},
  TAuth extends AuthConfig = AuthConfig,
  TProtocolKeyClass extends ProtocolKeyClass | undefined = undefined,
>(
  definition: TDef,
  ...args: TDef extends { readonly mode: 'composition' }
    ? [options: CompositionSessionOptions<TDef, TStackPrograms, TAuth, TProtocolKeyClass>]
    : [options?: SessionOptions<TDef, TStackPrograms, TAuth, TProtocolKeyClass>]
): Promise<Session<TDef, TStackPrograms, SessionProtocolKeyClass<TAuth, TProtocolKeyClass>>> {
  type TKeyClass = SessionProtocolKeyClass<TAuth, TProtocolKeyClass>;
  const options = withSessionCredential(args[0] as SessionOptions<TDef, TStackPrograms> | undefined);
  const explicitStackEntries = Object.entries(definition.stacks ?? {});
  const explicitProgramEntries = Object.entries(definition.programs ?? {});
  const protocolEntries = Object.entries(definition.protocols ?? {});
  if (
    explicitStackEntries.length === 0
    && explicitProgramEntries.length === 0
    && protocolEntries.length === 0
  ) {
    throw new AreteError('createSession requires at least one stack or program member', 'INVALID_CONFIG');
  }
  for (const [key, protocol] of protocolEntries) {
    if (!protocol || protocol.kind !== 'protocol') {
      throw new AreteError(
        `Session protocol '${key}' is not a generated protocol definition`,
        'INVALID_CONFIG'
      );
    }
  }

  // Protocols hoist their stacks and programs into the session. A stack the
  // session already has under any key (the same object or the same package
  // release) is shared; a different stack under a taken key is a conflict.
  const stackKeyDefinitions = new Map<string, { source: string; stack: StackDefinition }>(
    explicitStackEntries.map(([key, stack]) => [key, { source: `stack '${key}'`, stack }])
  );
  const stackAliases = new Map<string, string>();
  const promotedStackEntries: [string, StackDefinition][] = [];
  for (const [protocolKey, protocol] of protocolEntries) {
    const source = `protocol '${protocolKey}'`;
    for (const [stackKey, stack] of Object.entries(protocol.stacks ?? {}) as [string, StackDefinition][]) {
      const existing = stackKeyDefinitions.get(stackKey);
      if (existing) {
        if (!isSameStackSdk(existing.stack, stack)) {
          throw sessionStackKeyConflict(stackKey, existing.source, existing.stack, source, stack);
        }
        continue;
      }
      stackKeyDefinitions.set(stackKey, { source, stack });
      const shared = [...stackKeyDefinitions].find(([key, entry]) =>
        key !== stackKey && !stackAliases.has(key) && isSameStackSdk(entry.stack, stack)
      );
      if (shared) {
        stackAliases.set(stackKey, shared[0]);
        continue;
      }
      promotedStackEntries.push([stackKey, stack]);
    }
  }
  const stackEntries = [...explicitStackEntries, ...promotedStackEntries];

  const programKeyDefinitions = new Map<string, { source: string; program: ProgramSdkDefinition }>(
    explicitProgramEntries.map(([key, program]) => [key, { source: `program '${key}'`, program }])
  );
  const promotedProgramEntries: [string, ProgramSdkDefinition][] = [];
  for (const [protocolKey, protocol] of protocolEntries) {
    const source = `protocol '${protocolKey}'`;
    for (const [programKey, program] of Object.entries(protocol.programs ?? {}) as [string, ProgramSdkDefinition][]) {
      const existing = programKeyDefinitions.get(programKey);
      if (existing) {
        if (compareProgramIdentity(existing.program, program) === 'different') {
          throw sessionProtocolProgramKeyConflict(
            programKey,
            existing.source,
            existing.program,
            source,
            program
          );
        }
        continue;
      }
      programKeyDefinitions.set(programKey, { source, program });
      promotedProgramEntries.push([programKey, program]);
    }
  }
  const programEntries = [...explicitProgramEntries, ...promotedProgramEntries];
  const generatedGateway = definition.gateway && (!options?.chain || !options?.transactions)
    ? createHostedSolanaGatewayTransports(definition.gateway, {
        auth: options?.auth,
        fetch: options?.fetch,
      })
    : undefined;
  const sharedChain = options?.chain ?? generatedGateway?.chain;
  const sharedTransactions = options?.transactions ?? generatedGateway?.transactions;
  if (definition.mode === 'composition' && (!sharedChain || !sharedTransactions)) {
    throw new AreteError(
      'composition sessions require generated or explicit chain and transaction transports',
      'INVALID_CONFIG'
    );
  }
  if (definition.mode === 'composition' && options?.endpoints !== undefined) {
    throw new AreteError(
      'composition sessions require per-member live endpoints, not shared fallback endpoints',
      'INVALID_CONFIG'
    );
  }
  if (definition.programReads) {
    const programKeys = explicitProgramEntries.map(([key]) => key);
    const descriptorKeys = Object.keys(definition.programReads);
    if (
      programKeys.some((key) => !descriptorKeys.includes(key))
      || descriptorKeys.some((key) => !programKeys.includes(key))
    ) {
      throw new AreteError(
        'Session definition programReads keys must exactly match standalone programs',
        'INVALID_CONFIG'
      );
    }
  }
  if (definition.mode === 'composition') {
    for (const [key, stack] of stackEntries) {
      const member = options?.stacks?.[key as keyof NonNullable<TDef['stacks']>];
      validateCompositionProgramReads(
        withPrograms(stack, member?.programs as Record<string, ProgramSdkDefinition> | undefined),
        member,
        options
      );
    }
    for (const [key, program] of programEntries) {
      validateCompositionProgramReads(
        programAsStack(key, program, definition.programReads?.[key]),
        options?.programs?.[key],
        options
      );
    }
  }

  // Program identity, resolved before anything connects. Every stack's
  // effective programs (its own plus member-attached ones, matched by
  // `withPrograms`) are the providers for each key.
  const effectiveStacks = new Map(stackEntries.map(([key, stack]) => {
    const memberOptions = options?.stacks?.[key as keyof NonNullable<TDef['stacks']>];
    return [key, withPrograms(
      stack,
      memberOptions?.programs as Record<string, ProgramSdkDefinition> | undefined
    ) as StackDefinition] as const;
  }));
  const stackProviders = new Map<string, { stackKey: string; program: ProgramSdkDefinition }[]>();
  for (const [stackKey, stack] of effectiveStacks) {
    for (const [programKey, program] of Object.entries(stack.programs ?? {})) {
      const providers = stackProviders.get(programKey) ?? [];
      providers.push({ stackKey, program });
      stackProviders.set(programKey, providers);
    }
  }
  // Standalone programs a stack already provides as the same program SDK, read
  // through the same program read configuration, are served by that stack's
  // connected instance instead of a second member. One with the same program
  // spec but no provable identity match, or the same program SDK with its own
  // `programReads` descriptor that differs from the stack's, gets its own
  // member and takes session.programs.<key>; the stacks keep theirs.
  const sharedStandalonePrograms = new Set<string>();
  if (definition.mode !== 'composition') {
    for (const [programKey, program] of programEntries) {
      const providers = stackProviders.get(programKey) ?? [];
      if (providers.length === 0) continue;
      const matches = providers.map((provider) => ({
        ...provider,
        match: compareProgramIdentity(provider.program, program),
      }));
      const conflicting = matches.find(({ match }) => match === 'different');
      if (conflicting) {
        throw sessionProgramKeyConflict(conflicting.stackKey, programKey, conflicting.program, program);
      }
      if (matches.some(({ match }) => match === 'unproven')) {
        warnUnprovenSessionProgram(programKey, program, providers);
        continue;
      }
      const explicitRead = definition.programReads?.[programKey];
      if (explicitRead !== undefined) {
        // Sharing would read through the serving stack's configuration, so an
        // explicit descriptor must resolve to the same one.
        const servingStack = providers[0]!.stackKey;
        const standaloneRead = memberProgramRead(
          programAsStack(programKey, program, explicitRead),
          programKey,
          options?.programs?.[programKey],
          options
        );
        const stackRead = memberProgramRead(
          effectiveStacks.get(servingStack)!,
          programKey,
          options?.stacks?.[servingStack as keyof NonNullable<TDef['stacks']>],
          options
        );
        if (!sameProgramRead(standaloneRead, stackRead)) {
          warnUnprovenSessionProgram(programKey, program, providers, 'program-read');
          continue;
        }
      }
      if (options?.programs?.[programKey] !== undefined) {
        throw new AreteError(
          `Standalone program '${programKey}' is the same program SDK that stack `
            + `'${providers[0]!.stackKey}' provides, so the session serves it from the stack's `
            + `connected instance; configure it through options.stacks.${providers[0]!.stackKey} `
            + `instead of options.programs.${programKey}`,
          'INVALID_CONFIG'
        );
      }
      sharedStandalonePrograms.add(programKey);
    }
  }
  const memberProgramEntries = programEntries.filter(([key]) => !sharedStandalonePrograms.has(key));

  let wallet = options?.wallet;
  const signerRegistry = options?.signerRegistry ?? createSignerRegistry();

  const connectedStacks = await Promise.all(
    stackEntries.map(async ([key, stack]) => {
      const memberOptions = options?.stacks?.[key as keyof NonNullable<TDef['stacks']>];
      const effectiveStack = effectiveStacks.get(key)!;
      const connectOptions = resolveMemberConnectOptions(
        effectiveStack,
        memberOptions,
        {
          ...options,
          chain: sharedChain,
          transactions: sharedTransactions,
        },
        false
      );
      const client = await Arete.connect(stack, {
        ...connectOptions,
        programs: memberOptions?.programs,
        programReads: connectOptions.programReads as any,
      });
      return [key, client] as const;
    })
  );

  const connectedPrograms = await Promise.all(
    memberProgramEntries.map(async ([key, program]) => {
      const syntheticStack = programAsStack(key, program, definition.programReads?.[key]);
      const connectOptions = resolveMemberConnectOptions(
        syntheticStack,
        options?.programs?.[key],
        {
          ...options,
          chain: sharedChain,
          transactions: sharedTransactions,
        },
        true
      );
      const client = await Arete.connect(syntheticStack, connectOptions);
      return [key, client] as const;
    })
  );

  const memberClients = [
    ...connectedStacks.map(([, client]) => client),
    ...connectedPrograms.map(([, client]) => client),
  ];
  // A session of protocols without stacks or programs has no execution host.
  const executionHost = memberClients[0];
  const noExecutionHost = () => new AreteError(
    'This session has no stack or program members to execute transactions through',
    'INVALID_CONFIG'
  );
  const transactions = sharedTransactions
    ?? executionHost?.transactions
    ?? unavailableInSession<TransactionTransport>('transactions');

  const connectedStackMap = Object.fromEntries(connectedStacks) as Record<string, unknown>;
  for (const [alias, target] of stackAliases) {
    connectedStackMap[alias] = connectedStackMap[target];
  }
  const stacks = connectedStackMap as unknown as AllSessionStacks<TDef, TStackPrograms>;
  const connectedProgramEntries = connectedPrograms.map(([key, client]) => [
    key,
    (client.programs as Record<string, ProgramInterface<ProgramSdkDefinition>>)[key],
  ] as const);
  const promotedPrograms = Object.fromEntries(connectedProgramEntries) as Record<
    string,
    ProgramInterface<ProgramSdkDefinition>
  >;

  if (definition.mode !== 'composition') {
    const connectedStackClients = new Map(connectedStacks);
    for (const [programKey, providers] of stackProviders) {
      if (Object.prototype.hasOwnProperty.call(promotedPrograms, programKey)) continue;
      const first = providers[0]!;
      const matches = providers.flatMap((left, index) =>
        providers.slice(index + 1).map((right) => compareProgramIdentity(left.program, right.program))
      );
      if (!matches.includes('different')) {
        // One program, however many stacks bundle it: the first stack's
        // connected instance serves the top-level key. Copies with the same
        // program spec that cannot be proven identical say so once.
        if (matches.includes('unproven')) warnUnprovenStackPrograms(programKey, providers);
        promotedPrograms[programKey] = (connectedStackClients.get(first.stackKey)!.programs as Record<
          string,
          ProgramInterface<ProgramSdkDefinition>
        >)[programKey]!;
        continue;
      }
      // Different program SDKs under one key: every stack keeps its own at
      // session.stacks.<stack>.programs.<key>, and the ambiguous top-level key
      // explains how to choose instead of silently picking one.
      const error = ambiguousSessionProgram(programKey, providers);
      Object.defineProperty(promotedPrograms, programKey, {
        get() {
          throw error;
        },
        enumerable: false,
        configurable: true,
      });
    }
  }

  const programs = promotedPrograms as unknown as AllSessionPrograms<TDef, TStackPrograms>;

  const chain =
    sharedChain ?? (options?.endpoints?.http !== undefined
      ? createChainClient(
          options.endpoints.http,
          (options?.fetch ?? globalThis.fetch?.bind(globalThis)) as typeof fetch
        )
      : executionHost?.chain ?? unavailableInSession<ChainClient>('chain'));

  const protocolClients = protocolEntries.map(([protocolKey, protocol]) => {
    const memberStacks: Record<string, unknown> = {};
    for (const stackKey of Object.keys(protocol.stacks ?? {})) {
      memberStacks[stackKey] = connectedStackMap[stackKey];
    }
    const memberPrograms: Record<string, unknown> = {};
    for (const programKey of Object.keys(protocol.programs ?? {})) {
      // Read through session.programs so an ambiguous key explains itself.
      Object.defineProperty(memberPrograms, programKey, {
        get: () => (promotedPrograms as Record<string, unknown>)[programKey],
        enumerable: true,
      });
    }
    const protocolOptions = (options?.protocols as Record<string, SessionProtocolOptions> | undefined)?.[protocolKey];
    const client = createProtocolClientWithMembers(
      protocol,
      {
        ...protocolOptions,
        auth: options?.auth,
        fetch: options?.fetch,
        keyClass: options?.protocolKeyClass,
      },
      { stacks: memberStacks, programs: memberPrograms }
    );
    return [protocolKey, client] as const;
  });
  const protocols = Object.fromEntries(protocolClients) as unknown as SessionProtocols<
    TDef,
    TStackPrograms,
    TKeyClass
  >;

  const session: Session<TDef, TStackPrograms, TKeyClass> = {
    stacks,
    programs,
    protocols,
    get wallet() {
      return wallet;
    },
    signerRegistry,
    chain,
    transactions,
    platform: options?.platform,
    transaction(instructions, transactionOptions) {
      const defaults = options?.execution;
      const configuredSigners = transactionOptions?.signers ?? defaults?.signers;
      const signers = [...new Set([...signerRegistry.values(), ...(configuredSigners ?? [])])];
      if (!executionHost) return Promise.reject(noExecutionHost());
      return executionHost.transaction(instructions, {
        wallet: transactionOptions?.wallet ?? defaults?.wallet,
        transactionTransport: transactionOptions?.transactionTransport ?? transactions,
        send: mergeSendOptions(defaults?.send, transactionOptions?.send),
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
      if (!executionHost) return Promise.reject(noExecutionHost());
      return executionHost.execute(prepared, {
        wallet: executionOptions?.wallet ?? defaults?.wallet,
        transactionTransport:
          executionOptions?.transactionTransport ?? defaults?.transactionTransport ?? transactions,
        send: mergeSendOptions(defaults?.send, executionOptions?.send),
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
      for (const [, client] of protocolClients) {
        client.close();
      }
      for (const client of memberClients) {
        client.disconnect();
      }
    },
  };

  return session;
}

/**
 * The session's options with one shared API credential source in `auth`
 * (every member, transport and the platform client fetch the credential
 * through it) and the session's platform client.
 */
function withSessionCredential<T extends SessionOptions<any, any> | undefined>(options: T): T {
  if (!options) return options;
  const fetchForAuth = ((input: RequestInfo | URL, init?: RequestInit) =>
    (options.fetch ?? globalThis.fetch)(input, init)) as typeof fetch;
  const auth = shareApiCredential(resolveAuthConfig(options.auth), fetchForAuth);
  const platform = options.platform
    ?? defaultPlatformHttpClient({ apiUrl: options.apiUrl, auth, fetch: fetchForAuth });
  return { ...options, auth, platform } as T;
}
