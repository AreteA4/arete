const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { canonicalPath, getBinaryPath } = require("./a4.js");

test("skips a symlink to the launcher and selects the next native binary", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "arete-a4-launcher-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const shimDir = path.join(root, "shim");
  const nativeDir = path.join(root, "native");
  fs.mkdirSync(shimDir);
  fs.mkdirSync(nativeDir);
  fs.symlinkSync(__dirname + "/a4.js", path.join(shimDir, "a4"));
  const native = path.join(nativeDir, "a4");
  fs.writeFileSync(native, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

  const selected = getBinaryPath({ PATH: [shimDir, nativeDir].join(path.delimiter) }, "darwin");
  assert.equal(canonicalPath(selected), canonicalPath(native));
});

test("returns null when PATH only resolves to the launcher", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "arete-a4-launcher-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.symlinkSync(__dirname + "/a4.js", path.join(root, "a4"));
  assert.equal(getBinaryPath({ PATH: root }, "darwin"), null);
});

test(
  "skips non-executable files and directories before a native binary",
  { skip: process.platform === "win32" },
  (t) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "arete-a4-launcher-"));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    const fileDir = path.join(root, "file");
    const directoryDir = path.join(root, "directory");
    const nativeDir = path.join(root, "native");
    fs.mkdirSync(fileDir);
    fs.mkdirSync(directoryDir);
    fs.mkdirSync(nativeDir);
    fs.writeFileSync(path.join(fileDir, "a4"), "not executable\n", { mode: 0o644 });
    fs.mkdirSync(path.join(directoryDir, "a4"));
    const native = path.join(nativeDir, "a4");
    fs.writeFileSync(native, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

    const selected = getBinaryPath({
      PATH: [fileDir, directoryDir, nativeDir].join(path.delimiter),
    });
    assert.equal(canonicalPath(selected), canonicalPath(native));
  }
);

test("skips Windows command shims and directories before a native binary", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "arete-a4-launcher-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const shimDir = path.join(root, "shim");
  const directoryDir = path.join(root, "directory");
  const nativeDir = path.join(root, "native");
  fs.mkdirSync(shimDir);
  fs.mkdirSync(directoryDir);
  fs.mkdirSync(nativeDir);
  fs.writeFileSync(path.join(shimDir, "a4"), "#!/bin/sh\n");
  fs.writeFileSync(path.join(shimDir, "a4.cmd"), "@echo off\n");
  fs.writeFileSync(path.join(shimDir, "a4.bat"), "@echo off\n");
  fs.mkdirSync(path.join(directoryDir, "a4.exe"));
  const native = path.join(nativeDir, "a4.exe");
  fs.writeFileSync(native, "native binary placeholder\n");

  const selected = getBinaryPath({
    PATH: [shimDir, directoryDir, nativeDir].join(path.delimiter),
  }, "win32");
  assert.equal(canonicalPath(selected), canonicalPath(native));
});

test("recursion sentinel stops the launcher before resolving another a4", () => {
  const result = spawnSync(process.execPath, [path.join(__dirname, "a4.js")], {
    encoding: "utf8",
    env: { ...process.env, ARETE_A4_LAUNCHER_ACTIVE: "1" },
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Refusing to recursively launch/);
});
