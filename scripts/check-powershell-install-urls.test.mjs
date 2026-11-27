// Windows PowerShell 5.1 (the PowerShell built into Windows) does not follow
// the 308 redirect https://arete.run/ answers with, so `irm https://arete.run/
// install.ps1 | iex` fails there; so does .NET's WebClient. Every PowerShell
// fetch must use the final host, https://docs.arete.run/. curl follows 308,
// so the install.sh curl lines stay on arete.run.
//
// Runs in an always-on CI job: it scans every tracked file, and a PR that
// only touches a README or the CLI does not run the docs job.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = execFileSync("git", ["rev-parse", "--show-toplevel"], {
  cwd: path.dirname(fileURLToPath(import.meta.url)),
  encoding: "utf8",
}).trim();
// This file names the forbidden form on purpose, for the matcher self-check.
const SELF = path
  .relative(ROOT, fileURLToPath(import.meta.url))
  .split(path.sep)
  .join("/");

// A PowerShell web fetch whose argument (before any pipe, `;` or `#` comment)
// is an arete.run URL. The scheme is optional: Invoke-WebRequest and
// Invoke-RestMethod assume http:// for `irm arete.run/install.ps1`.
const PS_FETCH_OF_ARETE_RUN =
  /\b(?:irm|iwr|Invoke-RestMethod|Invoke-WebRequest|DownloadString|DownloadFile)\b[^|;#\n]*?(?<![\w.-])(?:https?:\/\/)?arete\.run\//i;

// A file's text, or null when it is binary. Windows PowerShell 5.1's `>` and
// Out-File write UTF-16LE with a BOM, which contains NUL bytes.
function textOf(bytes) {
  if (bytes[0] === 0xff && bytes[1] === 0xfe) return bytes.toString("utf16le");
  return bytes.includes(0) ? null : bytes.toString("utf8");
}

test("files written by Windows PowerShell 5.1 are scanned, binaries are not", () => {
  const utf16 = Buffer.from(
    "\ufeffirm https://arete.run/install.ps1 | iex\r\n",
    "utf16le",
  );
  assert.match(textOf(utf16), PS_FETCH_OF_ARETE_RUN);
  assert.equal(textOf(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0x01])), null);
});

test("the matcher catches PowerShell fetches of arete.run, and only those", () => {
  for (const line of [
    "irm https://arete.run/install.ps1 | iex",
    "IRM HTTPS://ARETE.RUN/install.ps1 | iex",
    "irm http://arete.run/install.ps1 | iex",
    "iwr -useb https://arete.run/install.ps1 | iex",
    "irm arete.run/install.ps1 | iex",
    "irm 'arete.run/install.ps1' | iex",
    "& ([scriptblock]::Create((irm https://arete.run/install.ps1)))",
    "Invoke-RestMethod -Uri 'https://arete.run/install.ps1'",
    "Invoke-WebRequest https://arete.run/install.ps1 -OutFile i.ps1",
    "iex (New-Object Net.WebClient).DownloadString('https://arete.run/install.ps1')",
    "(New-Object Net.WebClient).DownloadFile('https://arete.run/install.ps1', 'i.ps1')",
    'die "Windows shell detected. Run in PowerShell instead: irm https://arete.run/install.ps1 | iex"',
  ]) {
    assert.match(line, PS_FETCH_OF_ARETE_RUN, line);
  }
  for (const line of [
    "irm https://docs.arete.run/install.ps1 | iex",
    "& ([scriptblock]::Create((irm https://docs.arete.run/install.ps1)))",
    "curl -fsSL https://arete.run/install.sh | sh",
    "curl -fsSL https://arete.run/install.sh | sh  # Windows: irm https://docs.arete.run/install.ps1 | iex",
    "irm https://docs.arete.run/install.ps1 | iex; curl -fsSL https://arete.run/install.sh | sh",
    "irm https://arete.run.example/install.ps1 | iex",
    "irm https://evilarete.run/install.ps1 | iex",
    "irm docs.arete.run/install.ps1 | iex",
    "irm https://docs.arete.run/install.ps1  # key from arete.run/keys",
    "a4 firm https://arete.run/",
  ]) {
    assert.doesNotMatch(line, PS_FETCH_OF_ARETE_RUN, line);
  }
});

test("no tracked file points PowerShell at the arete.run redirect", () => {
  const files = execFileSync("git", ["ls-files", "-z"], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  })
    .split("\0")
    .filter((file) => file && file !== SELF);
  const offenders = [];
  for (const file of files) {
    let bytes;
    try {
      bytes = readFileSync(path.join(ROOT, file));
    } catch {
      continue; // deleted in the working tree, or a submodule directory
    }
    const text = textOf(bytes);
    if (text === null) continue; // binary
    text.split(/\r?\n/).forEach((line, i) => {
      if (PS_FETCH_OF_ARETE_RUN.test(line)) {
        offenders.push(`${file}:${i + 1}: ${line.trim()}`);
      }
    });
  }
  assert.ok(
    offenders.length === 0,
    "Windows PowerShell 5.1 does not follow the 308 redirect https://arete.run/ " +
      "answers with; use https://docs.arete.run/ for PowerShell fetches:\n" +
      offenders.join("\n"),
  );
});
