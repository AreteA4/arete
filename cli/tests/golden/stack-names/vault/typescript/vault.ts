import { VAULT_STACK_CORE } from './vault-core.js';

export * from './vault-core.js';

export const VAULT_STACK = VAULT_STACK_CORE;

export type vaultStack = typeof VAULT_STACK;

export default VAULT_STACK;
