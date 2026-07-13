import type {
  ConnectionState,
  StackDefinition,
  AreteOptions,
  TypedViews,
  ConnectionStateCallback,
  SocketIssueCallback,
  UnsubscribeFn,
  ProgramSdkDefinition,
  ProgramAccountReadDefinition,
  ProgramQueryDefinition,
  StackQueryDefinition,
} from './types';
import { AreteError, parseErrorCode, shouldRefreshToken } from './types';
import { ConnectionManager } from './connection';
import { FrameProcessor } from './frame-processor';
import { MemoryAdapter } from './storage/memory-adapter';
import type { StorageAdapter } from './storage/adapter';
import { SortedStorageDecorator } from './storage/sorted-decorator';
import { SubscriptionRegistry } from './subscription';
import { createTypedViews } from './views';
import type { Frame } from './frame';
import type { WalletAdapter, BuiltInstruction, SendOptions } from './wallet/types';
import { createChainClient, deriveHttpEndpoint, type ChainClient } from './chain';
import type {
  InstructionHandler,
  ExecutionResult,
  BuildOptions,
} from './instructions';
import type { ErrorMetadata } from './instructions';
import {
  buildInstruction,
  parseInstructionError,
  InstructionError,
} from './instructions';
import {
  applyConnectedStackExtensions,
  getProgramRuntimeExtensions,
  type ProgramOperationsOf,
  type StackConnectedExtensions,
} from './stack-extensions';
import {
  executePreparedOperation,
  type OperationExecutionOptions,
  type OperationReceiptFor,
  type PreparedOperation,
} from './operations';
import { parseReadResponse } from './read';

type ProgramMap = Record<string, ProgramSdkDefinition>;

type NormalizeProgramMap<TPrograms> = TPrograms extends ProgramMap ? TPrograms : Record<string, never>;

export type MergeProgramMaps<TStackPrograms, TAttachedPrograms> =
  Omit<NormalizeProgramMap<TAttachedPrograms>, keyof NormalizeProgramMap<TStackPrograms>>
  & NormalizeProgramMap<TStackPrograms>;

export type StackWithAttachedPrograms<
  TStack extends StackDefinition,
  TAttachedPrograms extends ProgramMap | undefined,
> = Omit<TStack, 'programs'> & {
  programs: MergeProgramMaps<TStack['programs'], TAttachedPrograms>;
};

function mergeAttachedPrograms<
  TStack extends StackDefinition,
  TAttachedPrograms extends ProgramMap | undefined,
>(
  stack: TStack,
  attachedPrograms: TAttachedPrograms
): MergeProgramMaps<TStack['programs'], TAttachedPrograms> {
  const merged: ProgramMap = { ...(attachedPrograms ?? {}) };

  for (const [name, definition] of Object.entries(stack.programs ?? {})) {
    if (name in merged) {
      console.warn(
        `Ignoring attached program '${name}' for stack '${stack.name}' because the stack already defines that key`
      );
    }
    merged[name] = definition;
  }

  return merged as MergeProgramMaps<TStack['programs'], TAttachedPrograms>;
}

function normalizeProgramAccountWireKeys(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(normalizeProgramAccountWireKeys);
  }
  if (!value || typeof value !== 'object') {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value).map(([key, nestedValue]) => [
      key.replace(/[A-Z]/g, (letter, index) => `${index === 0 ? '' : '_'}${letter.toLowerCase()}`),
      normalizeProgramAccountWireKeys(nestedValue),
    ])
  );
}

function parseProgramAccountValue<T>(
  definition: ProgramAccountReadDefinition<T>,
  value: unknown
): T {
  const schema = definition.schema;
  if (!schema) {
    return value as T;
  }
  const parsed = schema.safeParse(value);
  if (parsed.success) {
    return parsed.data;
  }
  const normalized = schema.safeParse(normalizeProgramAccountWireKeys(value));
  if (normalized.success) {
    return normalized.data;
  }
  throw new Error(`Program account read '${definition.account}' failed schema validation`);
}

