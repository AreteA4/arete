#!/usr/bin/env bash
# End-to-end onboarding acceptance test (docs/internal/agent-first-onboarding.md, section 0)
# run inside a clean ubuntu:24.04 container: pass 1 with only curl/ca-certificates/git,
# pass 2 with nodejs/npm added (skills install, doctor ok).
#
# usage: scripts/e2e-onboarding.sh <version>          # or A4_VERSION=<version>
# env:   A4_INSTALL_URL     install script URL (default https://arete.run/install.sh);
#                           for pre-release testing serve docs/public/install.sh locally
#                           and point here (from inside docker use host.docker.internal
#                           on macOS/Windows or E2E_DOCKER_ARGS=--network=host on Linux)
#        A4_MANIFEST_BASE_URL, A4_LATEST_URL   forwarded into the container
#        E2E_IMAGE          docker image (default ubuntu:24.04)
#        E2E_DOCKER_ARGS    extra `docker run` arguments
#        E2E_MAX_INSTALL_SECONDS   step 1 budget (default 15)
#        E2E_PASSES         "plain node" (default both)
# Prints E2E_STEP1_SECONDS=<n> per pass and appends to $GITHUB_STEP_SUMMARY when set.
set -euo pipefail

if [[ "${1:-}" != "--inner" ]]; then
  # ---------------------------------------------------------------- outer: drive docker
  A4_VERSION="${1:-${A4_VERSION:-}}"
  if [[ -z "$A4_VERSION" ]]; then
    echo "usage: $0 <version>  (or set A4_VERSION)" >&2
    exit 2
  fi
  command -v docker >/dev/null || { echo "docker not found on PATH" >&2; exit 1; }
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  image="${E2E_IMAGE:-ubuntu:24.04}"
  overall=0
  for pass in ${E2E_PASSES:-plain node}; do
    echo "=== pass: $pass (a4 $A4_VERSION, $image)"
    start=$(date +%s)
    # shellcheck disable=SC2086
    if docker run --rm \
        -e A4_VERSION="$A4_VERSION" \
        -e A4_INSTALL_URL="${A4_INSTALL_URL:-https://arete.run/install.sh}" \
        -e A4_MANIFEST_BASE_URL="${A4_MANIFEST_BASE_URL:-}" \
        -e A4_LATEST_URL="${A4_LATEST_URL:-}" \
        -e E2E_MAX_INSTALL_SECONDS="${E2E_MAX_INSTALL_SECONDS:-15}" \
        -e E2E_PASS="$pass" \
        -v "$self:/e2e.sh:ro" \
        ${E2E_DOCKER_ARGS:-} \
        "$image" bash /e2e.sh --inner </dev/null | tee "/tmp/e2e-$pass.log"; then
      status=ok
    else
      status=FAILED; overall=1
    fi
    step1=$(sed -n 's/^E2E_STEP1_SECONDS=//p' "/tmp/e2e-$pass.log" | tail -n 1)
    echo "=== pass $pass: $status in $(( $(date +%s) - start ))s (step 1 install: ${step1:-n/a}s)"
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
      printf '| %s | %s | %s | %ss |\n' "$pass" "$A4_VERSION" "$status" "${step1:-n/a}" >> "$GITHUB_STEP_SUMMARY"
    fi
  done
  exit "$overall"
fi

# ------------------------------------------------------------------ inner: in the container
export DEBIAN_FRONTEND=noninteractive
pass="${E2E_PASS:-plain}"
max_install="${E2E_MAX_INSTALL_SECONDS:-15}"
fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo; echo "--- step $*"; }
# Every a4 invocation runs with stdin closed and a timeout: nothing may wait on input.
a4() { timeout 120 "$A4_BIN" "$@" </dev/null; }
json_has() { grep -Eq "$1" "$2" || fail "$3 (in $2: $(tr -d '\n' < "$2" | head -c 400))"; }

step 0 "prepare container ($pass)"
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends curl ca-certificates git >/dev/null
if [[ "$pass" == node ]]; then
  # apt's nodejs on ubuntu:24.04 is v18, but the skills CLI requires
  # Node >= 22.20; install a pinned upstream tarball instead.
  curl -fsSL https://nodejs.org/dist/v22.20.0/node-v22.20.0-linux-x64.tar.gz \
    | tar -xz -C /usr/local --strip-components=1
