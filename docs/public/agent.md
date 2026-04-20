# Arete Agent Setup

You are setting up Arete, a real-time Solana data streaming platform. Follow these steps in order.

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
npm install -g arete-cli
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
npx skills add usearete/skills --agent <your-agent>
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
npx skills add usearete/skills
```

This auto-detects which agents are present and installs to the correct location for each. You can also explicitly install for all detected agents:

```
npx skills add usearete/skills --all
```

This installs context files that teach you (the agent) how to use Arete's SDKs, CLI, and Rust DSL.

For the full `npx skills` documentation: https://github.com/vercel-labs/skills

Additional options:
- `npx skills add usearete/skills -a cursor -a opencode` targets multiple specific agents
- `npx skills add usearete/skills -g` installs globally (available across all projects)

## Step 3: Discover Available Data

```
a4 explore --json
```

This lists all available stacks you can stream data from. For each stack, get the full schema:

```
a4 explore <stack-name> --json
```

For field-level detail on a specific entity:

```
a4 explore <stack-name> <EntityName> --json
```

## Step 4: You Are Ready

You now have everything needed to build with Arete.

Key rules:
- ALWAYS run `a4 explore <stack> --json` before writing any Arete code. Never guess entity names, field paths, or types.
- Use `a4 explore <stack> <Entity> --json` to get exact field names, types, and view definitions.
- The primary public stack is `ore` (ORE mining data). Run `a4 explore ore --json` to see its entities.
- For React apps: install `arete-react` and `arete-stacks`
- For TypeScript apps: install `arete-typescript` and `arete-stacks`
- To scaffold a new project quickly: `npx arete-cli create my-app`

Full documentation: https://docs.arete.run
Agent skills reference: https://docs.arete.run/agent-skills/overview/
Prompt cookbook: https://docs.arete.run/agent-skills/prompts/