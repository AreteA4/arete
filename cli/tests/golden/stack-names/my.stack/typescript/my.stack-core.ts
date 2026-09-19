import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface VaultBalance {
  amount: bigint | null;
}

export interface VaultId {
  address: string;
}

export interface Vault {
  balance: VaultBalance;
  id: VaultId;
}

export const VaultBalanceSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  amount: value.amount,
}));

export const VaultBalancePatchSchema = z.object({
  amount: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)).nullable().optional(),
}).transform((value) => ({
  ...(value.amount !== undefined ? { amount: value.amount } : {}),
}));

export const VaultIdSchema = z.object({
  address: z.string(),
}).transform((value) => ({
  address: value.address,
}));

export const VaultIdPatchSchema = z.object({
  address: z.string().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
}));

export const VaultSchema = z.object({
  balance: VaultBalanceSchema,
  id: VaultIdSchema,
}).transform((value) => ({
  balance: value.balance,
  id: value.id,
}));

export const VaultPatchSchema = z.object({
  balance: VaultBalancePatchSchema.optional(),
  id: VaultIdPatchSchema.optional(),
}).transform((value) => ({
  ...(value.balance !== undefined ? { balance: value.balance } : {}),
  ...(value.id !== undefined ? { id: value.id } : {}),
}));

export const VaultCompletedSchema = z.object({
  balance: VaultBalanceSchema,
  id: VaultIdSchema,
}).transform((value) => ({
  balance: value.balance,
  id: value.id,
}));

export interface VaultVault {
  authority: string;
  balance: bigint;
}

export const VaultVaultSchema = z.object({
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
export type MyStackProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string };

const MY_STACK_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'AmountTooSmall', msg: 'Amount too small' },
];

export interface DepositParams {
  amount: bigint;
  authority: string;
  vault: string;
}

export type DepositError = MyStackProgramError;

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
  errors: MY_STACK_PROGRAM_ERRORS,
});

// ============================================================================
// View Definition Types (framework-agnostic)
// ============================================================================

export type ViewKeyFields<TKey> = unknown extends TKey
  ? readonly string[]
  : TKey extends object
    ? readonly Extract<keyof TKey, string>[]
    : readonly string[];

/** View definition with embedded entity and state-key types */
export interface ViewDef<T, TMode extends 'state' | 'list', TKey = unknown> {
  readonly mode: TMode;
  readonly view: string;
  readonly keyFields?: ViewKeyFields<TKey>;
  /** Phantom field for type inference - not present at runtime */
  readonly _entity?: T;
  readonly _key?: TKey;
}

/** Helper to create typed state view definitions (keyed lookups) */
function stateView<T, TKey = unknown>(
  view: string,
  keyFields: ViewKeyFields<TKey>
): ViewDef<T, 'state', TKey> {
  return { mode: 'state', view, keyFields } as const;
}

/** Helper to create typed list view definitions (collections) */
function listView<T>(view: string): ViewDef<T, 'list'> {
  return { mode: 'list', view } as const;
}

// ============================================================================
// Stack Definition
// ============================================================================

/** Stack definition for my.stack with 1 entities */
export const MY_STACK_STACK_CORE = {
  name: 'my.stack',
  endpoints: {
    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })
    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })
  },
  views: {
    Vault: {
      state: stateView<Vault, { address: string }>('Vault/state', ['address']),
      list: listView<Vault>('Vault/list'),
    },
  },
  schemas: {
    VaultBalance: VaultBalanceSchema,
    VaultCompleted: VaultCompletedSchema,
    VaultId: VaultIdSchema,
    Vault: VaultSchema,
    VaultVault: VaultVaultSchema,
  },
  patchSchemas: {
    Vault: VaultPatchSchema,
  },
  programs: {
    vault: {
      name: 'vault',
      programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:ff655190ce7ef9023a58300332fa23db04492f94a14da183c984331a98b61789',
      programSpecHash: 'arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba',
      idlContentHash: 'arete:h1:idl-content:sha256:85aefcaba918c8b89b2b25ef683a17be27b36c86eaf5be939303f62af74ea044',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:3de7188f6a21b4dfc7ca7e7db7b0730d692e33c56ec4985495382e73e0fd3140',
      accounts: {
        Vault: programAccountRead<VaultVault>({ account: 'Vault', schema: VaultVaultSchema }),
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
    },
  },
  programReads: {
    vault: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:472ece19e168367b6bd0a61a8e5908345db325bd1ac3f7d31febdd6a3fe09777", programSpecHash: "arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
  },
} as const;

/** Type alias for the core stack */
export type MyStackCoreStack = typeof MY_STACK_STACK_CORE;

/** Entity types in this stack */
export type MyStackEntity = Vault;

/** Default export for convenience */
export default MY_STACK_STACK_CORE;