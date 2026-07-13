export interface OreCloseParams {
  board?: string;
  rentPayer: string;
  round: string;
  treasury?: string;
}

export type OreCloseError = OreStreamOreProgramError;

/**
 * Closes an expired round account and returns rent to the payer.
 * Round PDA seeds: ["round", round_id].
 * Treasury PDA seeds: ["treasury"].
 */
export const oreCloseInstruction = createInstructionHandler<OreCloseParams, OreCloseError>({
  programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
  discriminator: [5],
  args: [],
  accounts: [
    { name: 'signer', isSigner: true, isWritable: true, category: 'signer' },
    { name: 'board', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'board' }] } },
    { name: 'rentPayer', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'round', isSigner: false, isWritable: true, category: 'userProvided' },
    { name: 'treasury', isSigner: false, isWritable: true, category: 'pda', pdaConfig: { seeds: [{ type: 'literal', value: 'treasury' }] } },
    { name: 'systemProgram', isSigner: false, isWritable: false, category: 'known', knownAddress: '11111111111111111111111111111111' },
  ],
  errors: ORE_STREAM_ORE_PROGRAM_ERRORS,
});
