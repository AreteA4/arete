#!/usr/bin/env node

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binName = process.platform === "win32" ? "a4.exe" : "a4";
const localBinPath = path.join(__dirname, binName);

const RECURSION_SENTINEL = "ARETE_A4_LAUNCHER_ACTIVE";

function canonicalPath(candidate) {
  try {
    return fs.realpathSync(candidate);
  } catch {
    return path.resolve(candidate);
  }
}

function pathCandidates(env = process.env, platform = process.platform) {
  const names = platform === "win32" ? ["a4.exe"] : ["a4"];
  return (env.PATH || "")
    .split(path.delimiter)
    .filter(Boolean)
    .flatMap((directory) => names.map((name) => path.join(directory, name)));
}

function isExecutableFile(candidate, platform = process.platform) {
  try {
    if (!fs.statSync(candidate).isFile()) return false;
    if (platform !== "win32") {
      fs.accessSync(candidate, fs.constants.X_OK);
    }
    return true;
  } catch {
    return false;
  }
}

// Try the concrete postinstall destination first, then every PATH candidate
// except this JavaScript launcher (including symlinks in node_modules/.bin).
function getBinaryPath(env = process.env, platform = process.platform) {
  const launcherPaths = new Set(
    [__filename, process.argv[1]].filter(Boolean).map(canonicalPath)
  );

  // 1. Check for bundled binary (npm postinstall)
  if (
    isExecutableFile(localBinPath, platform)
    && !launcherPaths.has(canonicalPath(localBinPath))
  ) {
    return localBinPath;
  }

  // 2. Check system PATH (cargo install, manual install)
  for (const candidate of pathCandidates(env, platform)) {
    if (!isExecutableFile(candidate, platform)) continue;
    if (launcherPaths.has(canonicalPath(candidate))) continue;
    return candidate;
  }

  return null;
}

function main() {
  if (process.env[RECURSION_SENTINEL] === "1") {
    console.error("Refusing to recursively launch the Arete CLI shim.");
    process.exit(1);
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
    env: { ...process.env, [RECURSION_SENTINEL]: "1" },
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
}

if (require.main === module) main();

module.exports = { canonicalPath, getBinaryPath, pathCandidates };
