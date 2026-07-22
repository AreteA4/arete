#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STACK_ID="${ARETE_ORE_STACK_ID:-ore}"
API_URL="${ARETE_API_URL:-https://api.arete.run}"
LOCAL_AST="${ARETE_ORE_AST_PATH:-$ROOT_DIR/stacks/ore/.arete/OreStream.stack.json}"
LOCAL_PROVENANCE="${ARETE_ORE_PROVENANCE_PATH:-$ROOT_DIR/examples/ore-react/src/generated/sdk-provenance.json}"

for command in curl jq; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Required command not found: $command" >&2
        exit 1
    fi
done

if [[ ! -f "$LOCAL_AST" ]]; then
    echo "Local ORE AST not found: $LOCAL_AST" >&2
    echo "Run scripts/generate-example-sdks.sh first." >&2
    exit 1
fi

remote_json="$(mktemp)"
trap 'rm -f "$remote_json"' EXIT

# The registry install endpoint is public. Deliberately send no credentials.
curl --fail --silent --show-error \
    "$API_URL/api/registry/stacks/$STACK_ID/install" \
    --output "$remote_json"

local_ast_hash="$(jq -er '.content_hash' "$LOCAL_AST")"
# astContentHash identifies the registry version wrapper. The embedded AST
# carries the semantic hash produced by the stack compiler.
remote_ast_hash="$(jq -er '.astPayload.content_hash // .astPayload.contentHash' "$remote_json")"

if [[ "$local_ast_hash" != "$remote_ast_hash" ]]; then
    echo "ORE deployment AST does not match the local build." >&2
    echo "  local:    $local_ast_hash" >&2
    echo "  deployed: $remote_ast_hash" >&2
    exit 1
fi

if [[ -f "$LOCAL_PROVENANCE" ]]; then
    provenance_ast_hash="$(jq -er '.input.sha256' "$LOCAL_PROVENANCE")"
    if [[ "$provenance_ast_hash" != "$local_ast_hash" ]]; then
        echo "Generated React SDK provenance does not match the local ORE AST." >&2
        echo "  SDK input: $provenance_ast_hash" >&2
        echo "  local AST: $local_ast_hash" >&2
        exit 1
    fi

    local_extensions_hash="$(jq -r '.extensions.sha256 // ""' "$LOCAL_PROVENANCE")"
    remote_extensions_hash="$(jq -r '.extensions.artifactHash // .extensions.artifact_hash // ""' "$remote_json")"
    if [[ -n "$remote_extensions_hash" ]]; then
        if [[ "$local_extensions_hash" != "$remote_extensions_hash" ]]; then
            echo "ORE deployment extensions do not match generated SDK provenance." >&2
            echo "  local:    ${local_extensions_hash:-<none>}" >&2
            echo "  deployed: ${remote_extensions_hash:-<none>}" >&2
            exit 1
        fi
    fi
fi

echo "ORE deployment matches local AST $local_ast_hash"
