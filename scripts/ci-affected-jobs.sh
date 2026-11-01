#!/usr/bin/env bash
# Decide which CI jobs a pull request can affect. Reads changed paths from
# stdin, one per line, and prints `<job>=true|false` for every job CI may skip.
#
# Usage: git diff --name-only --no-renames <base> <merge> | ci-affected-jobs.sh
#        ci-affected-jobs.sh --jobs    (list the jobs this script decides)
#
# A path this script does not name runs every job, so a new directory or a
# shared input is safe until someone classifies it. Only add a narrower rule
# after checking what reads the path: the suites read across directories, and
# those reads are listed below because they are easy to miss.
#
#   python/arete-sdk           Rust tests import generated packages against it
#   cli/tests/golden           the Python tests load the installed-protocol golden
#   arete-idl/tests/fixtures   the TypeScript hash tests read them
#   docs/src/data              the TypeScript protocol tests read them
#   typescript/core/package.json        a Rust test compares the zod range
#   typescript/protocol/package.json    a Rust test reads the package version
#   typescript/protocol/src/builtins.generated.ts  a Rust test detects staleness
#   typescript/dev-ui          Rust clippy embeds its build into the CLI
#   test-vectors, tests/fixtures, examples/ore-{rust,typescript,python}
#                              several languages read them, so they run everything
#
# Main and workflow_dispatch runs never come here: CI runs them in full.
set -euo pipefail

all_jobs="
  rust-workspace rust-features rust-consumers rust-clippy rust-solana-adapter
  windows-cli-lifecycle generated-example-sdks ore-react
  typescript-core typescript-hash typescript-dev-ui typescript-protocol
  typescript-automations typescript-automations-postgres
  typescript-adapters typescript-react
  python-matrix docs benchmarks subscriptions-billing
"

if [ "${1:-}" = "--jobs" ]; then
  printf '%s\n' $all_jobs
  exit 0
fi
if [ "$#" -ne 0 ]; then
  echo "usage: $0 [--jobs] < changed-paths" >&2
  exit 2
fi

# Everything that builds or tests the Rust workspace and its generated output.
rust_jobs="
  rust-workspace rust-features rust-consumers rust-clippy rust-solana-adapter
  windows-cli-lifecycle generated-example-sdks
"
# Everything that builds against the TypeScript SDK from this checkout.
sdk_consumers="
  typescript-core typescript-protocol
  typescript-automations typescript-automations-postgres
  typescript-adapters typescript-react ore-react generated-example-sdks
"

jobs_for_path() {
  case "$1" in
    README.md | CONTRIBUTING.md | packages/*) ;; # The always-on jobs cover these.

    docs/src/data/*) echo docs typescript-protocol ;;
    docs/*) echo docs ;;
    benchmarks/*) echo benchmarks ;;

    python/arete-sdk/tests/*) echo python-matrix ;;
    python/arete-sdk/*) echo python-matrix rust-workspace ;;

    typescript/core/package.json) echo "$sdk_consumers" rust-workspace ;;
    typescript/core/*) echo "$sdk_consumers" ;;
    typescript/hash/*) echo typescript-hash typescript-protocol ;;
    typescript/dev-ui/*) echo typescript-dev-ui rust-clippy ;;
    typescript/protocol/package.json | typescript/protocol/src/builtins.generated.ts)
      echo typescript-protocol rust-workspace
      ;;
    typescript/protocol/*) echo typescript-protocol ;;
    typescript/automations/*) echo typescript-automations typescript-automations-postgres ;;
    typescript/adapters/kit/*) echo typescript-adapters typescript-react ore-react ;;
    typescript/adapters/web3js/*) echo typescript-adapters ore-react ;;
    typescript/react/*) echo typescript-react ore-react ;;

    examples/ore-react/*) echo ore-react generated-example-sdks ;;
    examples/subscriptions-billing/*) echo subscriptions-billing ;;

    cli/tests/golden/*) echo "$rust_jobs" python-matrix ;;
    arete-idl/tests/fixtures/*) echo "$rust_jobs" typescript-hash ;;
    arete/* | arete-artifacts/* | arete-hash/* | arete-idl/* | arete-macros/* | cli/* | \
      interpreter/* | rust/* | stacks/* | Cargo.toml | Cargo.lock | Cross.toml | rust-toolchain.toml)
      echo "$rust_jobs"
      ;;

    *) echo "$all_jobs" ;;
  esac
}

affected=" "
seen_path=false
while IFS= read -r path; do
  [ -n "$path" ] || continue
  seen_path=true
  # Word splitting flattens each rule's job list.
  for job in $(jobs_for_path "$path"); do
    affected="$affected$job "
  done
done

# No paths means the diff could not say what changed, not that nothing did.
if [ "$seen_path" = false ]; then
  affected=" $(printf '%s ' $all_jobs)"
fi

for job in $all_jobs; do
  case "$affected" in
    *" $job "*) echo "$job=true" ;;
    *) echo "$job=false" ;;
  esac
done
