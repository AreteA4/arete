#!/usr/bin/env bash
# Test the release signature checks in docs/public/install.sh.
#
#   1. The embedded JavaScript verifier (node, and osascript on macOS) and the
#      OpenSSL path accept the fixture signature for its tag and reject another
#      tag, another key, a tampered file and a forged trusted comment.
#   2. install.sh, served a release whose checksums are not signed by the
#      release key, stops before running the downloaded binary, and stops when
#      no verifier is installed at all.
#
# Needs node, python3 and OpenSSL 3 (CI=true makes a missing OpenSSL 3 fail).
set -euo pipefail

cd "$(dirname "$0")/.."
FIXTURES=cli/tests/fixtures/selfhost
KEY=$(cat "$FIXTURES/test-minisign.pub" | sed -n 2p)
TAG=a4-cli-v0.0.0-test
work=$(mktemp -d)
server_pid=""
cleanup() {
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

failures=0
check() { # <description> <expected status> <command...>
  local description=$1 expected=$2
  shift 2
  local status=0
  "$@" >"$work/out" 2>&1 || status=$?
  if [ "$status" = "$expected" ]; then
    echo "ok   $description"
  else
    echo "FAIL $description: exit $status, expected $expected"
    sed 's/^/     /' "$work/out"
    failures=$((failures + 1))
  fi
}

# Signature variants built from the fixture.
cp "$FIXTURES/checksums.txt" "$work/checksums.txt"
cp "$FIXTURES/checksums.txt.minisig" "$work/good.minisig"
cp "$FIXTURES/checksums.txt.wrong-key.minisig" "$work/wrong-key.minisig"
{ cat "$FIXTURES/checksums.txt"; echo extra; } >"$work/tampered.txt"
sed "s/^trusted comment: .*/trusted comment: a4-cli-v9.9.9/" "$FIXTURES/checksums.txt.minisig" >"$work/forged.minisig"
# Another key's signature relabelled with the release key's id.
node -e '
const fs = require("fs");
const [key, wrong, out] = process.argv.slice(1);
const id = Buffer.from(key, "base64").subarray(2, 10);
const lines = fs.readFileSync(wrong, "utf8").split("\n");
const sig = Buffer.from(lines[1], "base64");
id.copy(sig, 2);
lines[1] = sig.toString("base64");
fs.writeFileSync(out, lines.join("\n"));
' "$KEY" "$FIXTURES/checksums.txt.wrong-key.minisig" "$work/relabelled.minisig"

cases() { # <runner...>: runs the cases with `<runner> KEY FILE SIG TAG`
  check "$label accepts the signature for its tag" 0 "$@" "$KEY" "$work/checksums.txt" "$work/good.minisig" "$TAG"
  check "$label rejects another tag" 1 "$@" "$KEY" "$work/checksums.txt" "$work/good.minisig" a4-cli-v0.0.1
  check "$label rejects another key" 1 "$@" "$KEY" "$work/checksums.txt" "$work/wrong-key.minisig" "$TAG"
  check "$label rejects another key under the release key id" 1 "$@" "$KEY" "$work/checksums.txt" "$work/relabelled.minisig" "$TAG"
  check "$label rejects a tampered file" 1 "$@" "$KEY" "$work/tampered.txt" "$work/good.minisig" "$TAG"
  check "$label rejects a forged trusted comment" 1 "$@" "$KEY" "$work/checksums.txt" "$work/forged.minisig" a4-cli-v9.9.9
}

# 1. The verifiers, extracted from install.sh.
sed -n "/<<'A4_VERIFY_JS'$/,/^A4_VERIFY_JS$/p" docs/public/install.sh | sed '1d;$d' >"$work/verify.js"
[ -s "$work/verify.js" ] || { echo "FAIL no verifier between the A4_VERIFY_JS markers in install.sh"; exit 1; }
label="verify.js (node)" cases node "$work/verify.js"
if command -v osascript >/dev/null 2>&1; then
  label="verify.js (osascript)" cases osascript -l JavaScript "$work/verify.js"
fi

sed -n '/^openssl_verify() {$/,/^}$/p' docs/public/install.sh >"$work/openssl.sh"
[ -s "$work/openssl.sh" ] || { echo "FAIL no openssl_verify function in install.sh"; exit 1; }
run_openssl() {
  # shellcheck disable=SC1091
  . "$work/openssl.sh"
  rm -rf "$work/openssl" && mkdir "$work/openssl"
  openssl_verify "$@" "$work/openssl"
}
supported=0
run_openssl "$KEY" "$work/checksums.txt" "$work/good.minisig" "$TAG" || supported=$?
if [ "$supported" = 2 ]; then
  if [ "${CI:-}" = true ]; then
    echo "FAIL openssl $(openssl version) does not support Ed25519 and BLAKE2b-512"
    failures=$((failures + 1))
  else
    echo "skip openssl_verify: $(openssl version) does not support Ed25519 and BLAKE2b-512"
  fi
else
  label="openssl_verify" cases run_openssl
fi

# 2. install.sh against a local release whose checksums the release key did
#    not sign. The "binary" records that it ran.
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) asset=a4-darwin-arm64 ;;
  Darwin-x86_64) asset=a4-darwin-x64 ;;
  Linux-aarch64 | Linux-arm64) asset=a4-linux-arm64 ;;
  *) asset=a4-linux-x64 ;;
esac
release="$work/release/a4-cli-v0.0.0-test"
mkdir -p "$release"
printf '#!/bin/sh\ntouch "%s/ran"\n' "$work" >"$release/$asset"
if command -v sha256sum >/dev/null 2>&1; then sha=$(sha256sum "$release/$asset" | cut -d' ' -f1)
else sha=$(shasum -a 256 "$release/$asset" | cut -d' ' -f1); fi
printf '%s  %s\n' "$sha" "$asset" >"$release/checksums.txt"
cp "$FIXTURES/checksums.txt.minisig" "$release/checksums.txt.minisig"

port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work/release" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
  curl -fsS -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null && break
  sleep 0.1
done

install_sh() { # <PATH>
  env -i HOME="$work/home" PATH="$1" A4_MANIFEST_BASE_URL="http://127.0.0.1:$port" \
    sh docs/public/install.sh 0.0.0-test --no-modify-path
}
expect_refusal() { # <description> <PATH> <message>
  rm -f "$work/ran"
  check "$1" 1 install_sh "$2"
  if ! grep -q "$3" "$work/out"; then
    echo "FAIL $1: output does not say '$3'"
    failures=$((failures + 1))
  fi
  if [ -e "$work/ran" ]; then
    echo "FAIL $1: the downloaded binary ran"
    failures=$((failures + 1))
  fi
}
expect_refusal "install.sh refuses checksums the release key did not sign" "$PATH" "signature verification of checksums.txt"

# Only the tools install.sh needs besides a verifier.
bin="$work/bin"
mkdir -p "$bin"
for tool in sh uname curl mktemp rm mkdir sed awk cut tr head cat chmod sha256sum shasum wc dd cmp printf grep; do
  found=$(command -v "$tool" 2>/dev/null) || continue
  case "$found" in /*) ln -s "$found" "$bin/$tool" ;; esac
done
expect_refusal "install.sh stops when no signature verifier is installed" "$bin" "cannot verify the release signature"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "All install.sh signature checks passed"
