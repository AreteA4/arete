import {
  buildInstruction,
  createPreparedInstruction,
  defineProgramExtensions,
  instructionOperation,
} from '@usearete/sdk';
import { depositInstruction, type VAULT } from './vault-core.js';

interface DepositToTreasuryInput {
  authority: string;
  mint: string;
  amount: bigint;
  signers?: readonly string[];
}

const TREASURY_ADDRESS = 'Treasury11111111111111111111111111111111111';

export default defineProgramExtensions<typeof VAULT>()({
  addresses: {
    treasury: () => TREASURY_ADDRESS,
  },
  createOperations: () => ({
    instructions: {
      treasury: {
        deposit: instructionOperation(async (input: DepositToTreasuryInput) => {
          const instruction = buildInstruction(depositInstruction, {
            authority: input.authority,
            vault: TREASURY_ADDRESS,
            mint: input.mint,
            amount: input.amount,
          });
          return createPreparedInstruction({
            name: 'treasury.deposit',
            instruction,
            artifacts: { instruction },
            signers: input.signers,
          });
        }),
      },
    },
  }),
});
