import { describe, expect, it } from 'vitest';

import {
  deriveAssociatedTokenAccount,
  resolveTokenProgramAddress,
  SPL_TOKEN_PROGRAM_ADDRESS,
  TOKEN_2022_PROGRAM_ADDRESS,
} from './spl';

describe('deriveAssociatedTokenAccount', () => {
  it('matches a known mainnet USDC ATA derivation', () => {
    expect(
      deriveAssociatedTokenAccount({
        owner: 'So11111111111111111111111111111111111111112',
        mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
      })
    ).toBe('DHe62eeQVEnNK7vg5xUpDkJm7tuqHadjhvmPRFBG9UPo');
  });

  it('uses the token program in the PDA seeds', () => {
    const owner = 'So11111111111111111111111111111111111111112';
    const mint = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';

    expect(
      deriveAssociatedTokenAccount({
        owner,
        mint,
        tokenProgram: SPL_TOKEN_PROGRAM_ADDRESS,
      })
    ).not.toBe(
      deriveAssociatedTokenAccount({
        owner,
        mint,
        tokenProgram: TOKEN_2022_PROGRAM_ADDRESS,
      })
    );
  });
});

describe('resolveTokenProgramAddress', () => {
  it('returns an explicit override without reading the mint', async () => {
    const chain = { mint: async () => { throw new Error('unexpected read'); } } as never;
    await expect(resolveTokenProgramAddress(chain, 'mint', 'custom-program')).resolves.toBe(
      'custom-program'
    );
  });

  it.each([SPL_TOKEN_PROGRAM_ADDRESS, TOKEN_2022_PROGRAM_ADDRESS])(
    'infers supported mint owner %s',
    async (ownerProgram) => {
      const chain = { mint: async () => ({ ownerProgram }) } as never;
      await expect(resolveTokenProgramAddress(chain, 'mint')).resolves.toBe(ownerProgram);
    }
  );

  it('rejects missing mints and unsupported inferred owners', async () => {
    await expect(
      resolveTokenProgramAddress({ mint: async () => null } as never, 'missing')
    ).rejects.toThrow('Mint account not found');
    await expect(
      resolveTokenProgramAddress(
        { mint: async () => ({ ownerProgram: 'unsupported' }) } as never,
        'mint'
      )
    ).rejects.toThrow('unsupported token program');
  });
});
