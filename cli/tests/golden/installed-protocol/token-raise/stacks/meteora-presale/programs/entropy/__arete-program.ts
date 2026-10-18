import { withProgramIdentity, withProgramRead } from '@usearete/sdk/program';

import { ENTROPY as ENTROPY_PROGRAM_CORE, ENTROPY_READ as ENTROPY_PROGRAM_READ_CORE } from './entropy-core.js';

export * from './entropy-core.js';
export { ENTROPY as ENTROPY_PROGRAM_CORE } from './entropy-core.js';

export const ENTROPY_PROGRAM = withProgramIdentity(
  withProgramRead(ENTROPY_PROGRAM_CORE, ENTROPY_PROGRAM_READ_CORE),
  { packageReleaseHash: 'arete:registry-package-release:v2:sha256:6666666666666666666666666666666666666666666666666666666666666666' },
);
export const ENTROPY_PROGRAM_READ = ENTROPY_PROGRAM_READ_CORE;

export type EntropyProgram = typeof ENTROPY_PROGRAM;

export default ENTROPY_PROGRAM;
