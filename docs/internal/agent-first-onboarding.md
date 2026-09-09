# Agent-First Onboarding: Implementation Spec

> Status: approved design, ready to implement. Written 2026-09-03.
> Research and ecosystem survey: `agent-first-onboarding-research.md`.
> Out of scope (deferred): plugin marketplaces, `.well-known` skill indexes,
> MCP registry listings, Homebrew/winget/binstall. Do not add them here.

This document is written for an implementer with no prior context. It states
what to build, where the current code is, the exact schemas and command
surfaces, and how to verify each piece. Work packages (WP) are ordered by
dependency; WP1–WP4 are urgent because npm 12 disables `postinstall` scripts
by default, which breaks today's install path.

---

## 0. Goal and acceptance test

The primary user is a coding agent (Claude Code, Cursor, Codex, OpenCode,
Gemini CLI, Copilot, …). A human pastes one prompt; the agent does the rest.

**Target flow**

```text
Human pastes into any agent:
  Read https://docs.arete.run/agent.md and follow it to set up Arete in this
  project, then tell me what live data is available.

Agent runs:
  curl -fsSL https://arete.run/install.sh | sh      # or: npx @usearete/a4 install
  a4 init -y
  a4 doctor --json
  a4 explore --json
```

**Acceptance test** (must pass before this work is "done"):

Starting from a clean Linux container with only `curl`, `sh` and `git`
(no Node, no Rust, no `~/.local/bin` on PATH):

1. `curl -fsSL https://arete.run/install.sh | sh` exits 0 in under 15 s,
   prints `A4_BIN=/root/.local/bin/a4` as its last-but-one line, and
   `/root/.local/bin/a4 --version` prints the released version.
2. In an empty git repo, `a4 init -y --json` exits 0, creates `arete.toml`,
   `AGENTS.md`, `CLAUDE.md`, `.mcp.json`, and reports `skills: skipped` with
   the reason "npx not found" (no Node in this container).
3. `a4 doctor --json` exits 0 with `status: "warn"` (skills missing) and every
   other check `ok`.
4. `a4 explore --json` exits 0 and lists at least the `ore` stack. No API key
   is involved anywhere in steps 1–4.
5. Running steps 2–3 again changes no file (`git status` clean after a commit)
   and every entry in the `init` JSON is `unchanged`.
6. `a4 self update --check --json` exits 0 with `update_available: false`.
7. With Node present, step 2 also installs the skills into the skill
   directories of the selected agents (for example `.claude/skills/` for
   Claude Code), and `doctor` is `ok`. Interactive runs (TTY, no `-y`,
   no `--agents`) let the human pick the agents in a multi-select with the
   detected ones pre-selected; non-interactive runs use the detected set.
8. No step ever waits on stdin.

Windows (PowerShell) and macOS runs of steps 1, 3, 4, 6 must also pass.

---

## 1. Current state: what an implementer must know

All paths relative to this repo unless stated.

| Thing | Where | Notes |
|---|---|---|
| CLI crate | `cli/` (`a4-cli`, binary `a4`) | clap derive; `Cli` struct and `Commands` enum in `cli/src/main.rs`; dispatch in `run()`; telemetry names in `command_name()`; global flags `--config`, `--json`, `--verbose`, `--api-url` |
| Command modules | `cli/src/commands/*.rs`, registered in `cli/src/commands/mod.rs` | |
| `a4 init` today | `cli/src/commands/config.rs::init` | Discovers `.arete/*.stack-manifest.json` / `*.program-spec.json`, prompts for project name on raw stdin (`prompt_project_name`), bails if `arete.toml` exists. `Commands::Init` has no args |
| `a4 update` today | `Commands::Update` | Advances **dependency** versions. Name is taken; self-update must be `a4 self update` |
| `a4 create` | `cli/src/commands/create.rs` | `dialoguer` prompts when `name`/`template` omitted; no TTY check |
| Auth | `cli/src/commands/auth.rs`; storage helpers `ApiClient::save_api_key`, `load_api_key_for_url`, `list_credentials`, `delete_api_key_for_url` in `cli/src/api_client.rs` (~line 1550) | Credentials file `~/.arete/credentials.toml`, keyed by API URL. `auth login --key` prompts (`rpassword`) when key omitted |
| TTY checks | Only `cli/src/commands/programs.rs` uses `std::io::IsTerminal` | |
| Telemetry banner | `cli/src/telemetry.rs::show_consent_banner_if_needed`, called from `main()` | stderr only, once; disabled under `CI`, `DO_NOT_TRACK` |
| Home state dir | `~/.arete/` | `credentials.toml`, `templates/<version>/`, telemetry config |
| Templates | `cli/src/templates.rs` | Downloads `arete-templates-v<ver>.tar.gz` from the `a4-cli-v<ver>` release |
| npm CLI package | `packages/arete/` (`@usearete/a4`) | `scripts/postinstall.js` downloads `a4-<platform>` from GitHub release `a4-cli-v<pkg version>` into `packages/arete/bin/`, verifies SHA-256 against `checksums.txt`; `bin/a4.js` spawns it or any `a4` on PATH; tests in `bin/a4.test.js` (`node --test`) |
| npm MCP package | `packages/mcp/` (`@usearete/mcp`) | Same pattern for `a4-mcp` |
| MCP server crate | `rust/arete-mcp/` (`arete-mcp`, binary `a4-mcp`) | All code in `src/main.rs` (1173 lines) + `connections.rs`, `credentials.rs`, `filter.rs`, `registry.rs`, `subscriptions.rs`. Uses `rmcp` 1.3 over stdio, `reqwest` **0.12** async. The CLI uses `reqwest` **0.11** blocking; both already appear in `Cargo.lock` |
| Release workflow | `.github/workflows/release-please.yml` | Jobs `build-cli-binaries` (matrix of 5 targets → assets `a4-darwin-arm64`, `a4-darwin-x64`, `a4-linux-x64`, `a4-linux-arm64`, `a4-win32-x64.exe`), `publish-cli-checksums` (`checksums.txt`, format `<sha256>  <asset>` per line), `bundle-templates`, `build-mcp-binaries`, `publish-mcp-checksums`, `publish-npm-cli`, `publish-npm-mcp`. Uploads via `scripts/upload-release-asset.sh` (refuses to replace a non-identical asset). Release tag format `a4-cli-v<version>` |
| Signing today | none | Only `checksums.txt`. No attestations |
| release-please | `release-please-config.json` | Component `cli` bumps the latest-version JSON pointer via `extra-files`; generation provenance is recorded separately |
| Docs site | `docs/` (Astro Starlight) | Static files in `docs/public/` are served at `https://docs.arete.run/<name>` (`agent.md`, `skill.md`, `ore.json` today). `[...slug].md.ts` serves every page as markdown. **Verify the deploy trigger** (no workflow in this repo; likely Vercel git integration on `docs/`) |
| Landing site | `../hyper-stack-platform/landing/` (Astro on Vercel) | `vercel.json` already permanently redirects `/agent.md` and `/skill.md` to `docs.arete.run`. Add the same redirects for `/install.sh` and `/install.ps1` |
| Hosted agent docs | `docs/public/agent.md`, `docs/public/skill.md` | `agent.md` is the concise installer-first bootstrap; `skill.md` is the hosted identity/key/MCP/API reference |
| Skills | GitHub `AreteA4/skills` (`arete`, `arete-streams`, `arete-programs`, `arete-stack-authoring`, `arete-deploy`) | Installed together with `npx skills add AreteA4/skills`; each skill is a separate workflow activation unit |
| Agent self-signup API | `POST https://api.arete.run/api/agents/signup` body `{"display_name": "..."}` → `{"slug","display_name","api_key","message"}`; 5/hour/IP → `429 rate-limit-exceeded`; `GET /api/agents/me` with Bearer key | Documented in `docs/public/skill.md`; wrapped by `a4 auth signup` and `a4 auth whoami` |
| Measured | 2026-09-02, macOS arm64, npm 11.17 | cold `npx @usearete/a4 --version` 3.9 s, warm 0.55 s; release binaries 18–29 MB |

