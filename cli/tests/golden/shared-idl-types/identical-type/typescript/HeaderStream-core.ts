import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk';

export interface AlphaVaultId {
  address: string;
}

export interface AlphaVaultState {
  header: Header | null;
}

export interface AlphaVault {
  id: AlphaVaultId;
  state: AlphaVaultState;
}

export interface Header {
  version: number;
  owner: string;
}

export const HeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
}));

export const HeaderPatchSchema = z.object({
  version: z.number().optional(),
  owner: z.string().optional(),
}).transform((value) => ({
  ...(value.version !== undefined ? { version: value.version } : {}),
  ...(value.owner !== undefined ? { owner: value.owner } : {}),
}));

export const AlphaVaultIdSchema = z.object({
  address: z.string(),
}).transform((value) => ({
  address: value.address,
}));

export const AlphaVaultIdPatchSchema = z.object({
  address: z.string().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
}));

export const AlphaVaultStateSchema = z.object({
  header: HeaderSchema.nullable().optional(),
}).transform((value) => ({
  header: value.header,
}));

export const AlphaVaultStatePatchSchema = z.object({
  header: HeaderPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.header !== undefined ? { header: value.header } : {}),
}));

export const AlphaVaultSchema = z.object({
  id: AlphaVaultIdSchema,
  state: AlphaVaultStateSchema,
}).transform((value) => ({
  id: value.id,
  state: value.state,
}));

export const AlphaVaultPatchSchema = z.object({
  id: AlphaVaultIdPatchSchema.optional(),
  state: AlphaVaultStatePatchSchema.optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
}));

export const AlphaVaultCompletedSchema = z.object({
  id: AlphaVaultIdSchema,
  state: AlphaVaultStateSchema,
}).transform((value) => ({
  id: value.id,
  state: value.state,
}));

export interface BetaVaultId {
  address: string;
}

export interface BetaVaultState {
  header: Header | null;
}

export interface BetaVault {
  id: BetaVaultId;
  state: BetaVaultState;
}

export const BetaVaultIdSchema = z.object({
  address: z.string(),
}).transform((value) => ({
  address: value.address,
}));

export const BetaVaultIdPatchSchema = z.object({
  address: z.string().optional(),
}).transform((value) => ({
  ...(value.address !== undefined ? { address: value.address } : {}),
}));

export const BetaVaultStateSchema = z.object({
  header: HeaderSchema.nullable().optional(),
}).transform((value) => ({
  header: value.header,
}));

export const BetaVaultStatePatchSchema = z.object({
  header: HeaderPatchSchema.nullable().optional(),
}).transform((value) => ({
  ...(value.header !== undefined ? { header: value.header } : {}),
}));

export const BetaVaultSchema = z.object({
  id: BetaVaultIdSchema,
  state: BetaVaultStateSchema,
}).transform((value) => ({
  id: value.id,
  state: value.state,
}));

export const BetaVaultPatchSchema = z.object({
  id: BetaVaultIdPatchSchema.optional(),
  state: BetaVaultStatePatchSchema.optional(),
}).transform((value) => ({
  ...(value.id !== undefined ? { id: value.id } : {}),
  ...(value.state !== undefined ? { state: value.state } : {}),
}));

export const BetaVaultCompletedSchema = z.object({
  id: BetaVaultIdSchema,
  state: BetaVaultStateSchema,
}).transform((value) => ({
  id: value.id,
  state: value.state,
}));

export interface AlphaHeader {
  version: number;
  owner: string;
}

export interface Vault {
  authority: string;
  header: AlphaHeader;
}

export interface BetaHeader {
  version: number;
  owner: string;
}

export interface BetaVault2 {
  authority: string;
  header: BetaHeader;
}

export const AlphaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
}));

export const VaultSchema = z.object({
  authority: z.string(),
  header: z.lazy(() => AlphaHeaderSchema),
}).transform((value) => ({
  authority: value.authority,
  header: value.header,
}));

export const BetaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
}));

