// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// `useArete(stack)` on a stack whose programs were extended the way generated
// code extends them keeps every extension operation's hooks typed, with or
// without attached programs.

import {
  PROGRAM_OPERATION_EXTENSIONS,
  defineProgramExtensions,
  extendPrograms,
  extendStack,
  type InstructionOperation,
  type ProgramSdkDefinition,
  type TransactionOperation,
} from '@usearete/sdk';

import { useArete } from './stack';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

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

const CORE_STACK = {
  name: 'ore',
  endpoints: { ws: 'wss://ore.invalid' },
  views: {},
  programs: {
    ore: {
      name: 'ore',
      programId: 'ore-program',
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return { instructions: { deploy: rawDeploy } };
        },
      },
    },
  },
} as const;

const STACK = extendStack(
  {
    ...CORE_STACK,
    programs: extendPrograms(CORE_STACK.programs, {
      ore: defineProgramExtensions<typeof CORE_STACK.programs.ore>()({
        createOperations() {
          return {
            instructions: { mining: { deploy } },
            transactions: { mining: { deployWithCheckpoint } },
          };
        },
      }),
    }),
  } as const,
  {},
);

const arete = useArete(STACK);
const wide = useArete(STACK, { programs: {} as Record<string, ProgramSdkDefinition> });

export type UnattachedTransactionHookIsTyped = Assert<Equal<
  Parameters<typeof arete.programs.ore.transactions.mining.deployWithCheckpoint.execute>[0],
  DeployWithCheckpointInput
>>;
export type UnattachedInstructionHooksAreTyped = Assert<Equal<
  [
    Parameters<typeof arete.programs.ore.instructions.deploy.execute>[0],
    Parameters<typeof arete.programs.ore.instructions.mining.deploy.execute>[0],
  ],
  [RawDeployParams, DeployInput]
>>;
export type WideAttachmentKeepsTransactionHook = Assert<Equal<
  Parameters<typeof wide.programs.ore.transactions.mining.deployWithCheckpoint.execute>[0],
  DeployWithCheckpointInput
>>;
