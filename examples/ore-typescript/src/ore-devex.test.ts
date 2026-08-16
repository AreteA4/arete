import assert from 'node:assert/strict';
import test from 'node:test';

import {
  AutomationStrategy,
  ORE_PROGRAM_ADDRESS,
  U64_MAX,
  buildPreparedOreInstruction,
  didHitMotherlode,
  prepareConfigureAutomation,
  prepareDeploy,
  quoteAutomationFunding,
  quoteManualDeployment,
  previewSolClaim,
  reverseBits64,
} from './generated/ore-devex.js';
import { ORE_STREAM_STACK } from './generated/ore-stack.js';

function deployed(entries: ReadonlyArray<readonly [number, bigint]> = []): bigint[] {
  const amounts = Array<bigint>(25).fill(0n);
  for (const [square, amount] of entries) {
    amounts[square] = amount;
  }
  return amounts;
}

test('keeps the validated automation strategy discriminants', () => {
  assert.deepEqual(AutomationStrategy, {
    Random: 0,
    Preferred: 1,
    Discretionary: 2,
  });

  const prepared = prepareConfigureAutomation({
    authority: 'authority',
    executor: 'permissionless',
    amountPerSquare: 1n,
    deposit: 5_001n,
    executorFee: 5_000n,
    selection: { kind: 'preferred', squares: [0, 24] },
    automation: 'automation',
    miner: 'miner',
  });
  assert.equal(prepared.params.strategy, AutomationStrategy.Preferred);
});

test('builds a prepared deploy without the optional entropy program', () => {
  assert.doesNotThrow(() =>
    buildPreparedOreInstruction(
      prepareDeploy({
        signer: ORE_PROGRAM_ADDRESS,
        amountPerSquare: 1n,
        squares: [0],
        roundId: 1n,
      }),
    ),
  );
});

test('classifies the transaction needed to realize and claim SOL rewards', () => {
  const checkpoint = {
    status: 'claimable' as const,
    rewardsSol: 50n,
    rewardsOre: 0n,
    botFee: 0n,
    winningSquare: 0 as const,
    solDestination: 'miner' as const,
  };
  assert.deepEqual(previewSolClaim({ checkpointedRewardsSol: 25n, checkpoint }), {
    checkpointedRewardsSol: 25n,
    unresolvedRewardsSol: 50n,
    totalClaimableSol: 75n,
    totalClaimableSolUi: '0.000000075',
    checkpoint,
    action: 'checkpointAndClaim',
  });
  assert.equal(
    previewSolClaim({
      checkpointedRewardsSol: 0n,
      checkpoint: { ...checkpoint, solDestination: 'authority' },
    }).action,
    'checkpoint',
  );
  assert.equal(previewSolClaim({ checkpointedRewardsSol: 25n }).action, 'claim');
});

test('exposes the named quote helpers on the generated stack API', () => {
  assert.equal(ORE_STREAM_STACK.math.quoteManualDeployment, quoteManualDeployment);
  assert.equal(ORE_STREAM_STACK.math.quoteAutomationFunding, quoteAutomationFunding);
});

test('changes motherlode odds at round 335000', () => {
  const divisibleBy625 = reverseBits64(625n);
  const divisibleBy500 = reverseBits64(500n);

  assert.equal(didHitMotherlode(divisibleBy625, 334_999n), true);
  assert.equal(didHitMotherlode(divisibleBy625, 335_000n), false);
  assert.equal(didHitMotherlode(divisibleBy500, 335_000n), true);
});

test('quotes a first manual deploy with exact allocation and checkpoint reserve', () => {
  const quote = quoteManualDeployment({
    roundId: 42n,
    totalPrincipal: 11n,
    selectedSquares: [0, 2],
  });

  assert.deepEqual(quote.requestedSquares, [0, 2]);
  assert.deepEqual(quote.effectiveSquares, [0, 2]);
  assert.deepEqual(quote.alreadyDeployedSquares, []);
  assert.equal(quote.amountPerSquare, 5n);
  assert.equal(quote.allocatedPrincipal, 10n);
  assert.equal(quote.roundingRemainder, 1n);
  assert.equal(quote.maximumDeploymentTransfer, 10n);
  assert.equal(quote.checkpointReserve, 10_000n);
  assert.equal(quote.checkpointReserveUi, '0.00001');
  assert.equal(quote.maximumWalletDebit, 10_010n);
  assert.equal(quote.includesNetworkFee, false);
  assert.equal(quote.includesAccountRent, false);
});

