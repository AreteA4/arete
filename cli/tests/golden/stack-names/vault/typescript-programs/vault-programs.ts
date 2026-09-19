import { VAULT_PROGRAMS as VAULT_PROGRAMS_CORE } from './vault-programs-core.js';

export * from './vault-programs-core.js';

export const VAULT_PROGRAMS = VAULT_PROGRAMS_CORE;

export type vaultPrograms = typeof VAULT_PROGRAMS;

export default VAULT_PROGRAMS;
