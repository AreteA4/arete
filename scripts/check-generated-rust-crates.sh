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

# Compile probe: every account of the ProgramSpec has a typed reader on the
# program accessor, decoding into a model the crate root exports under the
# account's name (the bindings a program package's extension bundle uses).
PROGRAM_LIB="$(awk -F'"' '/^name = / { gsub("-", "_", $2); print $2; exit }' "$PROGRAM_CRATE/Cargo.toml")"
mkdir -p "$PROGRAM_CRATE/examples"
python3 - "$PROGRAM_SPEC" "$PROGRAM_LIB" >"$PROGRAM_CRATE/examples/account_bindings.rs" <<'PY'
import json
import re
import sys

spec = json.load(open(sys.argv[1]))
lib = sys.argv[2]
accounts = spec["payload"]["idlSnapshot"]["accounts"]
if not accounts:
    sys.exit("the ProgramSpec declares no accounts to probe")
print("#![allow(dead_code)]")
print(f"use {lib}::OrePrograms;")
print("")
print("fn probe(")
print("    client: &arete_sdk::Arete<arete_sdk::ProgramStack<OrePrograms>>,")
print(") -> Result<(), arete_sdk::AreteError> {")
print("    let program = &client.programs.ore;")
for account in accounts:
    name = account["name"]
    model = name[:1].upper() + name[1:]
    reader = re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()
    print(f"    let _: arete_sdk::AccountReader<{lib}::{model}> = program.{reader}_accounts()?;")
print("    Ok(())")
print("}")
print("")
print("fn main() {}")
PY

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
    # Compile the public optional adapter in this existing consumer as well.
    # This adds no transaction lifecycle or separate fixture crate.
    printf '\n[features]\ncheck-solana-adapter = ["arete-sdk/solana-adapter"]\n' >>"$manifest"
    mkdir -p "$crate_dir/examples"
    cat >"$crate_dir/examples/adapter_import.rs" <<'RS'
use arete_sdk::adapters::solana::{SolanaAdapterConfig, SolanaWalletAdapter};
fn main() {
    let _ = SolanaAdapterConfig::default();
    let _ = std::any::type_name::<SolanaWalletAdapter>();
}
RS
    case "$MODE" in
        local)
            # Temporary pre-publication patch: compile against this checkout's
            # unpublished linked crate. arete-a4-sdk has no other arete
            # dependencies, so it is the only crate that needs patching.
            printf '\n[patch.crates-io]\narete-a4-sdk = { path = %s }\n' \
                "\"$ROOT_DIR/rust/arete-a4-sdk\"" >>"$manifest"
            (cd "$crate_dir" && cargo check --quiet --features check-solana-adapter --examples)
            ;;
        registry)
            if grep -qE '^\[patch|path[[:space:]]*=' "$manifest"; then
                echo "registry mode must not carry a path or patch dependency: $manifest" >&2
                exit 1
            fi
            (cd "$crate_dir" && cargo generate-lockfile --quiet && cargo check --quiet --locked --features check-solana-adapter --examples)
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

# The golden installs with program package extensions: one Rust bundle staged
# at a standalone program crate's root and inside a stack crate's
# `programs::vault` module must compile in both.
GOLDEN_DIR="$ROOT_DIR/cli/tests/golden/installed-rust-python/rust"
EXTENDED_CRATES=()
for kind in programs stacks; do
    crate_dir="$WORK_DIR/installed-$kind-vault"
    cp -R "$GOLDEN_DIR/$kind/vault" "$crate_dir"
    sed "s/{{ARETE_VERSION}}/$SDK_VERSION/" "$crate_dir/Cargo.toml" >"$crate_dir/Cargo.toml.tmp"
    mv "$crate_dir/Cargo.toml.tmp" "$crate_dir/Cargo.toml"
    check_manifest "$crate_dir"
    EXTENDED_CRATES+=("$crate_dir")
done

echo "Compiling generated crates (mode: $MODE)..."
compile_crate "$PROGRAM_CRATE"
compile_crate "$STACK_CRATE"
for crate_dir in "${EXTENDED_CRATES[@]}"; do
    compile_crate "$crate_dir"
done
echo "Generated Rust program and stack crates, with and without program extensions, build against arete-a4-sdk $SDK_VERSION ($MODE mode)."
