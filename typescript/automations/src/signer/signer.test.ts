import { ed25519 } from '@noble/curves/ed25519';
import type { ProtocolTransactionFormat } from '@usearete/sdk';
import { describe, expect, it } from 'vitest';

import { keypairSigner } from '../keypair';
import { decodeTransaction } from '../policy/decode';
import { decodeBase64, encodeBase58, encodeBase64 } from '../policy/encoding';
import { intentFor, prepared, type WireMessage, wire } from '../test-support/wire';
import { negotiateTransactionFormat } from './format';
import type { ProtocolSigner } from './signer';
import { checkPreparedAgainstIntent, checkSignedTransactions } from './verify';

const SEED = Uint8Array.from({ length: 32 }, (_, index) => index + 1);
const WALLET = encodeBase58(ed25519.getPublicKey(SEED));
const OTHER_SEED = Uint8Array.from({ length: 32 }, (_, index) => 200 - index);
const OTHER_SIGNER = encodeBase58(ed25519.getPublicKey(OTHER_SEED));
const PROGRAM = encodeBase58(new Uint8Array(32).fill(40));
const VAULT = encodeBase58(new Uint8Array(32).fill(30));
const COMPUTE_BUDGET = 'ComputeBudget111111111111111111111111111111';

/** The wallet signs and pays; the vault is written; the program is invoked. */
function deposit(extra: WireMessage['instructions'] = [], keys: string[] = []): WireMessage {
  return {
    header: [1, 0, 1 + keys.length],
    keys: [WALLET, VAULT, PROGRAM, ...keys],
    instructions: [{ program: 2, accounts: [0, 1], data: [1, 0, 0, 0] }, ...extra],
  };
}

/** A fee payer that pre-signed (an ephemeral key), then the wallet. */
function coSigned(format: ProtocolTransactionFormat): Uint8Array {
  const bytes = wire(
    {
      header: [2, 0, 1],
      keys: [OTHER_SIGNER, WALLET, PROGRAM],
      instructions: [{ program: 2, accounts: [0, 1], data: [7] }],
    },
    format,
  );
  const decoded = decodeTransaction(bytes);
  const signature = ed25519.sign(bytes.subarray(decoded.message.start, decoded.message.end), OTHER_SEED);
  bytes.set(signature, decoded.signatureOffset);
  return bytes;
}

function setup(format: ProtocolTransactionFormat, transactions = [wire(deposit(), format)]) {
  return { intent: intentFor(WALLET, format, transactions), transactions: prepared(transactions) };
}

/** A signer that signs something other than the prepared bytes. */
function rewritingSigner(rewrite: (format: ProtocolTransactionFormat) => Uint8Array): ProtocolSigner {
  const inner = keypairSigner(SEED);
  return {
    ...inner,
    kind: 'rewriting',
    async signTransactions(input) {
      return inner.signTransactions({
        ...input,
        transactions: input.transactions.map((transaction) => ({
          index: transaction.index,
          transactionBase64: encodeBase64(rewrite(input.intent.transactionFormat)),
        })),
      });
    },
  };
}

