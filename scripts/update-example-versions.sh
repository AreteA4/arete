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
EXAMPLES_DIR="$ROOT_DIR/examples"

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
    
    sed -i.bak -E "s/(arete-[a-z-]+) = \"[0-9]+\.[0-9]+(\.[0-9]+)?\"/\1 = \"$MAJOR_MINOR\"/g" "$file"
    sed -i.bak -E "s/(arete-[a-z-]+ = \{[^}]*version = \")[0-9]+\.[0-9]+(\.[0-9]+)?(\".*)$/\1$MAJOR_MINOR\3/g" "$file"
    perl -i.bak -pe 'if (/^\s*arete-[^=]+\s*=\s*\{/) { s/path = "[^"]+",\s*//; s/,\s*path = "[^"]+"//; }' "$file"

    # Clean up backup files
    rm -f "$file.bak"
    
    echo "  Updated Arete Rust dependencies to $MAJOR_MINOR"
    UPDATED_FILES+=("$file")
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
    # Skip files that only use path dependencies (like ore-server)
    if grep -q 'arete-.*=.*"[0-9]' "$file" 2>/dev/null; then
        update_cargo_toml "$file"
    else
        echo "Skipping $file (no semver arete deps)"
    fi
done < <(find "$EXAMPLES_DIR" -name "Cargo.toml" -not -path "*/target/*" -print0)

echo ""
echo "=== Summary ==="
echo "Updated ${#UPDATED_FILES[@]} files"
if [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo "(dry run - no actual changes made)"
fi
