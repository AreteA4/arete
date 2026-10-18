#!/usr/bin/env bash
# Build the Yellowstone gRPC geyser plugin for one Surfpool release.
#
#   scripts/surfpool-geyser/build.sh [--surfpool VERSION] [--out DIR] [--jobs N]
#
# Geyser plugins have no stable ABI: the plugin must be built against the
# exact agave version Surfpool embeds, with the rustc that built Surfpool, or
# Surfpool crashes on the first notification. pins.json records, per Surfpool
# release, the agave version, the rustc and the yellowstone-grpc release to
# build. When yellowstone-grpc has no release for that exact agave patch
# version, the agave crates are pinned down from `pinAgaveFrom` with
# `cargo update --precise`.
#
# Output: <out>/<surfpool version>/<platform>/ with the plugin library, a
# Surfpool plugin config template and manifest.json (versions and sha256).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SURFPOOL_VERSION=""
OUT="${HERE}/../../target/surfpool-geyser"
JOBS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --surfpool) SURFPOOL_VERSION="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --jobs) JOBS="$2"; shift 2 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }
# Cargo runs inside WORK; all later build and copy paths must share one root.
mkdir -p "${OUT}"
OUT=$(cd "${OUT}" && pwd)
for tool in git cargo rustup jq shasum; do
  command -v "$tool" >/dev/null || die "$tool is required"
done

if [[ -z "${SURFPOOL_VERSION}" ]]; then
  command -v surfpool >/dev/null || die "pass --surfpool VERSION or put surfpool on PATH"
  SURFPOOL_VERSION=$(surfpool --version | awk '{print $2}')
fi

pin() { jq -er --arg v "${SURFPOOL_VERSION}" ".pins[\$v].$1" "${HERE}/pins.json"; }
AGAVE=$(pin agave) || die "no pin for Surfpool ${SURFPOOL_VERSION} in pins.json"
RUSTC=$(pin rustc)
TAG=$(pin yellowstoneTag)
COMMIT=$(pin yellowstoneCommit)
PIN_FROM=$(jq -r --arg v "${SURFPOOL_VERSION}" '.pins[$v].pinAgaveFrom // empty' "${HERE}/pins.json")

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLATFORM=darwin-arm64; LIB=libyellowstone_grpc_geyser.dylib ;;
  Darwin-x86_64) PLATFORM=darwin-x64; LIB=libyellowstone_grpc_geyser.dylib ;;
  Linux-x86_64) PLATFORM=linux-x64; LIB=libyellowstone_grpc_geyser.so ;;
  Linux-aarch64) PLATFORM=linux-arm64; LIB=libyellowstone_grpc_geyser.so ;;
  *) die "unsupported platform $(uname -s)-$(uname -m)" ;;
esac

rustup toolchain list | grep -q "^${RUSTC}-" || rustup toolchain install "${RUSTC}" --profile minimal

WORK="${OUT}/src/${COMMIT}"
if [[ ! -d "${WORK}/.git" ]]; then
  rm -rf "${WORK}"
  mkdir -p "${WORK}"
  git -C "${WORK}" init -q
  git -C "${WORK}" remote add origin https://github.com/rpcpool/yellowstone-grpc.git
  git -C "${WORK}" fetch -q --depth 1 origin "${COMMIT}"
  git -C "${WORK}" checkout -q FETCH_HEAD
fi
[[ "$(git -C "${WORK}" rev-parse HEAD)" == "${COMMIT}" ]] || die "checkout is not ${COMMIT} (${TAG})"

export RUSTUP_TOOLCHAIN="${RUSTC}"
if [[ -n "${PIN_FROM}" && "${PIN_FROM}" != "${AGAVE}" ]]; then
  # The release pins its agave crates exactly ("=<version>"), and those crates
  # pin each other exactly, so they move together: rewrite the workspace
  # requirements to Surfpool's agave version and let Cargo re-resolve them.
  git -C "${WORK}" checkout -q -- Cargo.lock
  while IFS= read -r -d '' manifest; do
    git -C "${WORK}" checkout -q -- "${manifest}"
  done < <(git -C "${WORK}" ls-files -z '*Cargo.toml')
  while IFS= read -r manifest; do
    sed -i.bak "s/\"=${PIN_FROM//./\\.}\"/\"=${AGAVE}\"/g" "${WORK}/${manifest}"
    rm -f "${WORK}/${manifest}.bak"
  done < <(git -C "${WORK}" ls-files '*Cargo.toml')
  (cd "${WORK}" && cargo update -q -w)
  if awk -v v="${PIN_FROM}" '/^name = "(agave|solana)-/ { n = 1; next } n && /^version = / { gsub(/"/, "", $3); if ($3 == v) bad = 1; n = 0 } END { exit bad ? 0 : 1 }' "${WORK}/Cargo.lock"; then
    die "some agave crates are still at ${PIN_FROM} after pinning; inspect ${WORK}/Cargo.lock"
  fi
fi

(cd "${WORK}" && CARGO_TARGET_DIR="${OUT}/target" cargo build --release -p yellowstone-grpc-geyser --lib ${JOBS:+-j "${JOBS}"})

DEST="${OUT}/${SURFPOOL_VERSION}/${PLATFORM}"
mkdir -p "${DEST}"
cp "${OUT}/target/release/${LIB}" "${DEST}/${LIB}"
SHA=$(shasum -a 256 "${DEST}/${LIB}" | awk '{print $1}')

jq -n --arg lib "${LIB}" '{
  libpath: $lib, log: {level: "info"},
  grpc: {address: "127.0.0.1:10000", x_token: null, max_decoding_message_size: 4194304,
         snapshot_plugin_channel_capacity: null, snapshot_client_channel_capacity: 50000000,
         channel_capacity: 100000, unary_concurrency_limit: 100, unary_disabled: false},
  prometheus: {address: "127.0.0.1:10001"}}' >"${DEST}/yellowstone-grpc.json"

jq -n --arg surfpool "${SURFPOOL_VERSION}" --arg agave "${AGAVE}" --arg rustc "${RUSTC}" \
  --arg tag "${TAG}" --arg commit "${COMMIT}" --arg platform "${PLATFORM}" \
  --arg lib "${LIB}" --arg sha "${SHA}" '{
  schema: "arete.surfpool-geyser-build/v1", surfpool: $surfpool, agave: $agave, rustc: $rustc,
  yellowstone: {tag: $tag, commit: $commit}, platform: $platform,
  library: {file: $lib, sha256: $sha}}' >"${DEST}/manifest.json"

echo "${DEST}/${LIB}"
