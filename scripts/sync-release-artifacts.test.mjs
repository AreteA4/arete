import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./sync-release-artifacts.sh", import.meta.url));

test("synchronizes release and linked package locks without changing registry resolutions", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "arete-release-sync-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const write = (relative, value) => {
    const target = path.join(root, relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, JSON.stringify(value, null, 2) + "\n");
  };
  write("core/package.json", { name: "@example/sdk", version: "0.29.1", dependencies: { zod: "^4" } });
  write("react/package.json", { name: "@example/react", version: "0.29.1", devDependencies: { "@example/sdk": "^0.29.0" } });
  const registry = { version: "4.0.0", resolved: "https://registry.npmjs.org/zod/-/zod-4.0.0.tgz", integrity: "fixture-integrity" };
  write("react/package-lock.json", {
    name: "@example/react", version: "0.29.0", lockfileVersion: 3,
    packages: {
      "": { name: "@example/react", version: "0.29.0", devDependencies: { "@example/sdk": "^0.29.0" } },
      "../core": { name: "@example/sdk", version: "0.29.0", dev: true, dependencies: { zod: "^3" } },
      "node_modules/@example/sdk": { resolved: "../core", link: true },
      "node_modules/zod": registry,
    },
  });
  write("hash/package.json", { name: "@example/hash", version: "0.6.1" });
  write("hash/package-lock.json", { name: "@example/hash", version: "0.6.0", lockfileVersion: 3, packages: { "": { name: "@example/hash", version: "0.6.0" } } });
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  execFileSync("git", ["add", "."], { cwd: root });
  const run = () => execFileSync("bash", [script], {
    env: { ...process.env, RELEASE_SYNC_ROOT: root, LC_ALL: "C", LANG: "C", LC_CTYPE: "C" },
    stdio: "pipe",
  });
  run();
  const lock = JSON.parse(fs.readFileSync(path.join(root, "react/package-lock.json"), "utf8"));
  assert.equal(lock.version, "0.29.1");
  assert.equal(lock.packages[""].version, "0.29.1");
  assert.equal(lock.packages[""].devDependencies["@example/sdk"], "^0.29.1");
  assert.equal(lock.packages["../core"].version, "0.29.1");
  assert.equal(lock.packages["../core"].dev, true);
  assert.deepEqual(lock.packages["../core"].dependencies, { zod: "^4" });
  assert.deepEqual(lock.packages["node_modules/@example/sdk"], { resolved: "../core", link: true });
  assert.deepEqual(lock.packages["node_modules/zod"], registry);
  const hash = JSON.parse(fs.readFileSync(path.join(root, "hash/package-lock.json"), "utf8"));
  assert.equal(hash.version, "0.6.1");
  assert.equal(hash.packages[""].version, "0.6.1");
  const first = fs.readFileSync(path.join(root, "react/package-lock.json"), "utf8");
  run();
  assert.equal(fs.readFileSync(path.join(root, "react/package-lock.json"), "utf8"), first);
});
