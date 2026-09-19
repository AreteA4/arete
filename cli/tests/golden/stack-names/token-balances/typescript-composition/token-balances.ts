import { createSession, type CompositionSessionOptions } from '@usearete/sdk';
import AlphaStack from './alpha-stack.js';
import BetaStack from './beta-stack.js';



export const TOKEN_BALANCES_SESSION_DEFINITION = {
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

export type TokenBalancesSessionDefinition = typeof TOKEN_BALANCES_SESSION_DEFINITION;
export const TOKEN_BALANCES_SDK = TOKEN_BALANCES_SESSION_DEFINITION;
export type TokenBalancesSdk = TokenBalancesSessionDefinition;

export function createTokenBalancesSession(
  options: CompositionSessionOptions<TokenBalancesSessionDefinition>
) {
  return createSession(TOKEN_BALANCES_SESSION_DEFINITION, options);
}
