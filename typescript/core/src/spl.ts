import { createPublicKeySeed, findProgramAddressSync } from './instructions';
import type { ChainClient } from './chain';

export const SPL_TOKEN_PROGRAM_ADDRESS = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
export const TOKEN_2022_PROGRAM_ADDRESS = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
export const ASSOCIATED_TOKEN_PROGRAM_ADDRESS = 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL';
export const SYSTEM_PROGRAM_ADDRESS = '11111111111111111111111111111111';

export async function resolveTokenProgramAddress(
  chain: ChainClient,
  mint: string,
  override?: string
): Promise<string> {
  if (override) {
    return override;
  }
  const mintAccount = await chain.mint(mint);
  if (!mintAccount) {
    throw new Error(`Mint account not found while resolving token program: ${mint}`);
  }
  if (
    mintAccount.ownerProgram !== SPL_TOKEN_PROGRAM_ADDRESS &&
    mintAccount.ownerProgram !== TOKEN_2022_PROGRAM_ADDRESS
  ) {
    throw new Error(
      `Mint ${mint} is owned by unsupported token program ${mintAccount.ownerProgram}`
    );
  }
  return mintAccount.ownerProgram;
}

export function deriveAssociatedTokenAccount(input: {
  owner: string;
  mint: string;
  tokenProgram?: string;
}): string {
  const [address] = findProgramAddressSync(
    [
      createPublicKeySeed(input.owner),
      createPublicKeySeed(input.tokenProgram ?? SPL_TOKEN_PROGRAM_ADDRESS),
      createPublicKeySeed(input.mint),
    ],
    ASSOCIATED_TOKEN_PROGRAM_ADDRESS
  );
  return address;
}
