import { describe, expect, it } from 'vitest';
import {
  Keypair,
  VersionedTransaction,
  type Connection,
  type Signer,
  type VersionedTransactionResponse,
} from '@solana/web3.js';
import type { BuiltInstruction } from '@usearete/sdk';

import {
  connectionAccountLoader,
  createWalletAdapter,
  type VersionedTransactionSigner,
} from './index';

const SYSTEM_PROGRAM = '11111111111111111111111111111111';

function hasSignature(signature: Uint8Array): boolean {
  return signature.some((byte) => byte !== 0);
}

function makeInstruction(signers: readonly string[]): BuiltInstruction {
  return {
    programId: SYSTEM_PROGRAM,
    keys: signers.map((pubkey, index) => ({
      pubkey,
      isSigner: true,
      isWritable: index === 0,
    })),
    data: new Uint8Array(),
  };
}

function createPrimarySigner(keypair: Keypair): VersionedTransactionSigner & { calls: number } {
  return {
    publicKey: keypair.publicKey,
    calls: 0,
    async signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction> {
      this.calls += 1;
      tx.sign([keypair]);
      return tx;
    },
  };
}

function createConnectionStub() {
  let sent: VersionedTransaction | null = null;
  let sendCalls = 0;

  const connection = {
    async getLatestBlockhash() {
      return { blockhash: SYSTEM_PROGRAM, lastValidBlockHeight: 123 };
    },
    async sendRawTransaction(raw: Buffer | Uint8Array) {
      sendCalls += 1;
      sent = VersionedTransaction.deserialize(Buffer.from(raw));
      return 'sig-web3js';
    },
    async confirmTransaction() {
      return {
        context: { slot: 456 },
        value: { err: null },
      } as VersionedTransactionResponse;
    },
  } as unknown as Connection;

  return {
    connection,
    getSent: () => sent,
    getSendCalls: () => sendCalls,
  };
}

describe('createWalletAdapter', () => {
  it('signs and sends with the primary signer by default', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    const result = await wallet.signAndSend([makeInstruction([primary.publicKey.toBase58()])]);

    expect(result).toEqual({ signature: 'sig-web3js', slot: 456 });
    expect(signer.calls).toBe(1);

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(1);
    expect(hasSignature(sent!.signatures[0]!)).toBe(true);
  });

  it('uses configured local signers for extra required signatures', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer, additionalSigners: [extra] });

    expect(wallet.signerAddresses).toEqual([
      primary.publicKey.toBase58(),
      extra.publicKey.toBase58(),
    ]);

    await wallet.signAndSend([
      makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()]),
    ]);

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('accepts per-send local signers', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await wallet.signAndSend(
      [makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()])],
      { additionalSigners: [extra] as readonly Signer[] }
    );

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('accepts standardized signers in send options', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await wallet.signAndSend(
      [makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()])],
      { signers: [extra] as readonly Signer[] }
    );

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('supports overriding the fee payer with a local signer', async () => {
    const primary = Keypair.generate();
    const feePayer = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer, additionalSigners: [feePayer] });

    await wallet.signAndSend([makeInstruction([])], { feePayer });

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.staticAccountKeys[0]!.toBase58()).toBe(feePayer.publicKey.toBase58());
    expect(signer.calls).toBe(0);
    expect(hasSignature(sent!.signatures[0]!)).toBe(true);
  });

  it('fails fast when a required signer cannot be satisfied', async () => {
    const primary = Keypair.generate();
    const missing = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await expect(
      wallet.signAndSend([
        makeInstruction([primary.publicKey.toBase58(), missing.publicKey.toBase58()]),
      ])
    ).rejects.toThrow(/Missing signer\(s\) for transaction/);
    expect(getSendCalls()).toBe(0);
  });
});

describe('connectionAccountLoader', () => {
  it('adapts a web3.js connection to the AccountLoader interface', async () => {
    const address = Keypair.generate().publicKey.toBase58();
    const getAccountInfo = async (publicKey: PublicKey) => {
      expect(publicKey.toBase58()).toBe(address);
      return { data: Buffer.from([1, 2, 3]) };
    };

    const loader = connectionAccountLoader({ getAccountInfo } as unknown as Connection);
    await expect(loader.getAccount(address)).resolves.toEqual({
      data: Uint8Array.from([1, 2, 3]),
    });
  });

  it('returns null when the connection misses', async () => {
    const address = Keypair.generate().publicKey.toBase58();
    const loader = connectionAccountLoader({
      async getAccountInfo() {
        return null;
      },
    } as unknown as Connection);

    await expect(loader.getAccount(address)).resolves.toBeNull();
  });
});

describe('instruction converters', () => {
  it('round-trips BuiltInstruction through TransactionInstruction', async () => {
    const { toTransactionInstruction, fromTransactionInstruction } = await import('./index');
    const signer = Keypair.generate().publicKey.toBase58();
    const writable = Keypair.generate().publicKey.toBase58();
    const original: BuiltInstruction = {
      programId: SYSTEM_PROGRAM,
      keys: [
        { pubkey: signer, isSigner: true, isWritable: false },
        { pubkey: writable, isSigner: false, isWritable: true },
      ],
      data: new Uint8Array([1, 2, 3, 255]),
    };

    const web3Instruction = toTransactionInstruction(original);
    expect(web3Instruction.programId.toBase58()).toBe(SYSTEM_PROGRAM);
    expect(web3Instruction.keys[0]!.pubkey.toBase58()).toBe(signer);
    expect(web3Instruction.keys[0]!.isSigner).toBe(true);

    const roundTripped = fromTransactionInstruction(web3Instruction);
    expect(roundTripped).toEqual(original);
  });
});
