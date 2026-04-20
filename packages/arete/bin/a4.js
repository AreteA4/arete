#!/usr/bin/env node

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binName = process.platform === "win32" ? "a4.exe" : "a4";
const localBinPath = path.join(__dirname, binName);

// Try local binary first, then fall back to PATH
function getBinaryPath() {
  // 1. Check for bundled binary (npm postinstall)
  if (fs.existsSync(localBinPath)) {
    return localBinPath;
  }

  // 2. Check system PATH (cargo install, manual install)
  const whichCmd = process.platform === "win32" ? "where" : "which";
  const result = spawnSync(whichCmd, ["a4"], { encoding: "utf8" });
  if (result.status === 0 && result.stdout) {
    const systemBin = result.stdout.trim().split("\n")[0];
    if (fs.existsSync(systemBin)) {
      return systemBin;
    }
  }

  return null;
}

const binPath = getBinaryPath();

if (!binPath) {
  console.error(
    "Arete CLI binary not found. This usually means the postinstall script failed.\n" +
    "Try reinstalling: npm install @usearete/a4\n" +
    "\n" +
    "If the problem persists, you can install the CLI via Cargo:\n" +
    "  cargo install a4-cli"
  );
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
});

if (result.error) {
  if (result.error.code === "EACCES") {
    console.error(
      "Permission denied. Try running:\n" +
      `  chmod +x "${binPath}"`
    );
  } else {
    console.error("Failed to run Arete CLI:", result.error.message);
  }
  process.exit(1);
}

process.exit(result.status ?? 1);
