import { extendProgram, withProgramIdentity, withProgramRead } from '@usearete/sdk/program';

import { VOTE_PROGRAM as VOTE_PROGRAM_CORE, VOTE_PROGRAM_READ as VOTE_PROGRAM_READ_CORE, VOTE_PROGRAM_INSTRUCTION_SHAPES } from './vote-core.js';
import programExtensions from './vote-extensions.js';

export * from './vote-core.js';
export { VOTE_PROGRAM as VOTE_PROGRAM_CORE } from './vote-core.js';

/** How this project installed the program (`programInstall` on VOTE_PROGRAM). */
export const VOTE_PROGRAM_INSTALL = {
  alias: 'vote',
  programReleaseHash: 'arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
  source: { kind: 'registry', registry: 'vote', version: '0.1.0' },
} as const;

export const VOTE_PROGRAM = Object.assign(
  withProgramIdentity(
    withProgramRead(extendProgram(VOTE_PROGRAM_CORE, programExtensions), VOTE_PROGRAM_READ_CORE),
    { packageReleaseHash: 'arete:registry-package-release:v2:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' },
  ),
  { programInstall: VOTE_PROGRAM_INSTALL, instructionShapes: VOTE_PROGRAM_INSTRUCTION_SHAPES },
);
export const VOTE_PROGRAM_READ = VOTE_PROGRAM_READ_CORE;

export type VoteProgram = typeof VOTE_PROGRAM;

export default VOTE_PROGRAM;