describe('keypairSigner', () => {
  for (const format of ['v1', 'v0'] as const) {
    it(`fills only the wallet's slot of a ${format} transaction`, async () => {
      const transactions = [wire(deposit(), format), coSigned(format)];
      const { intent, transactions: input } = setup(format, transactions);
      const signer = keypairSigner(SEED);
      const signed = await signer.signTransactions({ wallet: WALLET, intent, transactions: input });
      expect(checkSignedTransactions({ wallet: WALLET, intent, prepared: input, signed })).toBeNull();
      const second = decodeTransaction(decodeBase64(signed[1]!)!);
      const before = decodeTransaction(transactions[1]!);
      expect(second.signatures[0]).toEqual(before.signatures[0]);
      expect(second.signatures[1]!.some((byte) => byte !== 0)).toBe(true);
    });
  }

  it('accepts a Solana CLI secret key and checks its public key', () => {
    const cli = [...SEED, ...ed25519.getPublicKey(SEED)];
    expect(keypairSigner(cli).publicKey).toBe(WALLET);
    const wrong = [...SEED, ...ed25519.getPublicKey(OTHER_SEED)];
    expect(() => keypairSigner(wrong)).toThrow('does not match');
    expect(() => keypairSigner([1, 2, 3])).toThrow('64 bytes');
  });

  it('refuses to sign for another wallet and never exposes the key', async () => {
    const signer = keypairSigner(SEED);
    expect(await signer.canSign(WALLET)).toBe(true);
    expect(await signer.canSign(OTHER_SIGNER)).toBe(false);
    const { intent, transactions } = setup('v1');
    await expect(
      signer.signTransactions({ wallet: OTHER_SIGNER, intent, transactions }),
    ).rejects.toThrow(`signs for ${WALLET}`);
    const shown = `${JSON.stringify(signer)} ${String(signer)}`;
    expect(shown).not.toContain(SEED.join(','));
    expect(shown).not.toContain(encodeBase58(SEED));
  });
});

describe('checkSignedTransactions', () => {
  it('refuses a signer that adds a compute-budget instruction', async () => {
    const { intent, transactions } = setup('v1');
    const signer = rewritingSigner((format) =>
      wire(deposit([{ program: 3, accounts: [], data: [2, 0x40, 0x0d, 0x03, 0x00] }], [COMPUTE_BUDGET]), format),
    );
    const signed = await signer.signTransactions({ wallet: WALLET, intent, transactions });
    expect(checkSignedTransactions({ wallet: WALLET, intent, prepared: transactions, signed })).toEqual({
      rule: 'signer_modified',
      reason: 'the signer changed the message of transaction 0; its hash no longer matches the intent',
    });
  });

  it("refuses a signer that changes another signer's signature", async () => {
    const transactions = [coSigned('v0')];
    const { intent, transactions: input } = setup('v0', transactions);
    const signed = await keypairSigner(SEED).signTransactions({ wallet: WALLET, intent, transactions: input });
    const bytes = decodeBase64(signed[0]!)!;
    bytes[1] = bytes[1]! ^ 0xff; // the fee payer's signature
    const refusal = checkSignedTransactions({
      wallet: WALLET,
      intent,
      prepared: input,
      signed: [encodeBase64(bytes)],
    });
    expect(refusal?.rule).toBe('signer_modified');
    expect(refusal?.reason).toContain("signature 0 of transaction 0, which is not the wallet's");
  });

  it('refuses the same message and signatures under another encoding', async () => {
    const transactions = [coSigned('v0')];
    const { intent, transactions: input } = setup('v0', transactions);
    const [signed] = await keypairSigner(SEED).signTransactions({ wallet: WALLET, intent, transactions: input });
    // Two signatures, with the count written as a two-byte compact-u16 (0x82 0x00).
    const bytes = decodeBase64(signed!)!;
    const reencoded = new Uint8Array(bytes.length + 1);
    reencoded.set([0x82, 0x00]);
    reencoded.set(bytes.subarray(1), 2);
    expect(decodeTransaction(reencoded).signatures).toHaveLength(2);
    expect(
      checkSignedTransactions({ wallet: WALLET, intent, prepared: input, signed: [encodeBase64(reencoded)] }),
    ).toEqual({ rule: 'signer_modified', reason: "the signer changed bytes of transaction 0 outside the wallet's signature" });
  });

  it('refuses a missing, foreign or corrupted wallet signature', async () => {
    const { intent, transactions } = setup('v1');
    const unsigned = [transactions[0]!.transactionBase64];
    expect(checkSignedTransactions({ wallet: WALLET, intent, prepared: transactions, signed: unsigned })?.rule).toBe(
      'signer_signature',
    );
    const bytes = decodeBase64(transactions[0]!.transactionBase64)!;
    const decoded = decodeTransaction(bytes);
    bytes.set(ed25519.sign(bytes.subarray(decoded.message.start, decoded.message.end), OTHER_SEED), decoded.signatureOffset);
    expect(
      checkSignedTransactions({ wallet: WALLET, intent, prepared: transactions, signed: [encodeBase64(bytes)] })?.rule,
    ).toBe('signer_signature');
  });

  it('refuses a different number of transactions', async () => {
    const { intent, transactions } = setup('v1');
    expect(checkSignedTransactions({ wallet: WALLET, intent, prepared: transactions, signed: [] })?.rule).toBe(
      'signer_modified',
    );
  });
});

