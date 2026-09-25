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

if ! diff <(yq '.jobs | keys | .[]' "$1" | grep -vx record-verified-tree | sort) \
     <(yq '.jobs.record-verified-tree.needs[]' "$1" | sort); then
  echo "record-verified-tree in $1 must need exactly every other job (< missing, > unknown)" >&2
  exit 1
fi
echo "record-verified-tree in $1 needs every other job"
