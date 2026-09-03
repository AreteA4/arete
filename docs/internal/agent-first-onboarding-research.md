# Agent-First Onboarding: Research Notes

> Research and ecosystem survey behind `agent-first-onboarding.md` (the implementation spec). Kept for reference; distribution-channel ideas (plugin marketplaces, well-known skill indexes, registries, Homebrew) live here and are deferred.

## 0. TL;DR

The plan in the brief (hosted markdown + `npx` installer + self-updating signed
binary + `a4 init` that scaffolds agent config) is directionally right and
mostly matches where the ecosystem converged in 2026. Four corrections matter:

1. **Move the logic out of the markdown and into the CLI.** Every mature
   vendor (Stripe, Neon, Clerk, Railway, Sentry, Nx) ships a deterministic
   `<cli> agent setup`/`init` command that detects agents and writes skills +
   MCP config. The hosted markdown stays, but shrinks to ~30 lines whose only
   job is "run this one command, then run `a4 doctor`". Today's
   `docs.arete.run/agent.md` is 310 lines of instructions an LLM must execute
   correctly every time. That is the least reliable component in the chain.
2. **The npm `postinstall` design is about to break by default.** pnpm 10/11
   and bun already refuse dependency lifecycle scripts; npm 12 (est. July 2026)
   flips `allowScripts` off, and the changelog says this applies to `npx` too.
   This affects `@usearete/a4` *and* every `npx -y @usearete/mcp` line in every
   agent's MCP config. The fix is the one in the brief: make the npm package a
   *bootstrapper* whose `bin` downloads at run time (no lifecycle scripts), and
   stop pointing MCP configs at `npx` at all.
3. **Install to `~/.local/bin`, not `~/.arete/bin`.** uv, Claude Code and
   Cursor's agent CLI all converged on `~/.local/bin` in 2025–26, so it is the
   directory most likely to already be on an agent's PATH. Keep `~/.arete/` for
   state (credentials, templates, install receipt). And because agent shells
   snapshot PATH at session start, the installer must print the absolute binary
   path and the npm shim must `exec` it directly.
4. **`a4 update` is already taken** (dependency updates). Self-update must be
   `a4 self update`, with `a4 upgrade` as an alias.

Target end-to-end flow, from a human's point of view:

```
1. Human pastes one line into any agent:
     Set up Arete in this project: run `curl -fsSL https://arete.run/install.sh | sh`
     (or `npx @usearete/a4 install`), then `a4 init -y`, then `a4 doctor`.
2. Agent installs the signed binary (~2–4 s), runs `a4 init -y`
   (arete.toml, AGENTS.md block, skills, MCP config for every detected agent),
   runs `a4 doctor --json` and reports green.
3. Agent runs `a4 explore --json` (no key needed) and can immediately answer
   "what live data is available?" — the first "wow".
4. Anything needing an account: `a4 auth signup` (agent self-registers) or
   `a4 auth login --key` with a key from the minimal web UI.
