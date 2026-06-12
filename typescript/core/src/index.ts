export { Arete } from './client';
export type {
  ConnectOptions,
  AreteOptionsWithStorage,
  InstructionExecutorOptions,
  InstructionExecutor,
  TypedInstruction,
  InstructionsInterface,
} from './client';

export { ConnectionManager } from './connection';
export { SubscriptionRegistry } from './subscription';

export { FrameProcessor } from './frame-processor';
export type { FrameProcessorConfig } from './frame-processor';

export { EntityStore } from './store';
export type { EntityStoreConfig, ViewConfig } from './store';

export type { StorageAdapter, UpdateCallback, RichUpdateCallback, StorageAdapterConfig, ViewSortConfig } from './storage/adapter';
export { MemoryAdapter } from './storage/memory-adapter';

export { parseFrame, parseFrameFromBlob, isValidFrame, isSnapshotFrame, isSubscribedFrame, isEntityFrame } from './frame';
export type { EntityFrame, SnapshotFrame, SnapshotEntity, SubscribedFrame, SortConfig, SortOrder, Frame, FrameMode, FrameOp } from './frame';

export { createUpdateStream, createEntityStream, createRichUpdateStream } from './stream';
export {
  createTypedStateView,
  createTypedListView,
  createTypedViews,
} from './views';

export type {
  ConnectionState,
  Update,
  RichUpdate,
  ViewDef,
  StackDefinition,
  StackInstructionEntry,
  ViewGroup,
  Subscription,
  Schema,
  SchemaResult,
  WatchOptions,
  AreteOptions,
  AreteConfig,
  AuthConfig,
  AuthTokenResult,
  WebSocketFactoryInit,
  TypedViews,
  TypedViewGroup,
  TypedStateView,
  TypedListView,
  SubscribeCallback,
  UnsubscribeFn,
  ConnectionStateCallback,
  SocketIssue,
  SocketIssueCallback,
} from './types';

export { DEFAULT_CONFIG, DEFAULT_MAX_ENTRIES_PER_VIEW, AreteError } from './types';

// Wallet types
export type {
  WalletAdapter,
  WalletState,
  WalletConnectOptions,
  BuiltInstruction,
  BuiltAccountMeta,
  ConfirmationLevel,
  SendOptions,
  SendResult,
} from './wallet/types';

// Instruction execution
export type {
  AccountCategory,
  AccountMeta,
  PdaConfig,
  PdaSeed,
  ResolvedAccount,
  ResolvedAccounts,
  AccountResolutionResult,
  AccountResolutionOptions,
  ArgSchema,
  ArgType,
  ProgramError,
  ErrorMetadata,
  InstructionHandler,
  InstructionHandlerConfig,
  BuildOptions,
  ExecuteOptions,
  ExecutionResult,
  SeedDef,
  PdaDeriveContext,
  PdaFactory,
  ProgramPdas,
} from './instructions';

export {
  resolveAccounts,
  validateAccountResolution,
  findProgramAddress,
  findProgramAddressSync,
  derivePda,
  createSeed,
  createPublicKeySeed,
  decodeBase58,
  encodeBase58,
  serializeInstructionData,
  parseInstructionError,
  formatProgramError,
  InstructionError,
  buildInstruction,
  executeInstruction,
  createInstructionHandler,
  createInstructionExecutor,
  literal,
  account,
  arg,
  bytes,
  pda,
  createProgramPdas,
} from './instructions';
