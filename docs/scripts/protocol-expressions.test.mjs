// The protocol expression reference page is generated from the checker's
// exported reference (src/data/protocol-expressions.json) and must be current.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { PAGE, generate } from "./protocol-expressions.mjs";

const reference = JSON.parse(
  await readFile(new URL("../src/data/protocol-expressions.json", import.meta.url), "utf8"),
);
const page = await readFile(PAGE, "utf8");

test("the expression reference page is generated from the current JSON", async () => {
  assert.equal(
    page,
    await generate(),
    "run `node scripts/protocol-expressions.mjs` in docs/",
  );
});

test("the page lists every function, operator and check code", () => {
  for (const fn of reference.functions) assert.ok(page.includes(`\`${fn.signature}\``), fn.name);
  for (const op of [...reference.operators.binary, ...reference.operators.prefix]) {
    assert.ok(page.includes(`\`${op.symbol.replaceAll("|", "\\|")}\``), op.symbol);
  }
  for (const entry of reference.checkCodes) assert.ok(page.includes(`\`${entry.code}\``), entry.code);
});
