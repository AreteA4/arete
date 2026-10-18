import { describe, expect, it } from "vitest";

import { tipJar } from "../fixtures";
import { buildArgs, draftArgs, fieldInputs, initialValues, suggestedWallet } from "./args";
import type { ObjectSchema } from "../types";

const schema: ObjectSchema = {
  type: "object",
  properties: {
    recipient: { type: "string", format: "pubkey" },
    amount: { type: "string", format: "u64" },
    offset: { type: "string", format: "i64" },
    count: { type: "integer", minimum: 1, maximum: 9 },
    side: { type: "string", enum: ["bid", "ask"] },
    flag: { type: "boolean" },
    tags: { type: "array", items: { type: "string" } },
    memo: { type: "string", maxLength: 4 },
  },
  required: ["recipient", "amount"],
};

describe("step args", () => {
  it("maps formats to inputs", () => {
    expect(fieldInputs(schema).map((input) => [input.name, input.kind, input.required])).toEqual([
      ["recipient", "pubkey", true],
      ["amount", "integer", true],
      ["offset", "integer", false],
      ["count", "number", false],
      ["side", "enum", false],
      ["flag", "boolean", false],
      ["tags", "json", false],
      ["memo", "string", false],
    ]);
    expect(initialValues(schema)).toMatchObject({ side: "bid", flag: "false", amount: "" });
  });

  it("builds args and reports errors per field", () => {
    const good = buildArgs(schema, {
      recipient: "11111111111111111111111111111111",
      amount: "1000",
      offset: "-5",
      count: "3",
      side: "ask",
      flag: "true",
      tags: '["a"]',
      memo: "hi",
    });
    expect(good.errors).toEqual({});
    expect(good.args).toEqual({
      recipient: "11111111111111111111111111111111",
      amount: "1000",
      offset: "-5",
      count: 3,
      side: "ask",
      flag: true,
      tags: ["a"],
      memo: "hi",
    });
    const bad = buildArgs(schema, { recipient: "nope", amount: "-1", count: "10", tags: "{}", memo: "toolong", flag: "false" });
    expect(bad.errors).toEqual({
      recipient: "not a base58 public key",
      amount: "a non-negative integer",
      count: "at most 9",
      tags: "a JSON array",
      memo: "at most 4 characters",
    });
    expect(buildArgs(schema, { flag: "false" }).errors).toMatchObject({ recipient: "required", amount: "required" });
    // Optional empty fields are left out.
    expect(buildArgs(tipJar.steps.close!.args, { amount: "5" }).args).toEqual({ amount: "5" });
  });

  it("suggests the role holder's dev wallet", () => {
    const wallets = [
      { name: "creator", pubkey: "C" },
      { name: "recipient", pubkey: "R" },
    ];
    const instance = { roles: { recipient: "R" }, accounts: { creator: "C" } };
    expect(suggestedWallet({ kind: "role", role: "recipient" }, instance, wallets)).toBe("recipient");
    expect(suggestedWallet({ kind: "creator" }, instance, wallets)).toBe("creator");
    expect(suggestedWallet({ kind: "keeper" }, instance, wallets)).toBeUndefined();
  });

  it("drafts args for the copied command from what the form holds", () => {
    // Nothing entered: required fields as placeholders, the enum and the
    // boolean at their starting values, optional empty fields left out.
    expect(draftArgs(schema, initialValues(schema))).toEqual({
      recipient: "<recipient>",
      amount: "<amount>",
      side: "bid",
      flag: false,
    });
    // Valid values as the step takes them; invalid ones as typed.
    expect(draftArgs(schema, { ...initialValues(schema), amount: "1000", count: "3", memo: "toolong" })).toEqual({
      recipient: "<recipient>",
      amount: "1000",
      count: 3,
      side: "bid",
      flag: false,
      memo: "toolong",
    });
  });
});
