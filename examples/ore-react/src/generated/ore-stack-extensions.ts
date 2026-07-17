import {
  createPreparedInstruction,
  createPreparedTransaction,
  defineProgramExtensions,
  defineStackExtensions,
  instructionOperation,
  transactionOperation,
  type AmountInput,
} from '@usearete/sdk';

import type { ORE_STREAM_STACK_CORE } from './ore-stack-core.js';
import oreDevex, {
  BPS_DENOMINATOR,
  type Address,
  type AutomationSelection,
  type PrepareCheckpointInput,
  type PrepareClaimOreInput,
  type PrepareClaimSolInput,
  type PrepareCloseExpiredRoundInput,
  type PrepareConfigureAutomationInput,
  type PrepareDeployInput,
  type PrepareDisableAutomationInput,
  type PreparedOreInstruction,
  type QuoteAutomationFundingInput,
  type QuoteManualDeploymentInput,
} from './ore-devex.js';

const addresses = oreDevex.addresses;
const constants = oreDevex.constants;
const math = oreDevex.math;

export type DeploySemanticInput = Omit<
  PrepareDeployInput,
  'amountPerSquare' | 'roundId'
> & {
  amountPerSquare: AmountInput;
  roundId?: bigint;
};

export type DeployWithCheckpointInput = DeploySemanticInput & {
  checkpoint?: {
    signer?: Address;
    roundId?: bigint;
    automation?: Address;
    board?: Address;
    miner?: Address;
    round?: Address;
    treasury?: Address;
  };
};

export type CheckpointSemanticInput = Omit<PrepareCheckpointInput, 'roundId'> & {
  roundId?: bigint;
};

export type ConfigureAutomationSemanticInput = Omit<
  PrepareConfigureAutomationInput,
  'amountPerSquare' | 'deposit' | 'executorFee' | 'selection'
> & {
  amountPerSquare: AmountInput;
  deposit: AmountInput;
  executorFee: AmountInput;
  selection: AutomationSelection;
};

export type ClaimOreSemanticInput = Omit<PrepareClaimOreInput, 'bps'> & {
  bps?: number | bigint;
};

export interface ClaimAllSemanticInput
  extends Omit<PrepareClaimOreInput, 'bps'>,
    Omit<PrepareClaimSolInput, 'authority'> {
  bps?: number | bigint;
  includeSol?: boolean;
}

export type ManualDeploymentQuoteReadInput = Omit<
  QuoteManualDeploymentInput,
  'roundId' | 'miner' | 'automation'
> & {
  authority: Address;
  roundId?: bigint;
};

export type AutomationFundingQuoteReadInput = Omit<
  QuoteAutomationFundingInput,
  'miner' | 'automation'
> & {
  authority: Address;
};

function operationFromPrepared<TArtifacts>(
  name: string,
  prepared: PreparedOreInstruction<Record<string, unknown>>,
  artifacts: TArtifacts,
) {
  const instruction = oreDevex.buildPreparedInstruction(prepared);
  return createPreparedInstruction({
    name,
    instruction,
    requiredSignerAddresses: oreDevex.getRequiredSignerAddresses(prepared),
    errors: prepared.handler.errors,
    artifacts,
  });
}

export const oreProgramExtensions = defineProgramExtensions<
  typeof ORE_STREAM_STACK_CORE.programs.ore