function cloneStackWithPrograms<
  TStack extends StackDefinition,
  TAttachedPrograms extends ProgramMap | undefined,
>(
  stack: TStack,
  programs: MergeProgramMaps<TStack['programs'], TAttachedPrograms>
): StackWithAttachedPrograms<TStack, TAttachedPrograms> {
  const cloned = Object.create(
    Object.getPrototypeOf(stack),
    Object.getOwnPropertyDescriptors(stack)
  ) as TStack & { programs?: ProgramMap };
  cloned.programs = programs as ProgramMap;
  return cloned as unknown as StackWithAttachedPrograms<TStack, TAttachedPrograms>;
}

export function withPrograms<
  TStack extends StackDefinition,
  TAttachedPrograms extends ProgramMap | undefined,
>(
  stack: TStack,
  attachedPrograms: TAttachedPrograms
): StackWithAttachedPrograms<TStack, TAttachedPrograms> {
  return cloneStackWithPrograms(stack, mergeAttachedPrograms(stack, attachedPrograms));
}

export interface ConnectOptions<TPrograms extends ProgramMap | undefined = undefined> {
  url?: string;
  httpUrl?: string;
  /**
   * Transport mode. `'ws'` (default) opens the streaming WebSocket; `'http'`
   * skips the socket entirely — point reads, chain reads, and instruction
   * execution work, while views/subscriptions throw `WEBSOCKET_DISABLED`.
   */
  transport?: 'ws' | 'http';
  storage?: StorageAdapter;
  maxEntriesPerView?: number | null;
  autoReconnect?: boolean;
  reconnectIntervals?: number[];
  maxReconnectAttempts?: number;
  flushIntervalMs?: number;
  validateFrames?: boolean;
  /** Authentication configuration */
  auth?: import('./types').AuthConfig;
  /** Default wallet adapter used for instruction execution (overridable per call). */
  wallet?: WalletAdapter;
  /** Optional fetch implementation for HTTP point reads. */
  fetch?: typeof fetch;
  /** Additional program SDKs exposed under client.programs.<key>. */
  programs?: TPrograms;
  /** Default semantic-operation execution settings. */
  execution?: OperationExecutionOptions<any>;
}

/** @deprecated Use ConnectOptions instead */
export interface AreteOptionsWithStorage<TStack extends StackDefinition> extends AreteOptions<TStack> {
  httpUrl?: string;
  storage?: StorageAdapter;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  auth?: import('./types').AuthConfig;
  wallet?: WalletAdapter;
  fetch?: typeof fetch;
  execution?: OperationExecutionOptions<any>;
}

export interface TransactionOptions<TSigner = unknown> {
  wallet?: WalletAdapter;
  send?: SendOptions;
  errors?: ErrorMetadata[];
  signers?: readonly TSigner[];
}

/**
 * A typed, callable instruction.
 *
 * Calling it builds + signs + sends the transaction. The attached `build`
 * method is a pure prepare step that returns a {@link BuiltInstruction} for
 * batching/composition.
 */
export interface TypedInstruction<TParams, TError> {
  build(params: TParams, options?: BuildOptions): BuiltInstruction;
  /** Phantom error type for downstream inference. */
  readonly _error?: TError;
}

export interface TypedAccountReader<T> {
  fetch(address: string): Promise<T | null>;
  fetchMany(addresses: readonly string[]): Promise<Array<T | null>>;
  exists(address: string): Promise<boolean>;
}

export type TypedQueryExecutor<TParams, TResult> = (
  params: TParams
) => Promise<TResult>;

type TypedAccountReaderFor<TEntry> = TEntry extends ProgramAccountReadDefinition<infer T>
  ? TypedAccountReader<T>
  : TypedAccountReader<unknown>;

type TypedQueryFor<TEntry> = TEntry extends ProgramQueryDefinition<infer P, infer R>
  ? TypedQueryExecutor<P, R>
  : TEntry extends StackQueryDefinition<infer P, infer R>
    ? TypedQueryExecutor<P, R>
    : TypedQueryExecutor<Record<string, unknown>, unknown>;

