import { createSession, type CompositionSessionOptions } from '@usearete/sdk';
import AlphaStack from './alpha-stack.js';
import BetaStack from './beta-stack.js';



export const VAULT_SESSION_DEFINITION = {
  mode: 'composition',

  stacks: {
    alpha: AlphaStack,
    beta: BetaStack,
  },
  programs: {
    vault: AlphaStack.programs.vault,
  },
  programReads: {
    vault: AlphaStack.programReads.vault,
  },
} as const;

export type VaultSessionDefinition = typeof VAULT_SESSION_DEFINITION;
export const VAULT_SDK = VAULT_SESSION_DEFINITION;
export type VaultSdk = VaultSessionDefinition;

export function createVaultSession(
  options: CompositionSessionOptions<VaultSessionDefinition>
) {
  return createSession(VAULT_SESSION_DEFINITION, options);
}
