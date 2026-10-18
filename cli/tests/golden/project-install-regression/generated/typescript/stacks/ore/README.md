# `ore` TypeScript SDK reference

Generated from stack package `ore` 1.0.0, installed as `ore`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, auth, `get`/`getOne`/`use`/`watch`, preparing and executing operations), see the `arete-streams` and `arete-programs` agent skills or https://docs.arete.run/sdks/typescript/.
Look parts up with `a4 sdk describe ore [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Contents

- Views: `OreRound/state`, `OreRound/list`, `OreRound/latest`, `OreBoard/state`, `OreBoard/list`, `OreTreasury/state`, `OreTreasury/list`, `OreMiner/state`, `OreMiner/list`
- Programs: `programs.ore`, `programs.entropy`

To print one part with the row fields it uses, run `a4 sdk describe ore --read <name>` or `--view <Entity/view>`. The sections below list everything.

## Import

```ts
import { ORE_STREAM_STACK } from './generated/typescript/stacks/ore/ore.js'; // from the project root
```

The path is relative to the importing file: from `src/`, import `'../generated/typescript/stacks/ore/ore.js'`. TypeScript resolves the `.js` to `ore.ts`.

Register it as `ore`: `createSession({ stacks: { ore: ORE_STREAM_STACK } })` gives `session.stacks.ore`; in React, `useArete(ORE_STREAM_STACK)`. Paths below start there. Row types (`OreRound`, `OreBoard`, `OreTreasury`, `OreMiner`) are exported from the same module (declared in `ore-core.ts`).

## Example

A script in `src/` that reads once, prints the result and exits (run it with `npx tsx`):

```ts
import { createSession } from '@usearete/sdk';
import { ORE_STREAM_STACK } from '../generated/typescript/stacks/ore/ore.js';