export type RawInstructionsInterface<
  TInstructions extends Record<string, InstructionHandler<any, any>> | undefined,
> = TInstructions extends Record<string, InstructionHandler<any, any>>
  ? { [K in keyof TInstructions]: TInstructions[K] extends InstructionHandler<infer P, infer E>
      ? TypedInstruction<P, E>
      : TypedInstruction<Record<string, unknown>, unknown> }
  : Record<string, never>;

type ProgramAccountsInterface<
  TAccounts extends Record<string, ProgramAccountReadDefinition<unknown>> | undefined,
> = TAccounts extends Record<string, ProgramAccountReadDefinition<unknown>>
  ? { [K in keyof TAccounts]: TypedAccountReaderFor<TAccounts[K]> }
  : Record<string, never>;

type ProgramQueriesInterface<
  TQueries extends Record<string, ProgramQueryDefinition<unknown, unknown>> | undefined,
> = TQueries extends Record<string, ProgramQueryDefinition<unknown, unknown>>
  ? { [K in keyof TQueries]: TypedQueryFor<TQueries[K]> }
  : Record<string, never>;

type ProgramNamespace<TNamespace> = TNamespace extends Record<string, unknown>
  ? TNamespace
  : Record<string, never>;

type OperationField<TOperations, TKey extends PropertyKey> =
  TKey extends keyof TOperations
    ? ProgramNamespace<TOperations[TKey]>
    : Record<string, never>;

export type ProgramInterface<TProgram extends ProgramSdkDefinition> = {
  name: TProgram['name'];
  programId: TProgram['programId'];
  schemas: TProgram['schemas'];
  pdas: TProgram['pdas'] extends Record<string, unknown> ? TProgram['pdas'] : Record<string, never>;
  accounts: ProgramAccountsInterface<TProgram['accounts']>;
  queries: ProgramQueriesInterface<TProgram['queries']>;
  raw: RawInstructionsInterface<TProgram['rawInstructions']>;
  addresses: ProgramNamespace<TProgram['addresses']>;
  constants: ProgramNamespace<TProgram['constants']>;
  defaults: ProgramNamespace<TProgram['defaults']>;
  math: ProgramNamespace<TProgram['math']>;
  instructions: OperationField<ProgramOperationsOf<TProgram>, 'instructions'>;
  transactions: OperationField<ProgramOperationsOf<TProgram>, 'transactions'>;
  flows: OperationField<ProgramOperationsOf<TProgram>, 'flows'>;
};

export type ProgramsInterface<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined,
> = TPrograms extends Record<string, ProgramSdkDefinition>
  ? { [K in keyof TPrograms]: ProgramInterface<TPrograms[K]> }
  : Record<string, never>;

export type QueriesInterface<
  TQueries extends Record<string, StackQueryDefinition<unknown, unknown>> | undefined,
> = TQueries extends Record<string, StackQueryDefinition<unknown, unknown>>
  ? { [K in keyof TQueries]: TypedQueryFor<TQueries[K]> }
  : Record<string, never>;

export type RawProgramsInterface<
  TPrograms extends Record<string, ProgramSdkDefinition> | undefined,
> = TPrograms extends Record<string, ProgramSdkDefinition>
  ? {
      [K in keyof TPrograms]: RawInstructionsInterface<TPrograms[K]['rawInstructions']>;
    }
  : Record<string, never>;

/** @deprecated Retained for backward compatibility; prefer {@link TypedInstruction}. */
export type InstructionExecutor = TypedInstruction<Record<string, unknown>, unknown>;

export type ConnectedArete<
  TStack extends StackDefinition,
  TExtensions = TStack,
> = Arete<TStack> & StackConnectedExtensions<TExtensions>;

