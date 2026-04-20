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
