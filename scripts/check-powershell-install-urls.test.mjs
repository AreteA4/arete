// Windows PowerShell 5.1 (the PowerShell built into Windows) does not follow
// the 308 redirect https://arete.run/ answers with, so `irm https://arete.run/
// install.ps1 | iex` fails there; so does .NET's WebClient. www.arete.run
// redirects to arete.run, so it fails too. Every PowerShell fetch must use the
// final host, https://docs.arete.run/. curl follows 308, so the install.sh
// curl lines stay on arete.run.
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

const CMDLET =
  /(?<![\w$.-])(?:irm|iwr|Invoke-RestMethod|Invoke-WebRequest)(?![\w-])/gi;
// A WebClient method call; the `.` keeps a JS `downloadFile(...)` out.
const WEBCLIENT = /\.Download(?:String|File)\s*\(/gi;
// Switch parameters of Invoke-RestMethod / Invoke-WebRequest. Any other
// parameter takes a value, so the token after it is not the URL.
const SWITCHES = [
  "allowinsecureredirect",
  "allowunencryptedauthentication",
  "disablekeepalive",
  "followrellink",
  "noproxy",
  "passthru",
  "preserveauthorizationonredirect",
  "preservehttpmethodonredirect",
  "proxyusedefaultcredentials",
  "resume",
  "skipcertificatecheck",
  "skipheadervalidation",
  "skiphttperrorcheck",
  "usebasicparsing",
  "usedefaultcredentials",
];
// `$name = 'literal'` or `$name = "literal"`, alone up to `;`, `|`, `#` or
// the end of the line.
const ASSIGNMENT = /\$(\w+)\s*=\s*(?:'([^']*)'|"([^"]*)")(?=\s*(?:$|[;|#]))/g;

// A file's text, or null when it is binary. Windows PowerShell 5.1's `>` and
// Out-File write UTF-16LE with a BOM, which contains NUL bytes.
function textOf(bytes) {
  if (bytes[0] === 0xff && bytes[1] === 0xfe) return bytes.toString("utf16le");
  return bytes.includes(0) ? null : bytes.toString("utf8");
}

// Joins PowerShell backtick continuations into logical lines, each with the
// number of its first physical line. A continuation is a backtick after
// whitespace (or alone) at the end of a line, optionally followed by spaces or
// tabs. The backtick must follow whitespace so that markdown inline code that
// closes at the end of a line (`irm`) is not joined to the next line.
function logicalLines(text) {
  const lines = [];
  let pending = null;
  text.split(/\r?\n/).forEach((line, i) => {
    pending ??= { line: i + 1, text: "" };
    const continued = /(^|\s)`[ \t]*$/.exec(line);
    if (continued) {
      pending.text += line.slice(0, continued.index) + " ";
      return;
    }
    pending.text += line;
    lines.push(pending);
    pending = null;
  });
  if (pending) lines.push(pending);
  return lines;
}

// One PowerShell value starting at `i`: a quoted string (its content), a
// balanced (), {}, [], @() or @{} group (its source), or a bare word.
// Returns [value, next index].
function readValue(text, i) {
  const quote = text[i];
  if (quote === "'" || quote === '"') {
    let j = i + 1;
    while (j < text.length) {
      if (quote === '"' && text[j] === "`") j += 2;
      else if (text[j] === quote && text[j + 1] === quote) j += 2;
      else if (text[j] === quote) break;
      else j++;
    }
    return [text.slice(i + 1, j), j + 1];
  }
  const start = i;
  if (text[i] === "@" && "({".includes(text[i + 1])) i++;
  if ("({[".includes(text[i])) {
    let depth = 0;
    for (; i < text.length; i++) {
      if (text[i] === "'" || text[i] === '"') {
        i = readValue(text, i)[1] - 1;
      } else if ("({[".includes(text[i])) depth++;
      else if (")}]".includes(text[i]) && --depth === 0) break;
    }
    return [text.slice(start, i + 1), i + 1];
  }
  const end = text.slice(i).search(/[\s|;,)}]|$/);
  return [text.slice(i, i + end), i + end];
}

