#!/usr/bin/env bash
#
# Generate a minimal Rust program SDK and a minimal Rust stack SDK with the
# CLI from this checkout, assert both depend on the linked `arete-a4-sdk`
# release, and prove they compile.
#
# Usage:
#   ./scripts/check-generated-rust-crates.sh --mode local
#   ./scripts/check-generated-rust-crates.sh --mode registry
#
# --mode local     Pre-publication (CI / release PR). The generated crates are
#                  compiled against this checkout's `rust/arete-a4-sdk` via a
#                  temporary `[patch.crates-io]` appended to the *temporary*
#                  copy only. The patch never reaches user output.
# --mode registry  Post-publication (release workflow). The generated crates
#                  are compiled with no patch and no path dependency, so the
#                  exact emitted `arete-a4-sdk` version must resolve from
#                  crates.io.
#
# Environment:
#   A4_BIN   Optional prebuilt `a4` binary. Defaults to `cargo run -p a4-cli`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MODE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)
            MODE="${2:-}"
            shift 2
            ;;
        --mode=*)
            MODE="${1#--mode=}"
            shift
            ;;
        *)
            echo "Unknown argument: $1" >&2
            echo "Usage: $0 --mode <local|registry>" >&2
            exit 2
            ;;
    esac
done

if [[ "$MODE" != "local" && "$MODE" != "registry" ]]; then
    echo "Usage: $0 --mode <local|registry>" >&2
    exit 2
fi

