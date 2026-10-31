#!/usr/bin/env bash
# CI lets a job skip only on what ci-affected-jobs.sh prints, and a skipped job
# counts as passing for required checks. This pins the path rules, including
# the cross-directory reads they exist for, and checks ci.yml is wired to them.
set -euo pipefail

scripts_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$scripts_dir/ci-affected-jobs.sh"
workflow="$scripts_dir/../.github/workflows/ci.yml"
command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
command -v yq >/dev/null 2>&1 || { echo "yq is required" >&2; exit 1; }

failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

all_jobs="$(bash "$script" --jobs | sort | xargs)"
rust_jobs="generated-example-sdks rust-clippy rust-consumers rust-features rust-solana-adapter rust-workspace windows-cli-lifecycle"
sdk_consumers="generated-example-sdks ore-react typescript-adapters typescript-automations typescript-automations-postgres typescript-core typescript-protocol typescript-react"

# check <label> <expected jobs, in any order> <changed path>...
check() {
  local label="$1" expected="$2" actual
  shift 2
  expected="$(printf '%s\n' $expected | sed '/^$/d' | sort -u | xargs)"
  actual="$(printf '%s\n' "$@" | bash "$script" | sed -n 's/=true$//p' | sort | xargs)"
  [ "$actual" = "$expected" ] || fail "$label: expected [$expected], got [$actual]"
}

check "root README" "" README.md
check "npm wrapper" "" packages/arete/bin/a4.js
check "docs page" "docs" docs/src/content/docs/index.mdx
check "docs data the protocol tests read" "docs typescript-protocol" docs/src/data/protocol-expressions.json
check "benchmarks" "benchmarks" benchmarks/src/env.ts
check "Python tests" "python-matrix" python/arete-sdk/tests/test_views.py
check "Python SDK Rust tests import" "python-matrix rust-workspace" python/arete-sdk/arete/__init__.py
check "core source" "$sdk_consumers" typescript/core/src/index.ts
check "core manifest a Rust test reads" "$sdk_consumers rust-workspace" typescript/core/package.json
check "hash" "typescript-hash typescript-protocol" typescript/hash/src/index.ts
check "dev UI embedded by clippy" "typescript-dev-ui rust-clippy" typescript/dev-ui/src/main.tsx
check "protocol source" "typescript-protocol" typescript/protocol/src/index.ts
check "protocol manifest a Rust test reads" "typescript-protocol rust-workspace" typescript/protocol/package.json
check "generated builtins a Rust test checks" "typescript-protocol rust-workspace" typescript/protocol/src/builtins.generated.ts
check "automations" "typescript-automations typescript-automations-postgres" typescript/automations/src/index.ts
check "Kit adapter" "typescript-adapters typescript-react ore-react" typescript/adapters/kit/src/index.ts
check "web3.js adapter" "typescript-adapters ore-react" typescript/adapters/web3js/src/index.ts
check "React" "typescript-react ore-react" typescript/react/src/index.ts
check "React example" "ore-react generated-example-sdks" examples/ore-react/src/App.tsx
check "Rust crate" "$rust_jobs" cli/src/main.rs
check "Rust lockfile" "$rust_jobs" Cargo.lock
check "Rust toolchain" "$rust_jobs" rust-toolchain.toml
check "ORE stack" "$rust_jobs" stacks/ore/src/lib.rs
check "golden the Python tests read" "$rust_jobs python-matrix" cli/tests/golden/installed-protocol-python/modules/x/__init__.py
check "IDL fixture the hash tests read" "$rust_jobs typescript-hash" arete-idl/tests/fixtures/address-lookup-table.json
check "paths combine" "docs python-matrix" docs/index.md python/arete-sdk/tests/test_views.py

# Anything shared or unknown runs everything.
for path in test-vectors/hash-v1.json tests/fixtures/websocket-v2/manifest.json \
  examples/ore-rust/src/main.rs examples/ore-typescript/src/index.ts \
  examples/ore-python/main.py scripts/ci-affected-jobs.sh .github/workflows/ci.yml \
  .release-please-manifest.json release-please-config.json package-lock.json \
  typescript/new-package/index.ts python/new-sdk/setup.py new-directory/file \
  '"docs/caf\303\251.md"'; do
  check "unknown path $path" "$all_jobs" "$path"
