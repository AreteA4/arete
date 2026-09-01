# Arete Agent Setup

You are setting up Arete, a real-time Solana data streaming platform. Follow these steps in order.

> **Need an Arete API key?** This file installs the local toolkit. To **register your agent on the Arete platform** and get an `a4_ak_*` API key, read https://docs.arete.run/skill.md instead — that's the platform-onboarding manual.

## Step 0: Check What Is Already Installed

Before installing anything, check what is already present.

### Arete CLI

Run:
```
a4 --version
```

- If this prints a version number, the CLI is already installed. Continue to check if it needs updating.
- If the command is not found, the CLI needs to be installed (see Step 1).

If the CLI is already installed, check if it is up to date:
```
cargo install a4-cli 2>&1 | head -5
```
If this says "already up to date" or similar, skip Step 1. If a newer version is available, it will update in place.

### Agent Skills

Check if Arete skills are already installed using the skills CLI:
```
npx skills list 2>/dev/null | grep -i arete
```

- If this shows arete skills, they are already installed. Run `npx skills check` to see if updates are available.
- If no output, skills need to be installed (see Step 2).

Alternatively, check for skill files directly in the project. The location depends on which agent you are:

| Agent | Skill directory |
|---|---|
| Cursor | `.cursor/skills/` or `.agents/skills/` |
| Claude Code | `.claude/skills/` |
| OpenCode | `.opencode/skills/` or `.agents/skills/` |
| Windsurf | `.windsurf/skills/` |
| Cline | `.cline/skills/` |
| GitHub Copilot | `.agents/skills/` |
| Other / Unknown | `.agents/skills/` |

If skill files already exist and are up to date, skip Step 2.

## Step 1: Install the Arete CLI

Only do this if `a4 --version` failed in Step 0.

Preferred (Cargo):
```
cargo install a4-cli
```

If Cargo is not available, fall back to npm:
```
npm install -g @usearete/a4
```

Verify installation:
```
a4 --version
```

## Step 2: Install Agent Skills

Only do this if skill files were not found in Step 0, or if you want to update to the latest version.

The `npx skills` tool installs skills for 40+ coding agents including Cursor, Claude Code, OpenCode, Codex, Windsurf, VS Code Copilot, Cline, Roo Code, and many more. It auto-detects which agents are present and installs to the correct location for each.

### Determine your agent identity

Before running the install, determine which agent harness you are running in. This lets you target the install correctly and avoids unnecessary prompts.

**How to tell which agent you are:**
- If you have a system prompt or context that identifies you (e.g. "You are Claude Code", "Cursor Agent", etc.), use that identity.
- If you can check your own configuration directory, look for which of these exist: `.claude/`, `.cursor/`, `.opencode/`, `.windsurf/`, `.cline/`.
- If you genuinely cannot determine your identity, fall back to the general `.agents/` install.

**If you know which agent you are**, install with the `--agent` flag for your specific harness. This ensures skills land in the correct directory:

```
npx skills add AreteA4/skills --agent <your-agent>
```

Common agent identifiers:

| Agent | Flag |
|---|---|
| Claude Code | `--agent claude-code` |
| Cursor | `--agent cursor` |
| OpenCode | `--agent opencode` |
| Windsurf | `--agent windsurf` |
| GitHub Copilot | `--agent github-copilot` |
| Cline | `--agent cline` |
| Codex | `--agent codex` |
| Roo Code | `--agent roo` |
| Gemini CLI | `--agent gemini-cli` |
| Goose | `--agent goose` |
| Continue | `--agent continue` |

**If you cannot determine which agent you are**, fall back to the general install which writes to `.agents/skills/` (readable by most agents):

```
npx skills add AreteA4/skills
```

This auto-detects which agents are present and installs to the correct location for each. You can also explicitly install for all detected agents:

```
npx skills add AreteA4/skills --all
```

This installs context files that teach you (the agent) how to use Arete's SDKs, CLI, and Rust DSL.

For the full `npx skills` documentation: https://github.com/vercel-labs/skills

Additional options:
- `npx skills add AreteA4/skills -a cursor -a opencode` targets multiple specific agents
- `npx skills add AreteA4/skills -g` installs globally (available across all projects)

## Step 3: Discover Available Data

```
a4 explore --json
```

