#!/usr/bin/env bash
#
# Publish a directory of release assets as a signed GitHub release of this
# repository, in the format `a4` verifies:
#
#   A4_MINISIGN_SECRET_KEY=<secret key file contents> GH_TOKEN=<token> \
#   scripts/sign-and-upload-external-assets.sh <release-tag> <dir>
#
# 1. writes <dir>/checksums.txt: `<sha256>  <asset>` for every other regular
#    file in <dir>, sorted by name;
# 2. signs it with the release key (scripts/sign-release-checksums.sh --tag,
#    trusted comment <release-tag>) and verifies the signature against the
#    public key embedded in the CLI;
# 3. creates the release <release-tag> if it doesn't exist (not marked
#    latest), then uploads every asset, checksums.txt and checksums.txt.minisig
#    with scripts/upload-release-asset.sh, which skips identical assets and
#    refuses to replace different ones.
#
# Used for the Surfpool Yellowstone geyser plugin release assets
# (`surfpool-geyser-v<surfpool version>`). a4's own releases (`a4-cli-v*`)
# are published by release-please.yml. DRY_RUN=1 stops after signing.

set -euo pipefail

TAG="${1:-}"
DIR="${2:-}"
if [[ -z "$TAG" || -z "$DIR" ]]; then
  echo "Usage: $0 <release-tag> <dir>" >&2
  exit 1
fi
if [[ ! "$TAG" =~ ^surfpool-geyser-v[0-9][A-Za-z0-9.+-]*$ ]]; then
  echo "Only surfpool-geyser-v<version> releases are supported: $TAG" >&2
  exit 1
fi
if [[ ! -d "$DIR" ]]; then
  echo "Asset directory not found: $DIR" >&2
  exit 1
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
DIR="$(cd "$DIR" && pwd)"

if command -v sha256sum >/dev/null; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
else
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

ASSETS=()
while IFS= read -r name; do
  ASSETS+=("$name")
done < <(
  find "$DIR" -mindepth 1 -maxdepth 1 -type f \
    ! -name checksums.txt ! -name checksums.txt.minisig -exec basename {} \; | LC_ALL=C sort
)
if [[ ${#ASSETS[@]} -eq 0 ]]; then
  echo "No assets in $DIR" >&2
  exit 1
fi
for name in "${ASSETS[@]}"; do
  if [[ ! "$name" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]; then
    echo "Asset name not allowed: $name" >&2
    exit 1
  fi
done

: > "$DIR/checksums.txt"
for name in "${ASSETS[@]}"; do
  printf '%s  %s\n' "$(sha256 "$DIR/$name")" "$name" >> "$DIR/checksums.txt"
done
rm -f "$DIR/checksums.txt.minisig"
"$HERE/sign-release-checksums.sh" --tag "$TAG" "$DIR/checksums.txt"
cat "$DIR/checksums.txt"

if [[ "${DRY_RUN:-}" == "1" ]]; then
  echo "DRY_RUN=1: signed $TAG; nothing uploaded"
  exit 0
fi

if ! gh release view "$TAG" >/dev/null 2>&1; then
  gh release create "$TAG" --title "$TAG" --latest=false \
    --notes "Release assets for \`$TAG\`. \`checksums.txt\` is signed with the Arete release key (\`checksums.txt.minisig\`, trusted comment \`$TAG\`); \`a4\` verifies both before using an asset."
fi

for name in "${ASSETS[@]}" checksums.txt checksums.txt.minisig; do
  "$HERE/upload-release-asset.sh" "$TAG" "$DIR/$name"
done
echo "Published $TAG (${#ASSETS[@]} asset(s))"
