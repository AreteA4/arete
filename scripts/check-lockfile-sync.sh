#!/bin/bash
# Verify every package-lock.json stays in sync with its sibling package.json.
# npm ci does not reliably reject spec drift for linked/registry deps, so the
# root dependency specs are compared directly.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

failed=0
while IFS= read -r lockfile; do
  dir="${lockfile%/package-lock.json}"
  manifest="$dir/package.json"
  [ -f "$manifest" ] || continue
  if ! node - "$manifest" "$lockfile" <<'EOF'
const [manifestPath, lockPath] = process.argv.slice(2);
const fs = require('fs');
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
const root = lock.packages?.[''] ?? {};
const sections = ['dependencies', 'devDependencies', 'peerDependencies'];
const problems = [];
for (const section of sections) {
  const manifestDeps = manifest[section] ?? {};
  const lockDeps = root[section] ?? {};
  for (const [name, spec] of Object.entries(manifestDeps)) {
    if (!(name in lockDeps)) {
      problems.push(`${section}.${name}: missing from lockfile root (package.json has "${spec}")`);
    } else if (lockDeps[name] !== spec) {
      problems.push(`${section}.${name}: package.json "${spec}" != lockfile "${lockDeps[name]}"`);
    }
  }
  for (const name of Object.keys(lockDeps)) {
    if (!(name in manifestDeps)) {
      problems.push(`${section}.${name}: present in lockfile root but not in package.json`);
    }
  }
}
if (problems.length > 0) {
  console.error(problems.map(p => `  ${p}`).join('\n'));
  process.exit(1);
}
EOF
  then
    echo "::error::Lockfile out of sync: $lockfile"
    failed=1
  fi
done < <(git ls-files '*package-lock.json')

if [ "$failed" -ne 0 ]; then
  echo "Run 'npm install --package-lock-only' in the affected package and commit the result."
  exit 1
fi
echo "All package-lock.json files are in sync with their package.json"
