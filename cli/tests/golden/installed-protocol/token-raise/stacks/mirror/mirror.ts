

import { ORE_STREAM_STACK_CORE } from './mirror-core.js';

import hostedOreProgram from './programs/ore/__arete-program.js';
import hostedEntropyProgram from './programs/entropy/__arete-program.js';

export * from './mirror-core.js';

const HOSTED_PROGRAMS: Omit<typeof ORE_STREAM_STACK_CORE.programs, 'ore' | 'entropy'> & {
  readonly ore: typeof hostedOreProgram;
  readonly entropy: typeof hostedEntropyProgram;
} = {
  ...ORE_STREAM_STACK_CORE.programs,
    ore: hostedOreProgram,
    entropy: hostedEntropyProgram,
};

const CORE: Omit<typeof ORE_STREAM_STACK_CORE, 'programs'> & { readonly programs: typeof HOSTED_PROGRAMS } = {
  ...ORE_STREAM_STACK_CORE,
  programs: HOSTED_PROGRAMS,
};

export const ORE_STREAM_STACK: typeof CORE = CORE;

export type OreStreamStack = typeof ORE_STREAM_STACK;

export default ORE_STREAM_STACK;
