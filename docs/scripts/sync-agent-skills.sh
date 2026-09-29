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

names=(arete arete-streams arete-programs arete-stack-authoring arete-deploy)
mkdir -p "$tmp/source" "$tmp/staged"
curl -fsSL "https://codeload.github.com/AreteA4/skills/tar.gz/$ref" |
  tar -xz -C "$tmp/source" --strip-components=1

# Stage every skill before touching the published copies, so a missing
# skill or a failed copy leaves them as they were.
for name in "${names[@]}"; do
  [ -f "$tmp/source/skills/$name/SKILL.md" ] || {
    echo "AreteA4/skills@$ref has no skills/$name/SKILL.md" >&2
    exit 1
  }
  cp -R "$tmp/source/skills/$name" "$tmp/staged/$name"
done
for name in "${names[@]}"; do
  rm -rf "${dest:?}/$name"
  mv "$tmp/staged/$name" "$dest/$name"
done

node "$docs/scripts/pack-agent-skills.mjs"
git -C "$docs" status --short -- public/.well-known/agent-skills