export class Arete<TStack extends StackDefinition> {
  private readonly connection: ConnectionManager;
  private readonly storage: StorageAdapter;
  private readonly processor: FrameProcessor;
  private readonly subscriptionRegistry: SubscriptionRegistry;
  private readonly _views: TypedViews<TStack['views']>;
  private readonly _queries: QueriesInterface<TStack['queries']>;
  private readonly _programs: ProgramsInterface<TStack['programs']>;
  private readonly _chain: ChainClient;
  private readonly stack: TStack;
  private readonly httpBaseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly executionDefaults?: OperationExecutionOptions<any>;
  private _wallet?: WalletAdapter;
  private _aggregatedErrors?: ErrorMetadata[];

  private constructor(
    url: string | null,
    httpBaseUrl: string,
    options: AreteOptionsWithStorage<TStack>
  ) {
    this.stack = options.stack;
    this._wallet = options.wallet;
    this.executionDefaults = options.execution;
    this.httpBaseUrl = httpBaseUrl;
    this.fetchImpl = options.fetch ?? this.resolveFetchImpl();
    this.storage = new SortedStorageDecorator(options.storage ?? new MemoryAdapter());
    this.processor = new FrameProcessor(this.storage, {
      maxEntriesPerView: options.maxEntriesPerView,
      flushIntervalMs: options.flushIntervalMs,
      schemas: this.stack.schemas,
      patchSchemas: this.stack.patchSchemas,
    });
    this.connection = new ConnectionManager({
      websocketUrl: url,
      reconnectIntervals: options.reconnectIntervals,
      maxReconnectAttempts: options.maxReconnectAttempts,
      auth: options.auth,
    });
    this.subscriptionRegistry = new SubscriptionRegistry(this.connection);

    this.connection.onFrame((frame: Frame) => {
      this.processor.handleFrame(frame);
    });

    this._views = createTypedViews(this.stack, this.storage, this.subscriptionRegistry);
    this._queries = this.buildQueries();
    this._chain = createChainClient(this.httpBaseUrl, this.authenticatedFetch.bind(this) as typeof fetch);
    this._programs = this.buildPrograms();
  }

  private resolveFetchImpl(): typeof fetch {
    if (typeof globalThis.fetch !== 'function') {
      throw new AreteError(
        'A fetch implementation is required for HTTP point reads (provide ConnectOptions.fetch or use an environment with global fetch)',
        'INVALID_CONFIG'
      );
    }
    return globalThis.fetch.bind(globalThis);
  }

  private buildQueries(): QueriesInterface<TStack['queries']> {
    const queries: Record<string, TypedQueryExecutor<Record<string, unknown>, unknown>> = {};

    for (const [name, definition] of Object.entries(this.stack.queries ?? {})) {
      queries[name] = this.createQueryExecutor(definition as StackQueryDefinition<unknown, unknown>);
    }

    return queries as QueriesInterface<TStack['queries']>;
  }

  private buildPrograms(): ProgramsInterface<TStack['programs']> {
    const bases: Record<string, Omit<ProgramInterface<ProgramSdkDefinition>, 'instructions' | 'transactions' | 'flows'>> = {};

    for (const [name, definition] of Object.entries(this.stack.programs ?? {})) {
      const instructions: Record<string, TypedInstruction<Record<string, unknown>, unknown>> = {};
      for (const [instructionName, handler] of Object.entries(definition.rawInstructions ?? {})) {
        instructions[instructionName] = this.createTypedInstruction(handler as InstructionHandler);
      }

      const accounts: Record<string, TypedAccountReader<unknown>> = {};
      for (const [accountName, accountDefinition] of Object.entries(definition.accounts ?? {})) {
        accounts[accountName] = this.createAccountReader(accountDefinition as ProgramAccountReadDefinition<unknown>);
      }

      const queries: Record<string, TypedQueryExecutor<Record<string, unknown>, unknown>> = {};
      for (const [queryName, queryDefinition] of Object.entries(definition.queries ?? {})) {
        queries[queryName] = this.createQueryExecutor(queryDefinition as ProgramQueryDefinition<unknown, unknown>);
      }

      bases[name] = {
        name: definition.name,
        programId: definition.programId,
        schemas: definition.schemas,
        pdas: definition.pdas ?? {},
        accounts,
        queries,
        raw: instructions,
        addresses: definition.addresses ?? {},
        constants: definition.constants ?? {},
        defaults: definition.defaults ?? {},
        math: definition.math ?? {},
      } as Omit<ProgramInterface<ProgramSdkDefinition>, 'instructions' | 'transactions' | 'flows'>;
    }

    const programs: Record<string, ProgramInterface<ProgramSdkDefinition>> = {};
    const client = this;
    for (const [name, definition] of Object.entries(this.stack.programs ?? {})) {
      const base = bases[name]!;
      const connectedProgram = {
        ...base,
        instructions: {},
        transactions: {},
        flows: {},
      } as ProgramInterface<ProgramSdkDefinition>;
      const runtime = getProgramRuntimeExtensions(definition);
      const operations = runtime?.createOperations({
        chain: this._chain,
        get wallet() {
          return client._wallet;
        },
        program: connectedProgram,
      });
      connectedProgram.instructions = operations?.instructions ?? {};
      connectedProgram.transactions = operations?.transactions ?? {};
      connectedProgram.flows = operations?.flows ?? {};
      programs[name] = connectedProgram;
    }
    return programs as ProgramsInterface<TStack['programs']>;
  }

