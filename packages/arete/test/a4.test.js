"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const a4 = require("../bin/a4.js");

const FIXTURES = path.join(__dirname, "fixtures");
const MINISIGN = path.join(FIXTURES, "minisign");
const RELEASE = path.join(FIXTURES, "release");
const LAUNCHER = path.join(__dirname, "..", "bin", "a4.js");
const TEST_PUBLIC_KEY = fs.readFileSync(path.join(MINISIGN, "test-minisign.pub"), "utf8");
const notWindows = { skip: process.platform === "win32" ? "fake a4 asset is a shell script" : false };

const read = (dir, name) => fs.readFileSync(path.join(dir, name));

function tempHome(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "a4-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

// Serves fixtures/release under /a4-cli-v<anything>/<file>, /redirect/... via 302,
// with per-file overrides (Buffer) for tamper tests.
function serveRelease(t, overrides = {}) {
  const server = http.createServer((req, res) => {
    const { pathname } = new URL(req.url, "http://localhost");
    const redirect = pathname.match(/^\/redirect(\/.*)$/);
    if (redirect) {
      res.writeHead(302, { location: redirect[1] });
      return res.end();
    }
    const match = pathname.match(/^\/a4-cli-v[^/]+\/([^/]+)$/);
    const name = match && match[1];
    if (name && name in overrides) {
      res.writeHead(200);
      return res.end(overrides[name]);
    }
    const file = name && path.join(RELEASE, name);
    if (!file || !fs.existsSync(file)) {
      res.writeHead(404);
      return res.end("not found");
    }
    res.writeHead(200);
    fs.createReadStream(file).pipe(res);
  });
  t.after(() => server.close());
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(`http://127.0.0.1:${server.address().port}`)));
}

function installEnv(home, base, extra = {}) {
  return {
    PATH: process.env.PATH,
    HOME: home,
    USERPROFILE: home,
    ARETE_HOME: path.join(home, ".arete"),
    A4_INSTALL_DIR: path.join(home, "bin"),
    A4_MANIFEST_BASE_URL: base,
    FAKE_A4_ARGS_FILE: path.join(home, "args.log"),
    ...extra,
  };
}

const readArgs = (home) => fs.readFileSync(path.join(home, "args.log"), "utf8").trim().split("\n");

// --- minisign ---------------------------------------------------------------

test("embedded production key parses as a minisign Ed25519 key", () => {
  const key = a4.parseMinisignPublicKey(a4.MINISIGN_PUBLIC_KEY);
  assert.equal(key.algorithm, "Ed");
  assert.equal(key.key.length, 32);
  // minisign prints key ids little-endian; keys.rs documents D4FB4D5B83098B6C.
  assert.equal(Buffer.from(key.keyId).reverse().toString("hex").toUpperCase(), "D4FB4D5B83098B6C");
});

test("verifies the Rust test vector (valid signature)", () => {
  const result = a4.verifyMinisign({
    publicKey: TEST_PUBLIC_KEY,
    signature: read(MINISIGN, "checksums.txt.minisig").toString("utf8"),
    message: read(MINISIGN, "checksums.txt"),
  });
  assert.equal(result.trustedComment, "a4-cli-v0.0.0-test");
});

test("rejects a signature made with another key", () => {
  assert.throws(
    () => a4.verifyMinisign({
      publicKey: TEST_PUBLIC_KEY,
      signature: read(MINISIGN, "checksums.txt.wrong-key.minisig").toString("utf8"),
      message: read(MINISIGN, "checksums.txt"),
    }),
    /key id .* does not match/
  );
});

test("rejects tampered checksums", () => {
  const tampered = Buffer.from(read(MINISIGN, "checksums.txt").toString("utf8").replace("9b18", "0b18"));
  assert.throws(
    () => a4.verifyMinisign({ publicKey: TEST_PUBLIC_KEY, signature: read(MINISIGN, "checksums.txt.minisig").toString("utf8"), message: tampered }),
    /signature does not match/
  );
});

test("rejects a tampered trusted comment (global signature)", () => {
  const sig = read(MINISIGN, "checksums.txt.minisig").toString("utf8").replace("a4-cli-v0.0.0-test", "a4-cli-v9.9.9");
  assert.throws(
    () => a4.verifyMinisign({ publicKey: TEST_PUBLIC_KEY, signature: sig, message: read(MINISIGN, "checksums.txt") }),
    /global signature/
  );
});

