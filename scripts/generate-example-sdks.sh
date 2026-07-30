#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONFIG_PATH="${ARETE_TEMPLATE_CONFIG_PATH:-$ROOT_DIR/stacks/ore/arete.toml}"
EXTENSIONS_PATH="${ARETE_TEMPLATE_EXTENSIONS_PATH:-$ROOT_DIR/stacks/ore/extensions}"

if [[ -n "$CONFIG_PATH" && "$CONFIG_PATH" != /* ]]; then
    CONFIG_PATH="$ROOT_DIR/$CONFIG_PATH"
fi

if [[ -n "$EXTENSIONS_PATH" && "$EXTENSIONS_PATH" != /* ]]; then
    EXTENSIONS_PATH="$ROOT_DIR/$EXTENSIONS_PATH"
fi

A4_CMD=(cargo run --quiet --manifest-path "$ROOT_DIR/cli/Cargo.toml" --)
if [[ -n "$CONFIG_PATH" ]]; then
    A4_CMD+=(--config "$CONFIG_PATH")
fi

echo ""
echo "Building local ORE stack AST"
cargo clean --quiet --manifest-path "$ROOT_DIR/stacks/ore/Cargo.toml" --package ore-stack
cargo build --quiet --locked --manifest-path "$ROOT_DIR/stacks/ore/Cargo.toml"

ORE_MANIFEST_PATH="$ROOT_DIR/stacks/ore/.arete/OreStream.stack-manifest.json"
if [[ ! -f "$ORE_MANIFEST_PATH" ]]; then
    echo "Expected ORE StackManifest was not generated: $ORE_MANIFEST_PATH" >&2
    exit 1
fi

echo "Generating example SDKs from StackManifest: $ORE_MANIFEST_PATH"

ARETE_TELEMETRY_DISABLED=1 "${A4_CMD[@]}" sdk create --manifest "$ORE_MANIFEST_PATH" --ts \
    --output "$ROOT_DIR/examples/ore-react/src/generated/ore-stack.ts" \
    --package-name "@usearete/react" \
    --extensions "$EXTENSIONS_PATH"

ARETE_TELEMETRY_DISABLED=1 "${A4_CMD[@]}" sdk create --manifest "$ORE_MANIFEST_PATH" --ts \
    --output "$ROOT_DIR/examples/ore-typescript/src/generated/ore-stack.ts" \
    --package-name "@usearete/sdk" \
    --extensions "$EXTENSIONS_PATH"

RUST_SDK_OUTPUT="$ROOT_DIR/examples/ore-rust/src/generated/ore"
RUST_SDK_TMP="$(mktemp -d "${RUST_SDK_OUTPUT}.tmp.XXXXXX")"
trap 'rm -rf "$RUST_SDK_TMP"' EXIT

ARETE_TELEMETRY_DISABLED=1 "${A4_CMD[@]}" sdk create --manifest "$ORE_MANIFEST_PATH" --rust \
    --output "$RUST_SDK_TMP" \
    --module

rm -rf "$RUST_SDK_OUTPUT"
mv "$RUST_SDK_TMP" "$RUST_SDK_OUTPUT"
