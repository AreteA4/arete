# Arete Agent Benchmarks

Runs real coding agents (Claude Code, Codex, OpenCode) against Arete tasks in
disposable Vercel Sandboxes. Each run records its speed, token use, tool calls
and full session history, so you can compare agents and find what in Arete's
docs, skills, CLI or SDKs slows them down.

Agents are driven through the Vercel AI SDK harness packages
(`@ai-sdk/harness-*`). Every sandbox follows the same path a user does with the
prompt on arete.run: install `a4`, run `a4 init`, sign in the restricted agent
profile, check `a4 doctor`, then discover, install and build.

## Quick start

```bash
cd benchmarks
npm install
cp .env.example .env   # fill in the keys, see below
npm run bench -- configs/smoke.json
npm run compare
```

Run one task directly:

```bash
npm run bench -- --task ore-live-round.ts --harness codex --model openai/gpt-5.6-sol
```

Run the full matrix (4 agents × 3 tasks × 3 repetitions, 4 at a time):

```bash
npm run bench -- configs/matrix.json
```

## Credentials

| Variable | Purpose |
| --- | --- |
| `AI_GATEWAY_API_KEY` | Model access for every harness through the Vercel AI Gateway. The team needs a card on file. `VERCEL_OIDC_TOKEN` also works. |
| `VERCEL_TOKEN`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID` | Vercel Sandbox. |
| `ARETE_AGENT_KEYS` | Comma-separated `a4_ak_*` keys, written into each sandbox's `agent` profile. Use keys with raised limits. Without keys, each run signs up a trial agent (5/hour/IP). |
| `BENCH_RESULTS_DIR` | Where run directories go. Point it at a clone of the private results repo. |
| `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY` | Only for `--model-auth direct`, which bypasses the gateway. |

The gateway key never enters the sandbox: the harness brokers it through
Vercel's request transformations, and the sandbox only sees a placeholder.

## Tasks

| Task | Track | Starts from | What it measures |
| --- | --- | --- | --- |
| `onboarding.ts` | onboarding | empty project | The verbatim arete.run prompt. Then, in a fresh session (simulating the "restart your agent" step), a live-data question that needs the new skills or MCP servers. |
| `ore-live-round.ts` | build | initialized project | Discover the ORE stack from intent, install it, and write a script whose output is checked against the live view. |
| `launchpad-discovery.ts` | discovery | initialized project | Answer a capability question from the catalog; scored by F1 against the catalog at verification time. |

An *initialized* project already has `a4` installed, `a4 init --agents <harness>`
applied, the agent profile signed in and `a4 doctor` passing before the agent
starts. A setup failure is reported as `setup-error`, separate from agent
failures.

Add a task by exporting a `TaskDefinition` from `tasks/<name>.ts`: its turns,
its setup mode, and a `verify()` that checks the sandbox after the agent
finishes. `tasks/lib.ts` has helpers for doctor, manifest, lockfile
reproducibility, secret scanning and live ground truth.

## What a run records

Each run writes `runs/<date>/<run-id>/` under the results directory:

| File | Contents |
| --- | --- |
| `report.json` | Metrics, verification, friction and versions (schema below). |
| `summary.txt` | The console summary. |
| `transcript.md` | Readable session history: every turn, step, message, reasoning summary and tool call with input, output and timing. |
| `transcript.json` | The same history as structured data. |
| `events.jsonl` | Every stream event, timestamped on the host. |
| `native/` | The harness's own session log: Claude Code JSONL, Codex rollout or OpenCode storage, plus the harness bridge event log. |
| `workspace/` | The final project, without `node_modules`. |
| `run.log` | Setup, verification and collection commands with exit codes and timings. |
| `harness-diagnostics.jsonl` | Harness bridge diagnostics. |

Known keys and any `a4_ak_`/`a4_sk_` strings are redacted from every file
before it is written.

### Metrics

- **Time:**
  - phases: sandbox, setup, agent, verify, collect;
  - model time versus tool time;
  - time to first token per step;
  - milestones: first `a4` command, `a4` installed, `doctor` ok, first dependency install, first program run.
- **Tokens:**
  - input split into uncached, cache read and cache write;
  - output and reasoning;
  - peak context, the largest single prompt, against the model's window;
  - compactions.
- **Cost:** model cost from the AI Gateway price list, including cache rates; Claude Code's own reported figure where available; and an upper bound for the sandbox.
- **Tool calls:**
  - by tool and by category;
  - `a4` subcommands parsed from shell calls, with failures and `--help` lookups;
  - MCP calls by server and tool;
  - skill reads and docs lookups;
  - direct API calls that bypass the CLI.
- **Arete usage:** change in the agent key's usage meters, read from the host before and after the run. It's marked `shared` if another concurrent run held the same key.
- **Workspace:** files created, modified and deleted, with line counts split into app code, generated SDK, Arete config and config.
- **Friction:**
  - failed and repeated commands;
  - CLI surface mismatches (unknown flag or subcommand);
  - help lookups and doctor warnings;
  - self-update attempts;
  - edits to generated SDK files;
  - credential access;
  - slow steps.
- **Verification:** the task's checks, run in the sandbox after the agent finishes. Required checks decide pass/fail; optional ones only add to the score.

## Comparing and reviewing runs

```bash
npm run compare                          # medians per task × harness × model, plus friction totals
npm run compare -- --since 2026-10-01 --task onboarding

