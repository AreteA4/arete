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

export interface BetaHeader {
  version: number;
  owner: string;
}

export const AlphaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
}));

export const BetaHeaderSchema = z.object({
  version: z.number(),
  owner: z.string(),
}).transform((value) => ({
  version: value.version,
  owner: value.owner,
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
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:76635e01963ac3bcba1ddd6f98d2c7725150850483cfe93ab740954ded75df00',
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
      sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:c7a8e8a9a2a060272b267fee758717d8507c363e736d52bb0891d55a5fa3aadf',
      programSpecHash: 'arete:h1:program-spec:sha256:09c1dbf3601259b356733fcca78684d5cfa7bb96d8455af06073d246cdf5819e',
      idlContentHash: 'arete:h1:idl-content:sha256:f71d7465982f766605b3f4ed4b54e266a36a650e5a3cd531deed3687deb17e79',
      normalizedIdlHash: 'arete:h1:idl-normalized:sha256:1a47e6f5552891691a91a51b34dd174a2086fb953c3b7b6155091bd093709406',
      accounts: {
        Header: programAccountRead<BetaHeader>({ account: 'Header', schema: BetaHeaderSchema }),
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
      release: { programReleaseHash: "arete:h1:program-release:sha256:45fb1bcf8ea2fd7105059c892d798acdd26f455c1842161ad1324f1ef9f6c856", programSpecHash: "arete:h1:program-spec:sha256:09c1dbf3601259b356733fcca78684d5cfa7bb96d8455af06073d246cdf5819e" },
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