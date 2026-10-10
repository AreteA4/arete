// Guards the shape of public/agent.md that matters to coding agents whose
// fetch tools summarise or truncate pages: the raw-fetch banner and the
// exact setup commands must come first, before any prose.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const AGENT_MD = new URL("../public/agent.md", import.meta.url);
const text = await readFile(AGENT_MD, "utf8");

const SETUP_COMMANDS = [
  "curl -fsSL https://arete.run/install.sh | sh",
  "a4 init -y",
  "a4 --profile agent auth signup --if-missing --json",
  "a4 doctor --json",
  "a4 explore catalog --vocabulary --json",
];

// Budget for everything up to the end of the setup block.
const SETUP_BUDGET = 600;

test("agent.md opens with the raw-fetch banner", () => {
  const head = text.slice(0, 250);
  assert.match(head, /^# Set up Arete\n/);
  assert.ok(head.includes("curl -fsSL https://docs.arete.run/agent.md"));
});

test("agent.md setup commands are the first code block, verbatim", () => {
  const match = /```sh\n([\s\S]*?)\n```/.exec(text);
  assert.ok(match, "missing ```sh setup block");
  assert.deepEqual(match[1].split("\n"), SETUP_COMMANDS);
  const end = match.index + match[0].length;
  assert.ok(
    end <= SETUP_BUDGET,
    `setup block ends at char ${end}; keep it within ${SETUP_BUDGET}`,
  );
  assert.ok(
    !text.slice(0, match.index).includes("    a4 "),
    "no other commands before the setup block",
  );
});

test("agent.md keeps the Windows installer next to the setup block", () => {
  const block = text.indexOf("```sh");
  const ps = text.indexOf("irm https://arete.run/install.ps1 | iex");
  assert.ok(ps > block && ps < SETUP_BUDGET + 200);
});

test("agent.md `a4 know search` uses flags, not a positional query", () => {
  for (const line of text.split("\n")) {
    if (!line.includes("a4 know search")) continue;
    assert.match(line, /a4 know search (--query|-q|--concept|--category)\b/);
  }
});
