import { defineStackExtensions } from '@usearete/sdk';
import type { VAULT_STREAM_STACK_CORE } from './vault-core.js';

interface VaultLimits {
  maxDeposit: bigint;
}

export default defineStackExtensions<typeof VAULT_STREAM_STACK_CORE>()({
  defaults: {
    limits(): VaultLimits {
      return { maxDeposit: 1_000_000n };
    },
  },
});