  private createTypedInstruction(
    handler: InstructionHandler
  ): TypedInstruction<Record<string, unknown>, unknown> {
    return {
      build: (params: Record<string, unknown>, options?: BuildOptions) =>
        buildInstruction(handler, params, this.withWallet(options)),
    };
  }

  private createAccountReader<T>(definition: ProgramAccountReadDefinition<T>): TypedAccountReader<T> {
    return {
      fetch: async (address: string): Promise<T | null> => {
        const result = await this.readJson<T | null>(`${definition.path}/${encodeURIComponent(address)}`);
        return result === null ? null : parseProgramAccountValue(definition, result);
      },
      fetchMany: async (addresses: readonly string[]): Promise<Array<T | null>> => {
        const result = await this.readJson<Array<T | null>>(definition.path, {
          method: 'POST',
          body: { addresses },
        });
        return result.map((value) =>
          value === null ? null : parseProgramAccountValue(definition, value)
        );
      },
      exists: async (address: string): Promise<boolean> => {
        const result = await this.readJson<{ exists: boolean }>(`${definition.path}/${encodeURIComponent(address)}/exists`);
        return result.exists;
      },
    };
  }

  private createQueryExecutor<TParams, TResult>(
    definition: ProgramQueryDefinition<TParams, TResult> | StackQueryDefinition<TParams, TResult>
  ): TypedQueryExecutor<TParams, TResult> {
    return async (params: TParams): Promise<TResult> => {
      const result = await this.readJson<TResult>(definition.path, {
        method: definition.method ?? 'POST',
        body: params,
      });
      if (!definition.schema) {
        return result;
      }
      const parsed = definition.schema.safeParse(result);
      if (!parsed.success) {
        throw new Error(`Query '${definition.name}' failed schema validation`);
      }
      return parsed.data;
    };
  }

  private async readJson<T>(
    path: string,
    options?: { method?: 'GET' | 'POST'; body?: unknown }
  ): Promise<T> {
    const response = await this.authenticatedFetch(this.resolveReadUrl(path), {
      method: options?.method ?? 'GET',
      headers: options?.body === undefined ? undefined : { 'content-type': 'application/json' },
      body: options?.body === undefined ? undefined : JSON.stringify(options.body),
    });

    return parseReadResponse<T>(response, path);
  }

  private async authenticatedFetch(input: string, init?: RequestInit): Promise<Response> {
    const attempt = async (forceRefresh = false): Promise<Response> => {
      const token = await this.connection.getHttpAuthToken(forceRefresh);
      const headers = new Headers(init?.headers ?? undefined);
      if (token) {
        headers.set('authorization', `Bearer ${token}`);
      }
      return this.fetchImpl(input, {
        ...init,
        headers,
      });
    };

    let response = await attempt(false);
    if (!response.ok) {
      const wireErrorCode = response.headers.get('X-Error-Code');
      const errorCode = wireErrorCode ? parseErrorCode(wireErrorCode) : undefined;
      if (errorCode && shouldRefreshToken(errorCode)) {
        this.connection.clearHttpAuthToken();
        response = await attempt(true);
      }
    }

    return response;
  }

