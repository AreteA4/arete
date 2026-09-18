import { createSession, type CompositionSessionOptions } from '@usearete/sdk';
import AlphaStack from './alpha-stack.js';
import BetaStack from './beta-stack.js';



export const VAULT_STREAM_SESSION_DEFINITION = {
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

export type VaultStreamSessionDefinition = typeof VAULT_STREAM_SESSION_DEFINITION;
export const VAULT_STREAM_SDK = VAULT_STREAM_SESSION_DEFINITION;
export type VaultStreamSdk = VaultStreamSessionDefinition;

export function createVaultStreamSession(
  options: CompositionSessionOptions<VaultStreamSessionDefinition>
) {
  return createSession(VAULT_STREAM_SESSION_DEFINITION, options);
}
