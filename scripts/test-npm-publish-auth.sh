#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-npm-auth-test.XXXXXX")"
FAKE_BIN="$TEST_DIR/bin"
PACKAGE_DIR="$TEST_DIR/package"

cleanup() {
  rm -rf "$TEST_DIR"
}

trap cleanup EXIT

mkdir -p "$FAKE_BIN" "$PACKAGE_DIR"

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [[ -n "$output_file" ]]; then
  printf '{}\n' >"$output_file"
fi
printf '%s' "${FAKE_CURL_STATUS:?}"
EOF

cat >"$FAKE_BIN/npm" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  whoami)
    if [[ -n "${NPM_WHOAMI_MARKER:-}" ]]; then
      touch "$NPM_WHOAMI_MARKER"
    fi
    if [[ -n "${NPM_TOKEN:-}" && -f "${NPM_CONFIG_USERCONFIG:-}" ]]; then
      IFS= read -r auth_line <"$NPM_CONFIG_USERCONFIG"
      if [[ "$auth_line" == "//registry.npmjs.org/:_authToken=${NPM_TOKEN}" ]]; then
        printf 'test-account\n'
        exit 0
      fi
    fi
    exit 1
    ;;
  install|ci|run)
    ;;
  publish)
    if [[ -n "${NPM_PUBLISH_MARKER:-}" ]]; then
      touch "$NPM_PUBLISH_MARKER"
    fi
    ;;
  *)
    printf 'Unexpected npm command: %s\n' "$*" >&2
    exit 1
    ;;
esac
EOF

chmod +x "$FAKE_BIN/curl" "$FAKE_BIN/npm"

cat >"$PACKAGE_DIR/package.json" <<'EOF'
{
  "name": "@usearete/auth-test",
  "version": "0.0.0"
}
EOF

assert_contains() {
  local output="$1"
  local expected="$2"

  if [[ "$output" != *"$expected"* ]]; then
    printf 'Expected output to contain %q, got:\n%s\n' "$expected" "$output" >&2
    exit 1
  fi
}

WHOAMI_MARKER="$TEST_DIR/whoami"
PUBLISH_MARKER="$TEST_DIR/publish"

package_output="$(
  env -u NPM_TOKEN \
    PATH="$FAKE_BIN:$PATH" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    NODE_AUTH_TOKEN="XXXXX-XXXXX-XXXXX-XXXXX" \
    FAKE_CURL_STATUS=404 \
    NPM_WHOAMI_MARKER="$WHOAMI_MARKER" \
    NPM_PUBLISH_MARKER="$PUBLISH_MARKER" \
    "$ROOT_DIR/scripts/publish-npm-package.sh" "$PACKAGE_DIR"
)"

assert_contains "$package_output" "with npm trusted publishing"
[[ ! -e "$WHOAMI_MARKER" ]]
[[ -e "$PUBLISH_MARKER" ]]

rm -f "$WHOAMI_MARKER" "$PUBLISH_MARKER"

if collision_output="$(
  env -u NPM_TOKEN \
    PATH="$FAKE_BIN:$PATH" \
    FAKE_CURL_STATUS=200 \
    NPM_PUBLISH_MARKER="$PUBLISH_MARKER" \
    "$ROOT_DIR/scripts/publish-npm-package.sh" "$PACKAGE_DIR" 2>&1
)"; then
  printf 'Normal npm publication unexpectedly accepted an existing version\n' >&2
  exit 1
fi
assert_contains "$collision_output" "refusing to reuse a version for new source"
[[ ! -e "$PUBLISH_MARKER" ]]

# Exercise authentication against release metadata, independently of the
# development checkout's intentionally unpublished hash/protocol versions.
VERSION_ROOT="$TEST_DIR/version-root"
mkdir -p "$VERSION_ROOT/scripts"
cp "$ROOT_DIR/scripts/publish-npm-version.sh" "$ROOT_DIR/scripts/check-protocol-release-dependency.mjs" "$VERSION_ROOT/scripts/"
python3 - "$ROOT_DIR" "$VERSION_ROOT" <<'PYFIXTURE'
import json
from pathlib import Path
import sys
source, target = map(Path, sys.argv[1:])
for directory in ['typescript/core', 'typescript/hash', 'typescript/protocol', 'typescript/react', 'typescript/adapters/kit', 'typescript/adapters/web3js', 'typescript/automations', 'packages/arete', 'packages/mcp']:
    root = target / directory
    root.mkdir(parents=True)
    for name in ['package.json', 'package-lock.json']:
        original = source / directory / name
        if original.exists():
            (root / name).write_bytes(original.read_bytes())
hash_file = target / 'typescript/hash/package.json'
data = json.loads(hash_file.read_text()); data['version'] = '0.7.0'
hash_file.write_text(json.dumps(data))
protocol_file = target / 'typescript/protocol/package.json'
data = json.loads(protocol_file.read_text()); data['dependencies']['@usearete/hash'] = '^0.7.0'
protocol_file.write_text(json.dumps(data))
PYFIXTURE

version_output="$(
  env -u NPM_TOKEN \
    PATH="$FAKE_BIN:$PATH" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    NODE_AUTH_TOKEN="XXXXX-XXXXX-XXXXX-XXXXX" \
    FAKE_CURL_STATUS=200 \
    NPM_WHOAMI_MARKER="$WHOAMI_MARKER" \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --dry-run
)"

assert_contains "$version_output" "Using npm trusted publishing"
[[ ! -e "$WHOAMI_MARKER" ]]

token_output="$(
  env PATH="$FAKE_BIN:$PATH" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    NODE_AUTH_TOKEN="XXXXX-XXXXX-XXXXX-XXXXX" \
    NPM_TOKEN="test-token" \
    FAKE_CURL_STATUS=404 \
    NPM_WHOAMI_MARKER="$WHOAMI_MARKER" \
    NPM_PUBLISH_MARKER="$PUBLISH_MARKER" \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --dry-run
)"

assert_contains "$token_output" "Using npm account: test-account"
assert_contains "$token_output" "Publishing @usearete/automations@0.3.0"
[[ -e "$WHOAMI_MARKER" ]]
[[ -e "$PUBLISH_MARKER" ]]

# Historical release sources without the new independent package still recover.
rm -r "$VERSION_ROOT/typescript/protocol"
legacy_output="$(
  env -u NPM_TOKEN PATH="$FAKE_BIN:$PATH" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    NODE_AUTH_TOKEN="XXXXX-XXXXX-XXXXX-XXXXX" FAKE_CURL_STATUS=200 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --dry-run
)"
assert_contains "$legacy_output" "Using npm trusted publishing"

# Sources from before @usearete/automations was public do not publish it.
python3 - "$VERSION_ROOT/typescript/automations/package.json" <<'PYPRIVATE'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text()); data['private'] = True
path.write_text(json.dumps(data))
PYPRIVATE
private_output="$(
  env PATH="$FAKE_BIN:$PATH" NPM_TOKEN="test-token" FAKE_CURL_STATUS=404 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --dry-run
)"
if [[ "$private_output" == *"@usearete/automations"* ]]; then
  printf 'A private automations package was published\n' >&2
  exit 1
fi

printf 'npm publishing authentication tests passed\n'