# The interpreter's package version is the linked SDK version emitted into
# generated crates (see GENERATED_RUST_SDK_VERSION in interpreter/src/rust.rs).
package_version() {
    awk '
        /^\[/ { in_package = ($0 == "[package]") }
        in_package && /^[[:space:]]*version[[:space:]]*=/ {
            gsub(/.*=[[:space:]]*"/, ""); gsub(/".*/, ""); print; exit
        }
    ' "$1"
}

INTERPRETER_VERSION="$(package_version "$ROOT_DIR/interpreter/Cargo.toml")"
SDK_VERSION="$(package_version "$ROOT_DIR/rust/arete-a4-sdk/Cargo.toml")"
if [[ -z "$INTERPRETER_VERSION" || -z "$SDK_VERSION" ]]; then
    echo "Unable to read arete-interpreter / arete-a4-sdk package versions" >&2
    exit 1
fi
if [[ "$INTERPRETER_VERSION" != "$SDK_VERSION" ]]; then
    echo "arete-interpreter ($INTERPRETER_VERSION) and arete-a4-sdk ($SDK_VERSION) are not at the same linked version" >&2
    echo "Reconcile the linked release group before checking generated crates." >&2
    exit 1
fi
EXPECTED_DEPENDENCY="arete-sdk = { package = \"arete-a4-sdk\", version = \"$SDK_VERSION\" }"

FIXTURE_DIR="$ROOT_DIR/stacks/ore/.arete"
PROGRAM_SPEC="$FIXTURE_DIR/ore.program-spec.json"
STACK_MANIFEST="$FIXTURE_DIR/OreStream.stack-manifest.json"
for fixture in "$PROGRAM_SPEC" "$STACK_MANIFEST"; do
    if [[ ! -f "$fixture" ]]; then
        echo "Missing checked-in fixture: $fixture" >&2
        exit 1
    fi
done

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-generated-crates.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT

if [[ -n "${A4_BIN:-}" ]]; then
    A4_CMD=("$A4_BIN")
else
    echo "Building a4 from this checkout..."
    cargo build --quiet --locked --manifest-path "$ROOT_DIR/cli/Cargo.toml"
    A4_CMD=("$ROOT_DIR/target/debug/a4")
fi

export ARETE_TELEMETRY_DISABLED=1

echo "Generating a standalone Rust program crate..."
# Project manifests reference artifacts and outputs by manifest-relative path.
PROGRAM_PROJECT="$WORK_DIR/program-project"
mkdir -p "$PROGRAM_PROJECT"
cp "$PROGRAM_SPEC" "$PROGRAM_PROJECT/ore.program-spec.json"
cat >"$PROGRAM_PROJECT/arete.toml" <<'TOML'
manifest_version = 1

[project]
name = "generated-crate-check"
private = true

[sdk]
targets = ["rust"]

[dependencies.programs.ore]
source = { path = "./ore.program-spec.json" }
targets = ["rust"]
outputs = { rust = "./generated/ore-program" }
TOML
(cd "$PROGRAM_PROJECT" && "${A4_CMD[@]}" --config "$PROGRAM_PROJECT/arete.toml" install)
PROGRAM_CRATE="$PROGRAM_PROJECT/generated/ore-program"

echo "Generating a standalone Rust stack crate..."
STACK_CRATE="$WORK_DIR/ore-stack"
"${A4_CMD[@]}" sdk create --manifest "$STACK_MANIFEST" --rust \
    --output "$STACK_CRATE" --crate-name ore-stack-check

check_manifest() {
    local crate_dir="$1"
    local manifest="$crate_dir/Cargo.toml"
    if [[ ! -f "$manifest" ]]; then
        echo "Generated crate has no Cargo.toml: $crate_dir" >&2
        exit 1
    fi
    if ! grep -qxF "$EXPECTED_DEPENDENCY" "$manifest"; then
        echo "Generated $manifest does not depend on the linked SDK release:" >&2
        echo "  expected: $EXPECTED_DEPENDENCY" >&2
        grep -n 'arete' "$manifest" >&2 || true
        exit 1
    fi
    if grep -qE 'arete-a4-sdk[^\n]*"0\.4"' "$manifest"; then
        echo "Generated $manifest still pins the obsolete 0.4 SDK" >&2
        exit 1
    fi
    # User-facing output must never contain local path or patch dependencies.
    if grep -qE '^\[patch|path[[:space:]]*=' "$manifest"; then
        echo "Generated $manifest contains a path or patch dependency" >&2
        exit 1
    fi
}

check_manifest "$PROGRAM_CRATE"
check_manifest "$STACK_CRATE"
echo "Generated manifests depend on arete-a4-sdk $SDK_VERSION"

compile_crate() {
    local crate_dir="$1"
    local manifest="$crate_dir/Cargo.toml"
    # Keep the check crates out of any enclosing workspace.
    if ! grep -q '^\[workspace\]' "$manifest"; then
        printf '\n[workspace]\n' >>"$manifest"
    fi
    case "$MODE" in
        local)
            # Temporary pre-publication patch: compile against this checkout's
            # unpublished linked crate. arete-a4-sdk has no other arete
            # dependencies, so it is the only crate that needs patching.
            printf '\n[patch.crates-io]\narete-a4-sdk = { path = %s }\n' \
                "\"$ROOT_DIR/rust/arete-a4-sdk\"" >>"$manifest"
            (cd "$crate_dir" && cargo check --quiet)
            ;;
        registry)
            if grep -qE '^\[patch|path[[:space:]]*=' "$manifest"; then
                echo "registry mode must not carry a path or patch dependency: $manifest" >&2
                exit 1
            fi
            (cd "$crate_dir" && cargo generate-lockfile --quiet && cargo check --quiet --locked)
            if ! grep -qE "^name = \"arete-a4-sdk\"" "$crate_dir/Cargo.lock"; then
                echo "arete-a4-sdk was not resolved into $crate_dir/Cargo.lock" >&2
                exit 1
            fi
            if ! awk -v want="$SDK_VERSION" '
                /^name = "arete-a4-sdk"$/ { seen = 1; next }
                seen && /^version = / { gsub(/version = |"/, ""); if ($0 == want) found = 1; seen = 0 }
                END { exit found ? 0 : 1 }
            ' "$crate_dir/Cargo.lock"; then
                echo "crates.io resolved a different arete-a4-sdk than the emitted $SDK_VERSION" >&2
                grep -A1 '^name = "arete-a4-sdk"' "$crate_dir/Cargo.lock" >&2 || true
                exit 1
            fi
            ;;
    esac
}

echo "Compiling generated crates (mode: $MODE)..."
compile_crate "$PROGRAM_CRATE"
compile_crate "$STACK_CRATE"
echo "Generated Rust program and stack crates build against arete-a4-sdk $SDK_VERSION ($MODE mode)."
