# Contributing to Documentation

Thank you for your interest in improving the Arete documentation!

## Documentation Stack

The Arete documentation is built using:

- [Astro](https://astro.build/) - Web framework for content-driven websites
- [Starlight](https://starlight.astro.build/) - Documentation theme for Astro
- [MDX](https://mdxjs.com/) - Markdown for the component era

The documentation content is located in `docs/src/content/docs/`.

## Local Development

To run the documentation site locally:

1. Navigate to the `docs` directory:
   ```bash
   cd docs
   ```
2. Install dependencies:
   ```bash
   npm install
   ```
3. Start the development server:
   ```bash
   npm run dev
   ```
4. Open your browser and navigate to `http://localhost:4321`.

## Content Structure

The documentation is organized into the following categories:

| Category | Path | Description |
|----------|------|-------------|
| Getting Started | `getting-started/` | Installation, quickstart, and tutorials |
| Concepts | `concepts/` | Core architecture and background |
| Stacks | `stacks/` | Documentation for specific stack components |
| SDKs | `sdks/` | Language-specific SDK guides (Rust, TS, Python) |
| CLI | `cli/` | Command-line interface reference |
| a4-server | `a4-server/` | Running stacks with a4-server |

## Writing Guidelines

### Frontmatter

Every MDX file must start with a YAML frontmatter block containing at least a title and description:

```markdown
---
title: My New Page
description: A brief overview of what this page covers.
---
```

### Voice and audience

Many readers are not Solana engineers. They are people building with a coding
agent, and the agent does most of the typing. Write so they can follow along,
without losing the precision an engineer (or an agent) needs.

- **Say what the reader gets before how it works.** Open each page with one or
  two plain sentences on what it is for and who needs it.
- **Talk to the reader as "you".** Describe what "your agent" does for them.
  Rules addressed to agents ("the agent must not...") belong in
  `public/agent.md` and the skills. In human-facing pages, explain what the
  agent does and why it matters to the reader.
- **Explain a term the first time it appears on a page**, in a clause, not a
  detour: "the descriptor (its exact spec sheet)". The full list lives in the
  [glossary](/getting-started/what-is-arete/#quick-glossary). Add new terms
  there.
- **Give a concrete example before the abstraction.** "A leaderboard sorted by
  score" lands faster than "an application-shaped read model".
- **Prefer short sentences and verbs** over stacked nouns. "Release-pinned,
  typed account reads" becomes "reads the program's accounts as typed data,
  matched to the release you installed".
- **Rephrase, never remove.** Safety boundaries (signing, submitting,
  deploying), caveats, commands, flags, and identifiers stay. If a sentence is
  hard to read, rewrite it. Do not cut the fact.
- **Do not add facts while simplifying.** A friendlier sentence must still be
  true. If a gloss describes behavior ("the next install replaces your edits"),
  check it against the code or CLI first.
- **Signpost depth.** Mark advanced sections so newcomers know they can skip
  them, for example with a `:::note[Going deeper]` aside.
- **Leave copy-and-paste prompts and reference tables precise.** Prompts are
  read by agents. Add a plain sentence above them saying when to use each one.

Reference pages (CLI, SDKs, Rust DSL, self-hosting) can stay technical in the
body. They still need a plain opening.

### Formatting

- Use clear and concise language.
- Use headers (`##`, `###`) to create a logical structure.
- Include code blocks with appropriate language hints (e.g., ` ```rust `, ` ```typescript `).
- Use tables for structured data like API parameters or configuration options.

### Sidebar

The sidebar is automatically generated based on the file structure and frontmatter. You do not need to manually update a sidebar configuration file for most contributions.

## Linting & Formatting

We use Prettier to maintain a consistent style across all documentation files.

| Action | Command |
|--------|---------|
| Check for issues | `npm run lint` |
| Fix formatting | `npm run lint:fix` |

## General Workflow

For the general contribution workflow (forking, branching, pull requests, and conventional commits), please refer to the main [CONTRIBUTING.md](../CONTRIBUTING.md) in the root of the repository.
