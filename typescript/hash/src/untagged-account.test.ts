import { expect, test } from "vitest";
import { parseIdlV1 } from "./index.js";

test.each([
  { name: "explicit marker", docs: ["arete.account_untagged=true"], bytes: [], expected: [] },
  { name: "trimmed marker", docs: ["  arete.account_untagged=true\n"], bytes: [], expected: [] },
  { name: "omitted bytes", docs: ["arete.account_untagged=true"], bytes: undefined, expected: [] },
  { name: "explicit bytes override marker", docs: ["arete.account_untagged=true"], bytes: [9], expected: [9] },
  { name: "unmarked empty bytes", docs: [], bytes: [], expected: undefined },
  { name: "false marker", docs: ["arete.account_untagged=false"], bytes: [], expected: undefined },
])("normalizes $name consistently with Rust", ({ docs, bytes, expected }) => {
  const document = parseIdlV1(new TextEncoder().encode(JSON.stringify({
    address: "11111111111111111111111111111111",
    name: "untagged_account",
    version: "0.1.0",
    instructions: [],
    accounts: [{ name: "Native", discriminator: bytes, docs }],
  })));
  const discriminator = document.normalizedSnapshot.accounts[0]!.discriminator;
  if (expected) expect(discriminator).toEqual(expected);
  else expect(discriminator).toHaveLength(8);
  expect(document.programSpec.idlSnapshot.accounts[0]!.discriminator).toEqual(discriminator);
});
