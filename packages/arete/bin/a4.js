#!/usr/bin/env node
"use strict";

// @usearete/a4 launcher. Scriptless bootstrapper for the Arete CLI (`a4`):
//
//   npx @usearete/a4 install [--install-dir DIR] [--no-modify-path] [--json]
//     Download the release asset for this package's version, verify sha256 +
//     minisign signature, then run `<tmp>/a4 self install --source npm ...`.
//   npx @usearete/a4 <anything else>
//     Run the binary recorded in ~/.arete/receipt.json (installing silently
//     first when there is no receipt) with the given argv.
//
// No lifecycle scripts, no dependencies, no network access at `npm install`.
// Design: docs/internal/agent-first-onboarding.md (WP3).

const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const pkg = require("../package.json");

const RECURSION_SENTINEL = "ARETE_A4_LAUNCHER_ACTIVE";
const MINISIGN_PUBLIC_KEY = "RWRsiwmDW0371BZbcE1IWD6Y8/KIoAArUAp7mpyG6VweJ5rE3Lf3g5qA";
const RELEASE_BASE = "https://github.com/AreteA4/arete/releases/download";
const MAX_REDIRECTS = 5;
const ASSETS = {
  "darwin-arm64": "a4-darwin-arm64",
  "darwin-x64": "a4-darwin-x64",
  "linux-x64": "a4-linux-x64",
  "linux-arm64": "a4-linux-arm64",
  "win32-x64": "a4-win32-x64.exe",
};
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

class InstallError extends Error {}

// ---------------------------------------------------------------------------
// Platform, URLs, receipt

function pickAsset(platform = process.platform, arch = process.arch) {
  return ASSETS[`${platform}-${arch}`] || null;
}

// Same semantics as cli/src/selfhost/platform.rs::release_base_url.
function releaseBaseUrl(version, env = process.env) {
  const override = (env.A4_MANIFEST_BASE_URL || "").trim();
  if (override) {
    const base = override.replace(/\/+$/, "");
    return base.includes("{version}")
      ? base.split("{version}").join(version)
      : `${base}/a4-cli-v${version}`;
  }
  return `${RELEASE_BASE}/a4-cli-v${version}`;
}

function areteHome(env = process.env, homedir = os.homedir()) {
  return env.ARETE_HOME ? env.ARETE_HOME : path.join(homedir, ".arete");
}

function receiptPath(env = process.env, homedir = os.homedir()) {
  return path.join(areteHome(env, homedir), "receipt.json");
}

// Parsed receipt when it exists, is well-formed and its binary is present; else null.
function readReceipt(env = process.env, homedir = os.homedir()) {
  let receipt;
  try {
    receipt = JSON.parse(fs.readFileSync(receiptPath(env, homedir), "utf8"));
  } catch {
    return null;
  }
  if (!receipt || typeof receipt !== "object" || typeof receipt.binary !== "string") return null;
  try {
    if (!fs.statSync(receipt.binary).isFile()) return null;
  } catch {
    return null;
  }
  return receipt;
}

// ---------------------------------------------------------------------------
// Checksums

// `<sha256>  <asset>` per line (optional `*` binary marker) -> Map(asset -> sha256 lowercase).
function parseChecksums(text) {
  const map = new Map();
  for (const line of String(text).split(/\r?\n/)) {
    const match = line.match(/^([a-fA-F0-9]{64})\s+\*?(\S.*?)\s*$/);
    if (match) map.set(match[2], match[1].toLowerCase());
  }
  return map;
}

function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash("sha256");
    fs.createReadStream(filePath)
      .on("error", reject)
      .on("data", (chunk) => hash.update(chunk))
      .on("end", () => resolve(hash.digest("hex")));
  });
}

// ---------------------------------------------------------------------------
// minisign (https://jedisct1.github.io/minisign/) verification in pure Node.
//
// Public key: base64 of  alg("Ed") || key_id(8) || ed25519_pk(32)
// Signature file:
//   untrusted comment: ...
//   base64( alg("ED") || key_id(8) || signature(64) )
//   trusted comment: <text>
//   base64( global_signature(64) )      over signature || trusted comment
// alg "ED" signs blake2b-512(message); legacy "Ed" (signs raw message) is rejected.

function parseMinisignPublicKey(text) {
  const line = String(text)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => l && !l.startsWith("untrusted comment:"));
  const raw = line ? Buffer.from(line, "base64") : Buffer.alloc(0);
  if (raw.length !== 42) throw new InstallError("minisign public key: expected 42 bytes");
  const algorithm = raw.subarray(0, 2).toString("latin1");
  if (algorithm !== "Ed") throw new InstallError(`minisign public key: unsupported algorithm ${JSON.stringify(algorithm)}`);
  return { algorithm, keyId: raw.subarray(2, 10), key: raw.subarray(10, 42) };
}

