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
  header: BetaHeader | null;
}

export interface BetaVault {
  id: BetaVaultId;
  state: BetaVaultState;
}

export interface BetaHeader {
  version: number;
  owner: string;
  flags: number;
}

export const BetaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
  flags: z.number(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
  flags: value.flags,
}));

export const BetaHeaderPatchSchema = z.object({
  version: z.number().optional(),
  owner: z.string().optional(),
  flags: z.number().optional(),
}).transform((value) => ({
  ...(value.version !== undefined ? { version: value.version } : {}),
  ...(value.owner !== undefined ? { owner: value.owner } : {}),
  ...(value.flags !== undefined ? { flags: value.flags } : {}),
}));

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
  header: BetaHeaderSchema.nullable().optional(),
}).transform((value) => ({
  header: value.header,
}));

export const BetaVaultStatePatchSchema = z.object({
  header: BetaHeaderPatchSchema.nullable().optional(),
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

export interface BetaHeader2 {
  version: number;
  owner: string;
  flags: number;
}

export const AlphaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
}));

export const BetaHeader2Schema = z.object({
  version: z.number(),
  owner: z.string(),
  flags: z.number(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
  flags: value.flags,
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

export interface BetaHeaderInput {
  version: number;
  owner: string;
  flags: number;
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
  header: BetaHeaderInput;
  authority: string;
  vault: string;
}

export type BetaConfigureError = HeaderStreamBetaProgramError;

export const betaConfigureInstruction = createInstructionHandler<BetaConfigureParams, BetaConfigureError>({
  programId: 'Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS',
  discriminator: [0],
  args: [
    { name: 'header', type: { struct: [{ name: 'version', type: 'u8' }, { name: 'owner', type: 'pubkey' }, { name: 'flags', type: 'u16' }] } },
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
    BetaHeader2: BetaHeader2Schema,
    BetaHeader: BetaHeaderSchema,
    BetaVaultCompleted: BetaVaultCompletedSchema,
    BetaVaultId: BetaVaultIdSchema,
    BetaVault: BetaVaultSchema,
    BetaVaultState: BetaVaultStateSchema,
    Header: HeaderSchema,
  },
  patchSchemas: {
    AlphaVault: AlphaVaultPatchSchema,
    BetaVault: BetaVaultPatchSchema,
  },
  programs: {
    alpha: {
      name: 'alpha',
      programId: '2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM',
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:64b24970e9209e0bdb1a33958061f08ce028f580b0a119efe97c2e93ea31487f',
      programSpecHash: 'arete:h1:program-spec:sha256:2cfe95cf0c7d0085085dab5d3bbf8faed0a698519e740ceb126751f45fa9a2b4',
      idlContentHash: 'arete:h1:idl-content:sha256:6e5dd6926c3f40fd2a5a546909c0c773cb9cd3349ac7c28227efbdc920f9e081',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:9ed97cc808f1573ab3e5511f10deeaffea26dd2157815a393b0e8b7c68e4a6ea',
      accounts: {
        Header: programAccountRead<AlphaHeader>({ account: 'Header', schema: AlphaHeaderSchema }),
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
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:d3ab2c3fc08ae86de35bd57e97a47a5db7801cf6290daf82cf43cca0f180aa7d',
      programSpecHash: 'arete:h1:program-spec:sha256:7d8147800015e11cbcff4976c7a05ab11c4a21a0b0c48f530930e20fcf6c3834',
      idlContentHash: 'arete:h1:idl-content:sha256:e4efcbe4e507599083453738cedd4458914f5f50ea03c0099ce777c5b1377b89',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:e5fa59e25754d764dcabc4fe1eb49c610379f1b63e93b368520e0031637bec6e',
      accounts: {
        Header: programAccountRead<BetaHeader2>({ account: 'Header', schema: BetaHeader2Schema }),
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
      release: { programReleaseHash: "arete:h1:program-release:sha256:ebac0e8bac3bd650c596de51e0bb2c7a355cf6169724de958b31ceed4eb3dda3", programSpecHash: "arete:h1:program-spec:sha256:2cfe95cf0c7d0085085dab5d3bbf8faed0a698519e740ceb126751f45fa9a2b4" },
      transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
    },
    beta: {
      release: { programReleaseHash: "arete:h1:program-release:sha256:dd00373868c8b427c3207815a4ee5e0d66b8f9df5d034cf84a1a3b904fda554b", programSpecHash: "arete:h1:program-spec:sha256:7d8147800015e11cbcff4976c7a05ab11c4a21a0b0c48f530930e20fcf6c3834" },
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