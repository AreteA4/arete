#!/usr/bin/env bash
# Optional internal project hook. It does nothing in an unmanaged public clone.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "$script_dir/arete-workspace.sh"
target="${T3CODE_WORKTREE_PATH:-${1:-$PWD}}"
project="${T3CODE_PROJECT_ROOT:-$target}"
if ! root="$(ARETE_DEV_HOME= arete_workspace_root "$target")"; then
  # A frontend-owned worktree may live outside the workspace. Its explicit
  # project marker must outrank stale GUI exports from another workspace.
  if ! root="$(ARETE_DEV_HOME= arete_workspace_root "$project")"; then
    if ! root="$(arete_workspace_root "$target")"; then
      printf 'Arete preparation not configured for this public/standalone checkout.\n'
      exit 0
    fi
  fi
fi
admin="$(arete_workspace_admin "$root")"
target="$(cd "$target" && pwd -P)"
# This blocks until the engine has persisted a completion/failure receipt. T3's
# earlier "started" response is not a readiness result. No model session starts
# here, and failed preparation leaves the source worktree intact.
exec "$admin" --output json dev --root "$root" prepare --path "$target"
