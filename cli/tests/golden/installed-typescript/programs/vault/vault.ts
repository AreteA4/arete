import { extendProgram, withProgramIdentity, withProgramRead } from '@usearete/sdk';

import { VAULT as VAULT_PROGRAM_CORE, VAULT_READ as VAULT_PROGRAM_READ_CORE } from './vault-core.js';
import programExtensions from './vault-extensions.js';

export * from './vault-core.js';
export { VAULT as VAULT_PROGRAM_CORE } from './vault-core.js';

export type VaultProgram = ReturnType<typeof extendProgram<typeof VAULT_PROGRAM_CORE, typeof programExtensions>>;

export const VAULT_PROGRAM: VaultProgram = withProgramIdentity(
  withProgramRead(
    extendProgram(VAULT_PROGRAM_CORE, programExtensions),
    VAULT_PROGRAM_READ_CORE,
  ),
  { packageReleaseHash: 'arete:registry-package-release:v2:sha256:7777777777777777777777777777777777777777777777777777777777777777' },
);
export const VAULT_PROGRAM_READ = VAULT_PROGRAM_READ_CORE;

export default VAULT_PROGRAM;
