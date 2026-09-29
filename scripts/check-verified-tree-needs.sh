#!/usr/bin/env bash
# Fail unless the record-verified-tree job of <workflow-file> needs every
# other job in that workflow. The job uploads the marker that lets a later
# run skip re-verifying the same tree, so a job it does not wait for could
# fail after the marker already claimed the tree was verified.
#
# Usage: check-verified-tree-needs.sh <workflow-file>
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <workflow-file>" >&2
  exit 2
fi
workflow="$1"
command -v yq >/dev/null 2>&1 || { echo "yq is required" >&2; exit 1; }

# Read both lists before comparing them: a failure inside a process
# substitution would leave diff comparing two empty lists, and pass.
jobs="$(yq -e '.jobs | keys | .[]' "$workflow" | grep -vx record-verified-tree | sort)"
needs="$(yq -e '.jobs.record-verified-tree.needs[]' "$workflow" | sort)"
if [ -z "$jobs" ] || [ -z "$needs" ]; then
  echo "Could not read the jobs or the record-verified-tree needs of $workflow" >&2
  exit 1
fi

if ! diff <(printf '%s\n' "$jobs") <(printf '%s\n' "$needs"); then
  echo "record-verified-tree in $workflow must need exactly every other job (< missing, > unknown)" >&2
  exit 1
fi
echo "record-verified-tree in $workflow needs every other job"
