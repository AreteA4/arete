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

# The fake npm refuses `npm install` and any install, pack or publish that
# could run lifecycle scripts, so these tests also prove the release scripts
# only install from lockfiles without install scripts.
cat >"$FAKE_BIN/npm" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${NPM_LOG:-}" ]]; then
  printf '%s\n' "$*" >>"$NPM_LOG"
fi

require_ignore_scripts() {
  local arg
  for arg in "$@"; do
    [[ "$arg" == "--ignore-scripts" ]] && return 0
  done
  printf 'npm %s without --ignore-scripts\n' "$*" >&2
  exit 1
}

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
  ci)
    require_ignore_scripts "$@"
    ;;
  run)
    ;;
  view)
    # Every published version propagates at once: print the requested one.
    printf '%s\n' "${2##*@}"
    ;;
  pack)
    require_ignore_scripts "$@"
    destination=""
    while [[ $# -gt 0 ]]; do
      if [[ "$1" == "--pack-destination" ]]; then
        destination="$2"
        shift 2
      else
        shift
      fi
    done
    staging="$(mktemp -d)"
    mkdir "$staging/package"
    cp package.json "$staging/package/package.json"
    file="$(node -p "const p = require('./package.json'); p.name.replace('@', '').replace('/', '-') + '-' + p.version + '.tgz'")"
    tar -czf "$destination/$file" -C "$staging" package
    rm -rf "$staging"
    ;;
  publish)
    require_ignore_scripts "$@"
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


assert_not_contains() {
  local output="$1"
  local unexpected="$2"

  if [[ "$output" == *"$unexpected"* ]]; then
    printf 'Expected output not to contain %q, got:\n%s\n' "$unexpected" "$output" >&2
    exit 1
  fi
}

make_tarball() {
  local destination="$1"
  local name="$2"
  local version="$3"
  local staging
  staging="$(mktemp -d "$TEST_DIR/tarball.XXXXXX")"
  mkdir "$staging/package"
  printf '{"name":"%s","version":"%s"}\n' "$name" "$version" >"$staging/package/package.json"
  tar -czf "$destination" -C "$staging" package
}

# A packed tarball publishes only when it holds the checked-out name and version.
NPM_LOG="$TEST_DIR/npm.log"
make_tarball "$TEST_DIR/auth-test.tgz" "@usearete/auth-test" "0.0.0"
: >"$NPM_LOG"
env -u NPM_TOKEN PATH="$FAKE_BIN:$PATH" NPM_LOG="$NPM_LOG" \
  ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
  ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
  FAKE_CURL_STATUS=404 \
  "$ROOT_DIR/scripts/publish-npm-package.sh" "$PACKAGE_DIR" --tarball "$TEST_DIR/auth-test.tgz" >/dev/null
assert_contains "$(cat "$NPM_LOG")" "publish $TEST_DIR/auth-test.tgz --access public --provenance --ignore-scripts"

make_tarball "$TEST_DIR/other.tgz" "@usearete/other" "0.0.0"
rm -f "$PUBLISH_MARKER"
if mismatch_output="$(
  env -u NPM_TOKEN PATH="$FAKE_BIN:$PATH" NPM_PUBLISH_MARKER="$PUBLISH_MARKER" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    FAKE_CURL_STATUS=404 \
    "$ROOT_DIR/scripts/publish-npm-package.sh" "$PACKAGE_DIR" --tarball "$TEST_DIR/other.tgz" 2>&1
)"; then
  printf 'A tarball for another package was published\n' >&2
  exit 1
fi
assert_contains "$mismatch_output" "expected @usearete/auth-test@0.0.0"
[[ ! -e "$PUBLISH_MARKER" ]]

# Restore a public automations package for the split recovery flow.
python3 - "$VERSION_ROOT/typescript/automations/package.json" <<'PYPUBLIC'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text()); data.pop('private', None)
path.write_text(json.dumps(data))
PYPUBLIC

# --pack-only builds, so it refuses to run where it could publish.
PACK_DIR="$TEST_DIR/packed"
mkdir "$PACK_DIR"
for credential in "NPM_TOKEN=test-token" "ACTIONS_ID_TOKEN_REQUEST_TOKEN=test-token" "NODE_AUTH_TOKEN=test-token"; do
  if credential_output="$(
    env -u NPM_TOKEN -u NODE_AUTH_TOKEN -u ACTIONS_ID_TOKEN_REQUEST_TOKEN \
      PATH="$FAKE_BIN:$PATH" FAKE_CURL_STATUS=404 "$credential" \
      "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --pack-only "$PACK_DIR" 2>&1
  )"; then
    printf -- '--pack-only ran with %s\n' "${credential%%=*}" >&2
    exit 1
  fi
  assert_contains "$credential_output" "without npm or OIDC credentials"
done
[[ -z "$(ls -A "$PACK_DIR")" ]]

