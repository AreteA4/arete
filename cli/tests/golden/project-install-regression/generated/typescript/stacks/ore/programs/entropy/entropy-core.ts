import { z } from 'zod';
import { programAccountRead, createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk/program';

export interface Var {
  authority: string;
  id: bigint;
  provider: string;
  commit: number[];
  seed: number[];
  slotHash: number[];
  value: number[];
  samples: bigint;
  isAuto: bigint;
  startAt: bigint;
  endAt: bigint;
}

export const VarSchema = z.object({
  authority: z.string(),
  id: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  provider: z.string(),
  commit: z.array(z.number()).length(32),
  seed: z.array(z.number()).length(32),
  slot_hash: z.array(z.number()).length(32),
  value: z.array(z.number()).length(32),
  samples: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  is_auto: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  start_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
  end_at: z.union([z.bigint(), z.string(), z.number().int()]).transform((value) => BigInt(value)),
}).transform((value) => ({
  authority: value.authority,
  id: value.id,
  provider: value.provider,
  commit: value.commit,
  seed: value.seed,
  slotHash: value.slot_hash,
  value: value.value,
  samples: value.samples,
  isAuto: value.is_auto,
  startAt: value.start_at,
  endAt: value.end_at,
}));

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Union of all program errors declared across this stack's instructions. */
export type EntropyProgramError =
  | { code: 0; name: 'IncompleteDigest'; msg: string }
  | { code: 1; name: 'InvalidSeed'; msg: string };

const ENTROPY_PROGRAM_ERRORS: ErrorMetadata[] = [
  { code: 0, name: 'IncompleteDigest', msg: 'Incomplete digest' },
  { code: 1, name: 'InvalidSeed', msg: 'Invalid seed' },
];

export interface OpenParams {
  id: bigint;
  commit: number[];
  isAuto: bigint;
  samples: bigint;
  endAt: bigint;
  authority: string;
  payer: string;
  provider: string;
  var: string;
}

export type OpenError = EntropyProgramError;

/**
 * Creates a new entropy var account.
 * Var PDA seeds: ["var", authority, id].
 */
export const openInstruction = createInstructionHandler<OpenParams, OpenError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [0],
  args: [
    { name: 'id', type: 'u64' },
    { name: 'commit', type: { array: ['u8', 32] } },
    { name: 'isAuto', type: 'u64' },
    { name: 'samples', type: 'u64' },
    { name: 'endAt', type: 'u64' },
  ],
  accounts: [
    { name: 'authority', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'provider', isSigner: false, isWritable: false, category: 'userProvided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ENTROPY_PROGRAM_ERRORS,
});

export interface CloseParams {
  signer: string;
  var: string;
}

export type CloseError = EntropyProgramError;

/**
 * Closes an entropy var account and returns rent to the authority.
 */
export const closeInstruction = createInstructionHandler<CloseParams, CloseError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [1],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ENTROPY_PROGRAM_ERRORS,
});

export interface NextParams {
  endAt: bigint;
  signer: string;
  var: string;
}

export type NextError = EntropyProgramError;

/**
 * Updates the var for the next random value sample.
 * Resets the commit to the previous seed and clears slot_hash, seed, and value.
 */
export const nextInstruction = createInstructionHandler<NextParams, NextError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [2],
  args: [
    { name: 'endAt', type: 'u64' },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: ENTROPY_PROGRAM_ERRORS,
});

export interface RevealParams {
  seed: number[];
  signer: string;
  var: string;
}

export type RevealError = EntropyProgramError;

/**
 * Reveals the seed and finalizes the random value.
 * The seed must hash to the commit stored in the var account.
 */
export const revealInstruction = createInstructionHandler<RevealParams, RevealError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [4],
  args: [
    { name: 'seed', type: { array: ['u8', 32] } },
  ],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
  ],
  errors: ENTROPY_PROGRAM_ERRORS,
});

export interface SampleParams {
  signer: string;
  var: string;
}

export type SampleError = EntropyProgramError;

/**
 * Samples the slot hash at the end_at slot.
 * Must be called after the end_at slot has passed.
 */