// The URL an Invoke-RestMethod / Invoke-WebRequest call fetches, from `text`
// (the line after the cmdlet name): the -Uri value or the first positional
// argument. Uri is the only positional parameter, so the first of either is
// the URL. Arguments end at a pipe, `;`, `,`, a closing bracket or a comment.
function cmdletUrl(text) {
  let i = 0;
  for (;;) {
    while (/\s/.test(text[i] ?? "")) i++;
    if (i >= text.length || "|;,)}]#".includes(text[i])) return null;
    const param = /^-([A-Za-z]\w*)(:?)/.exec(text.slice(i));
    if (!param) return readValue(text, i)[0];
    i += param[0].length;
    const name = param[1].toLowerCase();
    const isUri = name.length >= 2 && "uri".startsWith(name);
    // An exact name wins over a longer one it prefixes: -Proxy takes a value
    // although -ProxyUseDefaultCredentials is a switch.
    const isSwitch =
      !isUri && name !== "proxy" && SWITCHES.some((s) => s.startsWith(name));
    if (isSwitch && !param[2]) continue;
    // `-Uri: value` binds like `-Uri:value`.
    while (/\s/.test(text[i] ?? "")) i++;
    if (i >= text.length || "|;,)}]#".includes(text[i])) return null;
    const [value, next] = readValue(text, i);
    if (isUri) return value;
    i = next;
  }
}

// The URLs one logical line fetches with PowerShell: each cmdlet call's URL
// and the first argument of each WebClient DownloadString/DownloadFile call.
// A call's arguments are read from at most WINDOW characters after its name,
// which keeps a line full of cmdlet names or open brackets linear.
const WINDOW = 1024;
function fetchedUrls(line) {
  const urls = [];
  for (const m of line.matchAll(CMDLET)) {
    const start = m.index + m[0].length;
    const url = cmdletUrl(line.slice(start, start + WINDOW));
    if (url !== null) urls.push(url);
  }
  for (const m of line.matchAll(WEBCLIENT)) {
    const start = m.index + m[0].length;
    const args = line.slice(start, start + WINDOW).trimStart();
    if (args && args[0] !== ")") urls.push(readValue(args, 0)[0]);
  }
  return urls;
}

// Whether a URL (scheme optional: PowerShell assumes http:// for a bare
// host) points at a host that answers with a redirect PowerShell 5.1 does
// not follow.
function isRedirectingHost(url) {
  const host = /^(?:https?:\/\/)?([^/\\?#:\s'"`]*)/i
    .exec(url)[1]
    .toLowerCase()
    .replace(/\.$/, "");
  return host === "arete.run" || host === "www.arete.run";
}

// release-please copies commit subjects into changelogs, which may quote the
// old command; the changelog records history and is not documentation.
function isSkipped(file) {
  return path.posix.basename(file) === "CHANGELOG.md";
}

// `file:line: text` for each logical line of `text` that points a PowerShell
// fetch at a redirecting host. A `$name` URL argument resolves through the
// simple literal assignments (`$name = 'url'`) anywhere in the same file.
// Concatenations, environment variables and computed strings are not
// tracked.
function offendersIn(file, text) {
  if (isSkipped(file)) return [];
  const lines = logicalLines(text);
  const vars = new Map();
  for (const { text: line } of lines) {
    for (const m of line.matchAll(ASSIGNMENT)) {
      const name = m[1].toLowerCase();
      vars.set(name, [...(vars.get(name) ?? []), m[2] ?? m[3]]);
    }
  }
  const offenders = [];
  for (const { line, text: logical } of lines) {
    const hit = fetchedUrls(logical).some((url) => {
      const variable = /^\$(\w+)$/.exec(url);
      const values = variable ? vars.get(variable[1].toLowerCase()) : [url];
      return (values ?? []).some(isRedirectingHost);
    });
    if (hit) offenders.push(`${file}:${line}: ${logical.trim()}`);
  }
  return offenders;
}

const flagged = (snippet) => offendersIn("snippet.ps1", snippet).length > 0;

test("files written by Windows PowerShell 5.1 are scanned, binaries are not", () => {
  const utf16 = Buffer.from(
    "\ufeffirm https://arete.run/install.ps1 | iex\r\n",
    "utf16le",
  );
  assert.ok(flagged(textOf(utf16)));
  assert.equal(textOf(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0x01])), null);
});

