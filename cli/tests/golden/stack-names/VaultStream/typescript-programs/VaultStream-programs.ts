import { VAULT_STREAM_PROGRAMS as VAULT_STREAM_PROGRAMS_CORE } from './VaultStream-programs-core.js';

export * from './VaultStream-programs-core.js';

export const VAULT_STREAM_PROGRAMS = VAULT_STREAM_PROGRAMS_CORE;

export type VaultStreamPrograms = typeof VAULT_STREAM_PROGRAMS;

export default VAULT_STREAM_PROGRAMS;
