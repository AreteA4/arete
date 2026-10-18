import { withProgramIdentity, withProgramRead } from '@usearete/sdk/program';

import { VOTE_PROGRAM as SQUADS_V4_PROGRAM_CORE, VOTE_PROGRAM_READ as SQUADS_V4_PROGRAM_READ_CORE } from './squads-v4-core.js';

export * from './squads-v4-core.js';
export { VOTE_PROGRAM as SQUADS_V4_PROGRAM_CORE } from './squads-v4-core.js';

export const SQUADS_V4_PROGRAM = withProgramIdentity(
  withProgramRead(SQUADS_V4_PROGRAM_CORE, SQUADS_V4_PROGRAM_READ_CORE),
  { packageReleaseHash: 'arete:registry-package-release:v2:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' },
);
export const SQUADS_V4_PROGRAM_READ = SQUADS_V4_PROGRAM_READ_CORE;

export type SquadsV4Program = typeof SQUADS_V4_PROGRAM;

export default SQUADS_V4_PROGRAM;
