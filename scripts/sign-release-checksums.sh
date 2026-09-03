#!/usr/bin/env bash
#
# Sign a release checksums.txt with the Arete minisign release key and verify
# the signature against the public key embedded in the CLI.
#
#   A4_MINISIGN_SECRET_KEY=<secret key file contents> \
#   scripts/sign-release-checksums.sh <version> <checksums.txt>
#
# Writes <checksums.txt>.minisig next to the input with trusted comment
# `a4-cli-v<version>`. Used by release-please.yml and release-recovery.yml so
# a recovered release is installable exactly like a normal one. The secret key
# is stored unencrypted (minisign -G -W); A4_MINISIGN_PASSWORD is piped in for
# forward compatibility should the key ever be regenerated with one.

set -euo pipefail

VERSION="${1:-}"
CHECKSUMS="${2:-}"
if [[ -z "$VERSION" || -z "$CHECKSUMS" ]]; then
  echo "Usage: $0 <version> <checksums.txt>" >&2
  exit 1
fi
if [[ ! -f "$CHECKSUMS" ]]; then
  echo "checksums file not found: $CHECKSUMS" >&2
  exit 1
fi
if [[ -z "${A4_MINISIGN_SECRET_KEY:-}" ]]; then
  echo "::error::A4_MINISIGN_SECRET_KEY is not set (GitHub secret missing?)" >&2
  exit 1
fi
command -v minisign >/dev/null || { echo "minisign is not installed" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY_FILE="$(mktemp)"
PUB_FILE="$(mktemp)"
trap 'rm -f "$KEY_FILE" "$PUB_FILE"' EXIT
chmod 600 "$KEY_FILE"
printf '%s\n' "$A4_MINISIGN_SECRET_KEY" > "$KEY_FILE"

printf '%s\n' "${A4_MINISIGN_PASSWORD:-}" | \
  minisign -S -s "$KEY_FILE" -m "$CHECKSUMS" -t "a4-cli-v${VERSION}"

# The signature must verify with the key embedded in the CLI, or installers
# would reject the release.
PUBKEY="$(grep -oE 'RW[A-Za-z0-9+/]{54}' "$REPO_ROOT/cli/src/selfhost/keys.rs" | head -n1)"
printf 'untrusted comment: a4 release key\n%s\n' "$PUBKEY" > "$PUB_FILE"
minisign -V -p "$PUB_FILE" -m "$CHECKSUMS"
echo "Signed $CHECKSUMS as a4-cli-v${VERSION} (key $(printf '%s' "$PUBKEY" | cut -c1-12)…)"
