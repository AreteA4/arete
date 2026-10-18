#!/usr/bin/env bash
# Write the registry signing keys compiled into `a4` (cli/registry-keys.json).
#
# Run before a release: it reads the public key list from the registry,
# checks its shape and writes it sorted, so the release verifies registry
# package release signatures even before it can reach the registry.
#
# Usage: scripts/update-registry-keys.sh [api-url]
#   api-url  the registry API (default: $ARETE_API_URL, else https://api.arete.run)
#   --check  only check that cli/registry-keys.json is well formed
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEYS_FILE="$REPO_ROOT/cli/registry-keys.json"

validate() {
  python3 -I -c '
import base64, json, re, sys

KEY_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

def b58_len(text):
    number = 0
    for char in text:
        if char not in ALPHABET:
            raise ValueError("not base58")
        number = number * 58 + ALPHABET.index(char)
    zeros = len(text) - len(text.lstrip("1"))
    return zeros + (number.bit_length() + 7) // 8

document = json.load(sys.stdin)
if document.get("schema") != "arete.registry-keys/v1":
    sys.exit("unexpected schema: %r" % document.get("schema"))
seen = set()
keys = []
for key in document.get("keys", []):
    key_id = key.get("keyId", "")
    if not KEY_ID.match(key_id) or key_id in seen:
        sys.exit("invalid or repeated keyId: %r" % key_id)
    seen.add(key_id)
    if b58_len(key.get("publicKey", "")) != 32:
        sys.exit("publicKey of %s is not 32 bytes of base58" % key_id)
    entry = {"keyId": key_id, "publicKey": key["publicKey"]}
    if key.get("retiredAt") is not None:
        entry["retiredAt"] = int(key["retiredAt"])
    keys.append(entry)
keys.sort(key=lambda key: key["keyId"])
print(json.dumps({"schema": "arete.registry-keys/v1", "keys": keys}, indent=2))
'
}

if [[ "${1:-}" == "--check" ]]; then
  validate < "$KEYS_FILE" > /dev/null
  echo "$KEYS_FILE is well formed"
  exit 0
fi

API_URL="${1:-${ARETE_API_URL:-https://api.arete.run}}"
API_URL="${API_URL%/}"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

curl --fail --silent --show-error --max-time 30 "$API_URL/api/registry/v1/keys" | validate > "$TMP"
mv "$TMP" "$KEYS_FILE"
trap - EXIT
echo "Wrote $(python3 -I -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["keys"]))' "$KEYS_FILE") key(s) to $KEYS_FILE"
