import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, type BuildOptions, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction, type ProgramOperationContext, type AmountInput, resolveAmountToRaw, toRawAmount } from '@usearete/sdk';

export interface Vault {
  authority: string;
  balance: bigint;
  memo: string | null;
}

export const VaultSchema = z.object({
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
export type VaultProgramError =
  | { code: 0; name: 'AmountTooSmall'; msg: string };

const VAULT_PROGRAM_ERRORS: ErrorMetadata[] = [
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

export type DepositError = VaultProgramError;

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
  errors: VAULT_PROGRAM_ERRORS,
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

export type WithdrawError = VaultProgramError;

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
  errors: VAULT_PROGRAM_ERRORS,
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

export type PayFeeError = VaultProgramError;

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
  errors: VAULT_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'vault' */
export const VAULT = {
  name: 'vault',
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:70e4dac83b8e8a33162d145466bcc8706aad765044624bf9c3f4d87d3c6e1ca0',
  programSpecHash: 'arete:h1:program-spec:sha256:7f8377cbaeffdc6a33986144c5abb640d00d000519185ff85ac1ccfe1917b976',
  idlContentHash: 'arete:h1:idl-content:sha256:e9f168d8b448ac603f8ed6c99f9bb9ba82a0908a723610a5f89877120dfc194c',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:c79563495cb02035c1def338b7349e986cfc64349f306e79d601ad33207f9799',
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
  accounts: {
    Vault: programAccountRead<Vault>({ account: 'Vault', schema: VaultSchema }),
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
} as const;

/** Release and explicit read transport for 'vault' */
export const VAULT_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", programSpecHash: "arete:h1:program-spec:sha256:7f8377cbaeffdc6a33986144c5abb640d00d000519185ff85ac1ccfe1917b976" },
transport: { kind: 'hosted-binding', binding: { endpoint: "https://reads.example.test/vault/", programReadBindingId: "prb_00000000000000000000000000000001", auth: {"mode":"signed_session","required":true,"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"prb_00000000000000000000000000000001","targetKind":"program-read-binding"} } },
} as const;

/** All portable programs from the Vault stack */
export const VAULT_PROGRAMS = {
  vault: VAULT,
} as const;

/** Parallel release/read metadata keyed identically to VAULT_PROGRAMS */
export const VAULT_PROGRAM_READS = {
  vault: VAULT_READ,
} as const;

export type VaultPrograms = typeof VAULT_PROGRAMS;

export default VAULT_PROGRAMS;