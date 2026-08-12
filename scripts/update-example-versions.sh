#!/usr/bin/env bash
#
# Updates all Arete package versions in examples/ to the specified version.
# Handles both package.json (npm) and Cargo.toml (rust) files.
#
# Usage:
#   ./scripts/update-example-versions.sh <version>
#   ./scripts/update-example-versions.sh 0.5.0
#   ./scripts/update-example-versions.sh 0.5.0 --dry-run
#
# This script is called by the release pipeline before bundling templates.
# It converts any file:/path references to semver and updates existing semver refs.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN="${2:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--dry-run]"
    echo "Example: $0 0.5.0"
    exit 1
fi

# Extract major.minor for semver range (e.g., 0.5.0 -> 0.5)
MAJOR_MINOR="${VERSION%.*}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
EXAMPLES_DIR="${ARETE_EXAMPLES_DIR:-$ROOT_DIR/examples}"

echo "Updating examples to version: $VERSION (semver range: ^$MAJOR_MINOR)"
echo "Examples directory: $EXAMPLES_DIR"
[[ "$DRY_RUN" == "--dry-run" ]] && echo "DRY RUN - no files will be modified"
echo ""

# Track what we update
UPDATED_FILES=()

update_package_json() {
    local file="$1"
    echo "Processing: $file"
    
    if [[ "$DRY_RUN" == "--dry-run" ]]; then
        # Show what would change
        grep -E '"(@usearete/[^"]+|arete-[^"]+)":' "$file" || true
        return
    fi
    
    # Use node for reliable JSON manipulation
    node -e "
        const fs = require('fs');
        const pkg = JSON.parse(fs.readFileSync('$file', 'utf8'));
        let modified = false;
        
        for (const depType of ['dependencies', 'devDependencies', 'peerDependencies']) {
            if (pkg[depType]) {
                for (const [name, version] of Object.entries(pkg[depType])) {
                    if (name.startsWith('arete-') || name.startsWith('@usearete/')) {
                        pkg[depType][name] = '^$MAJOR_MINOR';
                        console.log('  Updated:', name, version, '->', '^$MAJOR_MINOR');
                        modified = true;
                    }
                }
            }
        }
        
        if (modified) {
            fs.writeFileSync('$file', JSON.stringify(pkg, null, 2) + '\n');
        }
    "
    UPDATED_FILES+=("$file")
}

update_cargo_toml() {
    local file="$1"
    echo "Processing: $file"
    
    if [[ "$DRY_RUN" == "--dry-run" ]]; then
        # Show what would change
        grep -E 'arete-' "$file" || true
        return
    fi
    
    local status=0
    node - "$file" "$MAJOR_MINOR" <<'EOF' || status=$?
const fs = require('node:fs');

const [file, version] = process.argv.slice(2);
const source = fs.readFileSync(file, 'utf8');
const lines = source.split('\n');
const output = [];
let section = '';
let patchBlock = null;
let changed = false;

function isAreteCrate(name) {
  return name === 'arete' || name.startsWith('arete-');
}

function assignment(line) {
  const match = line.match(/^(\s*(?:"([^"]+)"|'([^']+)'|([A-Za-z0-9_-]+))\s*=\s*)(.*)$/);
  if (!match) return null;
  return {
    prefix: match[1],
    name: match[2] ?? match[3] ?? match[4],
    value: match[5],
  };
}

function targetsArete(line) {
  const entry = assignment(line);
  if (!entry) return false;
  const packageName = entry.value.match(/\bpackage\s*=\s*"([^"]+)"/)?.[1];
  return isAreteCrate(entry.name) || (packageName && isAreteCrate(packageName));
}

function isDependencySection(name) {
  return /^(?:dependencies|dev-dependencies|build-dependencies)$/.test(name)
    || /\.(?:dependencies|dev-dependencies|build-dependencies)$/.test(name);
}

function convertDependency(line) {
  if (!targetsArete(line)) return line;
  const entry = assignment(line);
  let value = entry.value;

  if (/^"[^"]*"/.test(value)) {
    value = value.replace(/^"[^"]*"/, `"${version}"`);
  } else if (value.trimStart().startsWith('{')) {
    value = value
      .replace(/\bpath\s*=\s*"[^"]*"\s*,\s*/g, '')
      .replace(/,\s*path\s*=\s*"[^"]*"(?=\s*})/g, '')
      .replace(/\bpath\s*=\s*"[^"]*"(?=\s*})/g, '');

    if (/\bversion\s*=\s*"[^"]*"/.test(value)) {
      value = value.replace(/\bversion\s*=\s*"[^"]*"/, `version = "${version}"`);
    } else if (/^\s*\{\s*}\s*(?:#.*)?$/.test(value)) {
      value = value.replace(/\{\s*}/, `{ version = "${version}" }`);
    } else {
      value = value.replace(/\{\s*/, `{ version = "${version}", `);
    }
  } else {
    return line;
  }

  const converted = `${entry.prefix}${value}`;
  if (converted !== line) changed = true;
  return converted;
}

function flushPatchBlock() {
  if (!patchBlock) return;
  const body = patchBlock.lines.filter((line) => !targetsArete(line));
  if (body.length !== patchBlock.lines.length) changed = true;

  const hasEntries = body.some((line) => {
    const trimmed = line.trim();
    return trimmed !== '' && !trimmed.startsWith('#');
  });

  if (hasEntries) {
    output.push(patchBlock.header, ...body);
  } else {
    changed = true;
    while (output.at(-1)?.trim() === '') output.pop();
    while (output.at(-1)?.trim().startsWith('#')) output.pop();
    output.push('');
  }
  patchBlock = null;
}

for (const line of lines) {
  const sectionMatch = line.trim().match(/^\[([^[]+)]$/);
  if (sectionMatch) {
    flushPatchBlock();
    section = sectionMatch[1];
    if (section === 'patch.crates-io') {
      patchBlock = { header: line, lines: [] };
    } else {
      output.push(line);
    }
    continue;
  }

  if (patchBlock) {
    patchBlock.lines.push(line);
  } else {
    output.push(isDependencySection(section) ? convertDependency(line) : line);
  }
}
flushPatchBlock();

const converted = output.join('\n');
const modified = changed && converted !== source;
if (modified) fs.writeFileSync(file, converted);
process.exit(modified ? 0 : 3);
EOF

    if [[ "$status" -eq 0 ]]; then
        echo "  Updated Arete Rust dependencies to $MAJOR_MINOR"
        UPDATED_FILES+=("$file")
    elif [[ "$status" -eq 3 ]]; then
        echo "  No Arete Rust dependency changes needed"
    else
        return "$status"
    fi
}

# Find and update all package.json files in examples (excluding node_modules)
echo "=== Updating package.json files ==="
while IFS= read -r -d '' file; do
    update_package_json "$file"
done < <(find "$EXAMPLES_DIR" -name "package.json" -not -path "*/node_modules/*" -print0)

echo ""

# Find and update all Cargo.toml files in examples (excluding target)
echo "=== Updating Cargo.toml files ==="
while IFS= read -r -d '' file; do
    update_cargo_toml "$file"
done < <(find "$EXAMPLES_DIR" -name "Cargo.toml" -not -path "*/target/*" -print0)

echo ""
echo "=== Summary ==="
echo "Updated ${#UPDATED_FILES[@]} files"
if [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo "(dry run - no actual changes made)"
fi
