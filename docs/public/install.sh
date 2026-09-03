#!/bin/sh
# Arete CLI (a4) installer. Source: https://github.com/AreteA4/arete/blob/main/docs/public/install.sh
#
# usage: curl -fsSL https://arete.run/install.sh | sh [-s -- [VERSION] [--no-modify-path] [--install-dir DIR] [--json]]
# env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH
#        A4_MANIFEST_BASE_URL, A4_LATEST_URL (test overrides; same names as the a4 binary)
#
# Steps: detect platform, resolve version, download asset + checksums.txt +
# checksums.txt.minisig, verify sha256 (and the minisign signature when
# minisign is installed), then hand over to `a4 self install`, which copies the
# binary to ~/.local/bin, writes ~/.arete/receipt.json and prints
# `A4_BIN=<path>` as its last-but-one stdout line. Human output goes to stderr;
# stdout is reserved for `a4 self install`.
set -eu

A4_MINISIGN_PUBLIC_KEY="RWRsiwmDW0371BZbcE1IWD6Y8/KIoAArUAp7mpyG6VweJ5rE3Lf3g5qA"
LATEST_URL="${A4_LATEST_URL:-https://docs.arete.run/a4/latest.json}"

log() { printf '%s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# 1. Platform key (<os>-<arch>), same table as the a4 binary and the npm package.
os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    die "Windows shell detected. Run in PowerShell instead: irm https://arete.run/install.ps1 | iex" ;;
  *) die "unsupported OS '$os' (supported: darwin, linux). Build from source: https://github.com/AreteA4/arete" ;;
esac
case "$arch" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64) arch=x64 ;;
  *) die "unsupported architecture '$arch' (supported: arm64, x64). Build from source: https://github.com/AreteA4/arete" ;;
esac
platform="$os-$arch"
asset="a4-$platform"

# 2. Version: first positional argument -> A4_VERSION -> latest.json.
#    Every other argument is forwarded verbatim to `a4 self install`.
version=""
n=$#
while [ "$n" -gt 0 ]; do
  arg=$1; shift; n=$((n - 1))
  case "$arg" in
    -h|--help)
      log "usage: curl -fsSL https://arete.run/install.sh | sh [-s -- [VERSION] [--no-modify-path] [--install-dir DIR] [--json]]"
      log "env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH"
      exit 0 ;;
    --install-dir)
      [ "$n" -gt 0 ] || die "--install-dir needs a directory argument"
      set -- "$@" "$arg" "$1"; shift; n=$((n - 1)) ;;
    -*) set -- "$@" "$arg" ;;
    *)
      if [ -z "$version" ]; then version=$arg; else set -- "$@" "$arg"; fi ;;
  esac
done
[ -n "$version" ] || version="${A4_VERSION:-}"

have curl || have wget || die "need curl or wget on PATH (e.g. apt-get install -y curl ca-certificates)"

# http_get URL DEST: sets http_status to the HTTP status (000 on transport failure).
http_get() {
  if have curl; then
    http_status=$(curl -sSL --retry 2 -o "$2" -w '%{http_code}' "$1" 2>/dev/null) || http_status="000"
  else
    # wget exits 8 on an HTTP error response (treated as 404) and 4 on a network failure.
    if wget -qO "$2" "$1" 2>/dev/null; then http_status=200; elif [ $? -eq 8 ]; then http_status=404; else http_status=000; fi
  fi
}

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t a4-install)
trap 'rm -rf "$tmp"' EXIT INT TERM HUP

if [ -z "$version" ]; then
  http_get "$LATEST_URL" "$tmp/latest.json"
  [ "$http_status" = 200 ] || die "could not fetch $LATEST_URL (HTTP $http_status). Pass a version: sh -s -- 0.13.0 or set A4_VERSION"
  version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
  [ -n "$version" ] || die "no \"version\" field in $LATEST_URL. Pass a version: sh -s -- 0.13.0 or set A4_VERSION"
fi
version=${version#v}

# 3. Download asset, checksums.txt, checksums.txt.minisig.
if [ -n "${A4_MANIFEST_BASE_URL:-}" ]; then
  base=${A4_MANIFEST_BASE_URL%/}
  case "$base" in
    *"{version}"*) base=$(printf '%s' "$base" | sed "s/{version}/$version/g") ;;
    *) base="$base/a4-cli-v$version" ;;
  esac
else
  base="https://github.com/AreteA4/arete/releases/download/a4-cli-v$version"
fi

log "Installing a4 $version ($platform) from $base"
for file in "$asset" checksums.txt checksums.txt.minisig; do
  dest="$tmp/$file"
  [ "$file" = "$asset" ] && dest="$tmp/a4"
  http_get "$base/$file" "$dest"
  case "$http_status" in
    200) ;;
    404) die "Release $version is still publishing; retry in a few minutes (missing $base/$file)" ;;
    *) die "download of $base/$file failed (HTTP $http_status). Check network/proxy settings and retry" ;;
  esac
done

# 4. SHA-256 of the asset against checksums.txt.
if have sha256sum; then actual=$(sha256sum "$tmp/a4" | cut -d' ' -f1)
elif have shasum; then actual=$(shasum -a 256 "$tmp/a4" | cut -d' ' -f1)
else die "need sha256sum or shasum on PATH"; fi
expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a { print tolower($1); exit }' "$tmp/checksums.txt")
[ -n "$expected" ] || die "checksums.txt has no entry for $asset; the release may be incomplete, retry in a few minutes"
[ "$actual" = "$expected" ] || die "sha256 mismatch for $asset (expected $expected, got $actual). Retry; if it persists, report at https://github.com/AreteA4/arete/issues"

# 5. minisign signature over checksums.txt (a4 self install always verifies it too).
if have minisign; then
  minisign -Vq -P "$A4_MINISIGN_PUBLIC_KEY" -m "$tmp/checksums.txt" -x "$tmp/checksums.txt.minisig" >/dev/null 2>&1 \
    || die "minisign signature verification of checksums.txt failed. Do not use this download; report at https://github.com/AreteA4/arete/issues"
else
  log "note: minisign not found; signature will be verified by a4 self install"
fi

# 6. Hand over. `exec` would replace this shell and skip the EXIT trap, leaving
#    $tmp behind; `a4 self install` copies the binary out, so run it in the
#    foreground, clean up, and exit with its status.
chmod +x "$tmp/a4"
set +e
"$tmp/a4" self install --source sh --checksums "$tmp/checksums.txt" --signature "$tmp/checksums.txt.minisig" "$@"
status=$?
set -e
[ "$status" -ne 126 ] || log "hint: $tmp is not executable (noexec mount?). Retry with TMPDIR=\$HOME/.cache"
exit "$status"
