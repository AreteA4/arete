import { createSession, type CompositionSessionOptions } from '@usearete/sdk';
import AlphaStack from './alpha-stack.js';
import BetaStack from './beta-stack.js';



export const A9LIVES_SESSION_DEFINITION = {
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

export type A9livesSessionDefinition = typeof A9LIVES_SESSION_DEFINITION;
export const A9LIVES_SDK = A9LIVES_SESSION_DEFINITION;
export type A9livesSdk = A9livesSessionDefinition;

export function createA9livesSession(
  options: CompositionSessionOptions<A9livesSessionDefinition>
) {
  return createSession(A9LIVES_SESSION_DEFINITION, options);
}
