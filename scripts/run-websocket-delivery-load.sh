#!/usr/bin/env bash
#
# Run the WebSocket list-delivery load profiles and keep their JSON
# summaries with the machine and runtime context they were measured on.
#
# Usage:
#   ./scripts/run-websocket-delivery-load.sh [--release] [OUTPUT.jsonl]
#
# Without --release this is exactly the canonical baseline command recorded in
# docs/internal/websocket-delivery-load-baseline.md:
#
#   cargo test -p arete-server websocket::load_tests::profiles -- --ignored --nocapture
#
# --release  Also build optimized. Useful context, but never compare a release
#            run against a debug baseline.
#
# Output (default target/websocket-delivery-load/<commit>[-release].*):
#   .jsonl    one summary per scenario, exactly as the test printed it
#   .log      full test output
#   .context  commit, toolchain, host, CPU and any ARETE_WS_LOAD_* overrides
#
# Exits with the test's status, so a scenario that fails its correctness
# checks fails this script too. The summaries are written either way.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

RELEASE_FLAG=""
SUFFIX=""
if [[ "${1:-}" == "--release" ]]; then
    RELEASE_FLAG="--release"
    SUFFIX="-release"
    shift
fi

COMMIT="$(git rev-parse --short=8 HEAD)"
OUT="${1:-target/websocket-delivery-load/${COMMIT}${SUFFIX}.jsonl}"
BASE="${OUT%.jsonl}"
mkdir -p "$(dirname "$OUT")"

cpu_description() {
    if sysctl -n machdep.cpu.brand_string >/dev/null 2>&1; then
        echo "$(sysctl -n machdep.cpu.brand_string) ($(sysctl -n hw.ncpu) logical CPUs)"
    elif [[ -r /proc/cpuinfo ]]; then
        echo "$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs) ($(nproc) logical CPUs)"
    else
        echo "unknown"
    fi
}

{
    echo "commit: $(git rev-parse HEAD)"
    if ! git diff --quiet HEAD -- rust/arete-server; then
        echo "worktree: rust/arete-server has uncommitted changes"
    fi
    echo "measured_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "rustc: $(rustc --version)"
    echo "build: ${RELEASE_FLAG:-debug (cargo test default)}"
    echo "host: $(uname -srm)"
    echo "cpu: $(cpu_description)"
    env | grep '^ARETE_WS_LOAD_' | sed 's/^/override: /' || true
} > "$BASE.context"

set +e
# shellcheck disable=SC2086 # RELEASE_FLAG is empty or a single flag.
cargo test -p arete-server $RELEASE_FLAG websocket::load_tests::profiles \
    -- --ignored --nocapture 2>&1 | tee "$BASE.log"
STATUS=${PIPESTATUS[0]}
set -e

grep -o '{"scenario".*' "$BASE.log" > "$OUT" || true
echo "wrote $(wc -l < "$OUT" | tr -d ' ') summaries to $OUT (context: $BASE.context)"
exit "$STATUS"
