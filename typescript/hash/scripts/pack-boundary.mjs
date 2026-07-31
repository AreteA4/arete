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
const privateArtifactNeedles = [
  "DECODER_FIXTURE_",
  "DecoderFixtureAccountDecodeErrorCategory",
  "DecoderFixtureCaseV1",
  "DecoderFixtureExpectedV1",
  "DecoderFixturePrivateDiagnosticsV1",
  "DecoderFixturePublicValueDigest",
  "DecoderFixtureSetV1",
  "digestDecoderFixturePublicValueV1",
  "hashDecoderFixtureSetV1",
  "validateDecoderFixtureSetV1",
  "fixture.ts",
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

function assertPublicArtifactsOnly(unpackedRoot) {
  for (const file of expectedFiles.filter((path) => /\.(?:cjs|js|d\.ts|map)$/.test(path))) {
    const contents = readFileSync(join(unpackedRoot, file), "utf8");
    for (const needle of privateArtifactNeedles) {
      assert.equal(
        contents.includes(needle),
        false,
        `${file} exposes private decoder fixture detail ${needle}`,
      );
    }
  }
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
  assertPublicArtifactsOnly(unpackedRoot);

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
