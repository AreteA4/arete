#!/usr/bin/env bash
#
# Write the a4 CLI release manifest (manifest.json) to stdout.
#
#   scripts/write-release-manifest.sh <version> <checksums.txt> > manifest.json
#
# The manifest lets installers and `a4 self update` learn the asset names,
# checksums and the checksum/signature file names for one release without the
# GitHub API. Schema (docs/internal/agent-first-onboarding.md, WP1):
#
#   {
#     "schemaVersion": 1, "name": "a4", "version": "...", "tag": "a4-cli-v...",
#     "releasedAt": "<now, UTC>",
#     "assets": { "<platform-key>": { "name": "<asset>", "sha256": "..." }, ... },
#     "checksums": "checksums.txt", "signature": "checksums.txt.minisig",
#     "minimumVersion": null
#   }
#
# Requires jq.

set -euo pipefail

VERSION="${1:-}"
CHECKSUMS="${2:-}"

if [[ -z "$VERSION" || -z "$CHECKSUMS" ]]; then
  echo "Usage: $0 <version> <checksums.txt>" >&2
  exit 1
fi

if [[ ! -f "$CHECKSUMS" ]]; then
  echo "Checksums file not found: $CHECKSUMS" >&2
  exit 1
fi

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
  echo "Not a semver version: $VERSION" >&2
  exit 1
fi

# Platform keys in the order the spec lists them; every one must be present.
PLATFORMS=(darwin-arm64 darwin-x64 linux-x64 linux-arm64 win32-x64)

ASSETS='{}'
for platform in "${PLATFORMS[@]}"; do
  if [[ "$platform" == win32-* ]]; then
    asset="a4-${platform}.exe"
  else
    asset="a4-${platform}"
  fi
  # Lines look like "<sha256>  <asset>"; tolerate "*<asset>" (sha256sum -b).
  sha="$(awk -v name="$asset" '{ n = $2; sub(/^\*/, "", n); if (n == name) { print tolower($1); exit } }' "$CHECKSUMS")"
  if ! [[ "$sha" =~ ^[0-9a-f]{64}$ ]]; then
    echo "No sha256 for $asset in $CHECKSUMS" >&2
    exit 1
  fi
  ASSETS="$(jq -c --arg key "$platform" --arg name "$asset" --arg sha "$sha" \
    '. + { ($key): { name: $name, sha256: $sha } }' <<<"$ASSETS")"
done

RELEASED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
  --arg version "$VERSION" \
  --arg tag "a4-cli-v${VERSION}" \
  --arg releasedAt "$RELEASED_AT" \
  --argjson assets "$ASSETS" \
  '{
    schemaVersion: 1,
    name: "a4",
    version: $version,
    tag: $tag,
    releasedAt: $releasedAt,
    assets: $assets,
    checksums: "checksums.txt",
    signature: "checksums.txt.minisig",
    minimumVersion: null
  }'