---

## 2. Architecture

```text
 install.sh / install.ps1 / npx @usearete/a4        (bootstrappers, no logic beyond download+verify)
          │  download asset + checksums.txt + checksums.txt.minisig, verify
          ▼
 <downloaded a4> self install                       (binary finishes its own install)
          │  copy to ~/.local/bin/a4, write ~/.arete/receipt.json, PATH edit, print A4_BIN=
          ▼
 a4 init -y        writes arete.toml, AGENTS.md block, CLAUDE.md import, skills (via npx skills), MCP config
 a4 doctor         read-only check of everything init writes + environment; --fix re-runs init
 a4 self update    same download/verify code path as install; swaps binary in place
 a4 mcp            the stream MCP server, folded into the a4 binary (no npm in any MCP config)
 a4 auth signup    agent self-registration, stores key
```

Fixed decisions (do not relitigate):

- Install dir `~/.local/bin` (Windows `%USERPROFILE%\.local\bin`), override `A4_INSTALL_DIR`. State stays in `~/.arete/`.
- Self-update command is `a4 self update`, alias `a4 upgrade`. `a4 update` stays the dependency updater.
- Stream MCP server folds into `a4` as `a4 mcp`. `@usearete/mcp` is deprecated.
- Skills install shells out to `npx skills`; MCP config is written natively in Rust.
- Project scope by default for skills and MCP; `--global` opt-in.
- Signup rate limit stays 5/hour/IP. Web UI issues keys only.
- No background auto-update. No `cargo install` in any onboarding text.
- Version discovery never uses the GitHub REST API (60 req/h unauthenticated).

---

## 3. Cross-cutting conventions

Apply to every new command and retrofit where noted.

**Interactivity.** Add `fn interactive() -> bool` in `cli/src/ui.rs`:
`!(--non-interactive || --yes || env A4_NON_INTERACTIVE=1 || env CI set || !stdin.is_terminal())`.
Any prompt must be guarded by it; when not interactive and a required input
is missing, return an error that lists the exact flags to pass. Add global
flags `-y/--yes` and `--non-interactive` to `Cli` in `main.rs`.

**JSON.** Every new command supports the global `--json`. JSON goes to
stdout only; human logs and spinners to stderr only. Every JSON object has
`"schemaVersion": 1` at the top level (matches `a4 explore`).

**Exit codes.** 0 success (including warnings), 1 command failure, 2 usage
error (clap default), 10 = "update available" for `self update --check`.

**Environment variables (new).** `A4_INSTALL_DIR`, `A4_NO_MODIFY_PATH=1`,
`A4_NO_UPDATE_CHECK=1`, `A4_NON_INTERACTIVE=1`, `A4_MANIFEST_BASE_URL`
(test override for the release download base), `A4_LATEST_URL` (test
override for the latest pointer). Existing: `ARETE_API_URL`, `ARETE_API_KEY`,
`DO_NOT_TRACK`, `CI`.

**Errors name the next command.** e.g. `Not logged in. Run: a4 auth signup
(or a4 auth login --key <a4_ak_...>)`.

**Idempotency.** `init` and `doctor --fix` must be safe to run repeatedly;
every write is an upsert and reports `created | updated | unchanged`.

**Telemetry.** Add each new command to `command_name()`; no new event fields.

**Platform key.** `<os>-<arch>` with os ∈ `darwin|linux|win32`, arch ∈
`arm64|x64` (identical to `packages/arete/scripts/postinstall.js`). Asset
name `a4-<key>` plus `.exe` on win32.

---

## 4. Work packages

### WP1 — Release artifacts: manifest, signature, attestation, latest pointer

Prerequisite for every installer/updater. Files: `.github/workflows/release-please.yml`,
`release-please-config.json`, new `docs/public/a4/latest.json`, new `scripts/write-release-manifest.sh`.

1. **`manifest.json`** uploaded to the `a4-cli-v<ver>` release by the
   `publish-cli-checksums` job (it already has every checksum). Schema:

   ```json
   {
     "schemaVersion": 1,
     "name": "a4",
     "version": "0.13.0",
     "tag": "a4-cli-v0.13.0",
     "releasedAt": "2026-09-10T12:00:00Z",
     "assets": {
       "darwin-arm64": { "name": "a4-darwin-arm64", "sha256": "…" },
       "darwin-x64":   { "name": "a4-darwin-x64",   "sha256": "…" },
       "linux-x64":    { "name": "a4-linux-x64",    "sha256": "…" },
       "linux-arm64":  { "name": "a4-linux-arm64",  "sha256": "…" },
       "win32-x64":    { "name": "a4-win32-x64.exe","sha256": "…" }
     },
     "checksums": "checksums.txt",
     "signature": "checksums.txt.minisig",
     "minimumVersion": null
   }
   ```
   `minimumVersion` is reserved for future forced upgrades; readers must
   tolerate `null`/absent.