# The credential-free pack step installs only from lockfiles and writes the
# packages in publish order. (The fixture has had no protocol package since
# the historical-source case above.)
: >"$NPM_LOG"
env -u NPM_TOKEN -u NODE_AUTH_TOKEN -u ACTIONS_ID_TOKEN_REQUEST_TOKEN \
  PATH="$FAKE_BIN:$PATH" FAKE_CURL_STATUS=404 NPM_LOG="$NPM_LOG" \
  "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --pack-only "$PACK_DIR" >/dev/null
npm_calls="$(cat "$NPM_LOG")"
assert_contains "$npm_calls" "ci --ignore-scripts"
assert_not_contains "$npm_calls" "install"
assert_not_contains "$npm_calls" "publish"
packed_names="$(cut -d'|' -f1 "$PACK_DIR/packages.txt" | tr '\n' ' ')"
if [[ "$packed_names" != "@usearete/hash @usearete/sdk @usearete/adapter-kit @usearete/react @usearete/adapter-web3js @usearete/automations @usearete/a4 @usearete/mcp " ]]; then
  printf 'Unexpected pack order: %s\n' "$packed_names" >&2
  exit 1
fi

# The publish step installs and builds nothing; it publishes the tarballs.
: >"$NPM_LOG"
publish_packed_output="$(
  env -u NPM_TOKEN PATH="$FAKE_BIN:$PATH" NPM_LOG="$NPM_LOG" \
    ACTIONS_ID_TOKEN_REQUEST_URL="https://example.invalid/oidc" \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN="test-token" \
    NODE_AUTH_TOKEN="XXXXX-XXXXX-XXXXX-XXXXX" \
    FAKE_CURL_STATUS=404 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --publish-packed "$PACK_DIR" 2>&1
)"
npm_calls="$(cat "$NPM_LOG")"
assert_not_contains "$npm_calls" "ci "
assert_not_contains "$npm_calls" "run "
assert_not_contains "$npm_calls" "install"
assert_contains "$publish_packed_output" "Using npm trusted publishing"
assert_contains "$publish_packed_output" "Publish flow completed"
assert_contains "$npm_calls" "view @usearete/hash@0.7.0 version"
assert_contains "$npm_calls" "view @usearete/sdk@0.3.0 version"
if [[ "$(grep -c '^publish ' "$NPM_LOG")" != 8 ]]; then
  printf 'Expected eight tarball publishes, got:\n%s\n' "$npm_calls" >&2
  exit 1
fi
assert_contains "$npm_calls" "publish $PACK_DIR/08/usearete-mcp-0.3.0.tgz --access public --provenance --ignore-scripts"

# Anything else in the directory is refused before it reaches npm.
TAMPERED_DIR="$TEST_DIR/tampered"
cp -R "$PACK_DIR" "$TAMPERED_DIR"
printf '@usearete/sdk|9.9.9|02/usearete-sdk-0.3.0.tgz\n' >"$TAMPERED_DIR/packages.txt"
rm -f "$PUBLISH_MARKER"
if tampered_output="$(
  env -u NPM_TOKEN PATH="$FAKE_BIN:$PATH" NPM_TOKEN="test-token" NPM_PUBLISH_MARKER="$PUBLISH_MARKER" FAKE_CURL_STATUS=404 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --publish-packed "$TAMPERED_DIR" 2>&1
)"; then
  printf 'A packed directory with an unexpected version was published\n' >&2
  exit 1
fi
assert_contains "$tampered_output" "expected version 0.3.0"
[[ ! -e "$PUBLISH_MARKER" ]]

printf '@evil/package|0.3.0|02/usearete-sdk-0.3.0.tgz\n' >"$TAMPERED_DIR/packages.txt"
if tampered_output="$(
  env PATH="$FAKE_BIN:$PATH" NPM_TOKEN="test-token" NPM_PUBLISH_MARKER="$PUBLISH_MARKER" FAKE_CURL_STATUS=404 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --publish-packed "$TAMPERED_DIR" 2>&1
)"; then
  printf 'A packed directory with an unexpected package was published\n' >&2
  exit 1
fi
assert_contains "$tampered_output" "unexpected package @evil/package"
[[ ! -e "$PUBLISH_MARKER" ]]

printf '@usearete/react|0.3.0|02/usearete-sdk-0.3.0.tgz\n' >"$TAMPERED_DIR/packages.txt"
if tampered_output="$(
  env PATH="$FAKE_BIN:$PATH" NPM_TOKEN="test-token" NPM_PUBLISH_MARKER="$PUBLISH_MARKER" FAKE_CURL_STATUS=404 \
    "$VERSION_ROOT/scripts/publish-npm-version.sh" 0.3.0 --publish-packed "$TAMPERED_DIR" 2>&1
)"; then
  printf 'A tarball listed under another package name was published\n' >&2
  exit 1
fi
assert_contains "$tampered_output" "contains @usearete/sdk@0.3.0, expected @usearete/react@0.3.0"
[[ ! -e "$PUBLISH_MARKER" ]]

printf 'npm publishing authentication tests passed\n'