function parseMinisignSignature(text) {
  const lines = String(text).split(/\r?\n/).map((l) => l.trim());
  const [untrusted, sigLine, trustedLine, globalLine] = lines;
  if (!untrusted || !untrusted.startsWith("untrusted comment:") || !sigLine || !trustedLine || !globalLine) {
    throw new InstallError("minisign signature: malformed file (expected 4 lines)");
  }
  if (!trustedLine.startsWith("trusted comment:")) {
    throw new InstallError("minisign signature: missing trusted comment line");
  }
  const raw = Buffer.from(sigLine, "base64");
  if (raw.length !== 74) throw new InstallError("minisign signature: expected 74 bytes");
  const algorithm = raw.subarray(0, 2).toString("latin1");
  if (algorithm === "Ed") throw new InstallError("minisign signature: legacy (non-prehashed) signature rejected");
  if (algorithm !== "ED") throw new InstallError(`minisign signature: unsupported algorithm ${JSON.stringify(algorithm)}`);
  const globalSignature = Buffer.from(globalLine, "base64");
  if (globalSignature.length !== 64) throw new InstallError("minisign signature: global signature must be 64 bytes");
  return {
    algorithm,
    keyId: raw.subarray(2, 10),
    signature: raw.subarray(10, 74),
    trustedComment: trustedLine.slice("trusted comment:".length).replace(/^ /, ""),
    globalSignature,
  };
}

function ed25519KeyObject(rawKey) {
  return crypto.createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, rawKey]),
    format: "der",
    type: "spki",
  });
}

// Throws InstallError on any failure; returns { trustedComment } on success.
function verifyMinisign({ publicKey, signature, message }) {
  const pk = typeof publicKey === "string" ? parseMinisignPublicKey(publicKey) : publicKey;
  const sig = typeof signature === "string" ? parseMinisignSignature(signature) : signature;
  if (!pk.keyId.equals(sig.keyId)) {
    throw new InstallError(
      `minisign signature: key id ${sig.keyId.toString("hex")} does not match the embedded key ${pk.keyId.toString("hex")}`
    );
  }
  const key = ed25519KeyObject(pk.key);
  const prehash = crypto.createHash("blake2b512").update(message).digest();
  if (!crypto.verify(null, prehash, key, sig.signature)) {
    throw new InstallError("minisign signature: signature does not match checksums.txt");
  }
  const globalMessage = Buffer.concat([sig.signature, Buffer.from(sig.trustedComment, "utf8")]);
  if (!crypto.verify(null, globalMessage, key, sig.globalSignature)) {
    throw new InstallError("minisign signature: global signature (trusted comment) invalid");
  }
  return { trustedComment: sig.trustedComment };
}

// ---------------------------------------------------------------------------
// HTTP (follows up to MAX_REDIRECTS redirects; http: allowed for test servers)

function getResponse(url, depth = 0) {
  if (depth > MAX_REDIRECTS) return Promise.reject(new InstallError(`too many redirects fetching ${url}`));
  return new Promise((resolve, reject) => {
    const client = url.startsWith("http:") ? http : https;
    client
      .get(url, { headers: { "user-agent": `@usearete/a4 ${pkg.version}` } }, (response) => {
        const { statusCode, headers } = response;
        if ([301, 302, 303, 307, 308].includes(statusCode)) {
          response.resume();
          if (!headers.location) return reject(new InstallError(`redirect from ${url} has no location header`));
          return resolve(getResponse(new URL(headers.location, url).toString(), depth + 1));
        }
        resolve(response);
      })
      .on("error", reject);
  });
}

async function fetchToFile(url, dest, mode) {
  const response = await getResponse(url);
  if (response.statusCode !== 200) {
    response.resume();
    return response.statusCode;
  }
  await new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest, { mode });
    response.on("error", (err) => { file.destroy(); reject(err); });
    file.on("error", reject);
    file.on("finish", () => file.close(resolve));
    response.pipe(file);
  });
  return 200;
}

// ---------------------------------------------------------------------------
// Install

function stillPublishing(version, url) {
  return new InstallError(`Release ${version} is still publishing; retry in a few minutes (missing ${url})`);
}