2. **minisign signature over `checksums.txt`.** Generate a keypair once
   (`minisign -G`), store the secret key and password as GitHub secrets
   `A4_MINISIGN_SECRET_KEY` / `A4_MINISIGN_PASSWORD`. In
   `publish-cli-checksums`, after combining: install `minisign` (apt package
   `minisign` on ubuntu-latest), write the secret to a temp file, run
   `minisign -S -s <key> -m checksums.txt -t "a4-cli-v${VERSION}"`, upload
   `checksums.txt.minisig`. The **public key** string is embedded in four
   places: `cli/src/selfhost/keys.rs` (`pub const MINISIGN_PUBLIC_KEY`),
   `docs/public/install.sh`, `docs/public/install.ps1`, `packages/arete/bin/a4.js`.
   Add a CI check (`scripts/check-minisign-pubkey.sh`) that greps all four for
   the same value.

3. **GitHub artifact attestation.** In `build-cli-binaries`, add job-level
   `permissions: { contents: write, id-token: write, attestations: write }`
   and a step `uses: actions/attest-build-provenance@v2` with
   `subject-path: ${{ matrix.binary_name }}` before the upload step. This
   is for humans/agents with `gh` (`gh attestation verify <file> -R AreteA4/arete`);
   installers do not depend on it.

4. **Latest-version pointer without the GitHub API.** `releases/latest`
   cannot be used: this repo cuts many releases per cycle (`arete-v*`,
   `arete-mcp-v*`, `arete-python-v*`, …) and "latest" may not be the CLI.
   Instead add `docs/public/a4/latest.json`:
   ```json
   { "schemaVersion": 1, "version": "0.13.0" }
   ```
   and register it in `release-please-config.json` under the `cli` package's
   `extra-files` as `{ "type": "json", "path": "/docs/public/a4/latest.json", "jsonpath": "$.version" }`
   so the release PR bumps it. It is served at
   `https://docs.arete.run/a4/latest.json`. Known race: docs may redeploy a
   few minutes before binaries finish uploading; installers must turn a 404
   on the manifest into the message "Release <ver> is still publishing; retry
   in a few minutes" (not a generic download error).

   Download base: `https://github.com/AreteA4/arete/releases/download/a4-cli-v<ver>/`.

5. **Stop building/publishing `a4-mcp` artifacts** once WP6 ships (delete
   `build-mcp-binaries`, `publish-mcp-checksums`, `publish-npm-mcp` or gate
   them off). Keep them until then.

Done when: a release has `manifest.json`, `checksums.txt.minisig`, five
attested binaries; `latest.json` matches the CLI version;
`minisign -V -p <pubkey> -m checksums.txt` succeeds locally.

### WP2 — `a4 self install`

New module `cli/src/selfhost/` (`mod.rs`, `install.rs`, `receipt.rs`,
`path_edit.rs`, `manifest.rs`, `verify.rs`, `keys.rs`). New top-level
`Commands::SelfCmd(SelfCommands)` with clap `#[command(name = "self")]`.

```text
a4 self install [--install-dir <DIR>] [--no-modify-path] [--source <sh|ps1|npm|manual>]
                [--checksums <FILE> --signature <FILE>] [--force] [--json]
```

Behavior, in order:

1. `src = std::env::current_exe()`. If `--checksums`/`--signature` are given,
   verify: minisign signature over the checksums file with the embedded
   public key (`minisign-verify` crate), then the SHA-256 of `src` matches
   the entry for the current platform's asset name. On failure: exit 1,
   delete nothing, message names which check failed. If not given (manual
   install), skip and record `"verified": false` in the receipt.
2. Resolve `install_dir`: `--install-dir` → `A4_INSTALL_DIR` → `XDG_BIN_HOME`
   → `$HOME/.local/bin` (`%USERPROFILE%\.local\bin`). Create it (0755).
3. If `install_dir/a4[.exe]` exists and is the same file as `src`, this is a
   re-run: skip the copy. Otherwise copy `src` to `install_dir/a4.tmp-<pid>`,
   set mode 0755, atomically rename over `install_dir/a4`. On Windows, if
   the target is in use, use the `self-replace` crate's rename-aside strategy.
4. Write receipt `~/.arete/receipt.json`:
   ```json
   {
     "schemaVersion": 1,
     "version": "0.13.0",
     "binary": "/Users/x/.local/bin/a4",
     "installDir": "/Users/x/.local/bin",
     "platform": "darwin-arm64",
     "source": "sh",
     "verified": true,
     "modifyPath": true,
     "installedAt": "2026-09-10T12:00:00Z"
   }
   ```
5. PATH (skip when `--no-modify-path`, `A4_NO_MODIFY_PATH=1`, or `CI` set;
   record the outcome in `modifyPath`):
   - Unix: for each of `~/.profile`, and by `$SHELL` basename `~/.zshrc`
     (zsh), `~/.bashrc` + `~/.bash_profile` if it exists (bash),
     `~/.config/fish/conf.d/a4.fish` (fish): if the file does not already
     contain a line matching `.local/bin` on PATH, append
     `export PATH="$HOME/.local/bin:$PATH" # added by a4 self install`
     (fish: `fish_add_path -g $HOME/.local/bin`). Create `~/.profile` if
     missing; never create the others.
   - Windows: read `HKCU\Environment\Path`; if `install_dir` absent, append
     it (`REG_EXPAND_SZ`) and broadcast `WM_SETTINGCHANGE`.
   - If `$GITHUB_PATH` is set, append `install_dir` to that file.
6. Shadowing: scan PATH for another `a4` ahead of `install_dir` (typically
   `~/.cargo/bin/a4` or an npm global shim). Warn on stderr with the path and
   the removal command; never delete it.
7. Output. Human: a short summary. **Always**, even with `--json`, print
   these two lines last on stdout (agents parse them; their shells snapshot
   PATH at session start so rc edits are invisible until a new session):
   ```text
   A4_BIN=/Users/x/.local/bin/a4
   export PATH="$HOME/.local/bin:$PATH"
   ```
   With `--json`, the JSON object precedes them and contains the receipt plus
   `pathModified: [files]`, `shadowedBy: path|null`.

