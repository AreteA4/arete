import { describe, expect, it, vi } from 'vitest';
import {
  createPreparedFlow,
  executePreparedOperation,
  OperationExecutionError,
} from './operations';

describe('executePreparedOperation', () => {
  it('does not let an opaque signer hide a missing required signer', async () => {
    const transaction = vi.fn(async () => ({ signature: 'signature' }));
    const operation = createPreparedFlow({
      name: 'test-flow',
      artifacts: undefined,
      transactions: [
        {
          name: 'test-transaction',
          instructions: [
            {
              programId: 'program',
              keys: [],
              data: new Uint8Array(),
            },
          ],
          requiredSignerAddresses: ['wallet', 'missing'],
          errors: [],
        },
      ],
    });

    const result = executePreparedOperation(
      { publicKey: 'wallet', transaction },
      operation,
      { signers: [{ opaqueSigner: true }] }
    );

    await expect(result).rejects.toMatchObject({
      cause: new Error('Missing signer(s) for test-transaction: missing'),
    } satisfies Partial<OperationExecutionError>);
    expect(transaction).not.toHaveBeenCalled();
  });
});