```

Time budget: under 60 seconds wall-clock, zero Rust toolchain, zero prompts.

---

## 1. Where we are today (verified in this repo, 2026-09-02)

| Area | Current state | Gap for an agent-first flow |
|---|---|---|
| Hosted agent docs | `docs/public/agent.md` (310 lines) and `docs/public/skill.md` (503 lines, platform API + agent signup) exist; `CopyPrompt` component and a "Read https://docs.arete.run/agent.md…" one-liner already live on `agent-skills/overview` and `setup-tools` | agent.md prefers `cargo install a4-cli`; update path is `cargo install` again; the agent must hand-execute Step 0–6 including identifying which agent it is |
| npm package | `@usearete/a4` = `postinstall` downloads the binary from GitHub Releases into `node_modules/…/bin`, SHA-256 verified against `checksums.txt`; launcher falls back to any `a4` on PATH. `@usearete/mcp` is the same pattern | Lifecycle script (see §2.5). Binary lives in npx cache, so there is no stable location and no self-update target. Measured: cold `npx @usearete/a4 --version` = 3.9 s (24 MB download), warm = 0.55 s |
| Release pipeline | `release-please.yml` builds 5 targets (darwin arm64/x64, linux x64/arm64, win x64) with `cargo zigbuild`, uploads binaries + `checksums.txt` + templates tarball to `a4-cli-vX.Y.Z` | No signature, no attestation. `gh api …/attestations` returns 404. "Signed on GitHub" in the brief is currently only a checksum |
| `a4 init` | Writes `arete.toml` after discovering `.arete/*.json` artifacts; prompts for project name on stdin with no TTY check; bails if file exists | Not idempotent, blocks under a non-TTY agent shell, writes nothing for agents |
| `a4 create` | Template scaffold from release tarball, `dialoguer` prompts when args omitted | Same TTY problem |
| Skills | `AreteA4/skills` repo with `arete`, `arete-consume`, `arete-build`; installed via `npx skills add AreteA4/skills`; this repo has `skills-lock.json` | Skills still say "Prefer `a4` (Cargo)". No `/.well-known/skills` index, no Claude/Codex plugin manifest |
| MCP | stdio `a4-mcp` via `npx -y @usearete/mcp`; remote docs MCP at `https://docs.arete.run/mcp` | Every agent config we document embeds the `npx` launch that npm 12 will block |
| Auth | `a4 auth login --key`; credentials in `~/.arete/credentials.toml`; agents can self-register via `POST /api/agents/signup` (5/hour/IP, key shown once) | No CLI wrapper for signup; skill.md tells the agent to `curl` and then save the key by hand |
| Non-interactive safety | Global `--json`; telemetry banner goes to stderr and is suppressed under `CI`; only `programs.rs` checks `is_terminal()` | `create`, `init`, `auth login` will hang or fail when stdin is not a TTY |
| Docs site | Starlight with `llms.txt`/`llms-full.txt`, `[...slug].md.ts` markdown endpoints, docs MCP | Good foundation; missing skill discovery index |
| Local state | `~/.arete/credentials.toml`, `~/.arete/templates/<version>/`, telemetry config | Natural home for the install receipt |

---

## 2. What the ecosystem does (condensed; sources in §8)

### 2.1 Onboarding entry points

The dominant 2026 pattern is **not** "paste this URL". It is a single
detect-and-write command plus a short natural-language prompt:

- **Stripe**: `stripe agent setup` with `--client`, `--skills-scope local|global`,
  `--status`, `--json` (plan without writing), `-y`. Also
  `npx skills add https://docs.stripe.com` via `/.well-known/skills/index.json`,
  and a curl fallback for agents without Node.
- **Neon**: `npx neon@latest init -y --agent cursor` OAuths, mints a key, writes
  MCP + skills + `.env`. Then the human types "Get started with Neon".
- **Sentry**: `npx @sentry/agent-plugin install` detects assistants and wires
  plugin + hosted MCP into each.
- **Clerk**: `clerk init`, `clerk mcp install` (detects 10 agents).
- **Railway**: `railway setup agent`, `railway mcp`.
- **Convex**: skills via `npx skills add get-convex/agent-skills`; CLI "Agent
  Mode" never prompts for login in a non-interactive shell and scopes a deploy
  key to the agent's own dev deployment.
- **tRPC**: `npx @tanstack/intent install` *prints a prompt* telling the agent to
  configure itself. Clever fallback when you can't write the agent's files.

No major vendor uses "Read https://…/agent.md and follow it" as the primary
path. It still has a place: it is the only entry that requires nothing
installed, and the copy-prompt UI already exists. Keep it, shrink it.

Deeplinks exist if we ever want buttons on the web UI: Claude Desktop
`claude://code/new?q=<prompt>&folder=<path>`, Cursor
`cursor://anysphere.cursor-deeplink/mcp/install?name=…&config=<base64>`,
VS Code `vscode:mcp/install?<json>`.

### 2.2 Skills distribution

- **Agent Skills spec** (agentskills.io): `SKILL.md` + YAML frontmatter, ~45
  adopting clients. We already conform.
- **`npx skills`** (Vercel, 75+ agents): `add <source> [-a agents] [-s skills]
  [-g] [-y] [--all] [--copy]`, `update`/`check`, `list --json`, `find`. Detects
  agents purely by home-dir presence (`~/.claude`, `~/.cursor`, `~/.codex`,
  `$XDG_CONFIG_HOME/opencode`, `~/.gemini`, …). Writes `skills-lock.json`.
  No SDK; shell out. No `--ref` flag; pin by installing from a
  `https://github.com/AreteA4/skills/tree/<tag>` URL. Known gap: `check`/`update`
  only read the *global* lock (issue #690).
- **Discovery RFC** (Cloudflare): `/.well-known/agent-skills/index.json`
  (v0.2.0) — but Stripe and Mintlify still serve the older
  `/.well-known/skills/index.json`, and `npx skills` has an open bug on the new
  path. Serve both.
- **Plugins**: Claude Code (`claude plugin install x@marketplace -y`,
  `.claude-plugin/marketplace.json`, bundles skills + `.mcp.json` + hooks),
  Codex (`codex plugin add x@marketplace`), Cursor (`/add-plugin`),
  cross-agent `npx plugins add owner/repo`. Cloudflare, Stripe, Sentry, Vercel,
  Supabase, Expo all ship a plugin that bundles skills **and** MCP so one
  install does both.
- **Claude Code plugin hints**: a CLI that sees `CLAUDECODE=1` can print
  `<claude-code-hint v="1" type="plugin" value="arete@claude-plugins-official" />`
  to stderr once and Claude Code offers to install it. Only for the official
  marketplace.

### 2.3 MCP distribution

There is no standard, but the same two servers need six distinct config shapes:

| Agent | Project file | Shape |
|---|---|---|
| Claude Code | `.mcp.json` | `mcpServers` + `type: stdio|http` |
| Cursor | `.cursor/mcp.json` | `mcpServers`, remote = `url` (no type) |
| VS Code / Copilot | `.vscode/mcp.json` | **`servers`** + `type` |
| Copilot CLI | `.mcp.json` / `~/.copilot/mcp-config.json` | `type: local|http` required |
| Codex | `.codex/config.toml` (only if project trusted) / `~/.codex/config.toml` | `[mcp_servers.x] command=… args=[…]` / `url=…` |
| OpenCode | `opencode.json` | `mcp: { x: { type: local|remote, command:[…] | url } }` |
| Gemini CLI | `.gemini/settings.json` | `mcpServers`, remote = **`httpUrl`** |
| Windsurf | global only `~/.codeium/windsurf/mcp_config.json` | remote = `serverUrl` |
| Zed | `.zed/settings.json` | **`context_servers`** |
| Amp | `.amp/settings.json` | `amp.mcpServers` |
| Kiro / Roo / Cline / Goose | `.kiro/settings/mcp.json`, `.roo/mcp.json`, `~/.cline/mcp.json`, `.goose/config.yaml` | variants |

`npx add-mcp` (Neon, 24 agents, `-a`, `-g`, `-y`, `--header`) is the most
complete generic writer; Docker MCP Toolkit and dotagents (Sentry) are the
alternatives. All are JS.

### 2.4 AGENTS.md / CLAUDE.md

- AGENTS.md is read natively by Codex, Cursor, OpenCode, Amp, Copilot, Zed,
  Windsurf, Gemini CLI (only with `context.fileName` configured). **Claude Code
  does not read it**; the documented bridge is a `CLAUDE.md` containing
  `@AGENTS.md`.
- The idempotent pattern is Next.js's managed block:
  `<!-- BEGIN:nextjs-agent-rules --> … <!-- END:nextjs-agent-rules -->`,
  upserted on every `next dev`, content outside the markers preserved. Expo's
  `create-expo-app` writes AGENTS.md + `CLAUDE.md` = `@AGENTS.md` +
  `.claude/settings.json` enabling its plugin.

### 2.5 Binary install and self-update

- **Install dir convergence**: uv (`~/.local/bin` + `~/.local/share/uv`),
  Claude Code (`~/.local/bin/claude` → `~/.local/share/claude/versions/<v>`),
  Cursor agent (same shape). Older generation: `~/.bun/bin`, `~/.deno/bin`,
  `~/.fly/bin`, `~/.cargo/bin`.
- **PATH**: uv edits rc files silently (`UV_NO_MODIFY_PATH` to opt out) and
  appends to `$GITHUB_PATH`; Claude Code edits nothing and prints the export
  line. **Claude Code's Bash tool restores a PATH snapshot taken at session
  start before every command**, so rc-file edits are invisible for the rest of
  the session (anthropic/claude-code#43127, closed "not planned"). Installers
  aimed at agents must print the absolute path and offer a passthrough exec.
- **Version discovery without the GitHub API**:
  `https://github.com/<o>/<r>/releases/latest/download/<asset>` is a documented
  stable redirect and is not API-rate-limited (the API is 60 req/h
  unauthenticated, and uv's `self update` is known to hit that from agent
  sandboxes). deno uses `dl.deno.land/release-latest.txt`; Claude uses a
  `latest` file + `manifest.json`.
- **Self-replace**: `self-replace` crate (mitsuhiko, 1.5.0) handles the Windows
  in-use-binary dance; `self_update` 1.3.0 offers a static-manifest backend and
  zipsign; `axoupdater` 0.10.2 depends on cargo-dist receipts and the GitHub API.
- **Nudges**: stderr only, once per 24 h, off when `CI` set or stderr is not a
  TTY, env kill switch (`GH_NO_UPDATE_NOTIFIER`, `DENO_NO_UPDATE_CHECK`).
- **cargo-dist**: alive (0.32.0, 2026-05-22; Astral's fork archived because
  upstream resumed) but its npm installer is the exact `postinstall` pattern we
  are leaving and its updater hits the GitHub API. Hand-rolling is ~300 lines.
- **npm lifecycle scripts**: pnpm 10 blocks unless `onlyBuiltDependencies`;
  pnpm 11 uses `allowBuilds`; bun blocks unless `trustedDependencies` and
  `bunx` ignores postinstall for untrusted packages; **npm 12 flips
  `allowScripts` off for install, global install and `npx`** (github.blog,
  2026-06-09; est. July 2026). Verified today: local npm is 11.17.0, which is
  why `npx @usearete/a4` still works here.
- **Precedents for "npm was the bootstrapper"**: Claude Code (`install.sh` →
  bootstrap → binary runs `claude install`, npm demoted to "same native binary
  via optional deps"); Amp moved from `npx @sourcegraph/amp` to
  `curl …/install.sh` and made the npm package contain the executable.

### 2.6 Signing

- **GitHub artifact attestations** (`actions/attest-build-provenance`,
  `gh attestation verify`): best for humans and agents that have `gh` and a
  token. `gh attestation verify` **still requires a token even for public
  repos** (cli/cli#11803), so it cannot be the installer's sole check.
- **minisign** (zig, cargo-binstall `--only-signed`): Ed25519, verifiable
  offline with `minisign-verify` (Rust, zero deps) or Node's built-in
  `crypto.verify`. ~40 lines.
- **macOS/Windows**: Gatekeeper and SmartScreen only act on files carrying the
  quarantine xattr / Mark-of-the-Web, which curl, Node `fs` and PowerShell
  downloads do not set. Apple Silicon's mandatory signing is satisfied by the
  linker's ad-hoc signature. Notarization/Authenticode are not required for a
  curl/npm-delivered CLI. Claude Code does notarize; that is polish, not a
  prerequisite.

### 2.7 Agent-first CLI principles (Nx, Cloudflare `cf`, Google `gws`, Upstash)

Idempotent commands; `--json` as a first-class code path (not a formatter
bolted on); never prompt when stdin is not a TTY, fail listing the exact
missing flags instead; `--dry-run` for anything that writes; consistent verbs
across subcommands; env-var auth with documented precedence
(flags > env > config); schema introspection; detect the driving agent
(`CLAUDECODE=1`); errors that name the next command.

---

## 3. Proposed design

### 3.1 Entry point: one prompt, one tiny markdown file

Keep `https://docs.arete.run/agent.md` as the universal, install-nothing
entry, and keep the `CopyPrompt` block. Change its contents to this shape
(target ≤ 40 lines):

```markdown
# Set up Arete

1. Install the CLI (prebuilt, signed binary; no Rust needed):
     curl -fsSL https://arete.run/install.sh | sh        # macOS / Linux
     irm https://arete.run/install.ps1 | iex             # Windows
     npx @usearete/a4 install                            # if you prefer npm
   The installer prints `A4_BIN=/abs/path/a4`. Use that path if `a4` is not on
   your PATH yet (your shell may have snapshotted PATH before the install).
2. In the project directory run:  a4 init -y
   This writes arete.toml, an AGENTS.md block, Arete skills for every coding
   agent it detects, and MCP config for the docs and stream servers.
3. Verify:  a4 doctor --json   (exit 0 = ready)
4. Discover data:  a4 explore --json   (no account needed)

Need an account? `a4 auth signup` registers you as an agent and stores the key.
Human-owned key? `a4 auth login --key <a4_ak_…>` (keys: https://arete.run/keys).
Everything else lives in the installed skills. Do not `cargo install a4-cli`.
```

The copy-prompt on the site becomes:

> Read https://docs.arete.run/agent.md and follow it to set up Arete in this
> project, then tell me what live data is available.

The trailing clause matters: it gives the agent a concrete first task that
exercises `a4 explore --json` and produces the "wow" without a key.

Also publish:

- `/.well-known/skills/index.json` **and** `/.well-known/agent-skills/index.json`
  on docs.arete.run so `npx skills add https://docs.arete.run` works.
- `.md` URL suffix already exists via `[...slug].md.ts`; add
  `Accept: text/markdown` negotiation if cheap. Keep `llms.txt`.
- Put a one-line agent directive at the top of every docs page (Anthropic and
  Vercel do this): "Agents: run `a4 doctor`; if Arete is not set up, read
  https://docs.arete.run/agent.md".

Why keep the hosted file at all if the CLI does the work: it needs no
install, no Node, no plugin marketplace approval, and it is the only path that
works from a chat UI with a fresh sandbox. Everything after step 1 is
deterministic code, which is the point.

### 3.2 Install: two thin bootstrappers, one installer inside the binary

Follow the Claude Code shape: the bootstrappers only download + verify + hand
off; the binary finishes its own install, so install and self-update are the
same code path.

**Bootstrappers** (each ~50–100 lines, no dependencies):

- `install.sh` / `install.ps1` hosted at `arete.run/install.{sh,ps1}`
  (canonical; works in containers without Node). Served as static files from
  `hyper-stack-platform/landing/public/` (Astro on Vercel; already serves
  `llms.txt` the same way).
- `@usearete/a4` npm package: **no `postinstall`, no optional deps**. Its `bin`
  script, when invoked as `npx @usearete/a4 install [version]`, does the same
  download + verify + `a4 self install`. When invoked with any other args
  (`npx @usearete/a4 explore --json`) it installs if needed and then `exec`s
  the absolute installed path, so it works in the same agent session
  regardless of PATH. Pin with `npx @usearete/a4@0.13.0`.

Both:

1. Resolve version: `latest` → `https://github.com/AreteA4/arete/releases/latest/download/manifest.json`
   (stable redirect, no API, no token); pinned →
   `releases/download/a4-cli-v<ver>/manifest.json`.
2. Download the platform asset to `~/.arete/downloads/`.
3. Verify SHA-256 from `checksums.txt` and the minisign/Ed25519 signature over
   `checksums.txt` (public key embedded in the script, the npm shim, and the
   binary).
4. Run `<downloaded> self install [--install-dir …]`.

**`a4 self install`** (in the binary):

- Copies itself to `$A4_INSTALL_DIR` → `$XDG_BIN_HOME` → `~/.local/bin/a4`
  (`%USERPROFILE%\.local\bin\a4.exe` on Windows). Single file; no versions
  directory needed at 25 MB.
- Writes a receipt `~/.arete/receipt.json`:
  `{ "version", "install_dir", "source": "sh|ps1|npm|self-update", "modify_path", "installed_at" }`.
- PATH: idempotently append `~/.local/bin` to `~/.profile`/`.bashrc`/`.zshrc`/
  fish `conf.d`, and `HKCU\Environment\Path` on Windows, unless
  `A4_NO_MODIFY_PATH=1` or `CI` is set; append to `$GITHUB_PATH` when present.
  Then **always** print, as the last two lines:
  ```
  A4_BIN=/Users/x/.local/bin/a4
  export PATH="$HOME/.local/bin:$PATH"   # for this shell session
  ```
- Also installs `a4-mcp` next to it (or see §3.4 for folding MCP into `a4`).
- Removes a stale npm-global `a4` shim if it finds one on PATH ahead of the
  new binary (Amp does this), and warns about a `~/.cargo/bin/a4` that would
  shadow it.

**What we stop recommending**: `cargo install a4-cli` (keep publishing the
crate for Rust devs and `cargo-binstall`, but no doc mentions it in an
onboarding path), `npm install -g @usearete/a4` (works, but is now just the
bootstrapper).

Why `~/.local/bin` over `~/.arete/bin`: it is the one directory uv, Claude
Code and Cursor already push onto the PATH of every developer who has any of
them, so most agent sessions will find `a4` with no PATH edit at all.
`~/.arete/` remains the state dir (credentials, templates, receipt, downloads).

### 3.3 Self-update and signing

**Command**: `a4 self update [VERSION] [--check] [--dry-run] [--json]`,
alias `a4 upgrade`. (`a4 update` stays the dependency updater; do not overload
it.)

Behavior:

- Refuses (with the exact alternative command) if there is no receipt, i.e.
  the binary came from cargo or Homebrew.
- Fetches the manifest by the same redirect trick, verifies checksum +
  signature, stages next to the binary, swaps with the `self-replace` crate
  (handles Windows in-use replacement atomically).
- `--check`: exit 0 up-to-date, exit 10 update available, prints JSON with
  `--json` so agents can branch.
- Pin/downgrade: `a4 self update 0.12.0`.
- Nudge: at most once per 24 h, stderr only, skipped when stderr is not a
  TTY, `CI` is set, `--json` is passed, or `A4_NO_UPDATE_CHECK=1`. **No
  background auto-update**: agents need deterministic versions inside a task.
  Consider a `minimumVersion` in the manifest for hard-breaking API changes.

**Signing** (make "signed" true):

- CI: sign `checksums.txt` with minisign (key in a GitHub secret) →
  `checksums.txt.minisig`; add `actions/attest-build-provenance` on every
  binary. Both are one workflow step each.
- Installer/updater verify sha256 + minisign offline, no token.
- Docs tell agents with `gh` to additionally run
  `gh attestation verify ~/.local/bin/a4 -R AreteA4/arete`.
- Skip notarization/Authenticode until we ship a `.dmg`/`.msi`/winget.

### 3.4 MCP launch without npm

Every agent config we write today says `npx -y @usearete/mcp`. Under npm 12 /
pnpm / bun that either fails or hangs on an approval prompt the agent cannot
answer.

**Decision (2026-09-03): option A.** Fold the stream MCP server into the `a4`
binary as `a4 mcp` (the `arete-mcp` crate becomes a library used by both).
One binary, one update, one receipt. `a4 init` writes
`{"command": "a4", "args": ["mcp"]}` (absolute path from the receipt where the
host cannot rely on PATH); the docs server stays remote HTTP. `@usearete/mcp`
on npm is deprecated with a final version that prints the migration command;
`npx @usearete/a4 mcp` covers anyone who still wants an npm entry point.
(Option B, a second `a4-mcp` binary from the same installer, was rejected as
doubling the update surface.)

Write the absolute path from the receipt when the agent's config format
cannot rely on PATH (Claude Desktop, GUI editors launched from the Dock do not
inherit shell PATH).

### 3.5 `a4 init` and `a4 doctor`

Two commands, one engine. `init` writes, `doctor` checks the same things
read-only and `doctor --fix` re-runs the writer.

```
a4 init [--yes] [--non-interactive] [--json] [--dry-run] [--force]
        [--agents claude-code,cursor,codex,opencode,gemini-cli,windsurf,copilot,cline,zed,amp,kiro,roo,goose,all]
        [--global] [--no-skills] [--no-mcp] [--no-agents-md] [--no-manifest]
        [--skills-ref v1.2.0]
```

Detection order (union of what `skills` and `add-mcp` do):

1. Project signals: `.claude/`, `CLAUDE.md`, `.cursor/`, `.codex/`,
   `opencode.json(c)`, `.gemini/`, `.vscode/`, `.github/copilot-instructions.md`,
   `.windsurf/`, `.zed/`, `.amp/`, `.kiro/`, `.roo/`, `.agents/`.
2. Home dirs: `~/.claude`, `~/.cursor`, `$CODEX_HOME|~/.codex`,
   `$XDG_CONFIG_HOME/opencode`, `~/.gemini`, `~/.copilot`,
   `~/.codeium/windsurf`, `~/.cline`, `~/.config/zed`, `~/.config/amp`,
   `~/.kiro`, `~/.config/goose`, `~/.roo`.
3. Env hints for "who is running me": `CLAUDECODE=1`, `CURSOR_AGENT`,
   `CODEX_*`, `TERM_PROGRAM` (verify exact names during implementation).

If nothing is detected and `--agents` is absent, still write the
agent-independent set (`arete.toml`, `AGENTS.md`, `.agents/skills/`,
`.mcp.json`) and exit 0 with a `warnings` entry in `--json`. Do not hard-fail
like Neon does: the caller is usually an agent in a container.

What it writes, all as **upserts**:

| File | Rule |
|---|---|
| `arete.toml` | Create if absent (name defaults to the directory; never prompt when stdin is not a TTY). Never overwrite without `--force`. Keep today's artifact discovery. |
| `AGENTS.md` | Upsert a managed block `<!-- BEGIN:arete v1 --> … <!-- END:arete -->` (Next.js pattern). Content: what Arete is, the 8-line `a4` cheat sheet, "run `a4 explore … --json` before writing Arete code", pointers to skills. Version token lets `doctor` flag stale blocks. |
| `CLAUDE.md` | If missing, write `@AGENTS.md`. If present without the import, insert `@AGENTS.md` at the top. (Claude Code's documented mechanism; it does not read AGENTS.md.) |
| `.gemini/settings.json` | Merge `"context": {"fileName": ["AGENTS.md", "GEMINI.md"]}` when Gemini CLI detected. |
| Skills | Shell out: `npx -y skills add AreteA4/skills --skill '*' --agent <list> -y` (`--copy` on Windows without Dev Mode; `https://github.com/AreteA4/skills/tree/<ref>` when `--skills-ref`). Do **not** reimplement: `skills` owns 75 agent directories, symlinking and `skills-lock.json`. If `npx` is missing, report `skills: skipped (Node not found)` and continue; `doctor` keeps reminding. Long term, once we have a Claude/Codex plugin, prefer `claude plugin install arete@… -y` when `CLAUDECODE=1`. |
| MCP config | Implement **natively in Rust** (it is ~12 small serializers; `add-mcp` is JS and would cost a second `npx` round-trip). Parse-and-set with `serde_json` / `toml_edit` (already a dependency) / a JSONC-preserving pass for `opencode.jsonc`; set only the `arete` and `arete-docs` keys, leave everything else intact. Project scope by default, `--global` for Windsurf/Cline (no project scope). Warn that Codex only loads `.codex/config.toml` for trusted projects. |

Re-run behavior: every write reports `created | updated | unchanged`;
`--dry-run` prints the same plan. `--json` output is the plan plus results.
Upgrades: bumping the block version and the MCP entries happens in one place
in the binary; `doctor` diffs desired vs current.

**`a4 doctor [--json] [--fix]`** checks and prints
`{ "status": "ok|warn|fail", "checks": [{ "id", "status", "detail", "fix" }] }`
(exit 0 ok/warn, 1 fail, 2 internal; avoid brew's exit-1-on-warning):

- `a4` version vs latest (via manifest), install source, receipt present
- `arete.toml` present and valid; `arete.lock` fresh
- credentials present for the active API URL; `a4 auth whoami` reachable
- network: `api.arete.run`, `docs.arete.run/mcp`
- Node/`npx` available (needed for skills install only)
- per detected agent: MCP entries present and current; skills present in that
  agent's directory with `skills-lock.json` hash matching upstream;
  `AGENTS.md` block present and current; `CLAUDE.md` imports `AGENTS.md`;
  Gemini `context.fileName`; Codex trust
- Rust toolchain (only reported as `info` unless the project has authored stacks)

### 3.6 Auth for agents

Two principals, both without a web UI in the loop:

- **Agent-owned**: `a4 auth signup [--name x] [--json]` wraps
  `POST /api/agents/signup` and stores the key in `~/.arete/credentials.toml`
  immediately (today skill.md asks the agent to curl and then remember a
  key shown once). Surface the 5/hour/IP limit in the error with a retry hint.
- **Human-owned**: `a4 auth login --key` (exists) with the key copied from the
  minimal web UI. Consider `ARETE_API_KEY` precedence documentation
  (flags > env > credentials file) and, later, a device-code browser flow.

Decision (2026-09-03): the web UI issues keys and nothing else for now, and
the 5/hour/IP signup limit stays until usage patterns are known. Agent-only
flows (invites, raised limits) are deferred.

Convex's rule applies: commands that need auth must never prompt when stdin
is not a TTY; they fail with the exact command to run.

### 3.7 Agent-safe CLI conventions (apply across the surface)

- `--yes`/`--non-interactive` globally; treat `!stdin.is_terminal()` as
  non-interactive everywhere (`create`, `init`, `auth login`, `programs`).
- `--json` remains a first-class path; add `--json` to `create`, `init`,
  `doctor`, `self update`.
- `--dry-run` for every writer.
- Honor `NO_COLOR`, `CI`, `DO_NOT_TRACK` (already), `A4_NO_UPDATE_CHECK`,
  `A4_NO_MODIFY_PATH`, `A4_INSTALL_DIR`.
- Errors name the next command (`Not logged in. Run: a4 auth signup` or
  `a4 auth login --key …`).
- Detect `CLAUDECODE=1`; once `arete` is in the official Claude marketplace,
  print the one-time `<claude-code-hint …/>` on stderr.

### 3.8 Distribution channels (same SKILL.md bytes everywhere)

| Channel | Action |
|---|---|
| `npx skills add AreteA4/skills` | Already works. Tag releases so `--skills-ref` can pin. |
| `npx skills add https://docs.arete.run` | Serve `/.well-known/skills/index.json` + `/.well-known/agent-skills/index.json` with sha256 digests. |
| Claude Code plugin | Add `.claude-plugin/marketplace.json` + `plugin.json` to `AreteA4/skills` bundling the 3 skills and a `.mcp.json` (docs + stream). Apply to `claude-plugins-official`; until then `claude plugin marketplace add AreteA4/skills`. |
| Codex plugin | Same repo, `codex plugin marketplace add AreteA4/skills`. |
| `npx plugins add AreteA4/skills` | Free once the plugin manifest exists. |
| MCP | Docs MCP stays remote HTTP. Stream MCP moves to `a4 mcp` (§3.4). Register both in the official MCP registry (`server.json`) for discoverability. |
| Homebrew / winget / cargo-binstall | Later; `[package.metadata.binstall]` with minisign is a few lines once signing exists. |

---

## 4. Decisions versus the original brief

| Brief | Recommendation | Why |
|---|---|---|
| Hosted markdown skill is the onboarding | **Keep as entry, shrink to ≤40 lines**; logic moves into `a4 init`/`a4 doctor` | LLM-executed prose is the least reliable step; every mature vendor moved it into a deterministic command |
| Copy one prompt containing the URL | **Keep**, append a concrete first task ("…then tell me what live data is available") | Produces the first success on `a4 explore --json` with no account |
| Skill lists dependencies (Rust, cargo, npm, bun) | **Drop Rust/cargo from the consumer path entirely**; Node only needed for `npx skills` (and optional); Rust only for `arete-build` | Matches "user never touches Rust" |
| `npx` is the installer, not a wrapper | **Yes**, and add `install.sh`/`install.ps1` as the canonical route with identical logic | npm 12/pnpm/bun block postinstall; curl works where Node does not; both share `a4 self install` |
| Binary in a known Unix-standard location | **`~/.local/bin`**, override `A4_INSTALL_DIR`, receipt in `~/.arete/` | Most likely already on an agent's PATH; matches uv/Claude/Cursor |
| Self-update via an `update` command | **`a4 self update`** / `a4 upgrade` | `a4 update` already means dependency update |
| Signed binary on GitHub | **Add minisign + attestations**; today it is a checksum only | Installer must verify offline without a token |
| `a4 init` scaffolds arete.toml, skills, MCP, AGENTS.md | **Yes**, as idempotent upserts + `a4 doctor` | Next.js/Nx/Neon patterns |
| Lean on `npx skills` behind the scenes | **Yes** for skills; **no** for MCP (write natively in Rust) | `skills` is the de facto standard and CLI-only; MCP writers are small and a second `npx` hop costs seconds and fails without Node |
| Something similar for MCPs | `npx add-mcp` exists (Neon) but is JS; fold the server into `a4 mcp` and write configs ourselves | Removes the npm dependency from every agent's MCP launch |
| Minimal web UI for accounts/keys | **Agree**; add `a4 auth signup` so the agent path needs no UI at all | Agents can already self-register via the API |

---

## 5. Decisions taken (2026-09-03, Adrian)

1. **Fold `a4-mcp` into `a4`.** §3.4 option A is the plan: `arete-mcp`
   becomes a library crate, `a4 mcp` serves stdio, and `@usearete/mcp` is
   deprecated in favour of `npx @usearete/a4 mcp` / the installed binary.
2. **`install.sh` / `install.ps1` ship from the landing site.**
   `hyper-stack-platform/landing` is an Astro site on Vercel that already
   serves static files from `landing/public/` (for example `llms.txt`), so
   `landing/public/install.sh` and `install.ps1` give `https://arete.run/install.sh`
   with no server changes. Keep the scripts' source of truth in this repo and
   copy them over on release, or fetch-and-redirect to a release asset if we
   want them version-locked.
3. **Project scope by default, `--global` opt-in** for both skills and MCP
   config. `agent.md` mentions `--global` in its first lines.
4. **Signup rate limit stays at 5/hour/IP** until usage patterns are known.
   `a4 auth signup` must surface the limit clearly with a retry hint and the
   human-key alternative.
5. **The web UI issues API keys and nothing else** for now. Agent-specific
   flows (invites, higher limits) wait for real usage data.

## 5a. Still open

1. **Plugin marketplace approval timelines** for `claude-plugins-official` and
   `openai-curated` are unknown; the self-hosted marketplace works meanwhile
   and only the plugin-hint feature depends on the official one.
2. **Windows**: symlinked skills need Developer Mode; `--copy` fallback covers
   it. PATH edits via `HKCU\Environment` are invisible to the current shell,
   same `A4_BIN=` mitigation.
3. To verify during implementation: `npx skills` pinning to a commit via a
   tree URL; the env vars each agent sets (`CLAUDECODE=1` confirmed; Cursor,
   Codex, Gemini, OpenCode unconfirmed); whether `gh attestation verify` still
   needs a token for public repos; JSONC round-tripping for `opencode.jsonc`.

---

## 6. Phased roadmap

**Phase 1 — unblock the install path (highest urgency; npm 12 is imminent)**

1. `a4 self install` + receipt + `~/.local/bin` + `A4_BIN=` output.
2. `install.sh` / `install.ps1`; rewrite `@usearete/a4` as a scriptless
   bootstrapper with passthrough exec; version-lock to the binary.
3. CI: `manifest.json` release asset, minisign of `checksums.txt`,
   `actions/attest-build-provenance`.
4. `a4 self update` (+ `upgrade` alias), `--check`, nudge rules.
5. Global non-interactive guard (`is_terminal()` + `--yes`) in `create`,
   `init`, `auth login`.

**Phase 2 — make setup deterministic**

6. `a4 init` upserts (arete.toml, AGENTS.md block, CLAUDE.md import, skills via
   `npx skills`, native MCP writers for Claude Code, Cursor, VS Code, Codex,
   OpenCode, Gemini first; the rest after).
7. `a4 doctor` with `--json` and `--fix`.
8. `a4 auth signup`.
9. `a4 mcp` (fold `arete-mcp`), MCP configs point at the binary.

**Phase 3 — distribution and docs**

10. Rewrite `docs/public/agent.md` (≤40 lines) and `skill.md` sections 3 and 7;
    update `AreteA4/skills` to drop cargo; update `agent-skills/*` pages and
    the README's install text.
11. `/.well-known/skills/index.json` (+ `agent-skills` path) on docs.arete.run;
    agent directive banner on every docs page.
12. Plugin manifests (Claude, Codex) in `AreteA4/skills`; apply to official
    marketplaces; MCP registry entries.
13. `CLAUDECODE` detection + plugin hint; Homebrew/binstall metadata.

**Success metric**: from a clean container with only curl, an agent reaches a
green `a4 doctor` and a successful `a4 explore --json` in under 60 seconds with
zero prompts and zero Rust.

---

## 7. Things in the repo that this design changes

- `packages/arete/scripts/postinstall.js` and `packages/mcp/scripts/postinstall.js`: delete; bootstrapper logic moves into `bin/a4.js`.
- `packages/arete/bin/a4.js`: becomes download + verify + `a4 self install` + passthrough exec.
- `cli/src/main.rs`: new `Self { Install, Update }`, `Doctor`, `Mcp`, `Auth::Signup`; global `--yes`.
- `cli/src/commands/config.rs::init`: replace the stdin prompt and the "already exists" bail with upsert semantics; add agent-file writers (new module `cli/src/agents/`).
- `.github/workflows/release-please.yml`: manifest asset, minisign step, attestation step.
- `docs/public/agent.md`, `docs/public/skill.md`, `docs/src/content/docs/agent-skills/*.mdx`, `README.md`, `cli/README.md`, `packages/arete/README.md`: remove `cargo install` from onboarding paths.
- `AreteA4/skills` (separate repo): install snippet, plugin manifests, release tags.
- `hyper-stack-platform/landing/public/`: `install.sh`, `install.ps1` (copied from this repo on release).
- `rust/arete-mcp`: split into a library crate consumed by `a4 mcp`; `packages/mcp` deprecated.

---

## 8. Sources

Onboarding and skills: docs.stripe.com/building-with-ai, docs.stripe.com/cli/agent/setup, neon.com/docs/get-started/with-an-agent, neon.com/docs/reference/cli-init, developers.cloudflare.com/agent-setup, docs.sentry.io/ai/agent-plugin, github.com/PostHog/wizard, docs.convex.dev/ai/agent-skills, docs.convex.dev/cli/agent-mode, clerk.com/docs/guides/ai/skills, trpc.io/docs/skills, modal.com/docs/cli/latest/skills, agentskills.io, github.com/vercel-labs/skills (src/cli.ts, src/agents.ts, src/skill-lock.ts, issue #690), github.com/cloudflare/agent-skills-discovery-rfc, code.claude.com/docs/en/plugins-reference, code.claude.com/docs/en/plugin-hints, code.claude.com/docs/en/memory, nextjs.org/docs/app/guides/ai-agents, docs.expo.dev/agents, agents.md, support.claude.com (Open in Claude Code links), cursor.com/docs/context/mcp/install-links.

MCP config: code.claude.com/docs/en/mcp, cursor.com/docs/context/mcp, code.visualstudio.com/docs/copilot/customization/mcp-servers, learn.chatgpt.com/docs/config-file/config-reference, opencode.ai/docs/mcp-servers, geminicli.com/docs/tools/mcp-server, zed.dev/docs/ai/mcp, ampcode.com/docs/customize/mcp, kiro.dev/docs/mcp/configuration, add-mcp.com/docs, github.com/getsentry/dotagents, docs.docker.com/reference/cli/docker/mcp/client/connect.

Install/update/signing: releases.astral.sh/installers/uv/latest/uv-installer.sh, docs.astral.sh/uv/reference/installer, bun.sh/install, github.com/denoland/deno_install, docs.deno.com/runtime/reference/cli/upgrade, fly.io/install.sh, code.claude.com/docs/en/setup, code.claude.com/docs/en/troubleshoot-install, github.com/anthropics/claude-code/issues/43127 and /21365 and /52963, ampcode.com/news/npm-package-changes, github.blog/changelog/2026-06-09-upcoming-breaking-changes-for-npm-v12, pnpm.io/cli/approve-builds, pnpm.io/blog/releases/11.0, bun.com/docs/pm/lifecycle, github.com/npm/cli/issues/4828, docs.github.com (releases/latest/download redirect; REST rate limits; artifact attestations; offline verification), github.com/cli/cli/issues/11803, github.com/cargo-bins/cargo-binstall/blob/main/SIGNING.md, ziglang.org/download, crates.io/crates/{self-replace,self_update,axoupdater,minisign-verify}, github.com/axodotdev/cargo-dist/releases, github.com/astral-sh/cargo-dist (archived), eclecticlight.co on quarantine, signpath.io/knowledge-base/windows-platform.

Agent-first DX: nx.dev/blog (Agentic Experience Is the New Developer Experience), blog.cloudflare.com (Building a CLI for all of Cloudflare), agent-dx.com, builder.io/blog (Agent experience is the new developer experience), sarahdrasnerdesign (Agent-Friendly Docs), upstash.com/docs/agent-resources, openstatus.dev (CLI for humans and agents), clig.dev.

Local measurements (2026-09-02, macOS arm64, npm 11.17.0): cold `npx --yes @usearete/a4@latest --version` 3.94 s, warm 0.55 s; latest release `a4-cli-v0.12.0` assets: 5 binaries (18–29 MB), templates tarball, `checksums.txt`; no attestations.
