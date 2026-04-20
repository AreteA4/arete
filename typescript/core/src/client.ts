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
import type { WalletAdapter } from './wallet/types';
import type { InstructionHandler, ExecuteOptions, ExecutionResult } from './instructions';
import { executeInstruction } from './instructions';

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
}

/** @deprecated Use ConnectOptions instead */
export interface AreteOptionsWithStorage<TStack extends StackDefinition> extends AreteOptions<TStack> {
  storage?: StorageAdapter;
  maxEntriesPerView?: number | null;
  flushIntervalMs?: number;
  auth?: import('./types').AuthConfig;
}

export interface InstructionExecutorOptions extends Omit<ExecuteOptions, 'wallet'> {
  wallet: WalletAdapter;
}

export type InstructionExecutor = (
  args: Record<string, unknown>,
  options: InstructionExecutorOptions
) => Promise<ExecutionResult>;

export type InstructionsInterface<TInstructions extends Record<string, InstructionHandler> | undefined> = 
  TInstructions extends Record<string, InstructionHandler>
    ? { [K in keyof TInstructions]: InstructionExecutor }
    : {};

export class Arete<TStack extends StackDefinition> {
  private readonly connection: ConnectionManager;
  private readonly storage: StorageAdapter;
  private readonly processor: FrameProcessor;
  private readonly subscriptionRegistry: SubscriptionRegistry;
  private readonly _views: TypedViews<TStack['views']>;
  private readonly stack: TStack;
  private readonly _instructions: InstructionsInterface<TStack['instructions']>;

  private constructor(
    url: string,
    options: AreteOptionsWithStorage<TStack>
  ) {
    this.stack = options.stack;
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
    const instructions = {} as Record<string, InstructionExecutor>;
    
    if (this.stack.instructions) {
      for (const [name, handler] of Object.entries(this.stack.instructions)) {
        instructions[name] = (args: Record<string, unknown>, options: InstructionExecutorOptions) => {
          return executeInstruction(handler as InstructionHandler, args, options);
        };
      }
    }
    
    return instructions as InstructionsInterface<TStack['instructions']>;
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
