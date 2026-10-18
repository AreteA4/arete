import { withProgramRead } from '@usearete/sdk/program';

import { ENTROPY as ENTROPY_PROGRAM_CORE, ENTROPY_READ as ENTROPY_PROGRAM_READ_CORE } from './entropy-core.js';

export * from './entropy-core.js';
export { ENTROPY as ENTROPY_PROGRAM_CORE } from './entropy-core.js';

export const ENTROPY_PROGRAM = withProgramRead(ENTROPY_PROGRAM_CORE, ENTROPY_PROGRAM_READ_CORE);
export const ENTROPY_PROGRAM_READ = ENTROPY_PROGRAM_READ_CORE;

export type EntropyProgram = typeof ENTROPY_PROGRAM;

export default ENTROPY_PROGRAM;
