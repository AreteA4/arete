#!/usr/bin/env bash
# Refresh the well-known skill trees from AreteA4/skills, then rebuild their
# archives and index.json. The trees are copies: never edit them here. Fix
# the skill upstream, re-run this, and review the diff before committing.
# Usage: sync-agent-skills.sh [git-ref]   (default: main)
set -euo pipefail

ref=${1:-${ARETE_SKILLS_REF:-main}}
docs="$(cd "$(dirname "$0")/.." && pwd)"
dest="$docs/public/.well-known/agent-skills"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "https://codeload.github.com/AreteA4/skills/tar.gz/$ref" |
  tar -xz -C "$tmp" --strip-components=1

for name in arete arete-streams arete-programs arete-stack-authoring arete-deploy; do
  [ -f "$tmp/skills/$name/SKILL.md" ] || {
    echo "AreteA4/skills@$ref has no skills/$name/SKILL.md" >&2
    exit 1
  }
  rm -rf "${dest:?}/$name"
  cp -R "$tmp/skills/$name" "$dest/$name"
done

node "$docs/scripts/pack-agent-skills.mjs"
git -C "$docs" status --short -- public/.well-known/agent-skills