>()({
  addresses,
  constants,
  math,
  createOperations(context) {
    async function getBoard(address = addresses.board()) {
      const board = await context.program.accounts.Board.fetch(address);
      if (!board) {
        throw new Error(`ORE Board account not found: ${address}`);
      }
      return board;
    }

    async function getMiner(
      authority: Address,
      address = addresses.miner(authority),
    ) {
      return context.program.accounts.Miner.fetch(address);
    }

    async function resolveDeployInput(input: DeploySemanticInput) {
      const roundId = input.roundId ?? (await getBoard(input.board)).roundId;
      const amountPerSquare = oreDevex.math.amounts.resolveSol(
        input.amountPerSquare,
      );
      const prepared = oreDevex.prepare.deploy({
        ...input,
        roundId,
        amountPerSquare,
      });
      const squareMask = oreDevex.math.squares.encode(input.squares);
      return {
        prepared,
        artifacts: {
          authority: input.authority ?? input.signer,
          roundId,
          round: input.round ?? addresses.round(roundId),
          squareMask,
          squareCount: oreDevex.math.squares.count(squareMask),
          amountPerSquare,
          requestedMaximumDeployment:
            amountPerSquare * BigInt(input.squares.length),
        },
      };
    }

    async function resolveCheckpointInput(input: CheckpointSemanticInput) {
      let roundId = input.roundId;
      if (roundId === undefined) {
        const miner = await getMiner(input.authority, input.miner);
        if (!miner) {
          throw new Error(
            `ORE Miner account not found for authority ${input.authority}`,
          );
        }
        roundId = miner.roundId;
      }
      return {
        prepared: oreDevex.prepare.checkpoint({ ...input, roundId }),
        roundId,
      };
    }

    const deploy = instructionOperation(async (input: DeploySemanticInput) => {
      const resolved = await resolveDeployInput(input);
      return operationFromPrepared(
        'deploy',
        resolved.prepared,
        resolved.artifacts,
      );
    });

    const checkpoint = instructionOperation(
      async (input: CheckpointSemanticInput) => {
        const resolved = await resolveCheckpointInput(input);
        return operationFromPrepared('checkpoint', resolved.prepared, {
          authority: input.authority,
          roundId: resolved.roundId,
          round: input.round ?? addresses.round(resolved.roundId),
        });
      },
    );

    const claimOre = instructionOperation(
      async (input: ClaimOreSemanticInput) => {
        const prepared = oreDevex.prepare.claimOre(input);
        return operationFromPrepared('claimOre', prepared, {
          authority: input.authority,
          bps: math.amounts.resolveBps(input.bps),
          recipient:
            input.recipient ?? addresses.oreTokenAccount(input.authority),
        });
      },
    );

    const claimSol = instructionOperation(async (input: PrepareClaimSolInput) => {
      const prepared = oreDevex.prepare.claimSol(input);
      return operationFromPrepared('claimSol', prepared, {
        authority: input.authority,
      });
    });

    const configureAutomation = instructionOperation(
      async (input: ConfigureAutomationSemanticInput) => {
        const amountPerSquare = oreDevex.math.amounts.resolveSol(
          input.amountPerSquare,
        );
        const deposit = oreDevex.math.amounts.resolveSol(input.deposit);
        const executorFee = oreDevex.math.amounts.resolveSol(input.executorFee);
        const prepared = oreDevex.prepare.configureAutomation({
          ...input,
          amountPerSquare,
          deposit,
          executorFee,
        });
        return operationFromPrepared('configureAutomation', prepared, {
          authority: input.authority,
          executor: input.executor,
          selection: input.selection,
          amountPerSquare,
          deposit,
          executorFee,
          reloadWinnings: input.reloadWinnings ?? false,
        });
      },
    );

    const disableAutomation = instructionOperation(
      async (input: PrepareDisableAutomationInput) => {
        const prepared = oreDevex.prepare.disableAutomation(input);
        return operationFromPrepared('disableAutomation', prepared, {
          authority: input.authority,
          automation: input.automation ?? addresses.automation(input.authority),
        });
      },
    );

    const closeExpiredRound = instructionOperation(
      async (input: PrepareCloseExpiredRoundInput) => {
        const prepared = oreDevex.prepare.closeExpiredRound(input);
        return operationFromPrepared('closeExpiredRound', prepared, {
          roundId: input.roundId,
          round: input.round ?? addresses.round(input.roundId),
          rentPayer: input.rentPayer,
        });
      },
    );

    const deployWithCheckpoint = transactionOperation(
      async (input: DeployWithCheckpointInput) => {
        const deployResolved = await resolveDeployInput(input);
        const authority = input.authority ?? input.signer;
        const minerAddress =
          input.checkpoint?.miner ?? input.miner ?? addresses.miner(authority);
        const miner = await getMiner(authority, minerAddress);
        const operations = [];
        let checkpointRoundId: bigint | null = null;
        let checkpointRound: Address | null = null;

        if (!miner && input.checkpoint?.roundId !== undefined) {
          throw new Error(
            `Cannot checkpoint authority ${authority} before its ORE Miner account exists`,
          );
        }
        if (miner) {
          checkpointRoundId = input.checkpoint?.roundId ?? miner.roundId;
          checkpointRound =
            input.checkpoint?.round ?? addresses.round(checkpointRoundId);
          // ORE treats already-checkpointed and current-round checkpoints as safe no-ops.
          const checkpointPrepared = oreDevex.prepare.checkpoint({
            signer: input.checkpoint?.signer ?? input.signer,
            authority,
            roundId: checkpointRoundId,
            automation:
              input.checkpoint?.automation ??
              input.automation ??
              addresses.automation(authority),
            board: input.checkpoint?.board ?? input.board ?? addresses.board(),
            miner: minerAddress,
            round: checkpointRound,
            treasury: input.checkpoint?.treasury,
          });
          operations.push(
            operationFromPrepared('checkpoint', checkpointPrepared, {
              authority,
              roundId: checkpointRoundId,
              round: checkpointRound,
            }),
          );
        }

        operations.push(
          operationFromPrepared(
            'deploy',
            deployResolved.prepared,
            deployResolved.artifacts,
          ),
        );

        return createPreparedTransaction({
          name: 'deployWithCheckpoint',
          operations,
          artifacts: {
            ...deployResolved.artifacts,
            checkpointIncluded: checkpointRoundId !== null,
            checkpointRoundId,
            checkpointRound,
          },
        });
      },
    );

    const claimAll = transactionOperation(
      async (input: ClaimAllSemanticInput) => {
        const operations = [];
        if (input.includeSol !== false) {
          const prepared = oreDevex.prepare.claimSol(input);
          operations.push(
            operationFromPrepared('claimSol', prepared, {
              authority: input.authority,
            }),
          );
        }
        const prepared = oreDevex.prepare.claimOre(input);
        operations.push(
          operationFromPrepared('claimOre', prepared, {
            authority: input.authority,
            bps: math.amounts.resolveBps(input.bps),
            recipient:
              input.recipient ?? addresses.oreTokenAccount(input.authority),
          }),
        );
        return createPreparedTransaction({
          name: 'claimAll',
          operations,
          artifacts: {
            authority: input.authority,
            bps: math.amounts.resolveBps(input.bps),
            includedSolClaim: input.includeSol !== false,
          },
        });
      },
    );

    return {
      instructions: {
        mining: { deploy },
        miner: { checkpoint },
        rewards: { claimOre, claimSol },
        automation: {
          configure: configureAutomation,
          disable: disableAutomation,
        },
        round: { closeExpired: closeExpiredRound },
      },
      transactions: {
        mining: { deployWithCheckpoint },
        rewards: { claimAll },
      },
    };
  },
});