fi
command -v node >/dev/null && echo "node $(node --version)" || echo "no node (expected for pass=plain)"
[[ "$PATH" != *".local/bin"* ]] || fail "~/.local/bin already on PATH; the test requires a clean container"

step 1 "curl -fsSL $A4_INSTALL_URL | sh"
t0=$(date +%s%N)
set +e
# sh must read the script from the pipe; redirecting its stdin to /dev/null
# makes sh exit immediately and curl fails with (23) writing to the dead pipe.
# The container's own stdin is already closed by the outer `docker run </dev/null`.
install_out=$(curl -fsSL "$A4_INSTALL_URL" | sh)
rc=$?
set -e
t1=$(date +%s%N)
step1_seconds=$(( (t1 - t0) / 1000000 ))
step1_seconds="$(( step1_seconds / 1000 )).$(printf '%03d' $(( step1_seconds % 1000 )))"
printf '%s\n' "$install_out"
echo "E2E_STEP1_SECONDS=$step1_seconds"
[[ $rc -eq 0 ]] || fail "install.sh exited $rc"
[[ "$(printf '%s\n' "$install_out" | tail -n 2 | head -n 1)" == "A4_BIN=/root/.local/bin/a4" ]] || fail "last-but-one stdout line is not A4_BIN=/root/.local/bin/a4"
awk -v s="$step1_seconds" -v m="$max_install" 'BEGIN { exit !(s + 0 <= m + 0) }' || fail "install took ${step1_seconds}s (> ${max_install}s)"
A4_BIN=/root/.local/bin/a4
version_out=$($A4_BIN --version </dev/null)
[[ "$version_out" == *"$A4_VERSION"* ]] || fail "a4 --version printed '$version_out', expected $A4_VERSION"
[[ -f /root/.arete/receipt.json ]] || fail "receipt not written"
grep -q '"verified": *true' /root/.arete/receipt.json || fail "receipt does not record verified: true"

step 2 "a4 init -y --json in an empty git repo"
mkdir -p /work && cd /work
git init -q && git config user.email e2e@arete.run && git config user.name e2e
a4 init -y --json > init.json || fail "a4 init exited $?"
for f in arete.toml AGENTS.md CLAUDE.md .mcp.json; do [[ -f $f ]] || fail "init did not create $f"; done
if [[ "$pass" == node ]]; then
  [[ -d .agents/skills ]] || fail "init did not create .agents/skills with Node present"
  [[ -d .claude/skills ]] || fail "init did not create .claude/skills with Node present"
else
  json_has '"skills"' init.json "init JSON has no skills entry"
  json_has 'skipped' init.json "skills were not reported as skipped"
  json_has 'npx not found' init.json "skills skip reason is not 'npx not found'"
fi

step 3 "a4 doctor --json"
a4 doctor --json > doctor.json || fail "a4 doctor exited $?"
if [[ "$pass" == node ]]; then
  json_has '"status": *"ok"' doctor.json "doctor status is not ok with Node present"
else
  json_has '"status": *"warn"' doctor.json "doctor status is not warn without Node"
  ! grep -Eq '"status": *"(error|fail)"' doctor.json || fail "doctor reports a failing check"
fi

step 4 "a4 explore --json"
a4 explore --json > explore.json || fail "a4 explore exited $?"
json_has '"ore"' explore.json "explore does not list the ore stack"

step 5 "idempotent re-run"
git add -A && git commit -qm "init"
a4 init -y --json > init2.json || fail "second a4 init exited $?"
a4 doctor --json > doctor2.json || fail "second a4 doctor exited $?"
! grep -Eq '"(created|updated)"' init2.json || fail "second init changed files: $(tr -d '\n' < init2.json | head -c 400)"
[[ -z "$(git status --porcelain --untracked-files=all -- . ':!init.json' ':!init2.json' ':!doctor.json' ':!doctor2.json' ':!explore.json')" ]] \
  || { git status --short; fail "second run changed tracked files"; }

step 6 "a4 self update --check --json"
a4 self update --check --json > update.json || fail "a4 self update --check exited $? (10 = update available; expected the latest release)"
json_has '"update(_a|A)vailable": *false' update.json "self update --check did not report updateAvailable: false"

echo
echo "PASS ($pass): all steps ok; step 1 took ${step1_seconds}s"
