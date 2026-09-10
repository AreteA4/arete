/**
 * Wallet Standard bridge tests.
 *
 * The wallet is a controlled fixture rather than a keypair: a local keypair
 * test proves nothing about the byte-oriented handoff, the capability checks
 * or the validation of what comes back. Real kit codecs on both sides.
 */

import { describe, expect, it, vi } from 'vitest';
import {
  appendTransactionMessageInstructions,
  compileTransaction,
  createKeyPairSignerFromPrivateKeyBytes,
  createTransactionMessage,
  getTransactionDecoder,
  getTransactionEncoder,
  setTransactionMessageConfig,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransaction,
  type Transaction,
} from '@solana/kit';
import {
  SOLANA_SIGN_AND_SEND_TRANSACTION,
  SOLANA_SIGN_TRANSACTION,
  WalletStandardSignerError,
  createWalletStandardSigner,
  type WalletStandardAccount,
  type WalletStandardSignTransactionInput,
  type WalletStandardTransactionVersion,
} from './wallet-standard';

const MEMO_PROGRAM = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const BLOCKHASH = { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 100n };

const keypair = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(1));

async function transactionOfVersion(version: 'legacy' | 0 | 1): Promise<Transaction> {
  const base = version === 1
    ? setTransactionMessageConfig(
      { computeUnitLimit: 1_000, loadedAccountsDataSizeLimit: 32_768 },
      createTransactionMessage({ version: 1 })
    )
    : version === 0
      ? createTransactionMessage({ version: 0 })
      : createTransactionMessage({ version: 'legacy' });
  const withPayer = setTransactionMessageFeePayerSigner(keypair, base);
  const withLifetime = setTransactionMessageLifetimeUsingBlockhash(
    BLOCKHASH as Parameters<typeof setTransactionMessageLifetimeUsingBlockhash>[0],
    withPayer
  );
  return compileTransaction(appendTransactionMessageInstructions(
    [{ programAddress: MEMO_PROGRAM, data: new Uint8Array([1, 2, 3]) }],
    withLifetime
  ));
}

interface FixtureOptions {
  supportedTransactionVersions?: readonly WalletStandardTransactionVersion[];
  features?: readonly string[];
  /** Replace the bytes the wallet returns, to exercise validation. */
  respond?: (input: WalletStandardSignTransactionInput) => Promise<Uint8Array>;
}

/** A wallet that really signs the bytes it is handed, with kit's codec. */
function walletFixture(options: FixtureOptions = {}) {
  const account: WalletStandardAccount = {
    address: keypair.address,
    features: options.features ?? [SOLANA_SIGN_TRANSACTION],
  };
  const signTransactionSpy = vi.fn(
    async (...inputs: readonly WalletStandardSignTransactionInput[]) => Promise.all(
      inputs.map(async (input) => ({
        signedTransaction: options.respond
          ? await options.respond(input)
          : getTransactionEncoder().encode(
            await signTransaction(
              [keypair.keyPair],
              getTransactionDecoder().decode(input.transaction)
            )
          ) as Uint8Array,
      }))
    )
  );
  const wallet = {
    features: {
      [SOLANA_SIGN_TRANSACTION]: {
        version: '1.0.0',
        supportedTransactionVersions:
          options.supportedTransactionVersions ?? (['legacy', 0, 1] as const),
        signTransaction: signTransactionSpy,
      },
    },
  };
  return { wallet, account, signTransactionSpy };
}