export default defineStackExtensions<typeof ORE_STREAM_STACK_CORE>()({
  addresses,
  constants,
  math,
  createRead(client) {
    const program = client.programs.ore;

    async function board() {
      return program.accounts.Board.fetch(addresses.board());
    }

    async function config() {
      return program.accounts.Config.fetch(addresses.config());
    }

    async function treasury() {
      return program.accounts.Treasury.fetch(addresses.treasury());
    }

    async function miner(authority: Address) {
      return program.accounts.Miner.fetch(addresses.miner(authority));
    }

    async function automation(authority: Address) {
      return program.accounts.Automation.fetch(addresses.automation(authority));
    }

    async function round(roundId: bigint) {
      return program.accounts.Round.fetch(addresses.round(roundId));
    }

    async function boardState() {
      return client.views.OreBoard.state.get(addresses.board());
    }

    async function roundState(roundId: bigint) {
      return client.views.OreRound.state.get(roundId.toString());
    }

    async function resolveAuthoritativeRoundId(roundId?: bigint) {
      if (roundId !== undefined) {
        return roundId;
      }
      const entity = await boardState();
      const currentRoundId = entity?.state.roundId;
      if (currentRoundId == null) {
        throw new Error(
          `ORE Board state is unavailable; subscribe to OreBoard/state with key ${addresses.board()}`,
        );
      }
      return currentRoundId;
    }

    async function currentRound() {
      const [boardEntity, clock] = await Promise.all([
        boardState(),
        client.chain.clock(),
      ]);
      const roundId = boardEntity?.state.roundId;
      const startSlot = boardEntity?.state.startSlot;
      const endSlot = boardEntity?.state.endSlot;
      if (
        !boardEntity ||
        roundId == null ||
        startSlot == null ||
        endSlot == null
      ) {
        return null;
      }
      const roundAddress = addresses.round(roundId);
      const roundEntity = await roundState(roundId);
      const currentSlot = BigInt(clock.slot);
      return {
        board: boardEntity,
        round: roundEntity,
        roundAddress,
        clock,
        phase: math.round.phase({ startSlot, endSlot }, currentSlot),
      };
    }

    async function miningContext(authority: Address) {
      const [boardEntity, minerAccount, automationAccount, treasuryAccount, clock] =
        await Promise.all([
          boardState(),
          miner(authority),
          automation(authority),
          treasury(),
          client.chain.clock(),
        ]);
      const roundId = boardEntity?.state.roundId;
      const startSlot = boardEntity?.state.startSlot;
      const endSlot = boardEntity?.state.endSlot;
      if (
        !boardEntity ||
        roundId == null ||
        startSlot == null ||
        endSlot == null
      ) {
        return null;
      }
      const currentRoundAddress = addresses.round(roundId);
      const minerRoundId = minerAccount?.roundId ?? null;
      const minerRoundAddress =
        minerRoundId === null ? null : addresses.round(minerRoundId);
      const [currentRoundAccount, currentRoundEntity, minerRoundAccount] = await Promise.all([
        program.accounts.Round.fetch(currentRoundAddress),
        roundState(roundId),
        minerRoundAddress === null || minerRoundAddress === currentRoundAddress
          ? Promise.resolve(null)
          : program.accounts.Round.fetch(minerRoundAddress),
      ]);
      return {
        authority,
        board: boardEntity,
        boardAccount: boardEntity.boardSnapshot,
        currentRound: currentRoundAccount,
        currentRoundState: currentRoundEntity,
        currentRoundAddress,
        miner: minerAccount,
        minerRound:
          minerRoundAddress === currentRoundAddress
            ? currentRoundAccount
            : minerRoundAccount,
        minerRoundAddress,
        automation: automationAccount,
        treasury: treasuryAccount,
        clock,
        phase: math.round.phase({ startSlot, endSlot }, BigInt(clock.slot)),
      };
    }

    async function quoteManualDeployment(input: ManualDeploymentQuoteReadInput) {
      const { authority, roundId: requestedRoundId, ...quoteInput } = input;
      const roundId = await resolveAuthoritativeRoundId(requestedRoundId);
      const [minerAccount, automationAccount] = await Promise.all([
        miner(authority),
        automation(authority),
      ]);
      return math.quoteManualDeployment({
        ...quoteInput,
        roundId,
        miner: minerAccount,
        automation: automationAccount,
      });
    }

    async function quoteAutomationFunding(input: AutomationFundingQuoteReadInput) {
      const { authority, ...quoteInput } = input;
      const [minerAccount, automationAccount] = await Promise.all([
        miner(authority),
        automation(authority),
      ]);
      return math.quoteAutomationFunding({
        ...quoteInput,
        miner: minerAccount,
        automation: automationAccount,
      });
    }

    async function claimPreview(
      authority: Address,
      bps: bigint | number = BPS_DENOMINATOR,
    ) {
      const [minerAccount, treasuryAccount] = await Promise.all([
        miner(authority),
        treasury(),
      ]);
      if (!minerAccount || !treasuryAccount) {
        return null;
      }
      return math.miner.previewOreClaim(minerAccount, treasuryAccount, bps);
    }

    async function checkpointPreview(authority: Address) {
      const context = await miningContext(authority);
      if (!context?.miner || !context.minerRound) {
        return null;
      }
      return math.miner.previewCheckpoint({
        miner: context.miner,
        round: context.minerRound,
        boardRoundId: context.board.state.roundId!,
        currentSlot: BigInt(context.clock.slot),
        automation: context.automation,
      });
    }

    async function solClaimPreview(authority: Address) {
      const [minerAccount, boardAccount, automationAccount, clock] =
        await Promise.all([
          miner(authority),
          board(),
          automation(authority),
          client.chain.clock(),
        ]);
      if (!minerAccount) {
        return null;
      }
      const minerRound = boardAccount && minerAccount.checkpointId !== minerAccount.roundId
        ? await round(minerAccount.roundId)
        : null;
      const checkpoint = boardAccount && minerRound
        ? math.miner.previewCheckpoint({
            miner: minerAccount,
            round: minerRound,
            boardRoundId: boardAccount.roundId,
            currentSlot: BigInt(clock.slot),
            automation: automationAccount,
          })
        : null;
      return math.miner.previewSolClaim({
        checkpointedRewardsSol: minerAccount.rewardsSol,
        checkpoint,
      });
    }

    return {
      board,
      config,
      treasury,
      miner,
      automation,
      round,
      boardState,
      roundState,
      currentRound,
      miningContext,
      quoteManualDeployment,
      quoteAutomationFunding,
      claimPreview,
      checkpointPreview,
      solClaimPreview,
    };
  },
});
