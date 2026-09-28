import { extendStack } from '@usearete/sdk';

import { VAULT_STREAM_STACK_CORE } from './vault-core.js';
import stackExtensions from './vault-stack-extensions.js';
import hostedVaultProgram from './programs/vault/__arete-program.js';

export * from './vault-core.js';

const HOSTED_PROGRAMS: Omit<typeof VAULT_STREAM_STACK_CORE.programs, 'vault'> & {
  readonly vault: typeof hostedVaultProgram;
} = {
  ...VAULT_STREAM_STACK_CORE.programs,
    vault: hostedVaultProgram,
};

const CORE: Omit<typeof VAULT_STREAM_STACK_CORE, 'programs'> & { readonly programs: typeof HOSTED_PROGRAMS } = {
  ...VAULT_STREAM_STACK_CORE,
  programs: HOSTED_PROGRAMS,
};

export type VaultStreamStack = ReturnType<typeof extendStack<typeof CORE, typeof stackExtensions>>;

export const VAULT_STREAM_STACK: VaultStreamStack = extendStack(CORE, stackExtensions);

export default VAULT_STREAM_STACK;
