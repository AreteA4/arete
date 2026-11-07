#!/usr/bin/env bash
# The release gate trusts find-verified-tree.sh to accept only a successful
# push or dispatch run of the named workflow on main, from this repository
# rather than a fork, whose own commit has exactly this tree and which
# uploaded an unexpired marker for it. Markers from pull_request runs never
# count. This runs the script against canned API responses through a fake gh,
# one case per rule.
set -euo pipefail

script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/find-verified-tree.sh"
command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A repository whose HEAD tree the script reads.
git init -q "$work/repo"
git -C "$work/repo" -c user.name=test -c user.email=test@example.invalid \
  commit -q --allow-empty -m tree
tree="$(git -C "$work/repo" rev-parse 'HEAD^{tree}')"
other_tree=0000000000000000000000000000000000000001

# gh api <path> --jq <filter>, answered from $FAKE_DIR through the real jq.
mkdir -p "$work/bin" "$work/api"
cat > "$work/bin/gh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = api ] || { echo "fake gh supports only 'api'" >&2; exit 2; }
path="$2"
shift 2
filter="."
while [ "$#" -gt 0 ]; do
  case "$1" in
    --jq) filter="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$path" in
  */actions/artifacts\?*) file="$FAKE_DIR/artifacts.json" ;;
  */actions/runs/*) file="$FAKE_DIR/run-${path##*/}.json" ;;
  */git/commits/*) file="$FAKE_DIR/commit-${path##*/}.json" ;;
  *) echo "fake gh: unexpected path $path" >&2; exit 2 ;;
esac
jq -r "$filter" "$file"
FAKE
chmod +x "$work/bin/gh"

repo_id=111
fork_id=222

# artifact <run-id> <head-repository-id> <expired>
artifact() {
  printf '{"expired":%s,"workflow_run":{"id":%s,"repository_id":%s,"head_repository_id":%s}}' \
    "$3" "$1" "$repo_id" "$2"
}

failures=0
# check <label> <expected> <artifacts JSON array> [<run>]...
# Each <run> is <id>:<conclusion>:<path>:<event>:<branch>:<commit tree>; the
# run's commit is "commit<id>", whose tree the fake commits API reports.
check() {
  local label="$1" expected="$2" artifacts="$3" run id conclusion path event branch commit_tree actual
  shift 3
  rm -f "$work/api"/*
  printf '{"artifacts":%s}' "$artifacts" > "$work/api/artifacts.json"
  for run in "$@"; do
    IFS=: read -r id conclusion path event branch commit_tree <<<"$run"
    printf '{"conclusion":"%s","path":"%s","event":"%s","head_branch":"%s","head_sha":"commit%s"}' \
      "$conclusion" "$path" "$event" "$branch" "$id" > "$work/api/run-$id.json"
    printf '{"tree":{"sha":"%s"}}' "$commit_tree" > "$work/api/commit-commit$id.json"
  done
  actual="$(cd "$work/repo" && PATH="$work/bin:$PATH" FAKE_DIR="$work/api" \
    GITHUB_REPOSITORY=owner/repo GITHUB_REPOSITORY_ID="$repo_id" \
    bash "$script" .github/workflows/ci.yml verified-tree-ci 2>/dev/null || echo error)"
  if [ "$actual" = "$expected" ]; then
    printf '  ok   %s\n' "$label"
  else
    printf '  FAIL %s: expected %s, got %s\n' "$label" "$expected" "$actual" >&2
    failures=$((failures + 1))
  fi
}

ci=.github/workflows/ci.yml
check "a successful push run on main verifies the tree" true \
  "[$(artifact 1 "$repo_id" false)]" "1:success:$ci:push:main:$tree"
check "a successful dispatch run on main verifies the tree" true \
  "[$(artifact 2 "$repo_id" false)]" "2:success:$ci:workflow_dispatch:main:$tree"
check "a workflow path with a ref suffix still counts" true \
  "[$(artifact 3 "$repo_id" false)]" "3:success:$ci@refs/heads/main:push:main:$tree"
check "a marker from a pull_request run is never accepted" false \
  "[$(artifact 4 "$repo_id" false)]" "4:success:$ci:pull_request:feature:$tree"
check "a marker from a pull_request_target run is never accepted" false \
  "[$(artifact 5 "$repo_id" false)]" "5:success:$ci:pull_request_target:main:$tree"
check "a dispatch run on another branch is ignored" false \
  "[$(artifact 6 "$repo_id" false)]" "6:success:$ci:workflow_dispatch:feature:$tree"
check "a marker naming a tree its own commit does not have is ignored" false \
  "[$(artifact 7 "$repo_id" false)]" "7:success:$ci:push:main:$other_tree"
check "a marker uploaded from a fork is ignored" false \
  "[$(artifact 8 "$fork_id" false)]" "8:success:$ci:push:main:$tree"
check "a run of another workflow is ignored" false \
  "[$(artifact 9 "$repo_id" false)]" "9:success:.github/workflows/other.yml:push:main:$tree"
check "an unsuccessful run is ignored" false \
  "[$(artifact 10 "$repo_id" false)]" "10:failure:$ci:push:main:$tree"
check "an expired marker is ignored" false \
  "[$(artifact 11 "$repo_id" true)]" "11:success:$ci:push:main:$tree"
check "no marker at all" false "[]"
check "one valid run among rejected ones is enough" true \
  "[$(artifact 12 "$fork_id" false),$(artifact 13 "$repo_id" false),$(artifact 14 "$repo_id" false),$(artifact 15 "$repo_id" false)]" \
  "12:success:$ci:push:main:$tree" "13:success:$ci:pull_request:feature:$tree" \
  "14:failure:$ci:push:main:$tree" "15:success:$ci:push:main:$tree"

if [ "$failures" -gt 0 ]; then
  printf '%d verified-tree lookup check(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'Verified-tree lookup checks passed\n'
