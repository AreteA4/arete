import { Arete as BaseArete, withPrograms } from './client';
import { createSession } from './session';

const Arete = Object.assign(BaseArete, { session: createSession });

export { Arete, withPrograms, createSession };
export type {
  ConnectOptions,
  AreteOptionsWithStorage,
  ConnectedArete,
  MergeProgramMaps,
  TransactionOptions,
  InstructionExecutor,
  TypedInstruction,
  TypedAccountReader,
  TypedQueryExecutor,
  ProgramInterface,
  ProgramsInterface,
  QueriesInterface,
  RawInstructionsInterface,
  RawProgramsInterface,
  StackWithAttachedPrograms,
} from './client';

export {
  PROGRAM_OPERATION_EXTENSIONS,
  STACK_RUNTIME_EXTENSIONS,
  extendStack,
  extendProgram,
  extendPrograms,
  defineStackExtensions,
  defineProgramExtensions,
  getProgramRuntimeExtensions,
  getStackRuntimeExtensions,
  applyConnectedStackExtensions,
} from './stack-extensions';
export type {
  ProgramRuntimeExtensions,
  ProgramRuntimeExtensionCarrier,
  StackRuntimeExtensions,
  StackRuntimeExtensionCarrier,
  StackExtensionInput,
  StackExtensionClient,
  ExtendedStackDefinition,
  ExtendedProgramDefinition,
  ProgramExtensionInput,
  ProgramOperations,
  ProgramOperationsOf,
  ProgramOperationContext,
  ReadArgumentCount,
  ReadArgumentCounts,
  StackConnectedExtensions,
  ConnectedStackClient,
} from './stack-extensions';

export type {
  InstructionOperation,
  TransactionOperation,
  FlowOperation,
  AnyOperation,
  OperationNamespace,
  InstructionOperationNamespace,
  TransactionOperationNamespace,
  FlowOperationNamespace,
} from './program-instructions';
export {
  instructionOperation,
  transactionOperation,
  flowOperation,
} from './program-instructions';

export type {
  Session,
  SessionDefinition,
  SessionOptions,
  SessionMemberOptions,
} from './session';

export { createChainClient, deriveHttpEndpoint } from './chain';
export type {
  ChainClient,
  ChainClock,
  RawAccountInfo,
  MintAccountInfo,
  TokenAccountInfo,
  TokenBalanceInfo,
  NativeBalanceInfo,
  ContextSlotOptions,
  TokenBalanceInput,
} from './chain';

export { createTransactionTransport, TransactionTransportError } from './transactions';
export type {
  TransactionTransport,
  TransactionCommitment,
  TransactionRequestContext,
  LatestBlockhashResult,
  TransactionFeeResult,
  TransactionSimulationOptions,
  TransactionSimulationResult,
  TransactionSendOptions,
  TransactionSendResult,
  TransactionSignatureStatus,
  TransactionTransportErrorBody,
} from './transactions';

export { chainAccountLoader } from './account-loader';
export type { AccountLoader } from './account-loader';

export {
  SPL_TOKEN_PROGRAM_ADDRESS,
  TOKEN_2022_PROGRAM_ADDRESS,
  ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
  SYSTEM_PROGRAM_ADDRESS,
  deriveAssociatedTokenAccount,
  resolveTokenProgramAddress,
} from './spl';

export {
  parseUiAmountToRaw,
  formatRawToUi,
  toRawAmount,
  safeToRawAmount,
  getMintDecimals,
  resolveAmount,
  resolveAmountToRaw,
  resolveAmountsToRaw,
} from './amounts';
export type { AmountInput } from './amounts';

export { stringifyBigints } from './display';
export type { StringifiedBigints } from './display';

export {
  programAccountRead,
  programQuery,
  stackQuery,
  ReadRequestError,
} from './read';

export { ConnectionManager, isHostedAreteEndpoint } from './connection';
export {
  SubscriptionRegistry,
  canonicalQueryKey,
  createSubscriptionId,
  normalizeSubscription,
  normalizeSubscriptionQuery,
  normalizeSubscriptionRequest,
  validateSubscriptionId,
} from './subscription';
export { QueryStore } from './query-store';