Also add `a4 self uninstall [--json]`: removes the binary and the receipt
(not `~/.arete/credentials.toml`), removes the PATH lines it added, prints
what it left behind.

Tests: unit tests for receipt round-trip, PATH-edit idempotency on fixture
rc files, shadow detection with a fake PATH, minisign verification against a
checked-in test vector (`cli/tests/fixtures/selfhost/`), Windows path logic
behind `cfg(windows)`.

### WP3 — Bootstrappers: `install.sh`, `install.ps1`, npm package rewrite

Source of truth: `docs/public/install.sh`, `docs/public/install.ps1`
(served at `https://docs.arete.run/install.sh`; landing adds redirects from
`https://arete.run/install.sh` and `/install.ps1`, same as `agent.md`).

**`install.sh`** (POSIX sh, no bash-isms, ~120 lines):

```text
usage: curl -fsSL https://arete.run/install.sh | sh [-s -- [VERSION] [--no-modify-path] [--install-dir DIR]]
env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH
```
1. Detect os/arch → platform key; fail with a clear message on unsupported.
2. Version: arg → `A4_VERSION` → GET `https://docs.arete.run/a4/latest.json`
   (parse `"version"` with `sed`; no `jq` dependency).
3. Download to `mktemp -d`: asset, `checksums.txt`, `checksums.txt.minisig`.
   On 404 of the asset: print the "still publishing" message.
4. Verify SHA-256 with `sha256sum` or `shasum -a 256`.
5. If `minisign` is on PATH, verify the signature here too; otherwise print
   `note: minisign not found; signature will be verified by a4 self install`.
   (Trust model: first install trusts HTTPS + sha256; `self update` runs from
   a trusted binary and always verifies the signature.)
6. `chmod +x` and `exec "$tmp/a4" self install --source sh --checksums "$tmp/checksums.txt" --signature "$tmp/checksums.txt.minisig" "$@"`.
   Clean the temp dir via `trap`.

**`install.ps1`**: same steps with `Invoke-WebRequest`, `Get-FileHash`,
`--source ps1`. Usage `irm https://arete.run/install.ps1 | iex`; env vars as
above.

**`@usearete/a4` (`packages/arete/`)** becomes a scriptless bootstrapper:

- Delete `scripts/postinstall.js` and the `postinstall` entry; remove
  `"scripts"` from `files`. No `optionalDependencies`.
- `bin/a4.js`:
  - `npx @usearete/a4 install [--install-dir DIR] [--no-modify-path]`:
    download the asset for **the package's own version** (the package is
    version-locked to the CLI; no latest lookup) plus `checksums.txt` and
    `checksums.txt.minisig` into `os.tmpdir()`, verify SHA-256, verify the
    minisign signature in Node (Ed25519 via `crypto.verify(null, …)` and
    BLAKE2b-512 prehash via `crypto.createHash("blake2b512")`; parse the
    minisign key/sig base64 layout: 2-byte algorithm, 8-byte key id, then
    32-byte key / 64-byte signature), then spawn
    `<tmp>/a4 self install --source npm --checksums … --signature … [flags]`
    with `stdio: inherit`, and exit with its code.
  - Any other argv: if `~/.arete/receipt.json` exists and its `binary`
    exists, `spawnSync(receipt.binary, argv, { stdio: "inherit" })` and exit
    with its code. Otherwise run the install path silently first
    (`--json` output suppressed, `A4_BIN=` line captured), then spawn. This
    makes `npx @usearete/a4 explore --json` work in the same agent session
    regardless of PATH.
  - Keep the recursion sentinel from the current launcher. Remove the PATH
    scan fallback (the receipt is authoritative).
- `bin/a4.test.js`: replace PATH tests with: minisign verification against
  the same test vector as WP2 (copy it to `packages/arete/test/fixtures/`),
  receipt passthrough with a fake binary, install path with a local HTTP
  server serving fixture assets (set `A4_MANIFEST_BASE_URL`).
- README: `npx @usearete/a4 install` first; `npm install -g @usearete/a4`
  documented as equivalent.

Done when: acceptance test steps 1 and 7 pass via both `install.sh` and
`npx`; `pnpm dlx @usearete/a4 install` and `bunx @usearete/a4 install` also
work (no lifecycle scripts involved).

### WP4 — `a4 self update` and `a4 upgrade`

```text
a4 self update [VERSION] [--check] [--dry-run] [--json] [-y]
a4 upgrade …          (top-level alias; identical args, delegates)
```

1. Read the receipt. If absent: exit 1 with
   `a4 was not installed by the Arete installer. Reinstall with: curl -fsSL https://arete.run/install.sh | sh`
   (mention `cargo install a4-cli` only if `~/.cargo/bin/a4` is the running
   binary, and then as `cargo install a4-cli --force`).
2. Target version: `VERSION` arg, else `latest.json`. Downgrades are allowed
   when explicit.
3. `--check`: compare with `env!("CARGO_PKG_VERSION")`; print human or JSON
   `{ "current", "latest", "updateAvailable": bool }`; exit 0 if current, 10
   if an update is available. No download.
4. Otherwise download asset + `checksums.txt` + `.minisig` to
   `~/.arete/downloads/<ver>/`, verify SHA-256 and the minisign signature
   (mandatory here; the running binary is trusted), then replace the binary
   at `receipt.binary` with the `self-replace` crate, update the receipt
   (`source: "self-update"`, `version`), delete the download. `--dry-run`
   stops after verification and prints the plan.
5. Nudge (in `main()` after the command runs, before telemetry flush): at
   most once per 24 h (`~/.arete/update-check.json` `{ "checkedAt", "latest" }`),
   only when stderr is a TTY, `CI` unset, `--json` not passed,
   `A4_NO_UPDATE_CHECK` unset, and the command is not `self`/`upgrade`/`mcp`/
   `stream`. Fetch `latest.json` with a 2 s timeout; on any error stay silent.
   Message: `a4 0.14.0 is available (you have 0.13.0). Run: a4 self update`.

Tests: `--check` exit codes against a fixture `latest.json` served locally;
replace logic on a temp copy; nudge throttling with an injected clock.

### WP5 — Non-interactive guards

Retrofit using `ui::interactive()` from §3:

- `a4 create`: when not interactive and `name` or `--template` is missing,
  error: `Missing --template. Pass: a4 create <name> --template react-ore|rust-ore|typescript-ore|python-ore`.
  Add `--json` output `{ "path", "template", "installedDependencies": bool, "next": [commands] }`.
