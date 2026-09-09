import * as k from '@solana/kit';

// Fixed test-only seeds. Never used on any cluster.
const seed = (n) => new Uint8Array(32).fill(n);

const kp = async (n) => await k.createKeyPairFromPrivateKeyBytes(seed(n));
const addrOf = async (pair) => await k.getAddressFromPublicKey(pair.publicKey);

const payer = await kp(1);
const co = await kp(2);
const payerAddr = await addrOf(payer);
const coAddr = await addrOf(co);

const BLOCKHASH = { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 100n };
const MEMO = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';

function memoIx(bytes, signers = []) {
  return {
    programAddress: MEMO,
    accounts: signers.map((a) => ({ address: a, role: 3 })), // writable signer
    data: new Uint8Array(bytes),
  };
}

async function build({ version, instructions, signers }) {
  let m = k.createTransactionMessage({ version });
  m = k.setTransactionMessageFeePayer(payerAddr, m);
  m = k.setTransactionMessageLifetimeUsingBlockhash(BLOCKHASH, m);
  for (const ix of instructions) m = k.appendTransactionMessageInstruction(ix, m);
  const compiled = k.compileTransaction(m);
  const signed = await k.signTransaction(signers, compiled);
  const wire = k.getTransactionEncoder().encode(signed);
  return { signed, wire: new Uint8Array(wire) };
}

const out = {};
const b64 = (u8) => Buffer.from(u8).toString('base64');

for (const [name, version] of [['legacy', 'legacy'], ['v0', 0], ['v1', 1]]) {
  const { signed, wire } = await build({
    version,
    instructions: [memoIx([1, 2, 3])],
    signers: [payer],
  });
  out[name] = {
    version: name === 'legacy' ? 'legacy' : name === 'v0' ? 0 : 1,
    signatureCount: Object.keys(signed.signatures).length,
    firstSignature: Object.values(signed.signatures)[0]
      ? k.getBase58Decoder().decode(Object.values(signed.signatures)[0])
      : null,
    bytes: wire.length,
    base64: b64(wire),
  };
}

// A V1 payload past the legacy/v0 1232-byte ceiling.
const big = await build({
  version: 1,
  instructions: [memoIx(new Array(1400).fill(7))],
  signers: [payer],
});
out.v1_oversize = {
  version: 1,
  signatureCount: Object.keys(big.signed.signatures).length,
  firstSignature: k.getBase58Decoder().decode(Object.values(big.signed.signatures)[0]),
  bytes: big.wire.length,
  base64: b64(big.wire),
};

// Two required signatures, both present.
const multi = await build({
  version: 1,
  instructions: [memoIx([9], [payerAddr, coAddr])],
  signers: [payer, co],
});
out.v1_two_signatures = {
  version: 1,
  signatureCount: Object.keys(multi.signed.signatures).length,
  firstSignature: k.getBase58Decoder().decode(Object.values(multi.signed.signatures)[0]),
  bytes: multi.wire.length,
  base64: b64(multi.wire),
};

console.log(JSON.stringify({ payer: payerAddr, cosigner: coAddr, fixtures: out }, null, 2));
