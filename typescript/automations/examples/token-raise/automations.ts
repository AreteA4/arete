// The automations themselves: what happens when a raise fails. `a4 dev up`
// runs this module against the dev environment (`[dev.automations] entry`
// in the protocol project's arete.toml); server.ts runs the same definition
// in production.

import { defineAutomations } from '@usearete/automations';

import { TOKEN_RAISE_PROTOCOL } from './token-raise';

export default defineAutomations(
  TOKEN_RAISE_PROTOCOL,
  (automations) => {
    automations.on('raise.failed', async (event, ctx) => {
      const grants = await ctx.consents.list({ step: 'refund', instanceId: event.instanceId });
      const contributors: string[] = [];
      for (const grant of grants) {
        // Only wallets that contributed to this raise have anything to refund.
        try {
          await ctx.client.participants!.get({ instance: event.instanceId, wallet: grant.wallet });
          contributors.push(grant.wallet);
        } catch (error) {
          // Anything but "not a participant" fails the handler, so the delivery
          // is retried rather than completed without this wallet's refund.
          if ((error as { details?: { status?: number } }).details?.status !== 404) throw error;
        }
      }
      return contributors.map((wallet) => ctx.step('refund', { wallet, instance: event.instanceId }));
    });
  },
  {
    // A refund moves funds into the wallet; only the network fee leaves it.
    policy: { programs: JSON.parse(process.env.PROGRAMS ?? '[]') as string[], maxSolPerTransaction: '0.001' },
  },
);
