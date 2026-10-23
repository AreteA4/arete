/**
 * `@usearete/sdk/program`: the runtime that generated program modules
 * (`a4 install program ... --ts`) import.
 *
 * Everything here builds data: instruction handlers and their Borsh
 * encoding, PDA seeds, account read descriptors, operation metadata and
 * amount conversion. Nothing here opens a connection or touches the host:
 * the module graph reaches no client, WebSocket, `fetch`, `process`,
 * `Buffer` or Node built-in, so a program module bundles into code that
 * must run without network or file access, such as a protocol's handlers.
 *
 * The root `@usearete/sdk` entry re-exports every symbol here, and both
 * entries share one copy of each module, so values from either work with
 * the other.
 */

export {
  account,
  arg,
  buildInstruction,
  bytes,
  createInstructionHandler,
  literal,
  pda,
} from './instructions';
export type {
  AccountMeta,
  ArgSchema,
  ArgStructField,
  ArgType,
  BuildOptions,
  BuiltAccountMeta,
  BuiltInstruction,
  EnumVariant,
  ErrorMetadata,
  InstructionHandler,
  InstructionHandlerConfig,
  SeedDef,
} from './instructions';

export {
  PROGRAM_OPERATION_EXTENSIONS,
  defineProgramExtensions,
  extendProgram,
} from './stack-extensions';
export type { ProgramOperationContext } from './stack-extensions';

export {
  createPublicKeySeed,
  createSeed,
  decodeBase58,
  encodeBase58,
  findProgramAddressSync,
} from './instructions';
export type { ResolvedAccount } from './instructions';

export {
  flowOperation,
  instructionOperation,
  transactionOperation,
} from './program-instructions';
export {
  appendFlowTransactions,
  appendTransactionInstructions,
  createPreparedFlow,
  createPreparedInstruction,
  createPreparedTransaction,
  createPreparedTransactionBody,
  prependFlowTransactionInstructions,
  prependTransactionInstructions,
} from './operations';
export type {
  CreatePreparedFlowInput,
  CreatePreparedInstructionInput,
  CreatePreparedTransactionInput,
  PreparedFlow,
  PreparedInstruction,
  PreparedOperation,
  PreparedTransaction,
  PreparedTransactionBody,
} from './operations';
export { withProgramIdentity, withProgramRead } from './program-sdk';
export type { ProgramSdkIdentity } from './program-sdk';
export { ReadRequestError, programAccountRead } from './read';
export {
  formatRawToUi,
  getMintDecimals,
  parseUiAmountToRaw,
  resolveAmountToRaw,
  toRawAmount,
} from './amounts';
export type { AmountInput } from './amounts';
export {
  ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
  SPL_TOKEN_PROGRAM_ADDRESS,
  SYSTEM_PROGRAM_ADDRESS,
  TOKEN_2022_PROGRAM_ADDRESS,
  deriveAssociatedTokenAccount,
  resolveTokenProgramAddress,
} from './spl';

// Types only: an extension annotates reads and program surfaces with them.
// The values behind them (chain and client) stay out of this entry.
export type { ChainClient } from './chain';
export type { ProgramInterface, TypedAccountReader } from './client';
