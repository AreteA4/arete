#!/usr/bin/env bash
# Print "true" if a successful run of <workflow-path> on main already verified
# the exact tree of HEAD, otherwise "false". The reasoning goes to stderr.
#
# Usage: find-verified-tree.sh <workflow-path> <artifact-prefix>
# Requires gh, GITHUB_REPOSITORY and GITHUB_REPOSITORY_ID.
#
# The verifying workflow's last job uploads an artifact named
# <artifact-prefix>-<tree>, and only when every other job succeeded. That
# artifact is a claim made by whatever code the run executed, so it is only
# accepted when the API shows that code was HEAD's own:
# - the run is from this repository (not a fork), and was a push or a manual
#   dispatch on main. Those events run exactly the commit they name. A
#   pull_request run executes a merge whose base the API does not record, with
#   the pull request's own workflow files, so it could upload a marker for a
#   tree it never tested. Its markers are never accepted;
# - the tree of the commit the run executed, read from the API rather than
#   from the artifact name, is HEAD's tree. The run then executed exactly
#   HEAD's sources and workflows, and a false marker would have to come from
#   HEAD's own code.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <workflow-path> <artifact-prefix>" >&2
  exit 2
fi
workflow_path="$1"
prefix="$2"
trusted_branch=main

tree="$(git rev-parse 'HEAD^{tree}')"
: "${GITHUB_REPOSITORY:?}" "${GITHUB_REPOSITORY_ID:?}"
run_ids="$(gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts?name=${prefix}-${tree}&per_page=100" \
  --jq ".artifacts[] | select(.expired == false)
        | select(.workflow_run.repository_id == ${GITHUB_REPOSITORY_ID} and .workflow_run.head_repository_id == ${GITHUB_REPOSITORY_ID})
        | .workflow_run.id")"
for run_id in $run_ids; do
  # head_branch goes last: it can be empty, and read would merge an empty
  # middle field into the next one.
  IFS=$'\t' read -r conclusion path event head_sha branch < <(gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${run_id}" \
    --jq '[(.conclusion // "none"), .path, .event, .head_sha, (.head_branch // "")] | @tsv') || continue
  case "$path" in "$workflow_path" | "$workflow_path"@*) ;; *) continue ;; esac
  if [ "$conclusion" != success ]; then
    echo "run $run_id ignored: it concluded $conclusion" >&2
    continue
  fi
  case "$event" in
    push | workflow_dispatch) ;;
    *) echo "run $run_id ignored: only push and dispatch runs on $trusted_branch count, not $event" >&2; continue ;;
  esac
  if [ "$branch" != "$trusted_branch" ]; then
    echo "run $run_id ignored: it ran on ${branch:-no branch}, not $trusted_branch" >&2
    continue
  fi
  run_tree="$(gh api "repos/${GITHUB_REPOSITORY}/git/commits/${head_sha}" --jq '.tree.sha')"
  if [ "$run_tree" != "$tree" ]; then
    echo "run $run_id ignored: its commit $head_sha has tree $run_tree, not $tree" >&2
    continue
  fi
  echo "tree $tree verified by run $run_id ($event on $trusted_branch at $head_sha)" >&2
  echo true
  exit 0
done
echo "no successful verification of tree $tree" >&2
echo false