This lists all available stacks you can stream data from. Inspect one exact,
deployment-pinned stack descriptor with:

```
a4 explore stack <stack-ref> --json
```

Standalone programs are also discoverable without installing them:

```
a4 explore programs --json
a4 explore program <program-ref> --json
```

For field-level detail on a specific entity:

```
a4 explore <stack-name> <EntityName> --json
```

Exploration is descriptor-backed: it reads the same pinned descriptors `a4 install`
consumes. It never picks a "latest" AST on its own and never falls back when a
descriptor is incomplete — it fails instead. A refusal means the resource is not
installable. Do not work around it.

## Step 4: Generate a Typed SDK

Turn what you discovered into a client. This works against public resources without
owning a stack.

```
a4 install <stack-ref> --ts
a4 install <stack-ref> --rust
```

Both targets above generate typed program clients — instruction building, PDA resolution,
account readers, and program reads. A program SDK packaged on its own is TypeScript today:

```
a4 install program <program-ref> --ts
```

For an owner-private program, authenticate first and use the alias or stable ID
returned by `a4 program push --wait`:

```
a4 auth login
a4 install program my-program --ts
a4 install program upr_... --ts
```

Private programs are exact owner-scoped lookups and do not appear in
`a4 explore programs`. If a private alias matches a managed registry name, use
the stable `upr_...` ID.

Generated SDKs cover more than stream reads. They expose PDA derivation, account
resolution, instruction building, and transaction execution for the programs in scope.

## Step 5: Optional — Add the Stream MCP Server

Only do this if you want to discover registry resources and read live entity data
inside your own loop (exploration, debugging, answering questions about current chain
state). For code that ships, use the SDK from Step 4 instead.

```
npx -y @usearete/mcp
```

Add to your MCP client config:

```json
{
  "mcpServers": {
    "arete": {
      "command": "npx",
      "args": ["-y", "@usearete/mcp"]
    }
  }
}
```

Discovery tools (no auth needed): `explore_stacks`, `explore_stack`,
`explore_stack_schema`, `explore_programs`, `explore_program`, `resolve_artifact`.

Streaming tools: `ping`, `connect`, `disconnect`, `subscribe`, `unsubscribe`,
`query_entities`, `get_entity`, `list_entities`, `get_recent`, `list_subscriptions`,
`list_connections`.

Typical order: `explore_stacks` to pick a stack, `explore_stack_schema` to get the
exact view id, then `connect` -> `subscribe` -> `query_entities`.

Do not pass `api_key` in tool calls. Run `a4 auth login` once and let the server resolve
credentials from the credentials file or `ARETE_API_KEY`.

There is also a documentation MCP server at `https://docs.arete.run/mcp`
(`search_docs`, `fetch_page`) if you want doc search without scraping.

## Step 6: You Are Ready

You now have everything needed to build with Arete.

Key rules:
- ALWAYS run `a4 explore stack <stack-ref> --json` before writing any Arete code. Never guess descriptor identities, aliases, selected views, or Program Releases.
- Use `a4 explore <stack> <Entity> --json` to get exact field names, types, and view definitions.
- Run `a4 explore program <program-ref> --json` before generating code against a standalone program.
- Generate clients with `a4 install <ref> --ts|--rust` rather than hand-writing types.
- The primary public stack is `ore` (ORE mining data). Run `a4 explore stack ore --json` to inspect it.
- For React apps with generated stacks: install `@usearete/react @usearete/sdk zod` (not `zustand`) and generate from the exact local artifact with `a4 sdk create --manifest <path.stack-manifest.json> --ts`
- Hosted ORE browser clients require `VITE_ARETE_PUBLISHABLE_KEY` passed as `auth={{ publishableKey }}`. Read-only viewing requires authentication but not a wallet.
- For TypeScript apps: install `@usearete/sdk` and generate from the exact local artifact with `a4 sdk create --manifest <path.stack-manifest.json> --ts`
- To scaffold a new project quickly: `npx @usearete/a4 create my-app`

Full documentation: https://docs.arete.run
Platform onboarding (register, API key, capabilities): https://docs.arete.run/skill.md
Agent skills reference: https://docs.arete.run/agent-skills/overview/
Prompt cookbook: https://docs.arete.run/agent-skills/prompts/
