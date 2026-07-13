import { z } from 'zod';
import { Arete } from '@usearete/sdk';
import {
  ORE_STREAM_STACK,
  OreRoundSchema,
  OreTreasurySchema,
} from './generated/ore-stack';

type OreRoundWithId = z.infer<typeof OreRoundSchema>;
type OreTreasuryWithId = z.infer<typeof OreTreasurySchema>;

function printRound(round: OreRoundWithId) {
  console.log(`\n=== Round #${round.id?.roundId?.toString() ?? 'N/A'} ===`);
  console.log(`Address: ${round.id?.roundAddress ?? 'N/A'}`);
  console.log(`Motherlode: ${round.state?.motherlode ?? 'N/A'}`);
  console.log(`Total Deployed: ${round.state?.totalDeployed ?? 'N/A'}`);
  console.log(`Expires At: ${round.state?.expiresAt?.toString() ?? 'N/A'}`);
  console.log(`Deploy Count: ${round.metrics?.deployCount?.toString() ?? 0}`);
  console.log();
}

function printTreasury(treasury: OreTreasuryWithId) {
  console.log(`\n=== Treasury ===`);
  console.log(`Address: ${treasury.id?.address ?? 'N/A'}`);
  console.log(`Balance: ${treasury.state?.balance?.toString() ?? 'N/A'}`);
  console.log(`Motherlode: ${treasury.state?.motherlode ?? 'N/A'}`);
  console.log(`Total Refined: ${treasury.state?.totalRefined ?? 'N/A'}`);
  console.log(`Total Staked: ${treasury.state?.totalStaked ?? 'N/A'}`);
  console.log(`Total Unclaimed: ${treasury.state?.totalUnclaimed ?? 'N/A'}`);
  console.log();
}

async function main() {
  const a4 = await Arete.connect(ORE_STREAM_STACK, { url: 'http://localhost:8878' });

  console.log('--- Streaming OreRound and OreTreasury updates ---\n');

  const streamRounds = async () => {
    for await (const round of a4.views.OreRound.latest.use({
      take: 1,
      schema: OreRoundSchema,
    })) {
      printRound(round);
    }
  };

  const streamTreasury = async () => {
    for await (const treasury of a4.views.OreTreasury.list.use({
      take: 1,
      schema: OreTreasurySchema,
    })) {
      printTreasury(treasury);
    }
  };

  await Promise.all([streamRounds(), streamTreasury()]);
}

main().catch((err) => {
  console.error('Error:', err);
  process.exit(1);
});
