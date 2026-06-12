import type { WalletAdapter } from '../wallet/types';
import { findProgramAddressSync, decodeBase58, createSeed } from './pda';
import { serializeSeedValue } from './seed-serializer';

/**
 * Categories of accounts in an instruction.
 * - `signer`: Must sign the transaction (e.g., user wallet)
 * - `known`: Program-derived addresses with fixed known addresses (e.g., System Program)
 * - `pda`: Program-derived addresses computed from seeds
 * - `userProvided`: Must be provided by the caller (e.g., mint, bonding curve)
 */
export type AccountCategory = 'signer' | 'known' | 'pda' | 'userProvided';

/**
 * Metadata for a single account in an instruction.
 */
export interface AccountMeta {
  /** Account name (e.g., "user", "mint") */
  name: string;
  /** Whether this account must sign the transaction */
  isSigner: boolean;
  /** Whether this account is writable */
  isWritable: boolean;
  /** Category of this account */
  category: AccountCategory;
  /** Fixed address for "known" accounts (e.g., "11111111111111111111111111111111") */
  knownAddress?: string;
  /** PDA configuration for "pda" accounts */
  pdaConfig?: PdaConfig;
  /** Whether this account is optional */
  isOptional?: boolean;
}

/**
 * Configuration for PDA (Program-Derived Address) derivation.
 */
export interface PdaConfig {
  /** Program ID that owns this PDA (defaults to instruction's programId) */
  programId?: string;
  /** Seed definitions for PDA derivation */
  seeds: PdaSeed[];
}

/**
 * Single seed in a PDA derivation.
 */
export type PdaSeed =
  | { type: 'literal'; value: string }
  | { type: 'bytes'; value: number[] }
  | { type: 'argRef'; argName: string; argType?: string }
  | { type: 'accountRef'; accountName: string };

/**
 * Resolved account with its final address.
 */
export interface ResolvedAccount {
  /** Account name */
  name: string;
  /** The resolved public key address */
  address: string;
  /** Whether this account must sign */
  isSigner: boolean;
  /** Whether this account is writable */
  isWritable: boolean;
}

/**
 * Result of account resolution.
 */
export interface AccountResolutionResult {
  /** All resolved accounts in order */
  accounts: ResolvedAccount[];
  /** Accounts that need to be provided by the user */
  missingUserAccounts: string[];
}

/**
 * Options for account resolution.
 */
export interface AccountResolutionOptions {
  /** User-provided account addresses */
  accounts?: Record<string, string>;
  /** Wallet adapter for signer accounts */
  wallet?: WalletAdapter;
  /** Program ID for PDA derivation (required if any PDAs exist) */
  programId?: string;
}

/**
 * Topologically sort accounts so that dependencies (accountRef) are resolved first.
 * Non-PDA accounts come first, then PDAs in dependency order.
 */
function sortAccountsByDependency(accountMetas: AccountMeta[]): AccountMeta[] {
  // Separate non-PDA and PDA accounts
  const nonPda: AccountMeta[] = [];
  const pda: AccountMeta[] = [];
  
  for (const meta of accountMetas) {
    if (meta.category === 'pda') {
      pda.push(meta);
    } else {
      nonPda.push(meta);
    }
  }
  
  // Build dependency graph for PDAs
  const pdaDeps = new Map<string, Set<string>>();
  for (const meta of pda) {
    const deps = new Set<string>();
    if (meta.pdaConfig) {
      for (const seed of meta.pdaConfig.seeds) {
        if (seed.type === 'accountRef') {
          deps.add(seed.accountName);
        }
      }
    }
    pdaDeps.set(meta.name, deps);
  }
  
  // Topological sort PDAs
  const sortedPda: AccountMeta[] = [];
  const visited = new Set<string>();
  const visiting = new Set<string>();
  
  function visit(name: string): void {
    if (visited.has(name)) return;
    if (visiting.has(name)) {
      throw new Error('Circular dependency in PDA accounts: ' + name);
    }
    
    const meta = pda.find(m => m.name === name);
    if (!meta) return; // Not a PDA, skip
    
    visiting.add(name);
    
    const deps = pdaDeps.get(name) || new Set();
    for (const dep of deps) {
      // Only visit if dep is also a PDA
      if (pda.some(m => m.name === dep)) {
        visit(dep);
      }
    }
    
    visiting.delete(name);
    visited.add(name);
    sortedPda.push(meta);
  }
  
  for (const meta of pda) {
    visit(meta.name);
  }
  
  return [...nonPda, ...sortedPda];
}

/**
 * Resolves instruction accounts by categorizing and deriving addresses.
 * 
 * Resolution order:
 * 1. Non-PDA accounts (signer, known, userProvided) are resolved first
 * 2. PDA accounts are resolved in dependency order (accounts they reference come first)
 * 
 * @param accountMetas - Account metadata from the instruction definition
 * @param args - Instruction arguments (used for PDA derivation with argRef seeds)
 * @param options - Resolution options including wallet, user-provided accounts, and programId
 * @returns Resolved accounts and any missing required accounts
 */
