import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, type BuildOptions, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction, type ProgramOperationContext, type AmountInput, resolveAmountToRaw, toRawAmount } from '@usearete/sdk';

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
  memo: string | null;
}

export const VaultVaultSchema = z.object({
  authority: z.string(),
  balance: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  memo: z.string().nullable(),
}).transform((value) => ({
  authority: value.authority,
  balance: value.balance,
  memo: value.memo,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type VaultStreamProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string };

const VAULT_STREAM_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'AmountTooSmall', msg: 'Amount too small' },
];

export interface DepositParams {
  amount: bigint;
  authority: string;
  vault: string;
  mint: string;
}

export interface DepositSemanticParams {
  amount: AmountInput;
  amountDecimals?: number;
  authority: string;
  vault: string;
  mint: string;
  build?: BuildOptions;
}

export type DepositError = VaultStreamProgramError;

export const depositInstruction = createInstructionHandler<DepositParams, DepositError>({
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  discriminator: [0],
  args: [
    { name: 'amount', type: 'u64' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'vault', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'mint', isSigner: false, isWritable: false, category: 'userProvided' },
  ],
  errors: VAULT_STREAM_PROGRAM_ERRORS,
});

export interface WithdrawParams {
  amount: bigint;
  mint: string;
  authority: string;
  vault: string;
}

export interface WithdrawSemanticParams {
  amount: AmountInput;
  mint: string;
  amountDecimals?: number;
  authority: string;
  vault: string;
  build?: BuildOptions;
}

export type WithdrawError = VaultStreamProgramError;

export const withdrawInstruction = createInstructionHandler<WithdrawParams, WithdrawError>({
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  discriminator: [1],
  args: [
    { name: 'amount', type: 'u64' },
    { name: 'mint', type: 'pubkey' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'vault', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: VAULT_STREAM_PROGRAM_ERRORS,
});

export interface PayFeeParams {
  fee: bigint;
  tip: bigint;
  tipDecimals: number;
  authority: string;
}

export interface PayFeeSemanticParams {
  fee: AmountInput;
  tip: AmountInput;
  tipDecimals: number;
  authority: string;
  build?: BuildOptions;
}

export type PayFeeError = VaultStreamProgramError;

export const payFeeInstruction = createInstructionHandler<PayFeeParams, PayFeeError>({
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  discriminator: [2],
  args: [
    { name: 'fee', type: 'u64' },
    { name: 'tip', type: 'u64' },
    { name: 'tipDecimals', type: 'u8' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
  ],
  errors: VAULT_STREAM_PROGRAM_ERRORS,
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

/** Stack definition for VaultStream with 1 entities */
export const VAULT_STREAM_STACK_CORE = {
  name: 'vault-stream',
  endpoints: {
    ws: 'wss://vault.stack.example.test',
    http: 'https://vault.stack.example.test',
  },
  release: {
    stackManifestHash: 'arete:h1:stack-manifest:sha256:7b3b5a825854a0e561c1a7eeff95e5871be702009b8952b606d07b66a8641b2a',
    liveAlias: 'live',
  },
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
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
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:207e2d6db9dfd5d7bb6e4874fe61cfafc4e2762bf0e29d23cb6a21259d588bac',
      programSpecHash: 'arete:h1:program-spec:sha256:7f8377cbaeffdc6a33986144c5abb640d00d000519185ff85ac1ccfe1917b976',
      idlContentHash: 'arete:h1:idl-content:sha256:e9f168d8b448ac603f8ed6c99f9bb9ba82a0908a723610a5f89877120dfc194c',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:c79563495cb02035c1def338b7349e986cfc64349f306e79d601ad33207f9799',
      gateway: {"chain":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
      accounts: {
        Vault: programAccountRead<VaultVault>({ account: 'Vault', schema: VaultVaultSchema }),
      },
      rawInstructions: {
        deposit: depositInstruction,
        withdraw: withdrawInstruction,
        payFee: payFeeInstruction,
      },
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations(context: ProgramOperationContext) {
          return {
            instructions: {
            deposit: instructionOperation(async (params: DepositSemanticParams) => {
                const { build, amountDecimals, ...rawParams } = params;
                const amountRaw = await resolveAmountToRaw(context.chain, { mint: params.mint, amount: params.amount, decimals: params.amountDecimals });
                const instruction = buildInstruction(depositInstruction, {
                  ...rawParams,
                  amount: amountRaw,
                } as unknown as Record<string, unknown>, build);
                return createPreparedInstruction({
                  name: 'deposit',
                  instruction,
                  artifacts: { instruction },
                  errors: depositInstruction.errors,
                });
              }),
            withdraw: instructionOperation(async (params: WithdrawSemanticParams) => {
                const { build, amountDecimals, ...rawParams } = params;
                const amountRaw = await resolveAmountToRaw(context.chain, { mint: params.mint, amount: params.amount, decimals: params.amountDecimals });
                const instruction = buildInstruction(withdrawInstruction, {
                  ...rawParams,
                  amount: amountRaw,
                } as unknown as Record<string, unknown>, build);
                return createPreparedInstruction({
                  name: 'withdraw',
                  instruction,
                  artifacts: { instruction },
                  errors: withdrawInstruction.errors,
                });
              }),
            payFee: instructionOperation(async (params: PayFeeSemanticParams) => {
                const { build, ...rawParams } = params;
                const feeRaw = toRawAmount(params.fee, 6);
                const tipRaw = toRawAmount(params.tip, params.tipDecimals);
                const instruction = buildInstruction(payFeeInstruction, {
                  ...rawParams,
                  fee: feeRaw,
                  tip: tipRaw,
                } as unknown as Record<string, unknown>, build);
                return createPreparedInstruction({
                  name: 'payFee',
                  instruction,
                  artifacts: { instruction },
                  errors: payFeeInstruction.errors,
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
      release: { programReleaseHash: "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", programSpecHash: "arete:h1:program-spec:sha256:7f8377cbaeffdc6a33986144c5abb640d00d000519185ff85ac1ccfe1917b976" },
      transport: { kind: 'hosted-binding', binding: { endpoint: "https://reads.example.test/vault/", programReadBindingId: "prb_00000000000000000000000000000001", auth: {"mode":"signed_session","required":true,"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"prb_00000000000000000000000000000001","targetKind":"program-read-binding"} } },
    },
  },
} as const;

/** Type alias for the core stack */
export type VaultStreamCoreStack = typeof VAULT_STREAM_STACK_CORE;

/** Entity types in this stack */
export type VaultStreamEntity = Vault;

/** Default export for convenience */
export default VAULT_STREAM_STACK_CORE;