const session = await createSession({ stacks: { ore: ORE_STREAM_STACK } });
try {
  const result = await session.stacks.ore.views.OreRound.list.getOne();
  console.log(result);
} finally {
  session.close();
}
```

## Field names and types

- Rows use the paths below (`id.roundId`). `a4 get`, `a4 stream`, the MCP `read_view` tool and raw frames use the wire names, marked `←` (`id.round_id`): a wire path in TypeScript reads `undefined`.
- `bigint` fields are 64-bit integers: compare them with `n` literals, and convert them (`Number()`, `String()`) before mixing them with numbers or passing them to `JSON.stringify`.
- Token amounts say their scale: whole units (`1.5`) or raw integer base units (`1500000000`), and the power of ten between them.
- A nullable field is null until the stack has seen its data: check it rather than defaulting it, so a wrong path does not pass silently.

## Entities and views

### OreRound

Views: `views.OreRound.state` (state, key `{ roundId: bigint }`), `views.OreRound.list` (list), `views.OreRound.latest` (list).

Row `OreRound`:

```text
entropy.entropyEndAt            bigint | null          ← entropy_end_at
entropy.entropySamples          bigint | null          ← entropy_samples
entropy.entropySeed             string | null          ← entropy_seed
entropy.entropySlotHash         string | null          ← entropy_slot_hash
entropy.entropyStartAt          bigint | null          ← entropy_start_at
entropy.entropyValue            string | null          ← entropy_value
entropy.entropyVarAddress       string | null          ← entropy_var_address
entropy.resolvedSeed            number[] | null        ← resolved_seed
id.roundAddress                 string | null          ← round_address
id.roundId                      bigint | null          ← round_id
metrics.checkpointCount         bigint | null          ← checkpoint_count
metrics.deployCount             bigint | null          ← deploy_count
results.didHitMotherlode        boolean | null         ← did_hit_motherlode
results.expiresAtSlotHash       SlotHashBytes | null   ← expires_at_slot_hash
results.preRevealRng            bigint | null          ← pre_reveal_rng
results.preRevealRngCandidate   KeccakRngValue | null  ← pre_reveal_rng_candidate
results.preRevealWinningSquare  bigint | null          ← pre_reveal_winning_square
results.rentPayer               string | null          ← rent_payer
results.rng                     KeccakRngValue | null
results.slotHash                string | null          ← slot_hash
results.topMiner                string | null          ← top_miner
results.topMinerReward          number | null          ← top_miner_reward · whole units (raw / 10^11)
results.winningSquare           bigint | null          ← winning_square
state.closesAt                  bigint | null          ← closes_at
state.countPerSquare            bigint[] | null        ← count_per_square
state.deployedPerSquare         bigint[] | null        ← deployed_per_square · raw units (/ 10^9 for whole); whole: state.deployedPerSquareUi
state.deployedPerSquareUi       number[] | null        ← deployed_per_square_ui · whole units (raw / 10^9); raw: state.deployedPerSquare
state.estimatedExpiresAtUnix    bigint | null          ← estimated_expires_at_unix
state.expiresAt                 bigint | null          ← expires_at
state.motherlode                number | null          whole units (raw / 10^11)
state.totalDeployed             number | null          ← total_deployed · whole units (raw / 10^9)
state.totalMiners               bigint | null          ← total_miners
state.totalVaulted              number | null          ← total_vaulted · whole units (raw / 10^9)
state.totalWinnings             number | null          ← total_winnings · whole units (raw / 10^9)
treasury.motherlode             number | null          whole units (raw / 10^11)
oreMetadata                     TokenMetadata | null   ← ore_metadata
```

### OreBoard

Views: `views.OreBoard.state` (state, key `{ address: string }`), `views.OreBoard.list` (list).

Row `OreBoard`:

```text
id.address               string | null
state.endSlot            bigint | null                 ← end_slot
state.productionCostEma  bigint | null                 ← production_cost_ema
state.roundId            bigint | null                 ← round_id
state.startSlot          bigint | null                 ← start_slot
boardSnapshot            CaptureWrapper<Board> | null  ← board_snapshot
```

### OreTreasury

Views: `views.OreTreasury.state` (state, key `{ address: string }`), `views.OreTreasury.list` (list).

Row `OreTreasury`:

```text
id.address            string | null
state.motherlode      number | null                    whole units (raw / 10^11)
state.totalRefined    number | null                    ← total_refined · whole units (raw / 10^11)
state.totalUnclaimed  number | null                    ← total_unclaimed · whole units (raw / 10^11)
treasurySnapshot      CaptureWrapper<Treasury> | null  ← treasury_snapshot
```

### OreMiner

Views: `views.OreMiner.state` (state, key `{ authority: string }`), `views.OreMiner.list` (list).

Row `OreMiner`:

```text
automation.amount           bigint | null
automation.balance          bigint | null
automation.executor         string | null
automation.fee              bigint | null
automation.mask             bigint | null
automation.reload           bigint | null
automation.strategy         bigint | null
id.authority                string | null
id.automationAddress        string | null                      ← automation_address
id.minerAddress             string | null                      ← miner_address
rewards.lifetimeDeployed    bigint | null                      ← lifetime_deployed
rewards.lifetimeRewardsOre  bigint | null                      ← lifetime_rewards_ore
rewards.lifetimeRewardsSol  bigint | null                      ← lifetime_rewards_sol
rewards.refinedOre          bigint | null                      ← refined_ore
rewards.rewardsOre          bigint | null                      ← rewards_ore
rewards.rewardsSol          bigint | null                      ← rewards_sol
state.checkpointFee         bigint | null                      ← checkpoint_fee
state.checkpointId          bigint | null                      ← checkpoint_id
state.deployedPerSquare     bigint[] | null                    ← deployed_per_square · raw units (/ 10^9 for whole); whole: state.deployedPerSquareUi
state.deployedPerSquareUi   number[] | null                    ← deployed_per_square_ui · whole units (raw / 10^9); raw: state.deployedPerSquare
state.lastClaimOreAt        bigint | null                      ← last_claim_ore_at
state.lastClaimSolAt        bigint | null                      ← last_claim_sol_at
state.roundId               bigint | null                      ← round_id
state.totalDeployed         number | null                      ← total_deployed · whole units (raw / 10^9)
minerSnapshot               CaptureWrapper<Miner> | null       ← miner_snapshot
automationSnapshot          CaptureWrapper<Automation> | null  ← automation_snapshot
```

## Programs

### `programs.ore` (program `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`)

Reach it as `session.stacks.ore.programs.ore`. Descriptions: `programs/ore/README.md`, or `a4 sdk describe ore --program ore`.

#### Accounts and instructions

- Accounts (`accounts.<Name>.fetch(address)`): `Automation`, `Board`, `Config`, `Miner`, `Round`, `Treasury`
- Raw instructions (`raw.<name>`): `automate`, `checkpoint`, `claimSol`, `claimOre`, `close`, `deploy`, `log`, `reset`, `bury`, `buyback`, `wrap`, `setAdmin`, `newVar`
- PDAs (`pdas.<name>`): `automation`, `board`, `config`, `miner`, `treasury`

### `programs.entropy` (program `3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X`)

Reach it as `session.stacks.ore.programs.entropy`. Descriptions: `programs/entropy/README.md`, or `a4 sdk describe ore --program entropy`.

#### Accounts and instructions

- Accounts (`accounts.<Name>.fetch(address)`): `Var`
- Raw instructions (`raw.<name>`): `open`, `close`, `next`, `reveal`, `sample`

