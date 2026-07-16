'use strict';

const {
  createWalletAdapter,
  toTransactionInstruction,
} = require('@usearete/adapter-web3js');
const { Keypair } = require('@solana/web3.js');

if (typeof createWalletAdapter !== 'function') {
  throw new Error('Packed CommonJS export is unavailable');
}

const instruction = toTransactionInstruction({
  programId: '11111111111111111111111111111111',
  keys: [],
  data: Uint8Array.from([1, 2, 3]),
});

if (instruction.data[2] !== 3) {
  throw new Error('Packed CommonJS instruction conversion failed');
}

async function main() {
  const keypair = Keypair.generate();
  let sendCalls = 0;
  let statusCalls = 0;
  const wallet = createWalletAdapter({
    connection: {
      async getLatestBlockhash() {
        return { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 1 };
      },
      async sendRawTransaction() {
        sendCalls += 1;
        throw new Error('uncertain packed CommonJS send');
      },
      async getSignatureStatuses(_signatures, config) {
        statusCalls += 1;
        if (config?.searchTransactionHistory !== true) {
          throw new Error('Packed CommonJS status lookup did not search transaction history');
        }
        return { context: { slot: 1 }, value: [null] };
      },
    },
    signer: {
      publicKey: keypair.publicKey,
      supportedTransactionVersions: new Set([0]),
      async signTransaction(transaction) {
        transaction.sign([keypair]);
        return transaction;
      },
    },
  });

  let outcome;
  try {
    await wallet.signAndSend([{
      programId: '11111111111111111111111111111111',
      keys: [{ pubkey: keypair.publicKey.toBase58(), isSigner: true, isWritable: false }],
      data: new Uint8Array(),
    }]);
  } catch (error) {
    outcome = error.outcome;
  }

  if (outcome?.status !== 'submitted-unknown' || !outcome.signature) {
    throw new Error('Packed CommonJS uncertainty outcome or bs58 signature is unavailable');
  }
  if (sendCalls !== 1 || statusCalls !== 1) {
    throw new Error('Packed CommonJS uncertainty handling retried submission or status lookup');
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
