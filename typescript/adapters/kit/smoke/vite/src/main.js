import {
  createWalletAdapter,
  fromKitInstruction,
  toKitInstruction,
} from '@usearete/adapter-kit';

const systemAddress = '11111111111111111111111111111111';
const feePayer = 'mpngsFd4tmbUfzDYJayjKZwZcaR7aWb2793J6grLsGu';
const builtInstruction = {
  programId: systemAddress,
  keys: [],
  data: Uint8Array.from([1, 2, 3]),
};
const roundTripped = fromKitInstruction(toKitInstruction(builtInstruction));
if (roundTripped.data[2] !== 3) {
  throw new Error('Vite browser instruction conversion failed');
}

let signingCalls = 0;
let subscriptionCalls = 0;
const rpc = {
  getLatestBlockhash() {
    return {
      async send() {
        return {
          context: { slot: 10n },
          value: { blockhash: systemAddress, lastValidBlockHeight: 100n },
        };
      },
    };
  },
  getFeeForMessage() {
    return {
      async send() {
        return { context: { slot: 11n }, value: 5_000n };
      },
    };
  },
  simulateTransaction(_transaction, config) {
    if (config.sigVerify !== false || config.encoding !== 'base64') {
      throw new Error('Vite browser inspection did not remain unsigned');
    }
    return {
      async send() {
        return {
          context: { slot: 12n },
          value: { err: null, logs: ['Program log: inspected'], unitsConsumed: 200n },
        };
      },
    };
  },
};
const wallet = createWalletAdapter({
  rpc,
  rpcSubscriptions: new Proxy({}, {
    get() {
      subscriptionCalls += 1;
      throw new Error('Vite smoke must not open a subscription');
    },
  }),
  signer: {
    address: feePayer,
    async signTransactions() {
      signingCalls += 1;
      throw new Error('Vite smoke must not sign');
    },
  },
});

wallet.inspectTransaction([builtInstruction]).then((inspection) => {
  if (
    inspection.feeLamports !== 5_000
    || inspection.contextSlot !== 12
    || inspection.computeUnitsConsumed !== 200
  ) {
    throw new Error('Vite browser unsigned inspection failed');
  }
  if (wallet.publicKey !== feePayer || signingCalls !== 0 || subscriptionCalls !== 0) {
    throw new Error('Vite browser adapter inspection prompted or opened a subscription');
  }

  document.querySelector('#app').textContent = 'adapter-kit browser smoke passed';
});