export const sampleInstruction = createInstructionHandler<SampleParams, SampleError>({
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
    { name: 'var', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'slotHashesSysvar', isSigner: false, isWritable: false, category: 'known', knownAddress: 'SysvarS1otHashes111111111111111111111111111' },
  ],
  errors: ENTROPY_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'entropy' */
export const ENTROPY = {
  name: 'entropy',
  programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:7b02c7560668148dbdfaa200da75da66e2d1352861a5b1b497cd76cd7bdcf10c',
  programSpecHash: 'arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b',
  idlContentHash: 'arete:h1:idl-content:sha256:2b5b3ed4de83cd3803bd6b82b33cfbea0e8b7c6a7ada7b138fcb57bb2fe1a01f',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:adc67e46a2ffc5e26fcff489fa7e21d5aa0d6338243dc23330ab0e85c3e150fc',
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["anonymous","publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
  accounts: {
    Var: programAccountRead<Var>({ account: 'Var', schema: VarSchema }),
  },
  rawInstructions: {
    open: openInstruction,
    close: closeInstruction,
    next: nextInstruction,
    reveal: revealInstruction,
    sample: sampleInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        open: instructionOperation(async (params: OpenParams) => {
          const instruction = buildInstruction(openInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'open',
            instruction,
            artifacts: { instruction },
            errors: openInstruction.errors,
          });
        }),
        close: instructionOperation(async (params: CloseParams) => {
          const instruction = buildInstruction(closeInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'close',
            instruction,
            artifacts: { instruction },
            errors: closeInstruction.errors,
          });
        }),
        next: instructionOperation(async (params: NextParams) => {
          const instruction = buildInstruction(nextInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'next',
            instruction,
            artifacts: { instruction },
            errors: nextInstruction.errors,
          });
        }),
        reveal: instructionOperation(async (params: RevealParams) => {
          const instruction = buildInstruction(revealInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'reveal',
            instruction,
            artifacts: { instruction },
            errors: revealInstruction.errors,
          });
        }),
        sample: instructionOperation(async (params: SampleParams) => {
          const instruction = buildInstruction(sampleInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'sample',
            instruction,
            artifacts: { instruction },
            errors: sampleInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Instruction shapes for 'entropy', by IDL instruction name: argument and account names, and which accounts may be left out */
export const ENTROPY_INSTRUCTION_SHAPES = {
  open: {
    args: ['id', 'commit', 'isAuto', 'samples', 'endAt'],
    accounts: {
      authority: { optional: false, derive: null },
      payer: { optional: false, derive: null },
      provider: { optional: false, derive: null },
      var: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  close: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      var: { optional: false, derive: null },
      systemProgram: { optional: false, derive: [] },
    },
  },
  next: {
    args: ['endAt'],
    accounts: {
      signer: { optional: false, derive: null },
      var: { optional: false, derive: null },
    },
  },
  reveal: {
    args: ['seed'],
    accounts: {
      signer: { optional: false, derive: null },
      var: { optional: false, derive: null },
    },
  },
  sample: {
    args: [],
    accounts: {
      signer: { optional: false, derive: null },
      var: { optional: false, derive: null },
      slotHashesSysvar: { optional: false, derive: [] },
    },
  },
} as const;

/** Release and explicit read transport for 'entropy' */
export const ENTROPY_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", programSpecHash: "arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b" },
transport: { kind: 'hosted-binding', binding: { endpoint: "https://reads.example.test/private/", programReadBindingId: "prb_00000000000000000000000000000077", auth: {"acceptedKeyClasses":["publishable","secret"],"mode":"signed_session","required":true,"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"prb_00000000000000000000000000000077","targetKind":"program-read-binding"} } },
} as const;

/** All portable programs from the Entropy stack */
export const ENTROPY_PROGRAMS = {
  entropy: ENTROPY,
} as const;

/** Parallel release/read metadata keyed identically to ENTROPY_PROGRAMS */
export const ENTROPY_PROGRAM_READS = {
  entropy: ENTROPY_READ,
} as const;

export type EntropyPrograms = typeof ENTROPY_PROGRAMS;

export default ENTROPY_PROGRAMS;