- `a4 init`: remove `prompt_project_name` entirely; name = `--name` → directory basename.
- `a4 auth login`: when not interactive and `--key` missing, error with the
  `a4 auth signup` / `--key` alternatives.
- `programs.rs`: replace its ad-hoc `is_terminal` checks with `interactive()`.
- Telemetry banner: additionally skip when `!stderr.is_terminal()`.

Done when: every command run with `</dev/null` either succeeds or exits 1
with a flag-listing error; none blocks. Add a CI test that runs
`a4 create`, `a4 init`, `a4 auth login` with stdin closed and asserts this.

### WP6 — Fold the MCP server into `a4` as `a4 mcp`

1. `rust/arete-mcp`: add `src/lib.rs` exporting
   `pub async fn serve_stdio() -> anyhow::Result<()>` (the body of today's
   `main` minus runtime setup, including the tracing-subscriber init to
   **stderr**), and `pub use` nothing else. Move the tool server
   (`AreteMcp`, `lenient`, tool definitions) out of `main.rs` into
   `src/server.rs`. Keep `src/main.rs` as a 10-line binary calling
   `serve_stdio()` until the artifacts are removed (WP1 step 5), then delete
   the `[[bin]]`.
2. `cli/Cargo.toml`: add `arete-mcp = { path = "../rust/arete-mcp", version = "0.12.0" }`
   and add `arete-mcp` to the `linked-versions` group check (it is already in
   the group). Upgrade the CLI's `reqwest` from 0.11 to 0.12 (keep the
   `blocking` feature) so the binary does not link two reqwest/hyper stacks;
   if that upgrade turns out to be more than a mechanical bump, ship with
   both versions and file a follow-up.
3. `a4 mcp` (`Commands::Mcp { }`, hidden `--stdio` accepted for
   forward-compat): build a multi-thread tokio runtime and call
   `serve_stdio()`. Never print to stdout except MCP frames. Add `"mcp"` to
   `command_name()` and exclude it from the update nudge.
4. Credentials: `rust/arete-mcp/src/credentials.rs` already reads
   `ARETE_API_KEY` then `~/.arete/credentials.toml`; no change.
5. `packages/mcp`: publish one final version whose `bin` prints
   `@usearete/mcp is deprecated. Update your MCP config to run "a4 mcp" (see a4 init) or "npx @usearete/a4 mcp".`
   and exits 1; mark `deprecated` in `package.json`/npm. Update
   `.mcp.json` and `opencode.json` in this repo to `a4 mcp`.

Done when: `a4 mcp` responds to an MCP `initialize` over stdio (add an
integration test that pipes a JSON-RPC initialize request and asserts the
`serverInfo.name`); binary size increase is recorded in the PR.

### WP7 — `a4 init` rewrite

New module `cli/src/agents/` (`mod.rs`, `detect.rs`, `agents_md.rs`,
`skills.rs`, `mcp_config.rs`, `report.rs`). `Commands::Init(InitArgs)`:

```text
a4 init [-y] [--non-interactive] [--json] [--dry-run] [--force]
        [--name <project>]
        [--agents <list|all|none>]      # default: detected
        [--global]                      # user-scope skills + MCP config
        [--no-manifest] [--no-agents-md] [--no-skills] [--no-mcp]
        [--skills-ref <git ref>]        # default: main
```

**Detection** (`detect.rs`), returns `Vec<Agent>` with `how` = project|home|env:

| Agent id | Project signals | Home signals | Env |
|---|---|---|---|
| `claude-code` | `.claude/`, `CLAUDE.md`, `.mcp.json` | `~/.claude/` | `CLAUDECODE=1` |
| `cursor` | `.cursor/` | `~/.cursor/` | `CURSOR_AGENT` (cursor.com/docs/agent/tools/terminal) |
| `codex` | `.codex/` | `$CODEX_HOME` or `~/.codex/` | `CODEX_THREAD_ID`, `CODEX_SANDBOX`, `CODEX_SANDBOX_NETWORK_DISABLED` (openai/codex `codex-rs/protocol/src/shell_environment.rs`, `codex-rs/core/src/spawn.rs`) |
| `opencode` | `opencode.json`, `opencode.jsonc`, `.opencode/` | `$XDG_CONFIG_HOME/opencode/` or `~/.config/opencode/` | `OPENCODE_CLIENT` (only under the ACP/desktop hosts; anomalyco/opencode `packages/opencode/src/cli/cmd/acp.ts`; the shell tool otherwise just inherits `process.env`, no bare `OPENCODE` var) |
| `gemini-cli` | `.gemini/` | `~/.gemini/` | `GEMINI_CLI=1` (geminicli.com/docs/tools/shell) |
| `vscode` (Copilot) | `.vscode/` | — | — |
| `copilot-cli` | `.github/copilot-instructions.md` | `~/.copilot/` | — |
| `windsurf` | `.windsurf/` | `~/.codeium/windsurf/` | — |
| `cline` | `.clinerules/` | `~/.cline/` | — |
| `zed` | `.zed/` | `~/.config/zed/` | — |
| `amp` | `.amp/` | `~/.config/amp/` | — |
| `kiro` | `.kiro/` | `~/.kiro/` | — |
| `roo` | `.roo/` | `~/.roo/` | — |
| `goose` | `.goose/` | `~/.config/goose/` | — |

`.agents/` present ⇒ mark `universal`. Env-var names verified 2026-09-03
(sources in the table); any signal that could not be cited was dropped.
If nothing is detected and `--agents` is absent: proceed with the
agent-independent set (manifest, `AGENTS.md`, `CLAUDE.md`, `.agents/skills`,
`.mcp.json`) and add a warning; do not fail.

**Writers**, every one an upsert reporting `created|updated|unchanged|skipped(reason)`:

1. `arete.toml` — reuse today's discovery logic; create if absent; if present
   leave untouched (`unchanged`) unless `--force`, which rewrites only the
   `[project]` block. Name from `--name` or directory basename.
