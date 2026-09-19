import { createSession, type CompositionSessionOptions } from '@usearete/sdk';
import AlphaStack from './alpha-stack.js';
import BetaStack from './beta-stack.js';



export const MY_STACK_SESSION_DEFINITION = {
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

export type MyStackSessionDefinition = typeof MY_STACK_SESSION_DEFINITION;
export const MY_STACK_SDK = MY_STACK_SESSION_DEFINITION;
export type MyStackSdk = MyStackSessionDefinition;

export function createMyStackSession(
  options: CompositionSessionOptions<MyStackSessionDefinition>
) {
  return createSession(MY_STACK_SESSION_DEFINITION, options);
}
