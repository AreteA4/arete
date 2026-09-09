#!/usr/bin/env bash
# Sourced resolver. In an ordinary public checkout these functions use only the
# caller's explicit override/standalone fallback and never initialize anything.

arete_workspace_root() {
  local current
  current="$(cd "$1" && pwd -P)" || return
  while :; do
    if [[ -f "$current/.arete-workspace/workspace.json" ]]; then
      printf '%s\n' "$current"
      return
    fi
    [[ "$current" != / ]] || break
    current="${current%/*}"
    [[ -n "$current" ]] || current=/
  done
  if [[ -n "${ARETE_DEV_HOME:-}" && -f "$ARETE_DEV_HOME/.arete-workspace/workspace.json" ]]; then
    (cd "$ARETE_DEV_HOME" && pwd -P)
    return
  fi
  return 1
}

arete_workspace_admin() {
  local launcher="$1/.arete-workspace/bin/a4-admin"
  if [[ -x "$launcher" ]]; then printf '%s\n' "$launcher"; return; fi
  if [[ -n "${A4_ADMIN_BIN:-}" && -x "$A4_ADMIN_BIN" ]]; then printf '%s\n' "$A4_ADMIN_BIN"; return; fi
  if command -v a4-admin >/dev/null 2>&1; then command -v a4-admin; return; fi
  printf 'Managed workspace requires a4-admin; run bootstrap-dev.sh or build arete-admin.\n' >&2
  return 1
}

arete_repo_path() {
  local current="$1" repo="$2" explicit="${3:-}" fallback="${4:-}" root admin
  if root="$(arete_workspace_root "$current")"; then
    admin="$(arete_workspace_admin "$root")" || return
    # Bind cwd as well as root: stale task exports cannot select another task.
    (cd "$current" && "$admin" dev --root "$root" path "$repo")
  elif [[ -n "$explicit" ]]; then
    printf '%s\n' "$explicit"
  else
    printf '%s\n' "$fallback"
  fi
}

arete_service_env() {
  local current="$1" names="$2" root admin
  root="$(arete_workspace_root "$current")" || return 1
  admin="$(arete_workspace_admin "$root")" || return
  shift 2
  (cd "$current" && "$admin" dev --root "$root" services --names "$names" --shell bash "$@")
}
