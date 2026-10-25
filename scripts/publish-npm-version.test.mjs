import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./publish-npm-version.sh", import.meta.url));
const root = path.resolve(path.dirname(script), "..");

const read = (dir, relative) => JSON.parse(fs.readFileSync(path.join(dir, relative), "utf8"));

test("recovery staging gives React the release range for the SDK and the Kit adapter", (t) => {
  const staged = fs.mkdtempSync(path.join(os.tmpdir(), "arete-npm-stage-"));
  t.after(() => fs.rmSync(staged, { recursive: true, force: true }));
  const env = { ...process.env };
  delete env.NPM_TOKEN;
  execFileSync("bash", [script, "99.1.0", "--stage-only", staged], { env, stdio: "pipe" });

  const react = read(staged, "typescript/react/package.json");
  assert.equal(react.version, "99.1.0");
  for (const section of ["peerDependencies", "devDependencies"]) {
    assert.equal(react[section]["@usearete/sdk"], "^99.1.0", section);
    assert.equal(react[section]["@usearete/adapter-kit"], "^99.1.0", section);
  }

  const reactLock = read(staged, "typescript/react/package-lock.json");
  for (const section of ["peerDependencies", "devDependencies"]) {
    assert.equal(reactLock.packages[""][section]["@usearete/adapter-kit"], "^99.1.0", section);
  }
  assert.equal(reactLock.packages["../adapters/kit"].version, "99.1.0");
  assert.equal(reactLock.packages["../core"].version, "99.1.0");

  for (const relative of ["typescript/adapters/kit", "typescript/adapters/web3js"]) {
    const manifest = read(staged, `${relative}/package.json`);
    assert.equal(manifest.version, "99.1.0", relative);
    assert.equal(manifest.peerDependencies["@usearete/sdk"], "^99.1.0", relative);
  }

  // hash and protocol keep their own version line.
  const hash = read(root, "typescript/hash/package.json");
  assert.equal(read(staged, "typescript/hash/package.json").version, hash.version);
  if (fs.existsSync(path.join(root, "typescript/protocol/package.json"))) {
    const protocol = read(root, "typescript/protocol/package.json");
    const stagedProtocol = read(staged, "typescript/protocol/package.json");
    assert.equal(stagedProtocol.version, protocol.version);
    assert.deepEqual(stagedProtocol.dependencies, protocol.dependencies);
  }
});

test("stage-only refuses a directory that is not empty", (t) => {
  const staged = fs.mkdtempSync(path.join(os.tmpdir(), "arete-npm-stage-"));
  t.after(() => fs.rmSync(staged, { recursive: true, force: true }));
  fs.writeFileSync(path.join(staged, "keep"), "");
  assert.throws(() => execFileSync("bash", [script, "99.1.0", "--stage-only", staged], { stdio: "pipe" }));
  assert.ok(fs.existsSync(path.join(staged, "keep")));
});
