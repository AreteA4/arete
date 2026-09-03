# Set up Arete

You are a coding agent setting up Arete (typed Solana views and program operations).
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

Everything else is in the five installed workflow skills: discovery, streams,
programs, stack authoring, and deployment.
Platform API reference: https://docs.arete.run/skill.md