describe('checkPreparedAgainstIntent', () => {
  it('accepts what the intent committed to', () => {
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v1', ...setup('v1') })).toBeNull();
  });

  it('refuses bytes the intent did not commit to', () => {
    const { intent } = setup('v1');
    const other = prepared([wire(deposit([{ program: 2, accounts: [], data: [9] }]), 'v1')]);
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v1', intent, transactions: other })?.rule).toBe(
      'intent_mismatch',
    );
  });

  it('refuses legacy bytes for a v0 intent', () => {
    const legacy = Uint8Array.from(wire(deposit(), 'v0'));
    // Drop the v0 prefix byte and the trailing lookup-table count: the same message as legacy.
    const bytes = new Uint8Array([...legacy.subarray(0, 65), ...legacy.subarray(66, legacy.length - 1)]);
    expect(decodeTransaction(bytes).version).toBeNull();
    const intent = intentFor(WALLET, 'v0', [bytes]);
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v0', intent, transactions: prepared([bytes]) })?.rule).toBe(
      'transaction_format',
    );
  });

  it('refuses another format, another wallet, and a wallet that does not sign', () => {
    const v0 = setup('v0');
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v1', ...v0 })?.rule).toBe('transaction_format');
    const lying = { ...v0, intent: { ...v0.intent, transactionFormat: 'v1' as const } };
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v1', ...lying })?.rule).toBe('transaction_format');
    expect(checkPreparedAgainstIntent({ wallet: OTHER_SIGNER, format: 'v0', ...v0 })?.rule).toBe('intent_mismatch');
    const notSigner = [wire({ ...deposit(), keys: [OTHER_SIGNER, VAULT, PROGRAM] }, 'v0')];
    const foreign = { intent: intentFor(WALLET, 'v0', notSigner), transactions: prepared(notSigner) };
    expect(checkPreparedAgainstIntent({ wallet: WALLET, format: 'v0', ...foreign })?.reason).toContain(
      'is not a required signer',
    );
  });
});

describe('transaction format negotiation', () => {
  /** A deployment that prepares the deposit in whichever format it is asked for. */
  const deployment = {
    prepare(format: ProtocolTransactionFormat) {
      return setup(format, [wire(deposit(), format)]);
    },
  };

  async function run(signer: ProtocolSigner, preferred?: ProtocolTransactionFormat[]) {
    const format = negotiateTransactionFormat(signer.supportedTransactionFormats, preferred);
    if (format === null) return null;
    const { intent, transactions } = deployment.prepare(format);
    expect(checkPreparedAgainstIntent({ wallet: WALLET, intent, transactions, format })).toBeNull();
    const signed = await signer.signTransactions({ wallet: WALLET, intent, transactions });
    expect(checkSignedTransactions({ wallet: WALLET, intent, prepared: transactions, signed })).toBeNull();
    return decodeTransaction(decodeBase64(signed[0]!)!).version === 1 ? 'v1' : 'v0';
  }

  it('prepares v1 when the signer supports it', async () => {
    expect(await run(keypairSigner(SEED))).toBe('v1');
  });

  it('falls back to v0 for a signer that only signs v0', async () => {
    expect(await run(keypairSigner(SEED, { transactionFormats: ['v0'] }))).toBe('v0');
  });

  it("follows the deployment's preference", async () => {
    expect(await run(keypairSigner(SEED), ['v0', 'v1'])).toBe('v0');
  });

  it('finds no format when they share none', async () => {
    expect(await run(keypairSigner(SEED, { transactionFormats: ['v0'] }), ['v1'])).toBeNull();
  });
});
