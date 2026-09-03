# @usearete/a4

The Arete CLI (`a4`) as an npm package. Programmable, real-time Solana data
feeds: deploy stacks, explore live data, wire up your agent.

## Install

```bash
npx @usearete/a4 install
```

This downloads the `a4` release binary for your platform (the version is
locked to the package version), verifies its SHA-256 and minisign signature,
copies it to `~/.local/bin/a4` (`%USERPROFILE%\.local\bin\a4.exe` on
Windows), adds that directory to your shell PATH and writes
`~/.arete/receipt.json`. The last two stdout lines are always:

```text
A4_BIN=/home/you/.local/bin/a4
export PATH="$HOME/.local/bin:$PATH"
```

Agents: your shell snapshotted PATH when the session started, so either
`eval` the `export` line or call the `A4_BIN` path directly.

Equivalent:

```bash
npm install -g @usearete/a4 && a4 install   # global shim, same download path
pnpm dlx @usearete/a4 install
bunx @usearete/a4 install
```

Or without Node at all:

```bash
curl -fsSL https://arete.run/install.sh | sh          # macOS, Linux
irm https://arete.run/install.ps1 | iex                # Windows PowerShell
```

Flags and environment:

| Flag / env | Effect |
|---|---|
| `--install-dir DIR` / `A4_INSTALL_DIR` | Install somewhere other than `~/.local/bin` |
| `--no-modify-path` / `A4_NO_MODIFY_PATH=1` | Do not edit shell rc files / the Windows user PATH |
| `--json` | Print the install receipt as JSON (the `A4_BIN=` line still follows) |
| `A4_MANIFEST_BASE_URL`, `A4_LATEST_URL` | Test overrides for the release download base and latest pointer |

The package has no `postinstall` script and no dependencies; nothing touches
the network at `npm install` time.

## Run

Once installed, `a4` is on PATH in new shells. `npx @usearete/a4 <args>` also
works in the same session: it runs the binary recorded in
`~/.arete/receipt.json`, installing it silently first if needed, so
`npx @usearete/a4 explore --json` prints only the command's JSON.

```bash
a4 init -y          # arete.toml, AGENTS.md, CLAUDE.md, MCP config, skills
a4 doctor --json    # check the setup
a4 explore --json   # what live data is available
a4 self update      # upgrade in place (alias: a4 upgrade)
```

Full reference: <https://docs.arete.run/cli/commands/>. Agent setup guide:
<https://docs.arete.run/agent.md>.

## Building from source

Only needed for unsupported platforms (anything other than
darwin-arm64/x64, linux-x64/arm64, win32-x64):

```bash
cargo install a4-cli
a4 self install --source manual
```

## License

Apache-2.0
