import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface Wallet {
}

export const WalletSchema = z.object({});

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Program errors for this stack (none declared in the IDL). */
export type SystemProgramError = never;

const SYSTEM_PROGRAM_ERRORS: ErrorMetadata[] = [];

export interface TransferParams {
  lamports: bigint;
  from: string;
  to: string;
}

export type TransferError = SystemProgramError;

export const transferInstruction = createInstructionHandler<TransferParams, TransferError>({
  programId: '11111111111111111111111111111111',
  discriminator: [2, 0, 0, 0],
  args: [
    { name: 'lamports', type: 'u64' },
  ],
  accounts: [
    { name: 'from', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'to', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: SYSTEM_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'system' */
export const SYSTEM = {
  name: 'system',
  programId: '11111111111111111111111111111111',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:a1a70a9899c59464d08181e638ebe08e0693a01f2704f33e4467e172ce0fc5fe',
  programSpecHash: 'arete:h1:program-spec:sha256:c63f5b5944e7b9fc60d4c1a0abbc7b84c26e55c21475f12ffd7a7fe62a73740d',
  idlContentHash: 'arete:h1:idl-content:sha256:2b39a903f01e12242135db13cf8d9023704dc4a7313bb541d89a245187183fe7',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:478e5b307d07ba408dd9b5bb0a56ac1ca89d0d4e02da4252c35d9fcd3c54f406',
  accounts: {
    Wallet: programAccountRead<Wallet>({ account: 'Wallet', schema: WalletSchema }),
  },
  rawInstructions: {
    transfer: transferInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        transfer: instructionOperation(async (params: TransferParams) => {
          const instruction = buildInstruction(transferInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'transfer',
            instruction,
            artifacts: { instruction },
            errors: transferInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Release and explicit read transport for 'system' */
export const SYSTEM_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:c2fccc3dda11c72cb562d9275ebf984f23e4bba63d809ee23510fa27501c7bd2", programSpecHash: "arete:h1:program-spec:sha256:c63f5b5944e7b9fc60d4c1a0abbc7b84c26e55c21475f12ffd7a7fe62a73740d" },
transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
} as const;

/** All portable programs from the System stack */
export const SYSTEM_PROGRAMS = {
  system: SYSTEM,
} as const;

/** Parallel release/read metadata keyed identically to SYSTEM_PROGRAMS */
export const SYSTEM_PROGRAM_READS = {
  system: SYSTEM_READ,
} as const;

export type SystemPrograms = typeof SYSTEM_PROGRAMS;

export default SYSTEM_PROGRAMS;