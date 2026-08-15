#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="${RELEASE_SYNC_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

node - "$ROOT_DIR" <<'EOF'
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const [root] = process.argv.slice(2);

function trackedFiles(pattern) {
  return execFileSync('git', ['ls-files', '--', pattern], {
    cwd: root,
    encoding: 'utf8',
  }).trim().split('\n').filter(Boolean);
}

function packageVersion(manifestPath) {
  const lines = fs.readFileSync(manifestPath, 'utf8').split('\n');
  let inPackage = false;

  for (const line of lines) {
    const section = line.trim().match(/^\[([^[]+)]$/);
    if (section) {
      inPackage = section[1] === 'package';
      continue;
    }
    if (inPackage) {
      const version = line.match(/^\s*version\s*=\s*"([^"]+)"/);
      if (version) return version[1];
    }
  }

  throw new Error(`Missing [package].version in ${path.relative(root, manifestPath)}`);
}

for (const relativeManifest of trackedFiles('*Cargo.toml')) {
  const manifestPath = path.join(root, relativeManifest);
  let changed = false;
  const lines = fs.readFileSync(manifestPath, 'utf8').split('\n').map(line => {
    const dependencyPath = line.match(/\bpath\s*=\s*"([^"]+)"/);
    const dependencyVersion = line.match(/\bversion\s*=\s*"([^"]+)"/);
    if (!dependencyPath || !dependencyVersion) return line;

    const dependencyManifest = path.resolve(
      path.dirname(manifestPath),
      dependencyPath[1],
      'Cargo.toml',
    );
    if (!fs.existsSync(dependencyManifest)) return line;

    const targetVersion = packageVersion(dependencyManifest);
    if (dependencyVersion[1] === targetVersion) return line;

    changed = true;
    return line.replace(
      /\bversion\s*=\s*"[^"]+"/,
      `version = "${targetVersion}"`,
    );
  });

  if (changed) {
    fs.writeFileSync(manifestPath, lines.join('\n'));
    console.log(`Synchronized local Cargo dependencies in ${relativeManifest}`);
  }
}

const dependencySections = [
  'dependencies',
  'devDependencies',
  'optionalDependencies',
  'peerDependencies',
];

const packageFiles = trackedFiles('*package.json');
const localPackages = new Map(packageFiles.map(relativeManifest => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, relativeManifest), 'utf8'));
  return [manifest.name, manifest.version];
}).filter(([name, version]) => name && version));

for (const relativeManifest of packageFiles) {
  const manifestPath = path.join(root, relativeManifest);
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  // Private packages (the examples) are never published, so their registry
  // ranges must stay installable from the current registry at all times —
  // including inside a release PR, before the new version exists on npm.
  // Pin them to the release line (^major.minor, matching
  // scripts/update-example-versions.sh) instead of an exact patch version
  // so the checked-in lockfile keeps satisfying the manifest.
  const privatePackage = manifest.private === true;
  let changed = false;

  for (const section of dependencySections) {
    for (const [name, spec] of Object.entries(manifest[section] ?? {})) {
      const targetVersion = localPackages.get(name);
      if (!targetVersion || typeof spec !== 'string') continue;

      let targetSpec = null;
      if (privatePackage) {
        if (!/^[~^]?\d+\.\d+(?:\.\d+(?:-[0-9A-Za-z.-]+)?)?$/.test(spec)) continue;
        targetSpec = `^${targetVersion.split('.').slice(0, 2).join('.')}`;
      } else {
        const registryRange = spec.match(/^([~^]?)\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/);
        if (!registryRange) continue;
        targetSpec = `${registryRange[1]}${targetVersion}`;
      }
      if (spec === targetSpec) continue;

      manifest[section][name] = targetSpec;
      changed = true;
    }
  }

  if (changed) {
    fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(`Synchronized local npm dependencies in ${relativeManifest}`);
  }
}

for (const relativeLock of trackedFiles('*package-lock.json')) {
  const lockPath = path.join(root, relativeLock);
  const manifestPath = path.join(path.dirname(lockPath), 'package.json');
  if (!fs.existsSync(manifestPath)) continue;

  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
  const lockRoot = lock.packages?.[''];
  if (!lockRoot) throw new Error(`Missing packages[""] in ${relativeLock}`);

  for (const section of dependencySections) {
    if (manifest[section]) {
      const synchronized = {};
      for (const name of Object.keys(lockRoot[section] ?? {})) {
        if (name in manifest[section]) synchronized[name] = manifest[section][name];
      }
      for (const [name, spec] of Object.entries(manifest[section])) {
        if (!(name in synchronized)) synchronized[name] = spec;
      }
      lockRoot[section] = synchronized;
    } else {
      delete lockRoot[section];
    }
  }

  fs.writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
}
EOF

while IFS= read -r lockfile; do
  manifest="${lockfile%Cargo.lock}Cargo.toml"
  [ -f "$ROOT_DIR/$manifest" ] || continue
  cargo metadata \
    --manifest-path "$ROOT_DIR/$manifest" \
    --format-version 1 \
    > /dev/null
done < <(git -C "$ROOT_DIR" ls-files '*Cargo.lock')