export { FrameProcessor } from './frame-processor';
export { ProcessedSlotTimeoutError } from './frame-processor';
export type {
  FrameProcessorConfig,
  FrameValidationDiagnostic,
  WaitForProcessedSlotOptions,
} from './frame-processor';

export type { StorageAdapter, UpdateCallback, RichUpdateCallback, StorageAdapterConfig, ViewSortConfig } from './storage/adapter';
export { MemoryAdapter } from './storage/memory-adapter';

export {
  parseFrame,
  parseFrameFromBlob,
  isValidFrame,
  isSnapshotFrame,
  isSubscribedFrame,
  isUnsubscribedFrame,
  isErrorFrame,
  isEntityFrame,
} from './frame';
export type {
  EntityFrame,
  SnapshotFrame,
  SnapshotEntity,
  SubscribedFrame,
  UnsubscribedFrame,
  ErrorFrame,
  SortConfig,
  SortOrder,
  Frame,
  FrameMode,
  FrameOp,
} from './frame';

export { createUpdateStream, createEntityStream, createRichUpdateStream } from './stream';
export {
  createTypedStateView,
  createTypedListView,
  createTypedViews,
  serializeViewKey,
} from './views';

export type {
  ConnectionState,
  Update,
  RichUpdate,
  ViewDef,
  ViewKeyValue,
  ViewKeyFields,
  DefaultViewKey,
  StackDefinition,
  StackEndpoints,
  ReadTransportMethod,
  ProgramAccountReadDefinition,
  ProgramQueryDefinition,
  StackQueryDefinition,
  ProgramSdkDefinition,
  ViewGroup,
  Subscription,
  SubscriptionQuery,
  SubscriptionRequest,
  SubscriptionSnapshotOptions,
  SubscriptionIdentity,
  SubscriptionOptions,
  QueryLease,
  QuerySnapshot,
  Schema,
  SchemaResult,
  WatchOptions,
  AreteOptions,
  AreteConfig,
  AuthConfig,
  AuthTokenResult,
  AuthTokenRequest,
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

export type {
  OperationKind,
  NonEmptyReadonlyArray,
  JsonPrimitive,
  JsonValue,
  JsonObject,
  PreparedOperationDescription,
  OperationInspection,
  OperationInspectionOptions,
  PreparedTransactionBody,
  OperationPlan,
  PreparedInstruction,
  PreparedTransaction,
  PreparedFlow,
  PreparedOperation,
  PreparedTransactionInstruction,
  PreparedTransactionOperation,
  CreatePreparedInstructionInput,
  CreatePreparedTransactionInput,
  CreatePreparedFlowInput,
  OperationTransactionReceipt,
  SingleTransactionOperationReceipt,
  FlowOperationReceipt,
  OperationReceiptFor,
  OperationExecutionEvent,
  OperationExecutionSuccessEvent,
  OperationCallbackPhase,
  OperationExecutionOptions,
  OperationExecutionHost,
} from './operations';

export {
  createPreparedInstruction,
  createPreparedTransactionBody,
  createPreparedTransaction,
  createPreparedFlow,
  prependTransactionInstructions,
  appendTransactionInstructions,
  appendFlowTransactions,
  prependFlowTransactionInstructions,
  executePreparedOperation,
  inspectPreparedOperation,
  unwrapOperationExecutionError,
  OperationCallbackError,
  OperationExecutionError,
  toJsonValue,
  describePreparedOperation,
  formatPreparedOperation,
} from './operations';

export type { SignerRegistry } from './signer-registry';
export { createSignerRegistry } from './signer-registry';

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
  TransactionInspectionOptions,
  TransactionInspectionResult,
  WalletExecutionContext,
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
  TransactionFailureStatus,
  TransactionFailurePhase,
  ConfirmedTransactionOutcome,
  NotSubmittedTransactionOutcome,
  SubmittedUnknownTransactionOutcome,
  ChainFailedTransactionOutcome,
  TransactionFailureOutcome,
  TransactionOutcome,
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
  TransactionExecutionError,
  getTransactionFailureOutcome,
  normalizeTransactionError,
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
