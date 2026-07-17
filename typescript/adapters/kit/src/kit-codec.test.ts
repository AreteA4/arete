import { describe, expect, it } from 'vitest';
import {
  address,
  appendTransactionMessageInstructions,
  compileTransaction,
  createTransactionMessage,
  getBase64Decoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
} from '@solana/kit';

describe('@solana/kit 2.3 message encoding', () => {
  it('decodes compiled message bytes to the base64 value required by getFeeForMessage', () => {
    const systemAddress = address('11111111111111111111111111111111');
    const feePayer = address('mpngsFd4tmbUfzDYJayjKZwZcaR7aWb2793J6grLsGu');
    const message = pipe(
      createTransactionMessage({ version: 0 }),
      (transactionMessage) => setTransactionMessageFeePayer(feePayer, transactionMessage),
      (transactionMessage) => setTransactionMessageLifetimeUsingBlockhash({
        blockhash: systemAddress,
        lastValidBlockHeight: 1n,
      }, transactionMessage),
      (transactionMessage) => appendTransactionMessageInstructions([{
        programAddress: systemAddress,
        data: new Uint8Array(),
      }], transactionMessage)
    );
    const transaction = compileTransaction(message);

    const encodedMessage = getBase64Decoder().decode(transaction.messageBytes);

    expect(Buffer.from(encodedMessage, 'base64')).toEqual(
      Buffer.from(transaction.messageBytes)
    );
  });
});
