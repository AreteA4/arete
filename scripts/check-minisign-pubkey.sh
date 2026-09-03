#!/usr/bin/env bash
#
# Verify that the minisign release public key is embedded identically in every
# place that verifies release signatures:
#
#   cli/src/selfhost/keys.rs        (a4 self install / self update)
#   docs/public/install.sh          (curl | sh bootstrapper)
#   docs/public/install.ps1         (PowerShell bootstrapper)
#   packages/arete/bin/a4.js        (npx @usearete/a4 bootstrapper)
#
# Fails when any file is missing, contains no key, contains more than one
# distinct key, or when the four keys differ. Run from the repo root.

set -euo pipefail

cd "$(dirname "$0")/.."

FILES=(
  cli/src/selfhost/keys.rs
  docs/public/install.sh
  docs/public/install.ps1
  packages/arete/bin/a4.js
)

# A minisign public key is 42 bytes (2 algorithm + 8 key id + 32 Ed25519) in
# base64: 56 characters starting with "RW".
KEY_PATTERN='RW[A-Za-z0-9+/]{54}'

status=0
reference=""
reference_file=""

for file in "${FILES[@]}"; do
  if [[ ! -f "$file" ]]; then
    echo "::error::$file is missing (must embed the minisign public key)"
    status=1
    continue
  fi

  keys="$(grep -oE "$KEY_PATTERN" "$file" | sort -u || true)"
  count="$(printf '%s\n' "$keys" | sed '/^$/d' | wc -l | tr -d ' ')"

  if [[ "$count" -eq 0 ]]; then
    echo "::error::$file does not contain a minisign public key"
    status=1
    continue
  fi
  if [[ "$count" -gt 1 ]]; then
    echo "::error::$file contains $count different minisign public keys:"
    printf '  %s\n' $keys
    status=1
    continue
  fi

  if [[ -z "$reference" ]]; then
    reference="$keys"
    reference_file="$file"
  elif [[ "$keys" != "$reference" ]]; then
    echo "::error::$file embeds $keys but $reference_file embeds $reference"
    status=1
  fi
done

if [[ "$status" -eq 0 ]]; then
  echo "minisign public key is consistent across ${#FILES[@]} files: $reference"
fi

exit "$status"
