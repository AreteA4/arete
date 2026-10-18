#!/usr/bin/env bash
# Build the local TypeScript SDK packages so an example can use them.
#
# Usage: scripts/build-local-packages.sh [example-dir]
#   example-dir  the example to point at afterwards (default: examples/ore-react)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE_DIR="${1:-$REPO_ROOT/examples/ore-react}"

echo "Building local Arete packages..."

echo "Building @usearete/sdk (core SDK)..."
(cd "$REPO_ROOT/typescript/core" && npm install && npm run build)

echo "Building @usearete/react..."
(cd "$REPO_ROOT/typescript/react" && npm install && npm run build)

echo "All packages built successfully!"
echo ""
echo "Next steps:"
echo "1. cd $EXAMPLE_DIR"
echo "2. rm -rf node_modules package-lock.json"
echo "3. npm install"
echo "4. npm run dev"
