#!/usr/bin/env bash
#
# Staging end-to-end driver for owner-private program and stack installs
# (advisor plan 005). Runs the real `a4` CLI against a staging registry with
# task-specific credentials supplied only through the environment; it refuses
# to run when any required variable is missing and never prints credentials.
#
# Required environment:
#   ARETE_E2E_API_URL          staging API base URL
#   ARETE_E2E_OWNER_API_KEY    secret key of the owning account
#   ARETE_E2E_OTHER_API_KEY    secret key of a second, non-owning account
#   ARETE_E2E_PRIVATE_PROGRAM  owner-private program lookup (alias or upr_...)
#   ARETE_E2E_PRIVATE_STACK    owner-private production stack atom name
# Optional:
#   A4_BIN                     prebuilt a4 binary (default: cargo run -p a4-cli)
set -euo pipefail

for variable in ARETE_E2E_API_URL ARETE_E2E_OWNER_API_KEY ARETE_E2E_OTHER_API_KEY \
    ARETE_E2E_PRIVATE_PROGRAM ARETE_E2E_PRIVATE_STACK; do
    if [[ -z "${!variable:-}" ]]; then
        echo "Refusing to run: $variable is not set (task-specific staging credentials are required)." >&2
        exit 2
    fi
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if [[ -n "${A4_BIN:-}" ]]; then
    A4=("$A4_BIN")
else
    cargo build --quiet --locked --manifest-path "$ROOT_DIR/cli/Cargo.toml"
    A4=("$ROOT_DIR/target/debug/a4")
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-e2e-private.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT
export ARETE_TELEMETRY_DISABLED=1
export ARETE_API_URL="$ARETE_E2E_API_URL"

write_credentials() {
    local key="$1"
    export ARETE_CREDENTIALS_PATH="$WORK_DIR/credentials-$RANDOM.toml"
    umask 077
    printf '[keys]\n"%s" = "%s"\n' "$ARETE_E2E_API_URL" "$key" >"$ARETE_CREDENTIALS_PATH"
}

new_project() {
    local dir="$1"
    mkdir -p "$dir"
    cat >"$dir/arete.toml" <<'TOML'
manifest_version = 1

[project]
name = "e2e-private"
private = true

[sdk]
targets = ["typescript", "rust", "python"]
TOML
}

lock_hash() {
    grep -E '^package_release_hash' "$1/arete.lock" | head -1 | sed -E 's/.*= *"([^"]+)".*/\1/'
}

echo "== owner installs the private program by lookup"
write_credentials "$ARETE_E2E_OWNER_API_KEY"
new_project "$WORK_DIR/owner-program"
(cd "$WORK_DIR/owner-program" && "${A4[@]}" install program "$ARETE_E2E_PRIVATE_PROGRAM")
first_hash="$(lock_hash "$WORK_DIR/owner-program")"
test -n "$first_hash"
echo "   exact lock: $first_hash"

echo "== reinstall honors the exact lock"
(cd "$WORK_DIR/owner-program" && "${A4[@]}" install)
test "$(lock_hash "$WORK_DIR/owner-program")" = "$first_hash"

echo "== generated Rust crate builds"
rust_dir="$(find "$WORK_DIR/owner-program/generated/rust" -maxdepth 2 -name Cargo.toml -print -quit)"
(cd "$(dirname "$rust_dir")" && cargo check --quiet)

echo "== generated Python package compiles"
python3 -m compileall -q "$WORK_DIR/owner-program/generated/python" >/dev/null

echo "== owner installs the private stack by atom name"
new_project "$WORK_DIR/owner-stack"
(cd "$WORK_DIR/owner-stack" && "${A4[@]}" install stack "$ARETE_E2E_PRIVATE_STACK")
test -n "$(lock_hash "$WORK_DIR/owner-stack")"

# The anti-oracle property is that an authenticated non-owner cannot tell a
# private package owned by someone else apart from one that does not exist.
# Comparing each response against a list of acceptable strings does not test
# that, because two different acceptable strings would both pass. Compare the
# two responses to each other instead.
echo "== a second account cannot distinguish someone else's private package from a missing one"
absent_program="definitely-absent-$(date +%s)-$$"
write_credentials "$ARETE_E2E_OTHER_API_KEY"
new_project "$WORK_DIR/other"
other_message="$( (cd "$WORK_DIR/other" && "${A4[@]}" install program "$ARETE_E2E_PRIVATE_PROGRAM" 2>&1) || true)"
new_project "$WORK_DIR/other-absent"
other_absent_message="$( (cd "$WORK_DIR/other-absent" && "${A4[@]}" install program "$absent_program" 2>&1) || true)"
# Normalise only the looked-up name, which legitimately differs, so any other
# divergence between the two responses fails the check.
normalise() { printf '%s' "$1" | sed -e "s/$ARETE_E2E_PRIVATE_PROGRAM/<NAME>/g" -e "s/$absent_program/<NAME>/g"; }
if [[ "$(normalise "$other_message")" != "$(normalise "$other_absent_message")" ]]; then
    echo "existence oracle: a non-owner gets different responses for a private package and an absent one" >&2
    printf 'private: %s\nabsent:  %s\n' "$other_message" "$other_absent_message" >&2
    exit 1
fi
case "$other_message" in
    *"unavailable to this account or unknown"*) ;;
    *) echo "unexpected isolation response: $other_message" >&2; exit 1 ;;
esac

echo "== an anonymous client is asked to log in rather than told anything about the package"
printf '[keys]\n' >"$ARETE_CREDENTIALS_PATH"
new_project "$WORK_DIR/anonymous"
anonymous_message="$( (cd "$WORK_DIR/anonymous" && "${A4[@]}" install program "$ARETE_E2E_PRIVATE_PROGRAM" 2>&1) || true)"
case "$anonymous_message" in
    *"requires a login"*) ;;
    *) echo "unexpected anonymous response: $anonymous_message" >&2; exit 1 ;;
esac

for message in "$other_message" "$other_absent_message" "$anonymous_message"; do
    if [[ "$message" == *"$ARETE_E2E_OWNER_API_KEY"* || "$message" == *"$ARETE_E2E_OTHER_API_KEY"* ]]; then
        echo "credential leaked into CLI output" >&2
        exit 1
    fi
done

echo "Owner-private install end-to-end checks passed."
