#!/usr/bin/env bash

set -euo pipefail

CRATE_DIR="${1:-}"

if [[ -z "$CRATE_DIR" ]]; then
  echo "Usage: $0 <crate-directory>"
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST_PATH="$ROOT_DIR/$CRATE_DIR/Cargo.toml"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "Cargo manifest not found: $MANIFEST_PATH"
  exit 1
fi

MANIFEST_PATH="$(cd "$(dirname "$MANIFEST_PATH")" && pwd)/Cargo.toml"
METADATA="$(cargo metadata --locked --no-deps --format-version 1 --manifest-path "$MANIFEST_PATH")"
PACKAGE="$(jq -r --arg manifest "$MANIFEST_PATH" '.packages[] | select(.manifest_path == $manifest)' <<<"$METADATA")"
PACKAGE_NAME="$(jq -r '.name' <<<"$PACKAGE")"
PACKAGE_VERSION="$(jq -r '.version' <<<"$PACKAGE")"

if [[ -z "$PACKAGE_NAME" || "$PACKAGE_NAME" == "null" || -z "$PACKAGE_VERSION" || "$PACKAGE_VERSION" == "null" ]]; then
  echo "Unable to resolve package metadata for $MANIFEST_PATH"
  exit 1
fi

RESPONSE_FILE="$(mktemp "${TMPDIR:-/tmp}/crates-io-response.XXXXXX")"
trap 'rm -f "$RESPONSE_FILE"' EXIT

STATUS="$(curl --silent --show-error --retry 3 --retry-all-errors \
  --user-agent 'arete-release-workflow (https://github.com/AreteA4/arete)' \
  --output "$RESPONSE_FILE" \
  --write-out '%{http_code}' \
  "https://crates.io/api/v1/crates/$PACKAGE_NAME/$PACKAGE_VERSION")"

case "$STATUS" in
  200)
    if jq -e '.version.yanked == true' "$RESPONSE_FILE" >/dev/null; then
      echo "$PACKAGE_NAME@$PACKAGE_VERSION exists on crates.io but is yanked"
      exit 1
    fi
    echo "Skipping $PACKAGE_NAME@$PACKAGE_VERSION; already published"
    exit 0
    ;;
  404)
    ;;
  *)
    echo "Unable to check $PACKAGE_NAME@$PACKAGE_VERSION on crates.io (HTTP $STATUS)"
    cat "$RESPONSE_FILE"
    exit 1
    ;;
esac

echo "Publishing $PACKAGE_NAME@$PACKAGE_VERSION"
cargo publish --locked --manifest-path "$MANIFEST_PATH"

for attempt in {1..30}; do
  if cargo info "$PACKAGE_NAME@$PACKAGE_VERSION" >/dev/null 2>&1; then
    echo "$PACKAGE_NAME@$PACKAGE_VERSION is available from crates.io"
    exit 0
  fi

  echo "Waiting for $PACKAGE_NAME@$PACKAGE_VERSION to propagate ($attempt/30)"
  sleep 10
done

echo "Timed out waiting for $PACKAGE_NAME@$PACKAGE_VERSION to propagate"
exit 1
