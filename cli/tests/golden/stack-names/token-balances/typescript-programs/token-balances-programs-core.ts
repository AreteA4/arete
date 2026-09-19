import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface Vault {
  authority: string;
  balance: bigint;
}

export const VaultSchema = z.object({
  authority: z.string(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  balance: value.balance,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type TokenBalancesProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string };

const TOKEN_BALANCES_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'AmountTooSmall', msg: 'Amount too small' },
];

export interface DepositParams {
  amount: bigint;
  authority: string;
  vault: string;
}

export type DepositError = TokenBalancesProgramError;

export const depositInstruction = createInstructionHandler<DepositParams, DepositError>({
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  discriminator: [0],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'vault', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: TOKEN_BALANCES_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'vault' */
export const VAULT = {
  name: 'vault',
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:faa2e70cd0bc1bc2e84ef653a31755bcdaf7fd6a6f86a37f4353045c0e56b363',
  programSpecHash: 'arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba',
  idlContentHash: 'arete:h1:idl-content:sha256:85aefcaba918c8b89b2b25ef683a17be27b36c86eaf5be939303f62af74ea044',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:3de7188f6a21b4dfc7ca7e7db7b0730d692e33c56ec4985495382e73e0fd3140',
  accounts: {
    Vault: programAccountRead<Vault>({ account: 'Vault', schema: VaultSchema }),
  },
  rawInstructions: {
    deposit: depositInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        deposit: instructionOperation(async (params: DepositParams) => {
          const instruction = buildInstruction(depositInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'deposit',
            instruction,
            artifacts: { instruction },
            errors: depositInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Release and explicit read transport for 'vault' */
export const VAULT_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:472ece19e168367b6bd0a61a8e5908345db325bd1ac3f7d31febdd6a3fe09777", programSpecHash: "arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba" },
transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
} as const;

/** All portable programs from the token-balances stack */
export const TOKEN_BALANCES_PROGRAMS = {
  vault: VAULT,
} as const;

/** Parallel release/read metadata keyed identically to TOKEN_BALANCES_PROGRAMS */
export const TOKEN_BALANCES_PROGRAM_READS = {
  vault: VAULT_READ,
} as const;

export type TokenBalancesPrograms = typeof TOKEN_BALANCES_PROGRAMS;

export default TOKEN_BALANCES_PROGRAMS;