  private resolveReadUrl(path: string): string {
    return `${this.httpBaseUrl.replace(/\/$/, '')}${path.startsWith('/') ? path : `/${path}`}`;
  }

  /** Merge the client's default wallet into call options (call options win). */
  private withWallet<T extends BuildOptions>(options?: T): T {
    const merged = { ...(options ?? {}) } as T;
    if (!merged.wallet && this._wallet) {
      merged.wallet = this._wallet;
    }
    return merged;
  }

  static async connect<
    T extends StackDefinition,
    TPrograms extends ProgramMap | undefined = undefined,
  >(
    stack: T,
    options?: ConnectOptions<TPrograms>
  ): Promise<ConnectedArete<StackWithAttachedPrograms<T, TPrograms>, T>> {
    const requestedUrl = options?.url ?? stack.endpoints.ws;
    const autoReconnect = options?.autoReconnect !== false;
    const httpOnly = options?.transport === 'http' || (!requestedUrl && !autoReconnect);
    const url = httpOnly ? null : requestedUrl;
    const httpUrl =
      options?.httpUrl
      ?? stack.endpoints.http
      ?? (requestedUrl ? deriveHttpEndpoint(requestedUrl) : undefined);

    if (!httpOnly && !url) {
      throw new AreteError('WebSocket URL is required (provide url option or define endpoints.ws in stack)', 'INVALID_CONFIG');
    }
    if (!httpUrl) {
      throw new AreteError(
        'HTTP endpoint is required for transport: "http" (provide httpUrl option or define endpoints.http in stack)',
        'INVALID_CONFIG'
      );
    }

    const attachedPrograms = options?.programs as TPrograms;
    const effectiveStack = withPrograms(stack, attachedPrograms);

    const internalOptions: AreteOptionsWithStorage<StackWithAttachedPrograms<T, TPrograms>> = {
      stack: effectiveStack,
      httpUrl,
      storage: options?.storage,
      maxEntriesPerView: options?.maxEntriesPerView,
      flushIntervalMs: options?.flushIntervalMs,
      autoReconnect: options?.autoReconnect,
      reconnectIntervals: options?.reconnectIntervals,
      maxReconnectAttempts: options?.maxReconnectAttempts,
      validateFrames: options?.validateFrames,
      auth: options?.auth,
      wallet: options?.wallet,
      fetch: options?.fetch,
      execution: options?.execution,
    };

    const client = new Arete(url, httpUrl, internalOptions);

    if (!httpOnly && autoReconnect) {
      await client.connection.connect();
    }

    return applyConnectedStackExtensions(client, effectiveStack) as unknown as ConnectedArete<
      StackWithAttachedPrograms<T, TPrograms>,
      T
    >;
  }

  get views(): TypedViews<TStack['views']> {
    return this._views;
  }

  get queries(): QueriesInterface<TStack['queries']> {
    return this._queries;
  }

  get programs(): ProgramsInterface<TStack['programs']> {
    return this._programs;
  }

  get chain(): ChainClient {
    return this._chain;
  }

  /** The default wallet adapter, if one was configured. */
  get wallet(): WalletAdapter | undefined {
    return this._wallet;
  }

  /** The connected wallet address, if a default wallet was configured. */
  get publicKey(): string | undefined {
    return this._wallet?.publicKey;
  }

  /**
   * Set (or clear) the default wallet adapter used for instruction execution.
   * Useful for connecting/disconnecting a wallet after the client is created.
   */
  setWallet(wallet: WalletAdapter | undefined): void {
    this._wallet = wallet;
  }