// Download + verify into a fresh temp dir. Returns { dir, binary, checksums, signature }.
async function downloadRelease({ version = pkg.version, env = process.env, platform = process.platform, arch = process.arch, publicKey = MINISIGN_PUBLIC_KEY, log = () => {} } = {}) {
  const asset = pickAsset(platform, arch);
  if (!asset) {
    throw new InstallError(
      `no prebuilt a4 binary for ${platform}-${arch}. Build from source: https://github.com/AreteA4/arete#building-from-source`
    );
  }
  const base = releaseBaseUrl(version, env);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "a4-install-"));
  const binary = path.join(dir, platform === "win32" ? "a4.exe" : "a4");
  const checksums = path.join(dir, "checksums.txt");
  const signature = path.join(dir, "checksums.txt.minisig");
  log(`Installing a4 ${version} (${platform}-${arch}) from ${base}`);

  for (const [name, dest, mode] of [[asset, binary, 0o755], ["checksums.txt", checksums, 0o644], ["checksums.txt.minisig", signature, 0o644]]) {
    const url = `${base}/${name}`;
    let status;
    try {
      status = await fetchToFile(url, dest, mode);
    } catch (err) {
      throw new InstallError(`download of ${url} failed: ${err.message}. Check network/proxy settings and retry`);
    }
    if (status === 404) throw stillPublishing(version, url);
    if (status !== 200) throw new InstallError(`download of ${url} failed (HTTP ${status}). Retry in a few minutes`);
  }

  const checksumText = fs.readFileSync(checksums, "utf8");
  verifyMinisign({ publicKey, signature: fs.readFileSync(signature, "utf8"), message: Buffer.from(checksumText, "utf8") });
  const expected = parseChecksums(checksumText).get(asset);
  if (!expected) throw new InstallError(`checksums.txt has no entry for ${asset}; the release may be incomplete, retry in a few minutes`);
  const actual = await sha256File(binary);
  if (actual !== expected) {
    throw new InstallError(
      `sha256 mismatch for ${asset} (expected ${expected}, got ${actual}). Retry; if it persists, report at https://github.com/AreteA4/arete/issues`
    );
  }
  if (platform !== "win32") fs.chmodSync(binary, 0o755);
  return { dir, binary, checksums, signature };
}

// Run the full install path. `quiet` captures stdout and returns the A4_BIN path.
// Returns { status, a4Bin }.
async function runInstall(passthrough = [], options = {}) {
  const { quiet = false, env = process.env, stdio = "inherit", log = (m) => process.stderr.write(`${m}\n`) } = options;
  const release = await downloadRelease({ ...options, log });
  try {
    const args = ["self", "install", "--source", "npm", "--checksums", release.checksums, "--signature", release.signature, ...passthrough];
    const result = spawnSync(release.binary, args, {
      stdio: quiet ? ["ignore", "pipe", "inherit"] : stdio,
      encoding: "utf8",
      env: { ...env, [RECURSION_SENTINEL]: "1" },
    });
    if (result.error) throw new InstallError(`failed to run ${release.binary}: ${result.error.message}`);
    let a4Bin = null;
    if (quiet && result.stdout) {
      const line = result.stdout.split(/\r?\n/).reverse().find((l) => l.startsWith("A4_BIN="));
      if (line) a4Bin = line.slice("A4_BIN=".length).trim();
    }
    return { status: result.status ?? 1, a4Bin };
  } finally {
    fs.rmSync(release.dir, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------
// Launcher

function runBinary(binary, argv, { env = process.env, stdio = "inherit" } = {}) {
  const result = spawnSync(binary, argv, { stdio, env: { ...env, [RECURSION_SENTINEL]: "1" }, encoding: "utf8" });
  if (result.error) {
    const hint = result.error.code === "EACCES" ? ` Try: chmod +x "${binary}"` : "";
    throw new InstallError(`failed to run ${binary}: ${result.error.message}.${hint} Reinstall with: npx @usearete/a4 install`);
  }
  return result;
}

// Returns the process exit code. Options are for tests (env, homedir, publicKey, stdio).
async function launch(argv, options = {}) {
  const { env = process.env, homedir = os.homedir(), stdio = "inherit", log = (m) => process.stderr.write(`${m}\n`) } = options;
  if (env[RECURSION_SENTINEL] === "1") {
    log("Refusing to recursively launch the Arete CLI shim.");
    return 1;
  }
  if (argv[0] === "install") {
    return (await runInstall(argv.slice(1), { ...options, env, log })).status;
  }

  let receipt = readReceipt(env, homedir);
  if (!receipt) {
    log(`a4 is not installed yet (no ${receiptPath(env, homedir)}); installing a4 ${pkg.version} first`);
    const { status, a4Bin } = await runInstall([], { ...options, env, log, quiet: true });
    if (status !== 0 || !a4Bin) {
      throw new InstallError(`a4 self install failed (exit ${status}). Run: npx @usearete/a4 install`);
    }
    receipt = { binary: a4Bin };
  }
  const result = runBinary(receipt.binary, argv, { env, stdio });
  if (options.onResult) options.onResult(result);
  return result.status ?? 1;
}

async function main() {
  try {
    process.exitCode = await launch(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`error: ${err instanceof InstallError ? err.message : err.stack || err}\n`);
    process.exitCode = 1;
  }
}

if (require.main === module) main();

module.exports = {
  ASSETS,
  InstallError,
  MINISIGN_PUBLIC_KEY,
  RECURSION_SENTINEL,
  areteHome,
  downloadRelease,
  launch,
  parseChecksums,
  parseMinisignPublicKey,
  parseMinisignSignature,
  pickAsset,
  readReceipt,
  receiptPath,
  releaseBaseUrl,
  runInstall,
  sha256File,
  verifyMinisign,
};
