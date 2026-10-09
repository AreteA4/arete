# Set up Arete

You are a coding agent setting up Arete, an agent-first Solana application
toolkit. Use current CLI output and exact descriptors rather than guessing
program, account, view, operation, or endpoint names.

## Capabilities

Arete can:

- discover relevant programs and stacks from intent through the public catalog;
- explore deployed live views through MCP or `a4 stream`;
- generate typed TypeScript, React, Rust, or Python SDKs when supported;
- read typed program accounts and generic Solana chain state;
- derive addresses and build instructions, transactions, and multi-step flows;
- inspect and submit locally signed transactions through application wallets;
- compose Program SDKs and selected live views into application stacks; and
- author and deploy custom live read models when existing views are not enough.

1.  Install the signed CLI binary. No Rust toolchain is required:

        curl -fsSL https://arete.run/install.sh | sh        # macOS / Linux
        irm https://arete.run/install.ps1 | iex             # Windows PowerShell
        npx @usearete/a4 install                            # npm bootstrap

    The installer prints `A4_BIN=<absolute path>`. If `a4` is not found in the
    current shell, use that path or refresh PATH. Update later with
    `a4 self update`. Do not substitute a Cargo install from memory.

2.  In the project root:

        a4 init -y

    This writes `arete.toml`, managed project instructions, the five Arete
    skills, and the Arete stream and documentation MCP configuration. It is
    idempotent. Use `--global` only when user-scoped setup was requested.

3.  Verify the environment:

        a4 doctor --json

    The top-level JSON `status` must be `"ok"` before treating setup as ready.
    Exit 0 can also mean `"warn"`: inspect every warning and follow its exact
    `fix` when it affects this project or the current agent. If `a4 init` changed
    skills or MCP configuration but the current agent does not see them, tell
    the user to reload or restart the agent host. After the restart, run
    `a4 doctor --json` again instead of repeatedly rewriting config.

4.  Create or verify the restricted agent account used by hosted views and the
    knowledge layer:

        a4 --profile agent auth signup --if-missing --json

    This setup prompt authorizes creation of the agent account. The command
    stores its `a4_ak_*` credential in the `agent` profile and never prints the
    secret. Confirm `credentialStored` is `true`; do not request, copy, or use a
    human `a4_sk_*` credential. Generated MCP configuration is pinned to this
    agent profile.

    The CLI and the Arete MCP server load stored credentials for you. Do not
    search for, read, print, or copy credential files or keys into code,
    prompts, tool arguments, or transcripts. To inspect authentication, use:

        a4 --profile agent auth status
        a4 --profile agent auth whoami --json

    SDK code that runs on a server, in an agent, or as a local script
    authenticates with an agent key or secret key through the SDK's
    `secretKey` (TypeScript) or `secret_key` (Python, Rust) option, or with no
    auth option at all by setting the `ARETE_API_KEY` environment variable.
    Read it from the environment; never write the key into source. The
    TypeScript SDK refuses `secretKey` in a browser.

    Browser code needs a publishable key instead: it is bound to one origin and
    safe to ship to the client. Create one per origin with
    `a4 auth keys create-publishable --origin <scheme://host[:port]>`.

5.  Continue from what the user actually asked. For the bootstrap prompt, which
    asks only what Arete can do, summarize the capability list above and use the
    catalog vocabulary for current discovery categories:

        a4 explore catalog --vocabulary --json

    Do not invent an on-chain intent or select a package. When the user has
    supplied a concrete intent, search for it instead:

        a4 explore catalog --query "<user intent>" --json

    Filter with `--kind program|stack`, `--mode read|build|subscribe`, and
    `--target typescript|rust|python` when useful. Then inspect an exact result:

        a4 explore catalog program <slug> --json
        a4 explore catalog stack <slug> --json

    A catalog result is not permission to invent missing delivery. Respect its
    reported modes, SDK targets, authentication, bindings, and install command.

6.  Route the task:
    - Use the `arete` skill for discovery and project dependencies.
    - Use `arete-streams` for deployed views and live subscriptions.
    - Use `arete-programs` for account reads, PDAs, operations, and transactions.
    - Use `arete-stack-authoring` for custom read models and portable artifacts.
    - Use `arete-deploy` only for an explicitly authorized publication or
      hosted deployment task.

7.  Use MCP for exploration and generated SDKs for shipped code. For a hosted
    view, inspect the exact schema, connect with its descriptor, take a bounded
    sample, answer with provenance, and disconnect. If no suitable view exists,
    explain the gap. Do not construct endpoints.

8.  Add proven capabilities to the project:

        a4 install program <slug> --ts
        a4 install stack <slug> --ts
        a4 install --locked

    Use another target only when the exact descriptor verifies it. Do not edit
    generated output. `arete.toml` records intent; `arete.lock` records exact
    resolution.

    A stack includes the program SDKs for the programs its views index, at
    `arete.programs.<name>`, and they are the same SDKs a standalone program
    install gives. Install a stack when the app needs live views, with or
    without transactions; install a program on its own only when no stack you
    use covers it. Never merge stack and program objects by hand.

The setup command above is idempotent. When the user has instead supplied an
existing agent key, store it in the same restricted profile:

       a4 auth login --profile agent --key <a4_ak_...>

Never put a secret key in source, a prompt, or an MCP tool argument.

Preserve authority boundaries: reading, building, preparing, inspecting,
signing, submitting, compiling, publishing, and deploying are separate actions.

CLI field guide: https://docs.arete.run/skill.md
