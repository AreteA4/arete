import { withProgramRead } from '@usearete/sdk/program';

import { ORE as ORE_PROGRAM_CORE, ORE_READ as ORE_PROGRAM_READ_CORE } from './ore-core.js';

export * from './ore-core.js';
export { ORE as ORE_PROGRAM_CORE } from './ore-core.js';

export const ORE_PROGRAM = withProgramRead(ORE_PROGRAM_CORE, ORE_PROGRAM_READ_CORE);
export const ORE_PROGRAM_READ = ORE_PROGRAM_READ_CORE;

export type OreProgram = typeof ORE_PROGRAM;

export default ORE_PROGRAM;
