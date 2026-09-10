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
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:74b5845c9133060c61b330594b020adb6e19249b05b66091f906fb30e72d5b82',
  programSpecHash: 'arete:h1:program-spec:sha256:33b3ced1cf2a32886de725191b57ed80c735f198e80deb02c072c62ba44c2af4',
  idlContentHash: 'arete:h1:idl-content:sha256:79719791583a0e80d7398252da7c607ad71241a9fbaa83b3764b32ea92e08101',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:af2a015ad666dbc0afc100210cd1bc64e09f80f0c371b84f08280993f477ce7a',
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
release: { programReleaseHash: "arete:h1:program-release:sha256:b8c04409cae62b62cc7abe30274fcd593405ca192522776741ccbbacac185f64", programSpecHash: "arete:h1:program-spec:sha256:33b3ced1cf2a32886de725191b57ed80c735f198e80deb02c072c62ba44c2af4" },
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