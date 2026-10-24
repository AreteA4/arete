import {
  ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
  SPL_TOKEN_PROGRAM_ADDRESS,
  SYSTEM_PROGRAM_ADDRESS,
  buildInstruction,
  createPreparedFlow,
  createPreparedInstruction,
  createPreparedTransaction,
  createPreparedTransactionBody,
  createPublicKeySeed,
  createSeed,
  decodeBase58,
  defineProgramExtensions,
  deriveAssociatedTokenAccount,
  encodeBase58,
  findProgramAddressSync,
  flowOperation,
  instructionOperation,
  transactionOperation,
  type AmountInput,
  type BuiltInstruction,
  type InstructionHandler,
  type PreparedInstruction,
} from '@usearete/sdk/program';

import { pingInstruction, type VOTE_PROGRAM } from './vote-core.js';

// A curated extension shaped like a registry program's: helpers and
// operations built only from `@usearete/sdk/program`, so a handler that
// imports the program module bundles.

export { ASSOCIATED_TOKEN_PROGRAM_ADDRESS, SPL_TOKEN_PROGRAM_ADDRESS, SYSTEM_PROGRAM_ADDRESS };

export interface TippedPingInput {
  payer: string;
  mint: string;
  amount: AmountInput;
}

export function tipAccount(payer: string, mint: string): string {
  return deriveAssociatedTokenAccount({ owner: payer, mint, tokenProgram: SPL_TOKEN_PROGRAM_ADDRESS });
}

export function ballotAddress(payer: string): string {
  const [address] = findProgramAddressSync(
    [createSeed('ballot'), createPublicKeySeed(payer)],
    'Vote111111111111111111111111111111111111111',
  );
  return encodeBase58(decodeBase58(address));
}

function ping(payer: string): BuiltInstruction {
  const handler: InstructionHandler<unknown, unknown> = pingInstruction as InstructionHandler<unknown, unknown>;
  return buildInstruction(handler, { payer });
}

function preparedPing(payer: string): PreparedInstruction<{ instruction: BuiltInstruction }> {
  const instruction = ping(payer);
  return createPreparedInstruction({ name: 'ping', instruction, artifacts: { instruction } });
}

export default defineProgramExtensions<typeof VOTE_PROGRAM>()({
  createOperations: () => ({
    instructions: {
      tipped: {
        ping: instructionOperation(async (input: TippedPingInput) => preparedPing(input.payer)),
      },
    },
    transactions: {
      tipped: {
        ping: transactionOperation(async (input: TippedPingInput) =>
          createPreparedTransaction({
            name: 'tipped.ping',
            instructions: [preparedPing(input.payer)],
            artifacts: { tipAccount: tipAccount(input.payer, input.mint) },
          }),
        ),
      },
    },
    flows: {
      tipped: {
        ping: flowOperation(async (input: TippedPingInput) =>
          createPreparedFlow({
            name: 'tipped.ping',
            transactions: [createPreparedTransactionBody({ name: 'tipped.ping', instructions: [ping(input.payer)] })],
            artifacts: { ballot: ballotAddress(input.payer) },
          }),
        ),
      },
    },
  }),
});
