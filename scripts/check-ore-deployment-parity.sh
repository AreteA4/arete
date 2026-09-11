#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STACK_ID="${ARETE_ORE_STACK_ID:-ore}"
API_URL="${ARETE_API_URL:-https://api.arete.run}"
LOCAL_MANIFEST="${ARETE_ORE_MANIFEST_PATH:-$ROOT_DIR/stacks/ore/.arete/OreStream.stack-manifest.json}"
LOCAL_PROVENANCE="${ARETE_ORE_PROVENANCE_PATH:-$ROOT_DIR/examples/ore-react/src/generated/sdk-manifest.json}"

for command in curl jq; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Required command not found: $command" >&2
        exit 1
    fi
done

if [[ ! -f "$LOCAL_MANIFEST" ]]; then
    echo "Local ORE StackManifest not found: $LOCAL_MANIFEST" >&2
    echo "Run scripts/generate-example-sdks.sh first." >&2
    exit 1
fi

remote_json="$(mktemp)"
trap 'rm -f "$remote_json"' EXIT

# The versioned package resolver is public for public packages. Deliberately
# send no credentials.
curl --fail --silent --show-error \
    --request POST \
    --header 'content-type: application/json' \
    --data "{\"manifestVersion\":1,\"dependencies\":[{\"kind\":\"stack\",\"alias\":\"ore\",\"package\":\"$STACK_ID\",\"requirement\":\"*\"}],\"targets\":[\"typescript\"],\"generatorContract\":\"sdk-generator-v1\"}" \
    "$API_URL/api/registry/v1/resolve" \
    --output "$remote_json"

local_manifest_hash="$(jq -er '.artifactHash' "$LOCAL_MANIFEST")"
remote_manifest_hash="$(jq -er '.dependencies[0].stackManifestHash' "$remote_json")"

if [[ "$local_manifest_hash" != "$remote_manifest_hash" ]]; then
    echo "ORE registry package does not match the local StackManifest." >&2
    echo "  local:    $local_manifest_hash" >&2
    echo "  resolved: $remote_manifest_hash" >&2
    exit 1
fi

if [[ -f "$LOCAL_PROVENANCE" ]]; then
    provenance_manifest_hash="$(jq -er '.input.hash' "$LOCAL_PROVENANCE")"
    if [[ "$provenance_manifest_hash" != "$local_manifest_hash" ]]; then
        echo "Generated React SDK provenance does not match the local ORE StackManifest." >&2
        echo "  SDK input:      $provenance_manifest_hash" >&2
        echo "  local manifest: $local_manifest_hash" >&2
        exit 1
    fi

    local_extensions_hash="$(jq -r '.extensions.contentSha256 // .extensions.sha256 // ""' "$LOCAL_PROVENANCE")"
    remote_extensions_hash="$(jq -r '.dependencies[0].sdkExtensions[]? | select(.target == "typescript") | .contentHash' "$remote_json")"
    if [[ -n "$remote_extensions_hash" ]]; then
        if [[ "$local_extensions_hash" != "$remote_extensions_hash" ]]; then
            echo "ORE deployment extensions do not match generated SDK provenance." >&2
            echo "  local:    ${local_extensions_hash:-<none>}" >&2
            echo "  deployed: ${remote_extensions_hash:-<none>}" >&2
            exit 1
        fi
    fi
fi

echo "ORE registry package matches local StackManifest $local_manifest_hash"
