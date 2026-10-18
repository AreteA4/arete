import { z } from 'zod';
import { createInstructionHandler, type ErrorMetadata, buildInstruction, PROGRAM_OPERATION_EXTENSIONS, instructionOperation, createPreparedInstruction } from '@usearete/sdk/program';

// ============================================================================
// Instruction Handlers
// ============================================================================

/** Program errors for this stack (none declared in the IDL). */
export type SquadsV4ProgramError = never;

const SQUADS_V4_PROGRAM_ERRORS: ErrorMetadata[] = [];

export interface PingParams {
  payer: string;
}

export type PingError = SquadsV4ProgramError;

export const pingInstruction = createInstructionHandler<PingParams, PingError>({
  programId: 'Vote111111111111111111111111111111111111111',
  discriminator: [1, 2, 3, 4, 5, 6, 7, 8],
  args: [],
  accounts: [
    { name: 'payer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'provided' },
  ],
  errors: SQUADS_V4_PROGRAM_ERRORS,
});

// ============================================================================
// Program Definitions
// ============================================================================

/** Standalone program SDK for 'vote_program' */
export const VOTE_PROGRAM = {
  name: 'vote_program',
  programId: 'Vote111111111111111111111111111111111111111',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:733210b7dbb86e32b99118c4dfb3c056efa6a4663fe10f7b60f34724e5c65fd2',
  programSpecHash: 'arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86',
  idlContentHash: 'arete:h1:idl-content:sha256:e8807fcf09326e59d31b3601ef4cabe93b4a59c64da4a4609c7d3aac50e20468',
  normalizedIdlHash: 'arete:h1:idl-normalized:sha256:be085beefa8c6b82c87473f54f40b19f107b7418619d3ad1b38d0e46f7c04bd0',
  gateway: {"chain":{"auth":{"acceptedKeyClasses":["anonymous","publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["read"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":false},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"},"transactions":{"auth":{"acceptedKeyClasses":["publishable","secret"],"audience":"arete:solana-gateway","jwksUrl":"https://api.example.test/.well-known/jwks.json","mode":"signed_session","required":true,"scopes":["transaction:inspect","transaction:send"],"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"sgb_00000000000000000000000000000001","targetKind":"solana-gateway-binding","tokenTransport":"bearer","transactionEntitlementRequired":true},"authPolicy":"signed_session","cluster":"mainnet-beta","endpoint":"https://solana.example.test/gateway/","region":"us-west-1","solanaGatewayBindingId":"sgb_00000000000000000000000000000001"}},
  rawInstructions: {
    ping: pingInstruction,
  },
  [PROGRAM_OPERATION_EXTENSIONS]: {
    createOperations() {
      return {
        instructions: {
        ping: instructionOperation(async (params: PingParams) => {
          const instruction = buildInstruction(pingInstruction, params as unknown as Record<string, unknown>);
          return createPreparedInstruction({
            name: 'ping',
            instruction,
            artifacts: { instruction },
            errors: pingInstruction.errors,
          });
        }),
        },
      };
    },
  },
} as const;

/** Instruction shapes for 'vote_program', by IDL instruction name: argument and account names, and which accounts may be left out */
export const VOTE_PROGRAM_INSTRUCTION_SHAPES = {
  ping: {
    args: [],
    accounts: {
      payer: { optional: false, derive: null },
    },
  },
} as const;

/** Release and explicit read transport for 'vote_program' */
export const VOTE_PROGRAM_READ = {
release: { programReleaseHash: "arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc", programSpecHash: "arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86" },
transport: { kind: 'hosted-binding', binding: { endpoint: "https://reads.example.test/private/", programReadBindingId: "prb_00000000000000000000000000000077", auth: {"acceptedKeyClasses":["publishable","secret"],"mode":"signed_session","required":true,"sessionEndpoint":"https://api.example.test/ws/sessions","targetId":"prb_00000000000000000000000000000077","targetKind":"program-read-binding"} } },
} as const;

/** All portable programs from the SquadsV4 stack */
export const SQUADS_V4_PROGRAMS = {
  voteProgram: VOTE_PROGRAM,
} as const;

/** Parallel release/read metadata keyed identically to SQUADS_V4_PROGRAMS */
export const SQUADS_V4_PROGRAM_READS = {
  voteProgram: VOTE_PROGRAM_READ,
} as const;

export type SquadsV4Programs = typeof SQUADS_V4_PROGRAMS;

export default SQUADS_V4_PROGRAMS;