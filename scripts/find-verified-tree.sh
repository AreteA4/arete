#!/usr/bin/env bash
# Print "true" if a successful run of <workflow-path> from this repository
# already verified the exact tree of HEAD, otherwise "false". The reasoning
# goes to stderr.
#
# Usage: find-verified-tree.sh <workflow-path> <artifact-prefix>
# Requires gh, GITHUB_REPOSITORY and GITHUB_REPOSITORY_ID.
#
# The verifying workflow's last job uploads an artifact named
# <artifact-prefix>-<tree>, and only when every other job succeeded. A
# pull_request run checks out refs/pull/N/merge, so if the base branch has not
# moved before the merge, the merge commit has the same tree: identical
# sources, lockfiles, toolchain file and workflows. That tree needs no second
# full run.
#
# Markers from fork pull requests are ignored: a fork's run can upload an
# artifact with any name, so only runs whose head repository is this
# repository count.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <workflow-path> <artifact-prefix>" >&2
  exit 2
fi
workflow_path="$1"
prefix="$2"

tree="$(git rev-parse 'HEAD^{tree}')"
: "${GITHUB_REPOSITORY:?}" "${GITHUB_REPOSITORY_ID:?}"
run_ids="$(gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts?name=${prefix}-${tree}&per_page=100" \
  --jq ".artifacts[] | select(.expired == false)
        | select(.workflow_run.repository_id == ${GITHUB_REPOSITORY_ID} and .workflow_run.head_repository_id == ${GITHUB_REPOSITORY_ID})
        | .workflow_run.id")"
for run_id in $run_ids; do
  read -r conclusion path < <(gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${run_id}" --jq '[(.conclusion // "none"), .path] | @tsv')
  case "$path" in "$workflow_path" | "$workflow_path"@*) ;; *) continue ;; esac
  if [ "$conclusion" = success ]; then echo "tree $tree verified by run $run_id" >&2; echo true; exit 0; fi
done
echo "no successful verification of tree $tree" >&2
echo false
