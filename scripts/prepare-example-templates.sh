#!/usr/bin/env bash

set -euo pipefail

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 0.5.0"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STACK_ID="${ARETE_TEMPLATE_STACK_ID:-ore}"
CONFIG_PATH="${ARETE_TEMPLATE_CONFIG_PATH:-}"

if [[ -n "$CONFIG_PATH" && "$CONFIG_PATH" != /* ]]; then
    CONFIG_PATH="$ROOT_DIR/$CONFIG_PATH"
fi

"$ROOT_DIR/scripts/update-example-versions.sh" "$VERSION"

A4_CMD=(cargo run --quiet --manifest-path "$ROOT_DIR/cli/Cargo.toml" --)
if [[ -n "$CONFIG_PATH" ]]; then
    A4_CMD+=(--config "$CONFIG_PATH")
fi

echo ""
echo "Generating Ore SDK templates from stack source: $STACK_ID"

ARETE_TELEMETRY_DISABLED=1 "${A4_CMD[@]}" sdk create typescript "$STACK_ID" \
    --output "$ROOT_DIR/examples/ore-react/src/generated/ore-stack.ts" \
    --package-name "@usearete/react"

ARETE_TELEMETRY_DISABLED=1 "${A4_CMD[@]}" sdk create typescript "$STACK_ID" \
    --output "$ROOT_DIR/examples/ore-typescript/src/generated/ore-stack.ts" \
    --package-name "@usearete/sdk"
