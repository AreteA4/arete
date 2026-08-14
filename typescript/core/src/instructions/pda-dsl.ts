import { findProgramAddress, findProgramAddressSync, decodeBase58 } from './pda';
import { serializeSeedValue } from './seed-serializer';
import { getValueByPath } from './path-utils';

export type SeedDef =
  | { type: 'literal'; value: string }
  | { type: 'bytes'; value: Uint8Array }
  | { type: 'argRef'; argName: string; argType?: string }
  | { type: 'accountRef'; accountName: string };

export interface PdaDeriveContext {
  accounts?: Record<string, string>;
  args?: Record<string, unknown>;
  resolve?: Record<string, unknown>;
  programId?: string;
}

export type PdaProgramSelector =
  | string
  | { type: 'accountRef'; accountName: string }
  | { type: 'argRef'; argName: string };

export interface PdaFactory {
  readonly seeds: readonly SeedDef[];
  readonly programId?: string;
  readonly programSelector: PdaProgramSelector;
  program(programId: string): PdaFactory;
  derive(context: PdaDeriveContext): Promise<string>;
  deriveSync(context: PdaDeriveContext): string;
}

export function literal(value: string): SeedDef {
  return { type: 'literal', value };
}

export function account(name: string): SeedDef {
  return { type: 'accountRef', accountName: name };
}

export function arg(name: string, type?: string): SeedDef {
  return { type: 'argRef', argName: name, argType: type };
}

export function bytes(value: Uint8Array): SeedDef {
  return { type: 'bytes', value };
}

function resolveSeeds(seeds: readonly SeedDef[], context: PdaDeriveContext): Uint8Array[] {
  return seeds.map((seed) => {
    switch (seed.type) {
      case 'literal':
        return new TextEncoder().encode(seed.value);
      case 'bytes':
        return seed.value;
      case 'argRef': {
        const value = getValueByPath(context.args, seed.argName) ?? getValueByPath(context.resolve, seed.argName);
        if (value === undefined) {
          throw new Error(`Missing arg for PDA seed: ${seed.argName}`);
        }
        return serializeSeedValue(value, seed.argType);
      }
      case 'accountRef': {
        const address = context.accounts?.[seed.accountName];
        if (!address) {
          throw new Error(`Missing account for PDA seed: ${seed.accountName}`);
        }
        return decodeBase58(address);
      }
    }
  });
}

function resolveProgram(
  selector: PdaProgramSelector,
  context: PdaDeriveContext
): string {
  if (typeof selector === 'string') return context.programId ?? selector;
  if (selector.type === 'accountRef') {
    const address = context.accounts?.[selector.accountName];
    if (!address) throw new Error(`Missing account for PDA program: ${selector.accountName}`);
    return address;
  }
  const value = getValueByPath(context.args, selector.argName)
    ?? getValueByPath(context.resolve, selector.argName);
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`Missing argument for PDA program: ${selector.argName}`);
  }
  return value;
}

export function pda(programSelector: PdaProgramSelector, ...seeds: SeedDef[]): PdaFactory {
  return {
    seeds,
    programId: typeof programSelector === 'string' ? programSelector : undefined,
    programSelector,

    program(newProgramId: string): PdaFactory {
      return pda(newProgramId, ...seeds);
    },

    async derive(context: PdaDeriveContext): Promise<string> {
      const resolvedSeeds = resolveSeeds(this.seeds, context);
      const pid = resolveProgram(this.programSelector, context);
      const [address] = await findProgramAddress(resolvedSeeds, pid);
      return address;
    },

    deriveSync(context: PdaDeriveContext): string {
      const resolvedSeeds = resolveSeeds(this.seeds, context);
      const pid = resolveProgram(this.programSelector, context);
      const [address] = findProgramAddressSync(resolvedSeeds, pid);
      return address;
    },
  };
}

export type ProgramPdas<T extends Record<string, PdaFactory>> = T;

export function createProgramPdas<T extends Record<string, PdaFactory>>(pdas: T): ProgramPdas<T> {
  return pdas;
}
