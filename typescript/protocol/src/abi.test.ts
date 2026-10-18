import { describe, expect, it } from "vitest";

import { PROTOCOL_HANDLER_ABI_V1, wirePreparedOperation, type WireStepV1 } from "./index.js";

const WALLET = "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj";
const EPHEMERAL = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
const SYSTEM = "11111111111111111111111111111111";
const SQUADS = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";
const OTHER = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

const step: WireStepV1 = {
  programs: ["system"],
  input: { wallet: WALLET, ephemeral: { mint: EPHEMERAL }, programs: { system: SYSTEM, squadsV4: SQUADS } },
};

const ix = (overrides: Record<string, unknown> = {}) => ({
  programId: SYSTEM,
  keys: [
    { pubkey: WALLET, isSigner: true, isWritable: true },
    { pubkey: EPHEMERAL, isSigner: true, isWritable: true },
    { pubkey: OTHER, isSigner: false, isWritable: false },
  ],
  data: Uint8Array.from([1, 2, 3]),
  ...overrides,
});

describe("wirePreparedOperation", () => {
  it("wires core-SDK instructions to the handler ABI", () => {
    expect(wirePreparedOperation([{ name: "create", instructions: [ix()] }], step)).toEqual({
      abi: PROTOCOL_HANDLER_ABI_V1,
      transactions: [
        {
          name: "create",
          instructions: [
            {
              programId: SYSTEM,
              keys: [
                { pubkey: WALLET, isSigner: true, isWritable: true },
                { pubkey: EPHEMERAL, isSigner: true, isWritable: true },
                { pubkey: OTHER, isSigner: false, isWritable: false },
              ],
              dataBase64: "AQID",
            },
          ],
        },
      ],
    });
    // A core-SDK flow plan is accepted too.
    expect(wirePreparedOperation({ plan: { transactions: [{ name: "create", instructions: [ix()] }] } }, step).transactions).toHaveLength(1);
  });

  const cases: Array<[string, unknown, string]> = [
    ["signers", [{ name: "a", instructions: [ix()], signers: [] }], "signers-not-allowed"],
    ["empty flow", [], "empty-operation"],
    ["empty transaction", [{ name: "a", instructions: [] }], "empty-transaction"],
    ["undeclared program", [{ name: "a", instructions: [ix({ programId: SQUADS })] }], "undeclared-program"],
    [
      "foreign signer",
      [{ name: "a", instructions: [ix({ keys: [{ pubkey: OTHER, isSigner: true, isWritable: false }] })] }],
      "undeclared-signer",
    ],
    ["too many transactions", Array.from({ length: 9 }, () => ({ name: "a", instructions: [ix()] })), "too-many-transactions"],
    ["too many instructions", [{ name: "a", instructions: Array.from({ length: 65 }, () => ix()) }], "too-many-instructions"],
    [
      "too many accounts",
      [{ name: "a", instructions: [ix({ keys: Array.from({ length: 257 }, () => ({ pubkey: OTHER, isSigner: false, isWritable: false })) })] }],
      "too-many-accounts",
    ],
    ["data too large", [{ name: "a", instructions: [ix({ data: new Uint8Array(10 * 1024 + 1) })] }], "instruction-data-too-large"],
  ];
  for (const [name, operation, code] of cases) {
    it(`rejects ${name}`, () => {
      expect(() => wirePreparedOperation(operation as never, step)).toThrow(expect.objectContaining({ code }));
    });
  }

  it("rejects a step program missing from the invocation", () => {
    expect(() => wirePreparedOperation([{ name: "a", instructions: [ix()] }], { ...step, programs: ["jupiter"] })).toThrow(
      expect.objectContaining({ code: "undeclared-program" }),
    );
  });

  it("reports contract shape errors from the validator", () => {
    expect(() => wirePreparedOperation([{ name: "not-an-identifier", instructions: [ix()] }], step)).toThrow(
      expect.objectContaining({ code: "invalid-identifier", path: "/transactions/0/name" }),
    );
  });
});