test('excludes current-round deployed squares from a manual transfer', () => {
  const quote = quoteManualDeployment({
    roundId: 42n,
    totalPrincipal: 11n,
    selectedSquares: [0, 2],
    miner: {
      roundId: 42n,
      checkpointFee: 10_000n,
      deployed: deployed([[0, 99n]]),
    },
  });

  assert.deepEqual(quote.alreadyDeployedSquares, [0]);
  assert.deepEqual(quote.effectiveSquares, [2]);
  assert.equal(quote.effectiveSquareMask, 1n << 2n);
  assert.equal(quote.maximumDeploymentTransfer, 5n);
  assert.equal(quote.unspentPrincipal, 6n);
  assert.equal(quote.checkpointReserve, 0n);
  assert.equal(quote.maximumWalletDebit, 5n);
});

test('ignores prior-round positions and reserves a missing checkpoint fee', () => {
  const quote = quoteManualDeployment({
    roundId: 43n,
    totalPrincipal: 2n,
    selectedSquares: [0, 2],
    miner: {
      roundId: 42n,
      checkpointFee: 0n,
      deployed: deployed([[0, 99n]]),
    },
  });

  assert.deepEqual(quote.alreadyDeployedSquares, []);
  assert.deepEqual(quote.effectiveSquares, [0, 2]);
  assert.equal(quote.maximumDeploymentTransfer, 2n);
  assert.equal(quote.checkpointReserve, 10_000n);
});

test('flags active automation instead of treating a manual deploy as independent', () => {
  const quote = quoteManualDeployment({
    roundId: 42n,
    totalPrincipal: 1n,
    selectedSquares: [0],
    automation: { balance: 5_001n },
  });

  assert.equal(quote.hasActiveAutomation, true);
  assert.equal(quote.requiresDisableBeforeDeployment, true);
});

test('quotes two rounds and two preferred squares at one lamport each', () => {
  const quote = quoteAutomationFunding({
    totalPrincipal: 4n,
    rounds: 2n,
    preferredSquares: [0, 24],
    miner: { checkpointFee: 10_000n },
  });

  assert.equal(quote.preferredSquareCount, 2);
  assert.equal(quote.preferredSquareMask, 1n | (1n << 24n));
  assert.equal(quote.amountPerSquare, 1n);
  assert.equal(quote.principalPerRound, 2n);
  assert.equal(quote.allocatedPrincipal, 4n);
  assert.equal(quote.executorFeePerRound, 5_000n);
  assert.equal(quote.totalExecutorFees, 10_000n);
  assert.equal(quote.targetAutomationBalance, 10_004n);
  assert.equal(quote.incrementalDeposit, 10_004n);
  assert.equal(quote.estimatedFundedRounds, 2n);
  assert.equal(quote.fundingRemainder, 0n);
  assert.equal(quote.checkpointReserve, 0n);
});

test('reports active automation balance, exact top-up math, and replacement policy', () => {
  const quote = quoteAutomationFunding({
    totalPrincipal: 4n,
    rounds: 2n,
    preferredSquares: [0, 24],
    automation: { balance: 5_003n },
  });

  assert.equal(quote.existingAutomationBalance, 5_003n);
  assert.equal(quote.existingFundedRounds, 1n);
  assert.equal(quote.existingFundingRemainder, 1n);
  assert.equal(quote.targetAutomationBalance, 10_004n);
  assert.equal(quote.incrementalDeposit, 5_001n);
  assert.equal(quote.resultingAutomationBalance, 10_004n);
  assert.equal(quote.checkpointReserve, 10_000n);
  assert.equal(quote.maximumWalletDebit, 15_001n);
  assert.equal(quote.requiresDisableBeforeReplacement, true);
});

test('keeps principal and remainder exact above Number.MAX_SAFE_INTEGER', () => {
  const totalPrincipal = BigInt(Number.MAX_SAFE_INTEGER) + 12_352n;
  const quote = quoteAutomationFunding({
    totalPrincipal,
    rounds: 3n,
    preferredSquares: [1, 7],
    miner: { checkpointFee: 1n },
  });

  assert.equal(quote.amountPerSquare, totalPrincipal / 6n);
  assert.equal(quote.allocatedPrincipal, (totalPrincipal / 6n) * 6n);
  assert.equal(quote.roundingRemainder, totalPrincipal % 6n);
  assert.equal(
    quote.targetAutomationBalance,
    3n * ((totalPrincipal / 6n) * 2n + 5_000n),
  );
});

test('rejects u64 overflow in calculated deposits and wallet debits', () => {
  assert.throws(
    () =>
      quoteAutomationFunding({
        totalPrincipal: U64_MAX,
        rounds: 1n,
        preferredSquares: [0],
      }),
    /must be an unsigned 64-bit integer/,
  );
  assert.throws(
    () =>
      quoteManualDeployment({
        roundId: 1n,
        totalPrincipal: U64_MAX,
        selectedSquares: [0],
      }),
    /maximumWalletDebit must be an unsigned 64-bit integer/,
  );
});
