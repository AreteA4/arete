import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporaryRoot = mkdtempSync(join(tmpdir(), "arete-hash-pack-"));
const consumerRoot = join(temporaryRoot, "consumer");
const expectedFiles = [
  "LICENSE",
  "README.md",
  "dist/index.cjs",
  "dist/index.cjs.map",
  "dist/index.d.ts",
  "dist/index.js",
  "dist/index.js.map",
  "package.json",
];
const removedContractPatterns = [
  /DecoderFixture(?:Case|Expected|PrivateDiagnostics|Set)V1/,
  /(?:digestDecoderFixturePublicValue|hashDecoderFixtureSet|validateDecoderFixtureSet)V1/,
  /arete\.decoder-fixtures\/v1/,
];

function npm(args, cwd, stdio = ["ignore", "pipe", "pipe"]) {
  return execFileSync(process.platform === "win32" ? "npm.cmd" : "npm", args, {
    cwd,
    encoding: "utf8",
    stdio,
  });
}

function pack(args) {
  const result = JSON.parse(npm(["pack", "--json", "--ignore-scripts", ...args], packageRoot));
  assert.equal(result.length, 1, "npm pack must produce exactly one tarball");
  return result[0];
}

function packedFileNames(result) {
  return result.files.map(({ path }) => path).sort();
}

function listPackageFiles(root, current = root) {
  const files = [];
  for (const entry of readdirSync(current)) {
    const path = join(current, entry);
    if (statSync(path).isDirectory()) {
      files.push(...listPackageFiles(root, path));
    } else {
      files.push(relative(root, path));
    }
  }
  return files.sort();
}

function assertCurrentContractsOnly(unpackedRoot) {
  for (const file of expectedFiles.filter((path) => /\.(?:cjs|js|d\.ts|map)$/.test(path))) {
    const contents = readFileSync(join(unpackedRoot, file), "utf8");
    for (const pattern of removedContractPatterns) {
      assert.equal(
        pattern.test(contents),
        false,
        `${file} exposes removed fixture V1 contract ${pattern}`,
      );
    }
  }

  const declarations = readFileSync(join(unpackedRoot, "dist/index.d.ts"), "utf8");
  assert(declarations.includes("interface DecoderFixtureSetV2"));
  assert(declarations.includes("interface HostedManagedProgramReleaseV2"));
  assert(declarations.includes("interface HostedPrivateProgramReleaseV3"));
  assert(declarations.includes("interface SolanaExecutableIdentityV1"));
  const fixtureCase = declarations.match(
    /interface DecoderFixtureCaseV2 \{([\s\S]*?)\n\}/,
  );
  assert(fixtureCase, "packed declarations must expose DecoderFixtureCaseV2");
  assert.equal(
    /readonly address\??:/.test(fixtureCase[1]),
    false,
    "DecoderFixtureCaseV2 must remain address-free",
  );
}

try {
  const dryRun = pack(["--dry-run"]);
  assert.deepEqual(packedFileNames(dryRun), expectedFiles, "npm pack --dry-run crossed the package boundary");

  const packed = pack(["--pack-destination", temporaryRoot]);
  assert.deepEqual(packedFileNames(packed), expectedFiles, "npm pack crossed the package boundary");

  const tarball = join(temporaryRoot, packed.filename);
  execFileSync("tar", ["-xzf", tarball, "-C", temporaryRoot]);
  const unpackedRoot = join(temporaryRoot, "package");
  assert.deepEqual(listPackageFiles(unpackedRoot), expectedFiles, "unpacked tarball contains unexpected files");

  const manifest = JSON.parse(readFileSync(join(unpackedRoot, "package.json"), "utf8"));
  assert.equal(manifest.name, "@usearete/hash");
  assert.equal(manifest.license, "MIT");
  assert.equal(manifest.sideEffects, false);
  assert.deepEqual(manifest.files, ["dist", "README.md", "LICENSE"]);
  assert.deepEqual(manifest.exports, {
    ".": {
      types: "./dist/index.d.ts",
      import: "./dist/index.js",
      require: "./dist/index.cjs",
    },
  });
  assert.equal(manifest.main, "./dist/index.cjs");
  assert.equal(manifest.module, "./dist/index.js");
  assert.equal(manifest.types, "./dist/index.d.ts");
  assertCurrentContractsOnly(unpackedRoot);

  mkdirSync(consumerRoot);
  writeFileSync(
    join(consumerRoot, "package.json"),
    `${JSON.stringify({ name: "hash-packed-smoke", private: true, type: "module" }, null, 2)}\n`,
  );
  npm(
    ["install", "--ignore-scripts", "--no-audit", "--no-fund", "--no-package-lock", tarball],
    consumerRoot,
    "inherit",
  );
  cpSync(join(packageRoot, "smoke"), consumerRoot, { recursive: true });

  execFileSync(process.execPath, ["esm.mjs"], { cwd: consumerRoot, stdio: "inherit" });
  execFileSync(process.execPath, ["cjs.cjs"], { cwd: consumerRoot, stdio: "inherit" });
  execFileSync(
    process.execPath,
    [
      join(packageRoot, "node_modules", "typescript", "bin", "tsc"),
      "--noEmit",
      "--strict",
      "--skipLibCheck",
      "--target",
      "ES2022",
      "--module",
      "NodeNext",
      "--moduleResolution",
      "NodeNext",
      join(consumerRoot, "types.ts"),
    ],
    { cwd: consumerRoot, stdio: "inherit" },
  );

  console.log(`Pack boundary and ESM/CJS/type smokes passed for ${packed.filename}`);
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
