import { z } from 'zod';
import { Arete } from '@usearete/sdk';
import {
  ORE_STREAM_STACK,
  OreRoundSchema,
  OreRoundIdSchema,
  OreTreasurySchema,
  OreTreasuryIdSchema,
} from './generated/ore-stack';

const OreRoundWithIdSchema = OreRoundSchema.extend({
  id: OreRoundIdSchema.partial(),
});

const OreTreasuryWithIdSchema = OreTreasurySchema.extend({
  id: OreTreasuryIdSchema.partial(),
});
type OreRoundWithId = z.infer<typeof OreRoundWithIdSchema>;
type OreTreasuryWithId = z.infer<typeof OreTreasuryWithIdSchema>;

function printRound(round: OreRoundWithId) {
  console.log(`\n=== Round #${round.id.round_id ?? 'N/A'} ===`);
  console.log(`Address: ${round.id.round_address ?? 'N/A'}`);
  console.log(`Motherlode: ${round.state?.motherlode ?? 'N/A'}`);
  console.log(`Total Deployed: ${round.state?.total_deployed ?? 'N/A'}`);
  console.log(`Expires At: ${round.state?.expires_at ?? 'N/A'}`);
  console.log(`Deploy Count: ${round.metrics?.deploy_count ?? 0}`);
  console.log();
}

function printTreasury(treasury: OreTreasuryWithId) {
  console.log(`\n=== Treasury ===`);
  console.log(`Address: ${treasury.id.address ?? 'N/A'}`);
  console.log(`Balance: ${treasury.state?.balance ?? 'N/A'}`);
  console.log(`Motherlode: ${treasury.state?.motherlode ?? 'N/A'}`);
  console.log(`Total Refined: ${treasury.state?.total_refined ?? 'N/A'}`);
  console.log(`Total Staked: ${treasury.state?.total_staked ?? 'N/A'}`);
  console.log(`Total Unclaimed: ${treasury.state?.total_unclaimed ?? 'N/A'}`);
  console.log();
}

async function main() {
  const a4 = await Arete.connect(ORE_STREAM_STACK, { url: 'http://localhost:8878' });

  console.log('--- Streaming OreRound and OreTreasury updates ---\n');

  const streamRounds = async () => {
    for await (const round of a4.views.OreRound.latest.use({
      take: 1,
      schema: OreRoundWithIdSchema,
    })) {
      printRound(round);
    }
  };

  const streamTreasury = async () => {
    for await (const treasury of a4.views.OreTreasury.list.use({
      take: 1,
      schema: OreTreasuryWithIdSchema,
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
