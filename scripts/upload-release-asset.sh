#!/usr/bin/env bash

set -euo pipefail

TAG="${1:-}"
ASSET_PATH="${2:-}"

if [[ -z "$TAG" || -z "$ASSET_PATH" ]]; then
  echo "Usage: $0 <release-tag> <asset-path>"
  exit 1
fi

if [[ ! -f "$ASSET_PATH" ]]; then
  echo "Release asset not found: $ASSET_PATH"
  exit 1
fi

ASSET_NAME="$(basename "$ASSET_PATH")"
RELEASE="$(gh release view "$TAG" --json assets)"

if jq -e --arg name "$ASSET_NAME" 'any(.assets[]; .name == $name)' <<<"$RELEASE" >/dev/null; then
  DOWNLOAD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/release-asset.XXXXXX")"
  trap 'rm -rf "$DOWNLOAD_DIR"' EXIT
  gh release download "$TAG" --pattern "$ASSET_NAME" --dir "$DOWNLOAD_DIR"

  if cmp -s "$ASSET_PATH" "$DOWNLOAD_DIR/$ASSET_NAME"; then
    echo "Skipping $ASSET_NAME; identical asset already exists on $TAG"
    exit 0
  fi

  echo "Refusing to replace non-identical asset $ASSET_NAME on $TAG"
  exit 1
fi

gh release upload "$TAG" "$ASSET_PATH"