test("the matcher catches PowerShell fetches of arete.run, and only those", () => {
  for (const snippet of [
    "irm https://arete.run/install.ps1 | iex",
    "IRM HTTPS://ARETE.RUN/install.ps1 | iex",
    "irm http://arete.run/install.ps1 | iex",
    "iwr -useb https://arete.run/install.ps1 | iex",
    "irm arete.run/install.ps1 | iex",
    "irm 'arete.run/install.ps1' | iex",
    "& ([scriptblock]::Create((irm https://arete.run/install.ps1)))",
    "Invoke-RestMethod -Uri 'https://arete.run/install.ps1'",
    "Invoke-RestMethod -Uri:https://arete.run/install.ps1",
    "irm -Method Get https://arete.run/install.ps1",
    "Invoke-WebRequest https://arete.run/install.ps1 -OutFile i.ps1",
    "iex (New-Object Net.WebClient).DownloadString('https://arete.run/install.ps1')",
    "(New-Object Net.WebClient).DownloadFile('https://arete.run/install.ps1', 'i.ps1')",
    'die "Windows shell detected. Run in PowerShell instead: irm https://arete.run/install.ps1 | iex"',
    "irm https://www.arete.run/install.ps1 | iex",
    "irm `\n  https://arete.run/install.ps1 | iex",
    "Invoke-WebRequest -UseBasicParsing ` \r\n  -Uri https://arete.run/install.ps1",
    "irm -Uri: https://arete.run/install.ps1 | iex",
    "irm -Proxy http://proxy:8080 https://arete.run/install.ps1 | iex",
  ]) {
    assert.ok(flagged(snippet), snippet);
  }
  for (const snippet of [
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
    "use irm (Invoke-RestMethod). Create an API key at https://arete.run/keys",
    "irm fails when https://arete.run/install.ps1 answers 308 Permanent Redirect",
    "Invoke-WebRequest -Uri https://docs.arete.run/install.ps1 -Headers @{ Referer = 'https://arete.run/' }",
    "Run `irm`\nhttps://arete.run/ is the site",
    "irm https://docs.arete.run/install.ps1 -Proxy http://arete.run/",
    'await downloadFile("https://arete.run/a4/latest.json")',
  ]) {
    assert.ok(!flagged(snippet), snippet);
  }
});

test("a line full of cmdlet names or open brackets is scanned in linear time", () => {
  const started = performance.now();
  for (const unit of [
    "irm a ",
    "irm -Method ",
    "irm (",
    "irm @{ ",
    ".DownloadString((",
  ]) {
    assert.deepEqual(offendersIn("big.js", unit.repeat(10_000)), []);
  }
  // About 1 s here; the quadratic parser took over 25 s on these lines.
  assert.ok(performance.now() - started < 5_000);
});

test("a URL in a simple literal assignment is resolved (concatenations, environment variables and computed strings are not)", () => {
  assert.ok(flagged("$u = 'https://arete.run/install.ps1'\nirm $u | iex"));
  assert.ok(flagged('$U = "https://arete.run/install.ps1"\n\nirm -Uri $u'));
  assert.ok(!flagged("$u = 'https://docs.arete.run/install.ps1'\nirm $u"));
  assert.ok(
    !flagged("$site = 'https://arete.run/'\nirm https://docs.arete.run/x"),
  );
});

test("changelogs are not scanned, other files are", () => {
  const line = "irm https://arete.run/install.ps1 | iex\n";
  assert.deepEqual(offendersIn("CHANGELOG.md", line), []);
  assert.deepEqual(offendersIn("cli/CHANGELOG.md", line), []);
  assert.deepEqual(offendersIn("README.md", line), [
    "README.md:1: irm https://arete.run/install.ps1 | iex",
  ]);
  assert.deepEqual(offendersIn("docs/README.md", "x\n" + line), [
    "docs/README.md:2: irm https://arete.run/install.ps1 | iex",
  ]);
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
    offenders.push(...offendersIn(file, text));
  }
  assert.ok(
    offenders.length === 0,
    "Windows PowerShell 5.1 does not follow the 308 redirect https://arete.run/ " +
      "answers with; use https://docs.arete.run/ for PowerShell fetches:\n" +
      offenders.join("\n"),
  );
});