describe('createWalletStandardSigner', () => {
  it('refuses an account without the sign-transaction feature', () => {
    const { wallet, account } = walletFixture({
      features: [SOLANA_SIGN_AND_SEND_TRANSACTION],
    });

    expect(() => createWalletStandardSigner({ wallet, account })).toThrow(
      WalletStandardSignerError
    );
    // Sign-and-send is explicitly not evidence of sign-only support: it
    // cannot hand bytes back for Arete to relay.
    expect(() => createWalletStandardSigner({ wallet, account })).toThrow(
      /signAndSendTransaction/
    );
  });

  it('refuses a wallet that does not implement the feature it advertises', () => {
    const { account } = walletFixture();

    expect(() => createWalletStandardSigner({ wallet: { features: {} }, account }))
      .toThrow(/does not implement/);
  });

  it.each([['legacy'], [0], [1]] as const)(
    'hands version %s over as bytes and returns the wallet signature',
    async (version) => {
      const { wallet, account, signTransactionSpy } = walletFixture();
      const signer = createWalletStandardSigner({ wallet, account, chain: 'solana:devnet' });
      const transaction = await transactionOfVersion(version);

      const [signatures] = await signer.signTransactions([transaction]);

      const handedOver = signTransactionSpy.mock.calls[0][0];
      expect(handedOver.chain).toBe('solana:devnet');
      expect(handedOver.transaction).toBeInstanceOf(Uint8Array);
      // The wallet decodes the version out of the bytes; nothing converts it.
      expect(getTransactionDecoder().decode(handedOver.transaction).messageBytes)
        .toEqual(transaction.messageBytes);
      expect(signatures[keypair.address]).toBeInstanceOf(Uint8Array);
      expect(signatures[keypair.address]).not.toEqual(new Uint8Array(64));
    }
  );

  it('refuses a V1 transaction on a v0-only wallet before prompting', async () => {
    const { wallet, account, signTransactionSpy } = walletFixture({
      supportedTransactionVersions: ['legacy', 0],
    });
    const signer = createWalletStandardSigner({ wallet, account });

    await expect(signer.signTransactions([await transactionOfVersion(1)]))
      .rejects.toMatchObject({
        name: 'WalletStandardSignerError',
        code: 'transaction_version_unsupported',
      });
    expect(signTransactionSpy).not.toHaveBeenCalled();
  });

  it('still signs v0 on that same wallet', async () => {
    const { wallet, account, signTransactionSpy } = walletFixture({
      supportedTransactionVersions: ['legacy', 0],
    });
    const signer = createWalletStandardSigner({ wallet, account });

    await signer.signTransactions([await transactionOfVersion(0)]);

    expect(signTransactionSpy).toHaveBeenCalledTimes(1);
  });

  it('rejects a wallet that rewrote the message it was given', async () => {
    const other = await transactionOfVersion(0);
    const { wallet, account } = walletFixture({
      respond: async () => getTransactionEncoder().encode(other) as Uint8Array,
    });
    const signer = createWalletStandardSigner({ wallet, account });

    await expect(signer.signTransactions([await transactionOfVersion(1)]))
      .rejects.toMatchObject({ code: 'wallet_returned_invalid_transaction' });
  });

  it('rejects a wallet that returned no signature for the account', async () => {
    const { wallet, account } = walletFixture({
      respond: async (input) => input.transaction,
    });
    const signer = createWalletStandardSigner({ wallet, account });
    const transaction = await transactionOfVersion(1);

    await expect(signer.signTransactions([transaction])).rejects.toMatchObject({
      code: 'wallet_returned_invalid_transaction',
    });
  });

  it('rejects a wallet that answered a different number of transactions', async () => {
    const { account } = walletFixture();
    const wallet = {
      features: {
        [SOLANA_SIGN_TRANSACTION]: {
          version: '1.0.0',
          supportedTransactionVersions: ['legacy', 0, 1] as const,
          signTransaction: async () => [],
        },
      },
    };
    const signer = createWalletStandardSigner({ wallet, account });

    await expect(signer.signTransactions([await transactionOfVersion(1)]))
      .rejects.toMatchObject({ code: 'wallet_returned_invalid_transaction' });
  });

  it('never calls a disconnected wallet more than the caller asked', async () => {
    const rejection = new Error('User rejected the request');
    const { account } = walletFixture();
    const signTransaction = vi.fn(async () => {
      throw rejection;
    });
    const signer = createWalletStandardSigner({
      account,
      wallet: {
        features: {
          [SOLANA_SIGN_TRANSACTION]: {
            version: '1.0.0',
            supportedTransactionVersions: ['legacy', 0, 1] as const,
            signTransaction,
          },
        },
      },
    });

    await expect(signer.signTransactions([await transactionOfVersion(1)]))
      .rejects.toThrow(rejection);
    expect(signTransaction).toHaveBeenCalledTimes(1);
  });
});