done
check "one unknown path among narrow ones" "$all_jobs" docs/index.md new-directory/file

# An empty diff is not evidence that nothing changed.
actual="$(printf '' | bash "$script" | sed -n 's/=true$//p' | sort | xargs)"
[ "$actual" = "$all_jobs" ] || fail "no input must run every job, got [$actual]"
actual="$(printf '\n' | bash "$script" | sed -n 's/=true$//p' | sort | xargs)"
[ "$actual" = "$all_jobs" ] || fail "a blank line must run every job, got [$actual]"

# ci.yml: each decided job declares its output and gates on it, fail-open.
for job in $all_jobs; do
  output="$(yq -r ".jobs.changes.outputs.\"$job\"" "$workflow")"
  [ "$output" = "\${{ steps.affected.outputs.$job }}" ] || fail "changes does not output $job"
  condition="$(yq -r ".jobs.\"$job\".if" "$workflow")"
  [ "$condition" = "\${{ !cancelled() && needs.changes.outputs.$job != 'false' }}" ] ||
    fail "$job has the wrong condition: $condition"
done

# A new job has to choose: be decided here, or always run.
always_on="changes check-crate-files linked-release-versions lockfile-sync npm-cli-wrapper npm-publish-auth python record-verified-tree rust"
expected="$(printf '%s\n' $all_jobs $always_on | sort | xargs)"
actual="$(yq -r '.jobs | keys | .[]' "$workflow" | sort | xargs)"
[ "$actual" = "$expected" ] || fail "ci.yml jobs differ from the decided and always-on lists: [$actual]"

# The required aggregators: run the workflow's own check against each outcome.
# aggregate <expected exit> <job> <NEEDS JSON>
aggregate() {
  local expected="$1" job="$2" needs="$3" command status=0
  command="$(yq -r ".jobs.$job.steps[0].run" "$workflow")"
  NEEDS="$needs" bash -e -c "$command" >/dev/null 2>&1 || status=$?
  [ "$status" = "$expected" ] || fail "$job with $needs: expected exit $expected, got $status"
}
for job in rust python; do
  dep=rust-workspace
  [ "$job" = rust ] || dep=python-matrix
  decided_skip="{\"result\":\"success\",\"outputs\":{\"$dep\":\"false\"}}"
  decided_run="{\"result\":\"success\",\"outputs\":{\"$dep\":\"true\"}}"
  aggregate 0 "$job" "{\"changes\":$decided_run,\"$dep\":{\"result\":\"success\",\"outputs\":{}}}"
  aggregate 0 "$job" "{\"changes\":$decided_skip,\"$dep\":{\"result\":\"skipped\",\"outputs\":{}}}"
  aggregate 1 "$job" "{\"changes\":$decided_run,\"$dep\":{\"result\":\"failure\",\"outputs\":{}}}"
  aggregate 1 "$job" "{\"changes\":$decided_skip,\"$dep\":{\"result\":\"failure\",\"outputs\":{}}}"
  aggregate 1 "$job" "{\"changes\":$decided_run,\"$dep\":{\"result\":\"cancelled\",\"outputs\":{}}}"
  # Skipped without a decision: a cancelled run or a failed classifier.
  aggregate 1 "$job" "{\"changes\":$decided_run,\"$dep\":{\"result\":\"skipped\",\"outputs\":{}}}"
  aggregate 1 "$job" "{\"changes\":{\"result\":\"cancelled\",\"outputs\":{}},\"$dep\":{\"result\":\"skipped\",\"outputs\":{}}}"
  # A failed classifier runs the job, and its result is what counts.
  aggregate 0 "$job" "{\"changes\":{\"result\":\"failure\",\"outputs\":{}},\"$dep\":{\"result\":\"success\",\"outputs\":{}}}"
done

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "CI job selection behaves as expected"