2. `AGENTS.md` — managed block. Exact content (keep in
   `cli/src/agents/templates/agents-block.md`, `include_str!`):

   ```markdown
   <!-- BEGIN:arete v2 -->
   ## Arete

   This project uses Arete for typed Solana views and program operations. The
   `a4` CLI is the interface; the installed `arete`, `arete-streams`,
   `arete-programs`, `arete-stack-authoring`, and `arete-deploy` skills hold the
   detailed workflows.

   - Health check first: `a4 doctor --json` (exit 0 = ready). If `a4` is
     missing: `curl -fsSL https://arete.run/install.sh | sh`
   - Start from intent with `a4 know search --query "..." --json`, then inspect
     exact descriptors with `a4 explore stack <ref> --json` or
     `a4 explore program <ref> --json`.
   - Never guess schemas or SDK methods. Generate clients from the explored
     descriptor with `a4 install stack <ref> --ts` or
     `a4 install program <ref> --ts`; use `--rust` or `--python` only when the
     descriptor advertises that target.
   - Account: `a4 auth signup` (agent) or `a4 auth login --key <a4_ak_…>`.
   - Live data in your loop: the `arete` MCP server (`a4 mcp`) is configured;
     use it for exploration, use generated SDKs for shipped code.
   - Building or preparing does not authorize transaction submission or hosted
     deployment. Keep external mutations within the user's request.
   - Never `cargo install a4-cli`; update with `a4 self update`.

   Docs: https://docs.arete.run (agent entry: https://docs.arete.run/agent.md)
   <!-- END:arete -->
   ```
   Upsert: if `AGENTS.md` lacks the markers, append the block (with a blank
   line before). If present, replace between markers; if the `v2` token
   differs, that is an `updated`. Content outside the markers is untouched.
3. `CLAUDE.md` — if missing, create with exactly `@AGENTS.md\n`. If present
   and no line equals `@AGENTS.md`, insert it as the first line. Claude Code
   does not read `AGENTS.md`; this import is its documented bridge.
4. `.gemini/settings.json` — when `gemini-cli` selected: merge
   `"context": { "fileName": ["AGENTS.md", "GEMINI.md"] }` (union with any
   existing array).
5. **Skills** (`skills.rs`) — when not `--no-skills`: if `npx` is not on
   PATH → `skipped(npx not found; install Node or run: npx skills add AreteA4/skills)`.
   Otherwise run
   `npx -y skills add <source> --skill '*' --agent <ids> -y [--copy on Windows] [-g if --global]`
   where `<source>` = `AreteA4/skills` or
   `https://github.com/AreteA4/skills/tree/<ref>` when `--skills-ref` set,
   and `<ids>` = selected agent ids mapped to `skills` names
   (`claude-code, cursor, codex, opencode, gemini-cli, github-copilot,
   windsurf, cline, zed, amp, kiro-cli, roo, goose`; drop `vscode` and
   `copilot-cli` → `github-copilot`). Set `DO_NOT_TRACK=1` for the child.
   Timeout 120 s. Report `updated` if `skills-lock.json` changed, else
   `unchanged`. Run before the MCP writers so a failure here does not block them.
6. **MCP config** (`mcp_config.rs`) — two servers: `arete` (stdio,
   command `a4`, args `["mcp"]`) and `arete-docs` (remote,
   `https://docs.arete.run/mcp`). Use the absolute `receipt.binary` for the
   command when a receipt exists (GUI hosts do not inherit shell PATH),
   else `a4`. Per agent, parse the existing file (create if absent), set
   only these two keys, write back preserving everything else:

   | Agent | File (project / `--global`) | Shape |
   |---|---|---|
   | claude-code | `.mcp.json` / `~/.claude.json` | `mcpServers.arete = {"type":"stdio","command":C,"args":["mcp"]}`, `mcpServers["arete-docs"] = {"type":"http","url":U}` |
   | cursor | `.cursor/mcp.json` / `~/.cursor/mcp.json` | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"url":U}` |
   | vscode | `.vscode/mcp.json` / skip | `servers.arete = {"type":"stdio","command":C,"args":["mcp"]}`, `servers["arete-docs"] = {"type":"http","url":U}` |
   | copilot-cli | `.mcp.json` (shared with claude-code; add `"tools":["*"]` only if the file was created for copilot) / `~/.copilot/mcp-config.json` | `{"type":"local","command":C,"args":["mcp"],"tools":["*"]}`, `{"type":"http","url":U,"tools":["*"]}` |
   | codex | `.codex/config.toml` / `~/.codex/config.toml` | `[mcp_servers.arete] command = C, args = ["mcp"]`; `[mcp_servers.arete-docs] url = U`. Use `toml_edit`. Warn: project file only loads for trusted projects |
   | opencode | `opencode.json` (or existing `.jsonc`) / `~/.config/opencode/opencode.json` | `mcp.arete = {"type":"local","command":[C,"mcp"],"enabled":true}`, `mcp["arete-docs"] = {"type":"remote","url":U,"enabled":true}`; ensure `"$schema": "https://opencode.ai/config.json"` on create. For `.jsonc` with comments, use a comment-preserving parser (e.g. `jsonc-parser` crate edits) or fall back to `skipped(jsonc with comments; add manually: …)` |
   | gemini-cli | `.gemini/settings.json` / `~/.gemini/settings.json` | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"httpUrl":U}` |
   | windsurf | — / `~/.codeium/windsurf/mcp_config.json` | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"serverUrl":U}` (global only; project run reports `skipped(global only)`) |
   | cline | — / `~/.cline/mcp.json` | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"type":"streamableHttp","url":U}` |
   | zed | `.zed/settings.json` / `~/.config/zed/settings.json` | `context_servers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"url":U}` |
   | amp | `.amp/settings.json` / `~/.config/amp/settings.json` | `"amp.mcpServers".arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"url":U}` |
   | kiro | `.kiro/settings/mcp.json` / `~/.kiro/settings/mcp.json` | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"url":U}` |
   | roo | `.roo/mcp.json` / skip | `mcpServers.arete = {"command":C,"args":["mcp"]}`, `["arete-docs"] = {"type":"streamable-http","url":U}` |
   | goose | `.goose/config.yaml` / `~/.config/goose/config.yaml` | YAML `extensions.arete = {type: stdio, cmd: C, args: [mcp], enabled: true}`, `arete-docs = {type: streamable_http, uri: U, enabled: true}` (`serde_yaml`) |

   Implement claude-code, cursor, vscode, codex, opencode, gemini-cli first;
   the rest may follow in a second PR but the table is the contract.

**Output.** Human: one line per item with its status. `--json`:

```json
{
  "schemaVersion": 1,
  "dryRun": false,
  "detectedAgents": [{"id":"claude-code","how":"env"}, {"id":"cursor","how":"home"}],
  "selectedAgents": ["claude-code","cursor"],
  "results": [
    {"item":"arete.toml","status":"created","path":"arete.toml"},
    {"item":"agents-md","status":"updated","path":"AGENTS.md"},
    {"item":"claude-md","status":"unchanged","path":"CLAUDE.md"},
    {"item":"skills","status":"skipped","reason":"npx not found","fix":"npx skills add AreteA4/skills"},
    {"item":"mcp:claude-code","status":"created","path":".mcp.json"},
    {"item":"mcp:cursor","status":"created","path":".cursor/mcp.json"}
  ],
  "warnings": ["No coding agent detected; wrote agent-independent files only."],
  "next": ["a4 doctor --json", "a4 explore --json"]
}
```

`--dry-run` produces the same object with `dryRun: true` and statuses
prefixed `would-` (`would-create`, …). Exit 0 unless a writer errored
(`status: "error"`), then 1.

Tests: fixture directories under `cli/tests/fixtures/init/` covering: empty
dir; existing `AGENTS.md` with user content above and below the block;
stale `v0` block; existing `.mcp.json` with other servers; `opencode.jsonc`
with comments; Codex TOML with other tables; re-run produces all
`unchanged`. Skills step tested with a fake `npx` on PATH.

### WP8 — `a4 doctor`

`Commands::Doctor { json, fix }`. Shares `cli/src/agents/` with `init`.

Checks (id → what → fix text), each `ok | warn | fail | info`:

| id | check | on failure |
|---|---|---|
| `cli.version` | receipt present; current vs `latest.json` (2 s timeout; network error ⇒ `info`) | `warn`: `a4 self update` |
| `cli.install` | receipt `binary` exists and is the running exe; no shadowing `a4` earlier on PATH | `warn` with the shadowing path |
| `cli.path` | `installDir` is on PATH of this process | `warn`: print the export line |
| `project.manifest` | `arete.toml` parses (`installer::validate_project`) | `fail`: `a4 init` / `a4 config validate` |
| `project.lock` | `arete.lock` fresh | `warn`: `a4 install` |
| `auth.credentials` | key present for active API URL (`ARETE_API_KEY` or file) | `info` (not needed for explore): `a4 auth signup` |
| `auth.whoami` | only if credentials: `GET /api/agents/me` or user whoami succeeds | `fail`: `a4 auth login --key …` |
| `net.api` | `GET https://api.arete.run/api/registry` (or `/health` if present) 200 | `fail` |
| `net.docs-mcp` | `HEAD https://docs.arete.run/mcp` reachable | `warn` |
| `tools.node` | `npx` on PATH | `info`: needed only for skills |
| `tools.rust` | `cargo` on PATH | `info`; `warn` only if `arete.toml` has `[authoring]` stacks |
| `agents.detected` | list | `info` |
| `agents.<id>.mcp` | both servers present and equal to the desired shape | `warn`: `a4 doctor --fix` |
| `agents.<id>.skills` | skill dirs for that agent contain all five workflow skills | `warn`: `npx skills add AreteA4/skills --agent <id>` |
| `agents.agents-md` | block present and `v2` | `warn`: `a4 doctor --fix` |
| `agents.claude-md` | `CLAUDE.md` contains `@AGENTS.md` (only if claude-code selected) | `warn` |
| `agents.gemini-context` | Gemini `context.fileName` includes `AGENTS.md` (only if gemini-cli) | `warn` |
| `agents.codex-trust` | `.codex/config.toml` exists but project not trusted in `~/.codex/config.toml` | `info` |

