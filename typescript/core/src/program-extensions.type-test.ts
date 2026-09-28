// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// A stack composed the way generated code composes it (the core stack's
// programs extended with `extendPrograms`, then `extendStack`) must keep every
// extension operation type all the way to `client.programs.<key>`, whether or
// not programs are attached next to the stack.

import type { ConnectedArete, MergeProgramMaps, StackWithAttachedPrograms } from './client';
import type { InstructionOperation, TransactionOperation } from './program-instructions';
import { withProgramIdentity, withProgramRead } from './program-sdk';
import {
  PROGRAM_OPERATION_EXTENSIONS,
  defineProgramExtensions,
  extendProgram,
  extendPrograms,
  extendStack,
  type ProgramOperationsOf,
} from './stack-extensions';
import type { ProgramReadDescriptor, ProgramSdkDefinition } from './types';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;
type InputOf<TOperation> = TOperation extends { prepare(input: infer TInput): unknown }
  ? TInput
  : never;

interface RawDeployParams {
  amount: bigint;
}
interface DeployInput {
  amountPerSquare: number;
}
interface DeployWithCheckpointInput extends DeployInput {
  checkpoint?: boolean;
}

declare const rawDeploy: InstructionOperation<RawDeployParams, { raw: true }>;
declare const deploy: InstructionOperation<DeployInput, { squares: number }>;
declare const deployWithCheckpoint: TransactionOperation<
  DeployWithCheckpointInput,
  { checkpointIncluded: boolean }
>;
declare const readDescriptor: ProgramReadDescriptor;

// The generated core program carries its raw instruction operations.
const CORE_STACK = {
  name: 'ore',
  endpoints: { ws: 'wss://ore.invalid' },
  views: {},
  programs: {
    ore: {
      name: 'ore',
      programId: 'ore-program',
      programSpecHash: 'spec-ore',
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return { instructions: { deploy: rawDeploy } };
        },
      },
    },
    entropy: {
      name: 'entropy',
      programId: 'entropy-program',
    },
  },
} as const;

// The package extension adds semantic instructions and a transaction.
const oreProgramExtensions = defineProgramExtensions<typeof CORE_STACK.programs.ore>()({
  createOperations() {
    return {
      instructions: { mining: { deploy } },
      transactions: { mining: { deployWithCheckpoint } },
    };
  },
});

// `extendProgram` keeps both the core's and the extension's operations.
const extendedOre = extendProgram(CORE_STACK.programs.ore, oreProgramExtensions);
type ExtendedOreOperations = ProgramOperationsOf<typeof extendedOre>;
export type ExtendProgramKeepsTransactions = Assert<Equal<
  NonNullable<ExtendedOreOperations['transactions']>['mining']['deployWithCheckpoint'],
  typeof deployWithCheckpoint
>>;
export type ExtendProgramKeepsExtensionInstructions = Assert<Equal<
  NonNullable<ExtendedOreOperations['instructions']>['mining']['deploy'],
  typeof deploy
>>;
export type ExtendProgramKeepsCoreInstructions = Assert<Equal<
  NonNullable<ExtendedOreOperations['instructions']>['deploy'],
  typeof rawDeploy
>>;

// `extendPrograms` does the same per key and leaves other programs untouched.
const extendedPrograms = extendPrograms(CORE_STACK.programs, { ore: oreProgramExtensions });
export type ExtendProgramsKeepsTransactions = Assert<Equal<
  NonNullable<ProgramOperationsOf<typeof extendedPrograms.ore>['transactions']>['mining']['deployWithCheckpoint'],
  typeof deployWithCheckpoint
>>;
export type ExtendProgramsLeavesOtherPrograms = Assert<Equal<
  typeof extendedPrograms.entropy,
  typeof CORE_STACK.programs.entropy
>>;

// Stamping a read descriptor and identity keeps the program's type.
const stampedOre = withProgramIdentity(
  withProgramRead(extendedOre, readDescriptor),
  { packageReleaseHash: 'release-ore' },
);
export type IdentityKeepsOperations = Assert<Equal<typeof stampedOre, typeof extendedOre>>;

// The stack as a generated entry composes it.
const STACK = extendStack(
  {
    ...CORE_STACK,
    programs: extendPrograms(CORE_STACK.programs, { ore: oreProgramExtensions }),
  } as const,
  {},
);

type ConnectedPrograms<TAttached extends Record<string, ProgramSdkDefinition> | undefined> =
  ConnectedArete<StackWithAttachedPrograms<typeof STACK, TAttached>>['programs'];

// No attached programs: `Arete.connect(stack)` and `useArete(stack)`.
type Unattached = ConnectedPrograms<undefined>;
export type ConnectedKeepsTransactions = Assert<Equal<
  InputOf<Unattached['ore']['transactions']['mining']['deployWithCheckpoint']>,
  DeployWithCheckpointInput
>>;
export type ConnectedKeepsInstructions = Assert<Equal<
  InputOf<Unattached['ore']['instructions']['mining']['deploy']>,
  DeployInput
>>;
export type ConnectedKeepsOtherPrograms = Assert<Equal<
  Unattached['entropy']['programId'],
  'entropy-program'
>>;

// A map typed only by its index signature names no key, so it replaces none.
type WideAttached = ConnectedPrograms<Record<string, ProgramSdkDefinition>>;
export type WideAttachmentKeepsStackPrograms = Assert<Equal<
  InputOf<WideAttached['ore']['transactions']['mining']['deployWithCheckpoint']>,
  DeployWithCheckpointInput
>>;

// An attached program takes the key it names; the stack keeps the others.
declare const attachedOre: {
  readonly name: 'ore';
  readonly programId: 'ore-program';
  readonly programSpecHash: 'spec-ore';
};
type Attached = MergeProgramMaps<(typeof STACK)['programs'], { ore: typeof attachedOre }>;
export type AttachedProgramTakesItsKey = Assert<Equal<Attached['ore'], typeof attachedOre>>;
export type AttachedProgramLeavesOtherKeys = Assert<Equal<
  Attached['entropy'],
  typeof CORE_STACK.programs.entropy
>>;
