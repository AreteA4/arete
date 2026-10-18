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

export { instructionOperation } from './program-instructions';
export { createPreparedInstruction } from './operations';
export { withProgramIdentity, withProgramRead } from './program-sdk';
export type { ProgramSdkIdentity } from './program-sdk';
export { programAccountRead } from './read';
export { resolveAmountToRaw, toRawAmount } from './amounts';
export type { AmountInput } from './amounts';