test("rejects legacy non-prehashed signatures and malformed files", () => {
  const lines = read(MINISIGN, "checksums.txt.minisig").toString("utf8").split("\n");
  const raw = Buffer.from(lines[1], "base64");
  raw.write("Ed", 0, "latin1");
  lines[1] = raw.toString("base64");
  assert.throws(() => a4.parseMinisignSignature(lines.join("\n")), /legacy/);
  assert.throws(() => a4.parseMinisignSignature("untrusted comment: x\nAAAA\n"), /malformed/);
  assert.throws(() => a4.parseMinisignPublicKey("not base64 at all"), /42 bytes/);
});

// --- checksums / urls / receipt ------------------------------------------------

test("parseChecksums handles GNU, BSD-star and CRLF formats", () => {
  const text = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789  a4-linux-x64\r\n" +
    "0000000000000000000000000000000000000000000000000000000000000001 *a4-win32-x64.exe\n" +
    "garbage line\n";
  const map = a4.parseChecksums(text);
  assert.equal(map.get("a4-linux-x64"), "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
  assert.equal(map.get("a4-win32-x64.exe"), "0000000000000000000000000000000000000000000000000000000000000001");
  assert.equal(map.size, 2);
});

test("pickAsset and releaseBaseUrl", () => {
  assert.equal(a4.pickAsset("linux", "x64"), "a4-linux-x64");
  assert.equal(a4.pickAsset("win32", "x64"), "a4-win32-x64.exe");
  assert.equal(a4.pickAsset("freebsd", "x64"), null);
  assert.equal(a4.releaseBaseUrl("1.2.3", {}), "https://github.com/AreteA4/arete/releases/download/a4-cli-v1.2.3");
  assert.equal(a4.releaseBaseUrl("1.2.3", { A4_MANIFEST_BASE_URL: "http://h/base/" }), "http://h/base/a4-cli-v1.2.3");
  assert.equal(a4.releaseBaseUrl("1.2.3", { A4_MANIFEST_BASE_URL: "http://h/{version}/x" }), "http://h/1.2.3/x");
});

test("readReceipt honours ARETE_HOME and requires an existing binary", (t) => {
  const home = tempHome(t);
  assert.equal(a4.readReceipt({}, home), null);
  assert.equal(a4.receiptPath({}, home), path.join(home, ".arete", "receipt.json"));
  const areteHome = path.join(home, "custom");
  fs.mkdirSync(areteHome);
  const binary = path.join(home, "a4");
  fs.writeFileSync(path.join(areteHome, "receipt.json"), JSON.stringify({ schemaVersion: 1, binary }));
  assert.equal(a4.readReceipt({ ARETE_HOME: areteHome }, home), null, "binary missing");
  fs.writeFileSync(binary, "#!/bin/sh\n", { mode: 0o755 });
  assert.equal(a4.readReceipt({ ARETE_HOME: areteHome }, home).binary, binary);
  fs.writeFileSync(path.join(areteHome, "receipt.json"), "{not json");
  assert.equal(a4.readReceipt({ ARETE_HOME: areteHome }, home), null, "corrupt receipt");
});

// --- launcher ------------------------------------------------------------------

test("receipt passthrough runs the recorded binary with argv", notWindows, async (t) => {
  const home = tempHome(t);
  const areteHome = path.join(home, ".arete");
  fs.mkdirSync(areteHome);
  const binary = path.join(home, "fake-a4");
  fs.writeFileSync(binary, "#!/bin/sh\necho \"argv=$*\"\necho \"sentinel=$ARETE_A4_LAUNCHER_ACTIVE\"\nexit 3\n", { mode: 0o755 });
  fs.writeFileSync(path.join(areteHome, "receipt.json"), JSON.stringify({ schemaVersion: 1, binary }));
  let result;
  const status = await a4.launch(["explore", "--json"], {
    env: { PATH: process.env.PATH, ARETE_HOME: areteHome },
    homedir: home,
    stdio: "pipe",
    onResult: (r) => { result = r; },
  });
  assert.equal(status, 3);
  assert.equal(result.stdout, "argv=explore --json\nsentinel=1\n");
});

test("install: downloads, verifies and hands over to a4 self install --source npm", notWindows, async (t) => {
  const home = tempHome(t);
  const base = await serveRelease(t);
  const logs = [];
  const env = installEnv(home, base);
  const status = await a4.launch(["install", "--no-modify-path"], { env, homedir: home, publicKey: TEST_PUBLIC_KEY, stdio: "pipe", log: (m) => logs.push(m) });
  assert.equal(status, 0);
  const args = readArgs(home);
  assert.equal(args.length, 1);
  const match = args[0].match(/^self install --source npm --checksums (\S+[\\/]checksums\.txt) --signature (\S+[\\/]checksums\.txt\.minisig) --no-modify-path$/);
  assert.ok(match, `unexpected fake a4 argv: ${args[0]}`);
  assert.equal(path.dirname(match[1]), path.dirname(match[2]));
  assert.ok(!fs.existsSync(path.dirname(match[1])), "temp dir is removed after install");
  const receipt = JSON.parse(fs.readFileSync(path.join(home, ".arete", "receipt.json"), "utf8"));
  assert.equal(receipt.source, "npm");
  assert.equal(receipt.binary, path.join(home, "bin", "a4"));
  assert.match(logs[0], new RegExp(`^Installing a4 ${require("../package.json").version.replace(/\./g, "\\.")} .* from ${base}/a4-cli-v`));
});

test("install follows redirects", notWindows, async (t) => {
  const home = tempHome(t);
  const base = await serveRelease(t);
  const status = await a4.launch(["install"], { env: installEnv(home, `${base}/redirect`), homedir: home, publicKey: TEST_PUBLIC_KEY, stdio: "pipe", log() {} });
  assert.equal(status, 0);
  assert.equal(readArgs(home).length, 1);
});

test("first run without a receipt installs silently, then only the command's stdout is emitted", notWindows, async (t) => {
  const home = tempHome(t);
  const base = await serveRelease(t);
  const env = installEnv(home, base);
  const logs = [];
  let result;
  const status = await a4.launch(["explore", "--json"], {
    env, homedir: home, publicKey: TEST_PUBLIC_KEY, stdio: "pipe", log: (m) => logs.push(m), onResult: (r) => { result = r; },
  });
  assert.equal(status, 0);
  assert.equal(result.stdout, '{"schemaVersion":1,"fake":true,"argv":["explore","--json"]}\n');
  assert.match(logs[0], /not installed yet/);
  const args = readArgs(home);
  assert.equal(args.length, 2);
  assert.match(args[0], /^self install --source npm --checksums \S+ --signature \S+$/);
  assert.equal(args[1], "explore --json");

  // Second run uses the receipt; no reinstall.
  const again = await a4.launch(["--version"], { env, homedir: home, publicKey: TEST_PUBLIC_KEY, stdio: "pipe", log() {}, onResult: (r) => { result = r; } });
  assert.equal(again, 0);
  assert.equal(result.stdout, "a4 0.0.0-test\n");
  assert.equal(readArgs(home).length, 3);
});

test("install fails clearly when the release is not published yet (404)", async (t) => {
  const home = tempHome(t);
  const base = await serveRelease(t);
  await assert.rejects(
    a4.launch(["install"], { env: installEnv(home, `${base}/missing`), homedir: home, publicKey: TEST_PUBLIC_KEY, log() {} }),
    /Release \S+ is still publishing; retry in a few minutes/
  );
});

test("install rejects tampered checksums and mismatching binaries", async (t) => {
  const home = tempHome(t);
  const good = fs.readFileSync(path.join(RELEASE, "checksums.txt"), "utf8");
  const tamperedBase = await serveRelease(t, { "checksums.txt": Buffer.from(good.replace("1725", "0725")) });
  await assert.rejects(
    a4.launch(["install"], { env: installEnv(home, tamperedBase), homedir: home, publicKey: TEST_PUBLIC_KEY, log() {} }),
    /signature does not match/
  );
  const asset = a4.pickAsset();
  if (asset) {
    const swappedBase = await serveRelease(t, { [asset]: Buffer.from("#!/bin/sh\necho evil\n") });
    await assert.rejects(
      a4.launch(["install"], { env: installEnv(home, swappedBase), homedir: home, publicKey: TEST_PUBLIC_KEY, log() {} }),
      /sha256 mismatch/
    );
  }
  // The production key must reject the test-signed fixture.
  await assert.rejects(
    a4.launch(["install"], { env: installEnv(home, await serveRelease(t)), homedir: home, log() {} }),
    /key id .* does not match/
  );
  assert.ok(!fs.existsSync(path.join(home, "args.log")), "fake a4 never ran");
});

test("recursion sentinel stops the launcher before resolving another a4", () => {
  const result = spawnSync(process.execPath, [LAUNCHER, "--version"], {
    encoding: "utf8",
    env: { ...process.env, [a4.RECURSION_SENTINEL]: "1" },
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Refusing to recursively launch/);
});

test("package is scriptless and ships only bin + README", () => {
  const manifest = require("../package.json");
  assert.deepEqual(Object.keys(manifest.scripts), ["test"]);
  assert.equal(manifest.optionalDependencies, undefined);
  assert.equal(manifest.dependencies, undefined);
  assert.deepEqual(manifest.files, ["bin", "README.md"]);
  assert.ok(!fs.existsSync(path.join(__dirname, "..", "scripts")));
});
