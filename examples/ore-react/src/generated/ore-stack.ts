import { extendPrograms, extendStack } from '@usearete/sdk';

import { ORE_STREAM_STACK_CORE } from './ore-stack-core.js';
import stackExtensions, { oreProgramExtensions } from './ore-stack-extensions.js';

export * from './ore-stack-core.js';

const CORE = {
  ...ORE_STREAM_STACK_CORE,
  programs: extendPrograms(ORE_STREAM_STACK_CORE.programs, {
    ore: oreProgramExtensions,
  }),
} as const;

export const ORE_STREAM_STACK = extendStack(
  CORE,
  stackExtensions
);

export type OreStreamStack = typeof ORE_STREAM_STACK;

export default ORE_STREAM_STACK;