npm run review                           # LLM review of every unreviewed run → review.md/json
npm run review -- --aggregate            # merge reviews into reports/<date>-findings.md
npm run rescore                          # recompute metrics from saved transcripts after classifier changes
```

The reviewer reads the transcript, the friction signals and the verification
results. It files findings as docs, skill, CLI UX, CLI bug, SDK, catalog, MCP,
agent behaviour, harness or task problems, each with evidence and a concrete
suggestion. `--aggregate` merges findings that share a root cause across runs
into one ranked list.

## Configuration

A config file holds either one run (`harness`, `model`, `task`) or a sweep
(`agents`, `tasks`, `repetitions`, `concurrency`). Shared fields:

| Field | Default | Notes |
| --- | --- | --- |
| `a4Version` | `0.32.0` | Pinned `a4` release, also exported as `A4_VERSION` so an agent's own `install.sh` call gets the same version. |
| `skillsRef` | latest | `AreteA4/skills` tag passed to `a4 init --skills-ref`. |
| `keyMode` | `pool` | `pool` writes an `ARETE_AGENT_KEYS` key; `signup` lets the agent sign up. |
| `modelAuth` | `ai-gateway` | `direct` uses provider keys and provider-native model ids. OpenCode direct mode supports only `anthropic/` and `openai/` models. |
| `turnTimeoutMinutes` | `20` | Per-turn abort. |
| `sandbox.image` | `vercel/sandbox/universal` | Ubuntu 26.04. `sandbox.runtime: "node24"` selects the legacy Amazon Linux 2023 image, where `a4` ≤ 0.32.0 cannot start (its linux-x64 binary needs glibc 2.39). |
| `sandbox.vcpus` | `2` | |
| `sandbox.timeoutMinutes` | `45` | |

Agents in a sweep take `harness`, `model` (an AI Gateway id such as
`anthropic/claude-sonnet-5.5`), optional `label` and optional `effort`.

## Sandbox environment

Every sandbox process gets these environment variables:

- `A4_VERSION`: pins the installer.
- `A4_NO_UPDATE_CHECK=1`: so `doctor` doesn't warn about newer releases.
- `ARETE_TELEMETRY_DISABLED=1` and `DO_NOT_TRACK=1`: keep benchmark traffic out of product analytics.

The runner also accepts the host's trust prompt for the project, as a user
would: Claude Code project MCP servers are approved, and Codex marks the
project trusted.

## Results repository

Transcripts contain hosted endpoints and usage details, so keep them out of
this repository. Clone the private results repo and point `BENCH_RESULTS_DIR`
at it:

```bash
git clone git@github.com:<org>/<results-repo>.git ../bench-results
echo "BENCH_RESULTS_DIR=../bench-results" >> .env
npm run bench -- configs/matrix.json
npm run review && npm run review -- --aggregate
(cd ../bench-results && git add -A && git commit -m "bench: $(date +%F) matrix" && git push)
```

## CI

`.github/workflows/benchmarks.yml` runs a config weekly or on demand. It
commits the run directories to the results repo when `BENCH_RESULTS_REPO` and
`BENCH_RESULTS_TOKEN` are set; otherwise it uploads them as a workflow
artifact. It never runs on pull requests, because every run bills the sandbox
and the models.

## Layout

```
src/
  cli/run.ts       single runs and sweeps, with a concurrency pool
  cli/compare.ts   comparison and friction tables
  cli/review.ts    LLM session reviews and aggregated findings
  cli/rescore.ts   recompute transcript-derived metrics for saved runs
  run-one.ts       sandbox → setup → turns → verify → collect → report
  arete-setup.ts   a4 install, init, trust, credentials, doctor, usage
  harnesses.ts     adapters, auth mode and model ids per harness
  sandbox.ts       Vercel sandbox creation and the bash runner
  recorder.ts      stream parts → timestamped events and transcript
  classify.ts      tool calls → categories, a4 subcommands, MCP, failures
  metrics.ts       tokens, context, timing, milestones, tool summary
  friction.ts      deterministic friction signals
  pricing.ts       AI Gateway prices and context windows
  workspace.ts     snapshots, diffs and downloads
  transcript.ts    transcript.md renderer
  report.ts        redaction, run directory writer, console summary
tasks/             task definitions and verifier helpers
configs/           smoke and matrix configs
scripts/           probe-image.ts: inspect a sandbox image's toolchain
```
