import {
  createWalletAdapter,
  toTransactionInstruction,
} from '@usearete/adapter-web3js';
import { PublicKey } from '@solana/web3.js';

globalThis.Buffer = undefined;

const instruction = toTransactionInstruction({
  programId: '11111111111111111111111111111111',
  keys: [],
  data: Uint8Array.from([1, 2, 3]),
});

if (instruction.data[2] !== 3) {
  throw new Error('Vite browser instruction conversion failed');
}

let networkCalls = 0;
let signingCalls = 0;
const publicKey = new PublicKey('11111111111111111111111111111111');
const wallet = createWalletAdapter({
  connection: new Proxy({}, {
    get() {
      networkCalls += 1;
      throw new Error('Vite smoke must not access the network');
    },
  }),
  signer: {
    publicKey,
    supportedTransactionVersions: new Set([0]),
    async signTransaction(transaction) {
      signingCalls += 1;
      return transaction;
    },
  },
});

if (wallet.publicKey !== publicKey.toBase58() || !wallet.signerAddresses?.includes(wallet.publicKey)) {
  throw new Error('Vite browser adapter instantiation failed');
}
if (networkCalls !== 0 || signingCalls !== 0) {
  throw new Error('Vite browser adapter instantiation prompted or accessed the network');
}

document.querySelector('#app').textContent = 'adapter-web3js browser smoke passed';
