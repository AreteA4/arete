import { extendPrograms, extendStack } from '@usearete/sdk';

import { ORE_STREAM_STACK_CORE } from './ore-stack-core.js';
import stackExtensions, { oreProgramExtensions } from './ore-stack-extensions.js';

export * from './ore-stack-core.js';

const CORE: Omit<typeof ORE_STREAM_STACK_CORE, 'programs'> & {
  readonly programs: ReturnType<typeof extendPrograms<typeof ORE_STREAM_STACK_CORE.programs, { ore: typeof oreProgramExtensions }>>;
} = {
  ...ORE_STREAM_STACK_CORE,
  programs: extendPrograms(ORE_STREAM_STACK_CORE.programs, {
    ore: oreProgramExtensions,
  }),
};

export type OreStreamStack = ReturnType<typeof extendStack<typeof CORE, typeof stackExtensions>>;

export const ORE_STREAM_STACK: OreStreamStack = extendStack(
  CORE,
  stackExtensions
);

export default ORE_STREAM_STACK;