Output `--json`:
```json
{ "schemaVersion": 1, "status": "ok|warn|fail",
  "checks": [{ "id": "cli.version", "status": "ok", "detail": "0.13.0 (latest)", "fix": null }] }
```
Exit 0 for `ok`/`warn`, 1 for `fail`. `--fix` runs the `init` writers for
every `agents.*` warn (never touches auth, network, or tools) and re-runs the
checks. Human output: one line per check with a coloured status, then the
fixes as a list.

### WP9 — `a4 auth signup`

`AuthCommands::Signup { name: Option<String> }`, `--json` via global flag.

1. POST `<api_url>/api/agents/signup` with `{"display_name": name}` (omit
   when `None`). Add `ApiClient::agent_signup(&self, display_name: Option<&str>) -> Result<AgentSignupResponse>`
   (unauthenticated client).
2. On success: `ApiClient::save_api_key(&resp.api_key, Some(&api_url))`, then
   print slug and the credentials path. Never print the key in human mode;
   in `--json` print `{ "slug", "displayName", "credentialsPath", "apiKey" }`
   (the agent may need it for `ARETE_API_KEY` in a sub-process; document
   that it is a secret).
3. On 429: `Signup limit reached (5 per hour per IP). Retry later, or use a key from https://arete.run/keys: a4 auth login --key <a4_ak_…>`.
4. If credentials already exist for this API URL: refuse unless `--force`,
   pointing at `a4 auth status`.

Test with a mocked HTTP server (the crate already tests `ApiClient`; follow
that pattern).

### WP10 — Text: hosted files, docs pages, READMEs, skills repo

Replace `docs/public/agent.md` with this (verbatim; keep under 45 lines):

```markdown
# Set up Arete

You are a coding agent setting up Arete (real-time Solana data streams).
Run these in order. Nothing here needs Rust or an account.

1. Install the CLI (prebuilt, signed binary):

       curl -fsSL https://arete.run/install.sh | sh        # macOS / Linux
       irm https://arete.run/install.ps1 | iex             # Windows PowerShell
       npx @usearete/a4 install                            # if you prefer npm

   The installer prints `A4_BIN=<absolute path>`. If `a4` is not found
   afterwards, your shell captured PATH before the install: use that absolute
   path, or run `export PATH="$HOME/.local/bin:$PATH"`.

2. In the project directory:

       a4 init -y

   This writes `arete.toml`, an Arete section in `AGENTS.md` (and a
   `CLAUDE.md` import), installs the Arete skills for every coding agent it
   detects, and configures the `arete` (stream) and `arete-docs` MCP servers.
   Add `--global` to install skills and MCP config for your user instead of
   the project.

3. Verify:

       a4 doctor --json        # exit 0 = ready; each check carries a fix

4. Discover data (no account needed):

       a4 explore --json
       a4 explore stack ore --json

Need an account (deploying, knowledge layer)? `a4 auth signup` registers you
as an agent and stores the key. Have a human-issued key? `a4 auth login --key <a4_ak_…>`.
Update later with `a4 self update`. Never `cargo install a4-cli`.

Everything else (SDK patterns, building stacks) is in the installed skills.
Platform API reference: https://docs.arete.run/skill.md
```

