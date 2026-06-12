import type {
  ConnectionState,
  StackDefinition,
  AreteOptions,
  TypedViews,
  ConnectionStateCallback,
  SocketIssueCallback,
  UnsubscribeFn,
} from './types';
import { AreteError } from './types';
import { ConnectionManager } from './connection';
import { FrameProcessor } from './frame-processor';
import { MemoryAdapter } from './storage/memory-adapter';
import type { StorageAdapter } from './storage/adapter';
import { SortedStorageDecorator } from './storage/sorted-decorator';
import { SubscriptionRegistry } from './subscription';
import { createTypedViews } from './views';
import type { Frame } from './frame';
import type { WalletAdapter, BuiltInstruction, SendOptions } from './wallet/types';
import type {
  InstructionHandler,
  ExecuteOptions,
  ExecutionResult,
  BuildOptions,
} from './instructions';
import type { ErrorMetadata } from './instructions';
import {
  executeInstruction,
  buildInstruction,
  parseInstructionError,
  InstructionError,
} from './instructions';

export interface ConnectOptions {
  url?: string;
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
}

/** @deprecated Use ConnectOptions instead */
export interface AreteOptionsWithStorage<TStack extends StackDefinition> extends AreteOptions<TStack> {
  storage?: StorageAdapter;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  auth?: import('./types').AuthConfig;
  wallet?: WalletAdapter;
}

/**
 * Options accepted when calling a typed instruction.
 * `wallet` is optional when a default wallet was provided to the client.
 */
export interface InstructionExecutorOptions extends ExecuteOptions {
  wallet?: WalletAdapter;
}

/**
 * A typed, callable instruction.
 *
 * Calling it builds + signs + sends the transaction. The attached `build`
 * method is a pure prepare step that returns a {@link BuiltInstruction} for
 * batching/composition.
 */
export type TypedInstruction<TParams, TError> = {
  (params: TParams, options?: InstructionExecutorOptions): Promise<ExecutionResult>;
  build(params: TParams, options?: BuildOptions): BuiltInstruction;
  /** Phantom error type for downstream inference. */
  readonly _error?: TError;
};

/**
 * Maps one stack-definition instruction entry (handler or per-program map of
 * handlers) to its typed call surface.
 */
type TypedInstructionFor<TEntry> = TEntry extends InstructionHandler<infer P, infer E>
  ? TypedInstruction<P, E>
  : // eslint-disable-next-line @typescript-eslint/no-explicit-any
    TEntry extends Record<string, InstructionHandler<any, any>>
    ? {
        [K in keyof TEntry]: TEntry[K] extends InstructionHandler<infer P, infer E>
          ? TypedInstruction<P, E>
          : TypedInstruction<Record<string, unknown>, unknown>;
      }
    : TypedInstruction<Record<string, unknown>, unknown>;

export type InstructionsInterface<
  TInstructions extends Record<string, import('./types').StackInstructionEntry> | undefined,
> =
  TInstructions extends Record<string, import('./types').StackInstructionEntry>
    ? { [K in keyof TInstructions]: TypedInstructionFor<TInstructions[K]> }
    : Record<string, never>;

/** @deprecated Retained for backward compatibility; prefer {@link TypedInstruction}. */
export type InstructionExecutor = TypedInstruction<Record<string, unknown>, unknown>;

/**
 * Distinguishes a handler from a per-program map of handlers in a stack
 * definition's `instructions` block. Handlers are the only entries with a
 * `build` function.
 */
export function isInstructionHandler(
  entry: import('./types').StackInstructionEntry
): entry is InstructionHandler {
  return typeof (entry as InstructionHandler).build === 'function';
}

export class Arete<TStack extends StackDefinition> {
  private readonly connection: ConnectionManager;
  private readonly storage: StorageAdapter;
  private readonly processor: FrameProcessor;
  private readonly subscriptionRegistry: SubscriptionRegistry;
  private readonly _views: TypedViews<TStack['views']>;
  private readonly stack: TStack;
  private readonly _instructions: InstructionsInterface<TStack['instructions']>;
  private _wallet?: WalletAdapter;
  private _aggregatedErrors?: ErrorMetadata[];

