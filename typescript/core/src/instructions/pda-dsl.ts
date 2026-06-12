import { findProgramAddress, findProgramAddressSync, decodeBase58 } from './pda';
import { serializeSeedValue } from './seed-serializer';

export type SeedDef =
  | { type: 'literal'; value: string }
  | { type: 'bytes'; value: Uint8Array }
  | { type: 'argRef'; argName: string; argType?: string }
  | { type: 'accountRef'; accountName: string };

export interface PdaDeriveContext {
  accounts?: Record<string, string>;
  args?: Record<string, unknown>;
  programId?: string;
}

export interface PdaFactory {
  readonly seeds: readonly SeedDef[];
  readonly programId: string;
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
        const value = context.args?.[seed.argName];
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

export function pda(programId: string, ...seeds: SeedDef[]): PdaFactory {
  return {
    seeds,
    programId,

    program(newProgramId: string): PdaFactory {
      return pda(newProgramId, ...seeds);
    },

    async derive(context: PdaDeriveContext): Promise<string> {
      const resolvedSeeds = resolveSeeds(this.seeds, context);
      const pid = context.programId ?? this.programId;
      const [address] = await findProgramAddress(resolvedSeeds, pid);
      return address;
    },

    deriveSync(context: PdaDeriveContext): string {
      const resolvedSeeds = resolveSeeds(this.seeds, context);
      const pid = context.programId ?? this.programId;
      const [address] = findProgramAddressSync(resolvedSeeds, pid);
      return address;
    },
  };
}

export type ProgramPdas<T extends Record<string, PdaFactory>> = T;

export function createProgramPdas<T extends Record<string, PdaFactory>>(pdas: T): ProgramPdas<T> {
  return pdas;
}