Other text changes:

- `docs/public/skill.md`: section "3. Install the local toolkit" → the four
  lines above; section "7"/"MCP servers" → `a4 mcp` instead of
  `npx -y @usearete/mcp`; add `a4 auth signup` under "1. Register" as the
  preferred route; bump `version` in the frontmatter and add a "What's new"
  entry.
- `docs/src/content/docs/agent-skills/overview.mdx`, `setup.mdx`,
  `setup-tools.mdx`, `mcp.mdx`, `docs/src/content/docs/cli/commands.mdx`:
  replace every `cargo install a4-cli` / `npm install -g` install snippet
  with the installer; replace `npx -y @usearete/mcp` with `a4 mcp`; document
  `a4 init`, `a4 doctor`, `a4 self update`, `a4 self install`,
  `a4 auth signup`, `a4 mcp`. The copy-prompt text on `overview.mdx` and
  `setup-tools.mdx` becomes:
  `Read https://docs.arete.run/agent.md and follow it to set up Arete in this project, then tell me what live data is available.`
- `README.md` (root), `cli/README.md`, `cli/src/main.rs` doc comment,
  `packages/arete/README.md`: installer first; crates.io mentioned only
  under "Building from source".
- `AreteA4/skills` (separate repo): install the five workflow skills together:
  discovery/project dependencies, views/streams, program operations, stack
  authoring, and permission-aware deployment. Only stack authoring requires a
  Rust toolchain. Tag a release so `--skills-ref` can pin.
- Landing (`../hyper-stack-platform/landing/vercel.json`): add permanent
  redirects `/install.sh` → `https://docs.arete.run/install.sh` and
  `/install.ps1` → `https://docs.arete.run/install.ps1`.

---

## 5. Sequencing

```text
WP1 release artifacts ──┬── WP2 self install ── WP3 bootstrappers ── WP4 self update
                        │
WP5 non-interactive ────┤   (independent; small; do first or in parallel)
                        │
WP6 a4 mcp ─────────────┴── WP7 init ── WP8 doctor ── WP10 text
                                  │
WP9 auth signup ──────────────────┘   (independent of WP6–WP8)
```

Ship order for PRs: WP5 → WP1 → WP2+WP3 (one PR, tested against a release
candidate) → WP4 → WP6 → WP7 → WP8 → WP9 → WP10. WP10 must not merge before
WP2–WP4 are in a published release, because the hosted `agent.md` is live
immediately.

---

## 6. Verification checklist

Add `scripts/e2e-onboarding.sh` that runs the acceptance test from §0 inside
`docker run --rm -it ubuntu:24.04` (installs only `curl ca-certificates git`),
with `A4_VERSION` pointing at the candidate release, and a second pass with
Node installed (`apt install nodejs npm`). Add a `windows-latest` CI job that
runs `install.ps1`, `a4 doctor --json`, `a4 self update --check` against the
same release. Record the wall-clock time of step 1 in the job summary.

Manual checks before flipping `agent.md`:

- Paste the copy-prompt into Claude Code, Cursor and Codex in a fresh repo;
  each must reach a green `doctor` and answer the "what live data" question
  using `a4 explore --json`, with no human intervention.
- `pnpm dlx @usearete/a4 install` and `bunx @usearete/a4 install` work.
- `a4 self update` from N-1 to N on all three OSes, including while another
  `a4` process is running on Windows.

---

## 7. Decisions log

- 2026-09-02: agent-first framing; binary never requires Rust; `~/.local/bin`; `a4 self update`.
- 2026-09-03 (Adrian): fold `a4-mcp` into `a4`; project scope by default with `--global`; signup limit stays 5/hour/IP; web UI issues keys only; hosted `install.sh` via landing redirect to the docs site; distribution channels (plugin marketplaces, well-known indexes, registries, Homebrew) deferred and removed from this spec.

- 2026-09-03 (implementation): WP1–WP10 landed in one change set. The standalone `a4-mcp` binary was removed immediately (`arete-mcp` is a library crate; `release-recovery.yml` no longer builds MCP binaries and now signs `checksums.txt` and writes `manifest.json` like the release workflow). The `packages/mcp` release-please component stays until the final deprecated `@usearete/mcp` version is published, then it is removed. `--skills-ref` accepts branches and tags only (`npx skills` cannot pin a commit SHA). Interactive `a4 init` shows an agent picker with detected agents pre-selected; non-interactive runs keep the detected set. Windows paths (registry PATH, busy-binary swap) remain untested.

## 8. Verify during implementation

- ~~Env vars set by Cursor, Codex, Gemini CLI, OpenCode for "who is driving me" (only `CLAUDECODE=1` is confirmed).~~ resolved 2026-09-03: see the WP7 detection table; `CURSOR_AGENT`, `CODEX_THREAD_ID`/`CODEX_SANDBOX`/`CODEX_SANDBOX_NETWORK_DISABLED`, `GEMINI_CLI=1` confirmed from docs/source; OpenCode exports nothing to shell children except `OPENCODE_CLIENT` under ACP/desktop hosts (kept), bare `OPENCODE` dropped.
- ~~Whether `npx skills add https://github.com/AreteA4/skills/tree/<sha>` pins to a commit; if not, pin by tag.~~ resolved 2026-09-03: `skills` 1.5.23 runs `git clone --branch <ref>`, so `tree/<branch>` and `tree/<tag>` pin (lock records `"ref"`), `tree/<sha>` fails with `Remote branch <sha> not found`; `owner/repo#<ref>` also works, `owner/repo@<ref>` is silently ignored. `a4 init` rejects a hex SHA in `--skills-ref` with an actionable error; pin by tag.
- Docs site deploy trigger for `docs/public/` changes (needed for `latest.json` timing).
- `reqwest` 0.11 → 0.12 upgrade cost in `cli/`.
- Comment-preserving JSONC editing crate for `opencode.jsonc`.
- Exact `minisign` prehash behaviour (`ED` = BLAKE2b-512 prehashed) when writing the Node verifier; validate against the same fixture used by the Rust tests.
