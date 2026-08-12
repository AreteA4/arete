#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-template-versions-test.XXXXXX")"
EXAMPLES_DIR="$TEST_DIR/examples"

cleanup() {
  rm -rf "$TEST_DIR"
}

trap cleanup EXIT

mkdir -p "$EXAMPLES_DIR"
for template in ore-react ore-rust ore-typescript; do
  cp -R "$ROOT_DIR/examples/$template" "$EXAMPLES_DIR/$template"
done

# Exercise the converter's requirement to preserve bundled, non-Arete paths.
mkdir -p "$EXAMPLES_DIR/ore-rust/template-helper/src"
cat >"$EXAMPLES_DIR/ore-rust/template-helper/Cargo.toml" <<'EOF'
[package]
name = "template-helper"
version = "0.1.0"
edition = "2021"
EOF
cat >"$EXAMPLES_DIR/ore-rust/template-helper/src/lib.rs" <<'EOF'
pub fn helper() {}
EOF
node - "$EXAMPLES_DIR/ore-rust/Cargo.toml" <<'EOF'
const fs = require('node:fs');
const manifestPath = process.argv[2];
const manifest = fs.readFileSync(manifestPath, 'utf8');
fs.writeFileSync(
  manifestPath,
  manifest.replace('async-trait = "0.1"', 'async-trait = "0.1"\ntemplate-helper = { path = "template-helper" }'),
);
EOF

ARETE_EXAMPLES_DIR="$EXAMPLES_DIR" \
  "$ROOT_DIR/scripts/update-example-versions.sh" 0.5.0

node - "$EXAMPLES_DIR" <<'EOF'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const examplesDir = process.argv[2];
const rustManifest = fs.readFileSync(
  path.join(examplesDir, 'ore-rust', 'Cargo.toml'),
  'utf8',
);

assert.match(
  rustManifest,
  /arete-sdk\s*=\s*\{\s*package\s*=\s*"arete-a4-sdk",\s*version\s*=\s*"0\.5"\s*}/,
);
assert.match(
  rustManifest,
  /template-helper\s*=\s*\{\s*path\s*=\s*"template-helper"\s*}/,
);
assert.doesNotMatch(rustManifest, /^\[patch\.crates-io]$/m);

for (const template of ['ore-react', 'ore-rust', 'ore-typescript']) {
  const manifestPath = path.join(examplesDir, template, template === 'ore-rust' ? 'Cargo.toml' : 'package.json');
  const manifest = fs.readFileSync(manifestPath, 'utf8');
  assert.doesNotMatch(
    manifest,
    /path\s*=\s*"[^"]*(?:hyperstack-oss|\.\.\/\.\.\/(?:arete|interpreter|arete-macros|rust\/arete))/,
    `${template} retains a source-checkout Arete path`,
  );

  const sections = manifest.split(/^\s*(?=\[)/m);
  for (const section of sections) {
    if (!section.startsWith('[patch.crates-io]')) continue;
    const entries = section
      .split('\n')
      .slice(1)
      .filter((line) => line.trim() !== '' && !line.trim().startsWith('#'));
    assert.notEqual(entries.length, 0, `${template} retains an empty [patch.crates-io] section`);
  }
}

for (const template of ['ore-react', 'ore-typescript']) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(examplesDir, template, 'package.json'), 'utf8'),
  );
  for (const [name, version] of Object.entries(manifest.dependencies ?? {})) {
    if (name.startsWith('@usearete/') || name.startsWith('arete-')) {
      assert.equal(version, '^0.5', `${template} did not update ${name}`);
    }
  }
}
EOF

cargo metadata \
  --manifest-path "$EXAMPLES_DIR/ore-rust/Cargo.toml" \
  --format-version 1 \
  --no-deps \
  > /dev/null

printf 'Example template version conversion tests passed\n'