  /**
   * Sign and send a batch of pre-built instructions as a single transaction.
   *
   * Build instructions with `client.programs.<program>.raw.<name>.build(params)`
   * and compose them here. RPC/compilation/confirmation are owned by the adapter.
   *
   * On failure, the error is parsed against `options.errors` when given,
   * otherwise against error metadata aggregated from all the stack's handlers
   * (deduped by code, first-wins — if the stack bundles programs with
   * overlapping error codes, pass `options.errors` or use the per-instruction
   * call path for precise attribution).
   */
  async transaction(
    instructions: readonly BuiltInstruction[],
    options?: TransactionOptions
  ): Promise<ExecutionResult> {
    const wallet = options?.wallet ?? this._wallet;
    if (!wallet) {
      throw new Error('Wallet required to sign and send transaction');
    }
    try {
      const sendOptions: SendOptions = {
        ...(options?.send ?? {}),
      };
      if (options?.signers !== undefined) {
        sendOptions.signers = options.signers;
      }
      const result = await wallet.signAndSend(instructions, sendOptions);
      return { signature: result.signature, slot: result.slot };
    } catch (err) {
      const programError = parseInstructionError(err, options?.errors ?? this.aggregateErrors());
      if (programError) {
        throw new InstructionError(
          `${programError.name} (${programError.code}): ${programError.message}`,
          programError,
          err
        );
      }
      throw err;
    }
  }

  execute<TPrepared extends PreparedOperation, TSigner = unknown>(
    prepared: TPrepared,
    options?: OperationExecutionOptions<TSigner, TPrepared>
  ): Promise<OperationReceiptFor<TPrepared>> {
    const defaults = this.executionDefaults as OperationExecutionOptions<TSigner, TPrepared> | undefined;
    return executePreparedOperation(this, prepared, {
      wallet: options?.wallet ?? defaults?.wallet,
      send: defaults?.send || options?.send
        ? { ...(defaults?.send ?? {}), ...(options?.send ?? {}) }
        : undefined,
      signers: options?.signers ?? defaults?.signers,
      signerRegistry: options?.signerRegistry ?? defaults?.signerRegistry,
      availableSignerAddresses:
        options?.availableSignerAddresses ?? defaults?.availableSignerAddresses,
      onTransactionStart:
        options?.onTransactionStart ?? defaults?.onTransactionStart,
      onTransactionSuccess:
        options?.onTransactionSuccess ?? defaults?.onTransactionSuccess,
    });
  }

  /** Error metadata from every handler in the stack, deduped by code. */
  private aggregateErrors(): ErrorMetadata[] {
    if (!this._aggregatedErrors) {
      const all: ErrorMetadata[] = [];
      const seen = new Set<number>();
      const collect = (handler: InstructionHandler) => {
        for (const error of handler.errors ?? []) {
          if (!seen.has(error.code)) {
            seen.add(error.code);
            all.push(error);
          }
        }
      };
      for (const program of Object.values(this.stack.programs ?? {})) {
       for (const handler of Object.values(program.rawInstructions ?? {})) {
          collect(handler as InstructionHandler);
        }
      }
      this._aggregatedErrors = all;
    }
    return this._aggregatedErrors;
  }

  get connectionState(): ConnectionState {
    return this.connection.getState();
  }

  get stackName(): string {
    return this.stack.name;
  }

  get store(): StorageAdapter {
    return this.storage;
  }

  onConnectionStateChange(callback: ConnectionStateCallback): UnsubscribeFn {
    return this.connection.onStateChange(callback);
  }

  onFrame(callback: (frame: Frame) => void): UnsubscribeFn {
    return this.connection.onFrame(callback);
  }

  onSocketIssue(callback: SocketIssueCallback): UnsubscribeFn {
    return this.connection.onSocketIssue(callback);
  }

  async connect(): Promise<void> {
    await this.connection.connect();
  }

  disconnect(): void {
    this.subscriptionRegistry.clear();
    this.connection.disconnect();
  }

  isConnected(): boolean {
    return this.connection.isConnected();
  }

  clearStore(): void {
    this.storage.clear();
  }

  getStore(): StorageAdapter {
    return this.storage;
  }

  getConnection(): ConnectionManager {
    return this.connection;
  }

  getSubscriptionRegistry(): SubscriptionRegistry {
    return this.subscriptionRegistry;
  }
}
