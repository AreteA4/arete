import { describe, expect, it, vi } from 'vitest';
import { SYSTEM_PROGRAM_ADDRESS } from './generated/ore-devex';
import { oreProgramExtensions } from './generated/ore-stack-extensions';
import { ORE_STREAM_STACK } from './generated/ore-stack';

describe('ORE stream schemas', () => {
  it('accepts production-shaped sparse patches used for live state updates', () => {
    const board = ORE_STREAM_STACK.patchSchemas.OreBoard.safeParse({
      state: {
        end_slot: 434084887,
        production_cost_ema: 765386563,
        round_id: 339033,
        start_slot: 434084737,
      },
    });
    const round = ORE_STREAM_STACK.patchSchemas.OreRound.safeParse({
      metrics: { deploy_count: 56 },
      state: {
        count_per_square: [171, 165],
        deployed_per_square: [378259611, 388890919],
        total_miners: 181,
      },
    });
    const miner = ORE_STREAM_STACK.patchSchemas.OreMiner.safeParse({
      state: {
        round_id: 339105,
        deployed_per_square: [1_250_000_000, ...Array<number>(24).fill(0)],
        deployed_per_square_ui: [1.25, ...Array<number>(24).fill(0)],
        total_deployed: 1.25,
      },
      miner_snapshot: {
        account_address: 'miner-address',
        data: {
          authority: 'wallet-address',
          auto_return: 0,
          checkpoint_id: 339104,
          checkpoint_fee: 10000,
          deployed: [1_250_000_000, ...Array<number>(24).fill(0)],
          mass: Array<number>(25).fill(0),
          cumulative: Array<number>(25).fill(0),
          round_id: 339105,
          rewards_factor: {},
          rewards_sol: 0,
          refined_ore: 0,
          rewards_ore: 0,
          last_claim_ore_at: 0,
          last_claim_sol_at: 0,
          lifetime_rewards_ore: 0,
          lifetime_deployed: 1_250_000_000,
          lifetime_rewards_sol: 0,
        },
        signature: 'signature',
        slot: 434098167,
        timestamp: 1784550128,
      },
    });

    expect(board).toMatchObject({
      success: true,
      data: { state: { roundId: 339033n, endSlot: 434084887n } },
    });
    expect(round).toMatchObject({
      success: true,
      data: {
        metrics: { deployCount: 56n },
        state: {
          countPerSquare: [171n, 165n],
          deployedPerSquare: [378259611n, 388890919n],
          totalMiners: 181n,
        },
      },
    });
    expect(miner).toMatchObject({
      success: true,
      data: {
        state: {
          roundId: 339105n,
          deployedPerSquare: [1_250_000_000n, ...Array<bigint>(24).fill(0n)],
          deployedPerSquareUi: [1.25, ...Array<number>(24).fill(0)],
          totalDeployed: 1.25,
        },
        minerSnapshot: {
          accountAddress: 'miner-address',
          data: {
            roundId: 339105n,
            deployed: [1_250_000_000n, ...Array<bigint>(24).fill(0n)],
          },
        },
      },
    });
  });
});

describe('ORE deployment preparation', () => {
  it('fetches Board and rejects an explicit stale round before wallet approval', async () => {
    const fetchBoard = vi.fn().mockResolvedValue({ roundId: 43n });
    const createOperations = oreProgramExtensions.createOperations;
    if (!createOperations) throw new Error('ORE program operations are unavailable');
    const operations = createOperations({
      chain: null,
      wallet: undefined,
      program: {
        accounts: {
          Board: { fetch: fetchBoard },
        },
      },
    } as never);

    await expect(
      operations.instructions.mining.deploy.prepare({
        signer: SYSTEM_PROGRAM_ADDRESS,
        amountPerSquare: 1n,
        squares: [0],
        roundId: 42n,
      }),
    ).rejects.toThrow('ORE round 42 is stale; Board is currently on round 43');
    expect(fetchBoard).toHaveBeenCalledOnce();
  });
});