export function resolveAccounts(
  accountMetas: AccountMeta[],
  args: Record<string, unknown>,
  options: AccountResolutionOptions
): AccountResolutionResult {
  // Sort accounts by dependency
  const sorted = sortAccountsByDependency(accountMetas);
  
  // Track resolved accounts for PDA accountRef lookups
  const resolvedMap: Record<string, ResolvedAccount> = {};
  const missing: string[] = [];

  for (const meta of sorted) {
    const resolvedAccount = resolveSingleAccount(meta, args, options, resolvedMap);
    
    if (resolvedAccount) {
      resolvedMap[meta.name] = resolvedAccount;
    } else if (!meta.isOptional) {
      missing.push(meta.name);
    }
  }

  // Return accounts in original order (as defined in accountMetas).
  //
  // Omitted optional accounts that precede a resolved account cannot simply
  // be dropped — that would shift every later account into the wrong slot.
  // Anchor's convention is to pass the program ID as a placeholder; trailing
  // omitted optionals are dropped as usual.
  const lastResolvedIndex = accountMetas.reduce(
    (last, meta, index) => (resolvedMap[meta.name] ? index : last),
    -1
  );

  const orderedAccounts: ResolvedAccount[] = [];
  for (let i = 0; i < accountMetas.length; i++) {
    const meta = accountMetas[i]!;
    const resolved = resolvedMap[meta.name];
    if (resolved) {
      orderedAccounts.push(resolved);
    } else if (meta.isOptional && i < lastResolvedIndex) {
      if (!options.programId) {
        throw new Error(
          'Omitted optional account "' + meta.name + '" precedes other accounts and needs ' +
          'the program ID as a placeholder, but no programId was provided in options.'
        );
      }
      orderedAccounts.push({
        name: meta.name,
        address: options.programId,
        isSigner: false,
        isWritable: false,
      });
    }
  }

  return {
    accounts: orderedAccounts,
    missingUserAccounts: missing,
  };
}

function resolveSingleAccount(
  meta: AccountMeta,
  args: Record<string, unknown>,
  options: AccountResolutionOptions,
  resolvedMap: Record<string, ResolvedAccount>
): ResolvedAccount | null {
  switch (meta.category) {
    case 'signer':
      return resolveSignerAccount(meta, options.wallet);
    case 'known':
      return resolveKnownAccount(meta);
    case 'pda':
      return resolvePdaAccount(meta, args, resolvedMap, options.programId);
    case 'userProvided':
      return resolveUserProvidedAccount(meta, options.accounts);
    default:
      return null;
  }
}

function resolveSignerAccount(
  meta: AccountMeta,
  wallet?: WalletAdapter
): ResolvedAccount | null {
  if (!wallet) {
    return null;
  }

  return {
    name: meta.name,
    address: wallet.publicKey,
    isSigner: true,
    isWritable: meta.isWritable,
  };
}

function resolveKnownAccount(meta: AccountMeta): ResolvedAccount | null {
  if (!meta.knownAddress) {
    return null;
  }

  return {
    name: meta.name,
    address: meta.knownAddress,
    isSigner: meta.isSigner,
    isWritable: meta.isWritable,
  };
}

function resolvePdaAccount(
  meta: AccountMeta,
  args: Record<string, unknown>,
  resolvedMap: Record<string, ResolvedAccount>,
  programId?: string
): ResolvedAccount | null {
  if (!meta.pdaConfig) {
    return null;
  }

  // Determine which program to derive against
  const pdaProgramId = meta.pdaConfig.programId || programId;
  if (!pdaProgramId) {
    throw new Error(
      'Cannot derive PDA for "' + meta.name + '": no programId specified. ' +
      'Either set pdaConfig.programId or pass programId in options.'
    );
  }

  // Build seeds array
  const seeds: Uint8Array[] = [];
  
  for (const seed of meta.pdaConfig.seeds) {
    switch (seed.type) {
      case 'literal':
        seeds.push(createSeed(seed.value));
        break;

      case 'bytes':
        seeds.push(Uint8Array.from(seed.value));
        break;

      case 'argRef': {
        const argValue = args[seed.argName];
        if (argValue === undefined) {
          throw new Error(
            'PDA seed references missing argument: ' + seed.argName +
            ' (for account "' + meta.name + '")'
          );
        }
        seeds.push(serializeSeedValue(argValue, seed.argType));
        break;
      }
      
      case 'accountRef': {
        const refAccount = resolvedMap[seed.accountName];
        if (!refAccount) {
          throw new Error(
            'PDA seed references unresolved account: ' + seed.accountName +
            ' (for account "' + meta.name + '")'
          );
        }
        // Account addresses are 32 bytes
        seeds.push(decodeBase58(refAccount.address));
        break;
      }
      
      default:
        throw new Error('Unknown seed type');
    }
  }

  // Derive the PDA
  const [derivedAddress] = findProgramAddressSync(seeds, pdaProgramId);

  return {
    name: meta.name,
    address: derivedAddress,
    isSigner: meta.isSigner,
    isWritable: meta.isWritable,
  };
}

function resolveUserProvidedAccount(
  meta: AccountMeta,
  accounts?: Record<string, string>
): ResolvedAccount | null {
  const address = accounts?.[meta.name];
  
  if (!address) {
    return null;
  }

  return {
    name: meta.name,
    address,
    isSigner: meta.isSigner,
    isWritable: meta.isWritable,
  };
}

/**
 * Validates that all required accounts are present.
 * 
 * @param result - Account resolution result
 * @throws Error if any required accounts are missing
 */
export function validateAccountResolution(result: AccountResolutionResult): void {
  if (result.missingUserAccounts.length > 0) {
    throw new Error(
      'Missing required accounts: ' + result.missingUserAccounts.join(', ')
    );
  }
}