  private constructor(
    url: string,
    options: AreteOptionsWithStorage<TStack>
  ) {
    this.stack = options.stack;
    this._wallet = options.wallet;
    this.storage = new SortedStorageDecorator(options.storage ?? new MemoryAdapter());
    this.processor = new FrameProcessor(this.storage, {
      maxEntriesPerView: options.maxEntriesPerView,
      flushIntervalMs: options.flushIntervalMs,
      schemas: options.validateFrames ? this.stack.schemas : undefined,
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
    this._instructions = this.buildInstructions();
  }

  private buildInstructions(): InstructionsInterface<TStack['instructions']> {
    const instructions = {} as Record<
      string,
      | TypedInstruction<Record<string, unknown>, unknown>
      | Record<string, TypedInstruction<Record<string, unknown>, unknown>>
    >;

    if (this.stack.instructions) {
      for (const [name, entry] of Object.entries(this.stack.instructions)) {
        if (isInstructionHandler(entry)) {
          instructions[name] = this.createTypedInstruction(entry);
        } else {
          // Multi-program stacks namespace handlers one level deep.
          const nested: Record<string, TypedInstruction<Record<string, unknown>, unknown>> = {};
          for (const [instructionName, handler] of Object.entries(entry)) {
            nested[instructionName] = this.createTypedInstruction(handler as InstructionHandler);
          }
          instructions[name] = nested;
        }
      }
    }

    return instructions as InstructionsInterface<TStack['instructions']>;
  }

  private createTypedInstruction(
    handler: InstructionHandler
  ): TypedInstruction<Record<string, unknown>, unknown> {
    const fn = ((
      params: Record<string, unknown>,
      options?: InstructionExecutorOptions
    ) => {
      return executeInstruction(handler, params, this.withWallet(options));
    }) as TypedInstruction<Record<string, unknown>, unknown>;

    fn.build = (params: Record<string, unknown>, options?: BuildOptions) => {
      return buildInstruction(handler, params, this.withWallet(options));
    };

    return fn;
  }

  /** Merge the client's default wallet into call options (call options win). */
  private withWallet<T extends BuildOptions>(options?: T): T {
    const merged = { ...(options ?? {}) } as T;
    if (!merged.wallet && this._wallet) {
      merged.wallet = this._wallet;
    }
    return merged;
  }

  static async connect<T extends StackDefinition>(
    stack: T,
    options?: ConnectOptions
  ): Promise<Arete<T>> {
    const url = options?.url ?? stack.url;

    if (!url) {
      throw new AreteError('URL is required (provide url option or define url in stack)', 'INVALID_CONFIG');
    }

    const internalOptions: AreteOptionsWithStorage<T> = {
      stack,
      storage: options?.storage,
      maxEntriesPerView: options?.maxEntriesPerView,
      flushIntervalMs: options?.flushIntervalMs,
      autoReconnect: options?.autoReconnect,
      reconnectIntervals: options?.reconnectIntervals,
      maxReconnectAttempts: options?.maxReconnectAttempts,
      validateFrames: options?.validateFrames,
      auth: options?.auth,
      wallet: options?.wallet,
    };

    const client = new Arete(url, internalOptions);

    if (options?.autoReconnect !== false) {
      await client.connection.connect();
    }

    return client;
  }

  get views(): TypedViews<TStack['views']> {
    return this._views;
  }

  get instructions(): InstructionsInterface<TStack['instructions']> {
    return this._instructions;
  }

  /** The default wallet adapter, if one was configured. */
  get wallet(): WalletAdapter | undefined {
    return this._wallet;
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
   * Build instructions with `client.instructions.<name>.build(params)` and
   * compose them here. RPC/compilation/confirmation are owned by the adapter.
   *
   * On failure, the error is parsed against `options.errors` when given,
   * otherwise against error metadata aggregated from all the stack's handlers
   * (deduped by code, first-wins — if the stack bundles programs with
   * overlapping error codes, pass `options.errors` or use the per-instruction
   * call path for precise attribution).
   */
  async transaction(
    instructions: BuiltInstruction[],
    options?: { wallet?: WalletAdapter; send?: SendOptions; errors?: ErrorMetadata[] }
  ): Promise<ExecutionResult> {
    const wallet = options?.wallet ?? this._wallet;
    if (!wallet) {
      throw new Error('Wallet required to sign and send transaction');
    }
    try {
      const result = await wallet.signAndSend(instructions, options?.send ?? {});
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
      for (const entry of Object.values(this.stack.instructions ?? {})) {
        if (isInstructionHandler(entry)) {
          collect(entry);
        } else {
          for (const handler of Object.values(entry)) {
            collect(handler as InstructionHandler);
          }
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