export const BetaVault2Schema = z.object({
  authority: z.string(),
  header: z.lazy(() => BetaHeaderSchema),
}).transform((value) => ({
  authority: value.authority,
  header: value.header,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Program errors for this stack (none declared in the IDL). */
export type HeaderStreamAlphaProgramError = never;

const HEADER_STREAM_ALPHA_PROGRAM_ERRORS: ErrorMetadata[] = [];

/** Program errors for this stack (none declared in the IDL). */
export type HeaderStreamBetaProgramError = never;

const HEADER_STREAM_BETA_PROGRAM_ERRORS: ErrorMetadata[] = [];

export interface HeaderInput {
  version: number;
  owner: string;
}

export interface AlphaConfigureParams {
  header: HeaderInput;
  authority: string;
  vault: string;
}

export type AlphaConfigureError = HeaderStreamAlphaProgramError;

export const alphaConfigureInstruction = createInstructionHandler<AlphaConfigureParams, AlphaConfigureError>({
  programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
  discriminator: [0],
  args: [
    { name: 'header', type: { struct: [{ name: 'version', type: 'u8' }, { name: 'owner', type: 'pubkey' }] } },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'vault', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: HEADER_STREAM_ALPHA_PROGRAM_ERRORS,
});

export interface BetaConfigureParams {
  header: HeaderInput;
  authority: string;
  vault: string;
}

export type BetaConfigureError = HeaderStreamBetaProgramError;

export const betaConfigureInstruction = createInstructionHandler<BetaConfigureParams, BetaConfigureError>({
  programId: 'Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS',
  discriminator: [0],
  args: [
    { name: 'header', type: { struct: [{ name: 'version', type: 'u8' }, { name: 'owner', type: 'pubkey' }] } },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'vault', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: HEADER_STREAM_BETA_PROGRAM_ERRORS,
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

/** Stack definition for HeaderStream with 2 entities */
export const HEADER_STREAM_STACK_CORE = {
  name: 'header-stream',
  endpoints: {
    ws: '', // TODO: Set after first deployment or pass useArete(..., { url })
    http: '', // TODO: Set after first deployment or pass useArete(..., { httpUrl })
  },
  views: {
    AlphaVault: {
      state: stateView<AlphaVault, { address: string }>('AlphaVault/state', ['address']),
      list: listView<AlphaVault>('AlphaVault/list'),
    },
    BetaVault: {
      state: stateView<BetaVault, { address: string }>('BetaVault/state', ['address']),
      list: listView<BetaVault>('BetaVault/list'),
    },
  },
  schemas: {
    AlphaHeader: AlphaHeaderSchema,
    AlphaVaultCompleted: AlphaVaultCompletedSchema,
    AlphaVaultId: AlphaVaultIdSchema,
    AlphaVault: AlphaVaultSchema,
    AlphaVaultState: AlphaVaultStateSchema,
    BetaHeader: BetaHeaderSchema,
    BetaVault2: BetaVault2Schema,
    BetaVaultCompleted: BetaVaultCompletedSchema,
    BetaVaultId: BetaVaultIdSchema,
    BetaVault: BetaVaultSchema,
    BetaVaultState: BetaVaultStateSchema,
    Header: HeaderSchema,
    Vault: VaultSchema,
  },
  patchSchemas: {
    AlphaVault: AlphaVaultPatchSchema,
    BetaVault: BetaVaultPatchSchema,
  },
  programs: {
    alpha: {
      name: 'alpha',
      programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:020cfbb1ce5303c5fd7b5a4849eef9338c866ef9c3844c9cf9995befa13178dd',
      programSpecHash: 'arete:h1:program-spec:sha256:07ca57c38922e4521a86f57a079fc28eec26733af78a3bef1af19c69c65b88ab',
      idlContentHash: 'arete:h1:idl-content:sha256:d4ea46182744c2e5a400d3e980ca3a3f667aa11b8d47c066aab9590cce12de34',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:0424abdf515c78e92210c85712819d5919bde03a5ea2142ec2e3c40cfb385db2',
      accounts: {
        Vault: programAccountRead<Vault>({ account: 'Vault', schema: VaultSchema }),
      },
      rawInstructions: {
        configure: alphaConfigureInstruction,
      },
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return {
            instructions: {
            configure: instructionOperation(async (params: AlphaConfigureParams) => {
              const instruction = buildInstruction(alphaConfigureInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'configure',
                instruction,
                artifacts: { instruction },
                errors: alphaConfigureInstruction.errors,
              });
            }),
            },
          };
        },
      },
    },
    beta: {
      name: 'beta',
      programId: 'Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:5f972c866b71bb46f65440a9c92deacf63a629a59d188d9a9e64bb3fa9d4fd7c',
      programSpecHash: 'arete:h1:program-spec:sha256:4339c11f0c4e44ae7c5582bd54833b6d7fbcb48a428bdc0f1ecce4cbe3ee6fa3',
      idlContentHash: 'arete:h1:idl-content:sha256:67b5f19a27a00e45e1610af51f0097f7e2d81884676d5ee04a478c9a4e0810c9',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:0404d2bf7f43c52378d155144aeb9e171bb0c49fb368d913d0a9ef5ebf473572',
      accounts: {
        Vault: programAccountRead<BetaVault2>({ account: 'Vault', schema: BetaVault2Schema }),
      },
      rawInstructions: {
        configure: betaConfigureInstruction,
      },
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return {
            instructions: {
            configure: instructionOperation(async (params: BetaConfigureParams) => {
              const instruction = buildInstruction(betaConfigureInstruction, params as unknown as Record<string, unknown>);
              return createPreparedInstruction({
                name: 'configure',
                instruction,
                artifacts: { instruction },
                errors: betaConfigureInstruction.errors,
              });
            }),
            },
          };
        },
      },
    },
  },
  programReads: {
    alpha: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:7ca2173db2139bb3009579d116fbd6a1bedf01b1885f0b08ecc52cacf8219b7c", programSpecHash: "arete:h1:program-spec:sha256:07ca57c38922e4521a86f57a079fc28eec26733af78a3bef1af19c69c65b88ab" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
    beta: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:eab0552764322dca3997f69a01aa37be1445df1d2a7f4d60be4ebfc88c026678", programSpecHash: "arete:h1:program-spec:sha256:4339c11f0c4e44ae7c5582bd54833b6d7fbcb48a428bdc0f1ecce4cbe3ee6fa3" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
  },
} as const;

/** Type alias for the core stack */
export type HeaderStreamCoreStack = typeof HEADER_STREAM_STACK_CORE;

/** Entity types in this stack */
export type HeaderStreamEntity = AlphaVault | BetaVault;

/** Default export for convenience */
export default HEADER_STREAM_STACK_CORE;