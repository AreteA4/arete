# Hyperstack Documentation Structure Plan

> **Created**: January 6, 2026  
> **Contributors**: Principal Engineer, Dev-Rel, Librarian Research  
> **Status**: Planning - Ready for Implementation

---

## Executive Summary

This document captures the comprehensive documentation planning for Hyperstack, synthesizing insights from codebase exploration, competitive research, and advisory input. The goal is to maximize developer experience and minimize time-to-first-value for Solana developers.

**Key Decisions:**
- Lead with **Stack API** (not legacy hooks)
- Target **5-minute quickstart** to deployed stack
- Host user docs in **hyperstack-oss** repo
- Use existing **Docusaurus** setup (or migrate to Fumadocs)

---

## Research Findings

### Codebase Exploration

#### hyperstack-platform Repository

**Location**: `/Users/adrian/code/defi/hypertek/hyper-stack-platform`

**Key Components Discovered:**
| Component | Location | Documentation Need |
|-----------|----------|-------------------|
| spec-macros | `docs/oss/packages/spec-macros/` | Procedural macros reference |
| interpreter | `docs/oss/packages/interpreter/` | Architecture docs (internal) |
| CLI | `docs/oss/packages/cli/` | Command reference |
| hyperstack-server | `docs/oss/packages/rust/hyperstack-server/` | WebSocket API reference |
| ast-compiler | `docs/oss/packages/ast-compiler/` | Internal contributor docs |
| Backend API | `docs/backend/api/` | REST API reference |
| Docusaurus | `docusaurus-repo/` | **Existing doc site** |

**Key Finding**: Docusaurus repo already exists - we should build on this, not start fresh.

#### hyperstack-oss Repository

**Location**: `/Users/adrian/code/defi/hypertek/hyperstack-oss`

**Package Structure:**
```
hyperstack-oss/main/
├── hyperstack/           # Umbrella Rust crate
├── interpreter/          # Transform/VM package
├── spec-macros/          # Declarative macros
├── cli/                  # CLI tool
├── typescript/           # TS SDK with React hooks
├── python/hyperstack-sdk/  # Python SDK
├── rust/hyperstack-sdk/  # Rust SDK
└── docs/                 # Local testing guide
```

**Existing READMEs**: Each package has a README with varying completeness. CLI docs are most comprehensive.

**Relationship to Platform**: OSS enables self-hosting for single-stream. Platform adds multi-stream, auth, K8s orchestration, managed WebSocket serving.

---

### Competitive Research

#### Documentation Frameworks in Blockchain/Infra Space

| Project | Framework | Strengths | Weaknesses |
|---------|-----------|-----------|------------|
| **Anchor** (Solana) | Fumadocs + Next.js | MDX, interactive, modern | Complex setup |
| **Foundry** (Ethereum) | mdBook | Rust-native, fast, searchable | Less interactive |
| **Convex** | Docusaurus | Multi-quickstart, React components | Heavier |
| **Helius** (Solana) | Custom | Progressive disclosure, AI search | Not OSS |

**Recommendation**: Stick with **Docusaurus** (already set up) or migrate to **Fumadocs** (Anchor uses it, more modern). Don't use mdBook for user-facing docs (better for internal/Rust reference).

#### Patterns from Best-in-Class Docs

**Convex Pattern - Multi-path Quickstarts:**
```
quickstarts/
├── react/
├── nextjs/
├── vue/
├── svelte/
└── vanilla-js/
```

**Helius Pattern - Progressive Disclosure:**
1. Simple API call (copy-paste, works)
2. Working application (10 min)
3. Advanced patterns (deep dive)

**Anchor Pattern - Browser-first:**
- Solana Playground for zero-setup experimentation
- Consider: Can Hyperstack offer browser demo?

---

### API Discovery: Stack API vs Legacy Hooks

**Critical Finding**: The TypeScript SDK has TWO APIs. Documentation must lead with the new one.

#### Legacy API (being deprecated)
```typescript
// DON'T teach this in docs
const { entity: game } = useHyperState({ 
  view: 'SettlementGame/list', 
  key: 'game123' 
});
```

#### New Stack API (recommended)
```typescript
// LEAD with this in all docs
const stack = defineStack({
  name: 'settlement-game',
  views: {
    games: {
      state: createStateView('SettlementGame/state'),
      list: createListView('SettlementGame/list')
    }
  }
});

const { views } = useHyperstack(stack);
const game = views.games.state.use({ gameId: 123 });
```

**Documentation Implication**: 
- All quickstarts use Stack API
- Create migration guide for legacy users
- Don't bury new developers in deprecated patterns

---

### Data Source Configuration

The spec macro supports two data source formats:

| Format | Use Case | Example |
|--------|----------|---------|
| `idl = "file.json"` | Anchor programs | `idl = "settlement_game.json"` |
| `proto = ["file.proto"]` | gRPC/Protobuf sources | `proto = ["yellowstone.proto"]` |

**Documentation Need**: "Data Sources" guide explaining when to use each.

---

### Real Example Specs Available

Three complete, production-quality specs exist and should be annotated as learning resources:

| Spec | Location | Complexity | Good For Teaching |
|------|----------|------------|-------------------|
| `flip-atom` | `flip-atom/src/spec.rs` | High | Full-featured example |
| `pumpfun-atom` | `pumpfun-atom/src/spec.rs` | Medium | Token tracking |
| `ore-atom` | `ore-atom/src/spec.rs` | Medium | Game state |

---

## Proposed Documentation Architecture

### Site Structure

```
docs.hyperstack.xyz/
│
├── 🚀 GETTING STARTED
│   ├── quickstart/
│   │   ├── react.mdx              # Primary path (5 min)
│   │   ├── nextjs.mdx             # Next.js specific
│   │   ├── nodejs.mdx             # Plain Node.js
│   │   └── rust.mdx               # Rust SDK
│   ├── installation.mdx           # CLI setup, authentication
│   └── your-first-stack.mdx       # 15-min tutorial with explanation
│
├── 📚 CONCEPTS
│   ├── overview.mdx               # What is Hyperstack, mental model
│   ├── declarative-specs.mdx      # The core paradigm
│   ├── stack-api.mdx              # defineStack, views, subscriptions
│   ├── data-strategies.mdx        # SetOnce, LastWrite, Append + decision tree
│   ├── idl-vs-proto.mdx           # When to use each data source
│   └── architecture.mdx           # Compiler → Bytecode → Interpreter → Stream
│
├── 📖 GUIDES
│   ├── defining-specs/
│   │   ├── entities.mdx           # Entity definition deep dive
│   │   ├── sections.mdx           # Section patterns
│   │   ├── computed-fields.mdx    # Derived data
│   │   └── relationships.mdx      # Cross-entity references
│   ├── deployment/
│   │   ├── local-development.mdx  # Dev workflow
│   │   ├── staging-production.mdx # Environment management
│   │   └── ci-cd.mdx              # Automation
│   └── patterns/
│       ├── real-time-dashboards.mdx
│       ├── token-tracking.mdx
│       └── game-state.mdx
│
├── 🛠 SDKs
│   ├── typescript/
│   │   ├── overview.mdx           # Package installation, setup
│   │   ├── react-hooks.mdx        # useHyperstack, views
│   │   ├── vanilla.mdx            # Non-React usage
│   │   └── migration.mdx          # Legacy hooks → Stack API
│   ├── rust/
│   │   ├── overview.mdx
│   │   └── examples.mdx
│   └── python/
│       ├── overview.mdx
│       └── async-usage.mdx
│
├── ⌨️ CLI REFERENCE
│   ├── commands.mdx               # All commands with examples
│   ├── configuration.mdx          # hyperstack.toml
│   └── troubleshooting.mdx        # Common issues
│
├── 📋 SPEC DSL REFERENCE
│   ├── macros.mdx                 # #[stream_spec], #[entity], #[map]
│   ├── strategies.mdx             # Full reference + decision tree
│   ├── types.mdx                  # Supported types, mappings
│   └── examples/
│       ├── flip-atom.mdx          # Annotated real spec
│       ├── pumpfun-atom.mdx       # Annotated real spec
│       └── ore-atom.mdx           # Annotated real spec
│
├── 🔧 SELF-HOSTING (OSS)
│   ├── overview.mdx               # When to self-host vs managed
│   ├── requirements.mdx           # Geyser gRPC, infrastructure
│   ├── single-stream-setup.mdx    # Step-by-step
│   └── limitations.mdx            # What managed service adds
│
├── 💡 WHY HYPERSTACK
│   ├── vs-raw-rpcs.mdx            # Side-by-side code comparison
│   ├── vs-helius.mdx              # "Raw data vs app-ready data"
│   └── case-studies.mdx           # Customer stories (future)
│
└── 🆘 SUPPORT
    ├── faq.mdx                    # Common questions
    ├── common-errors.mdx          # Error codes and fixes
    ├── discord.mdx                # Community link
    └── changelog.mdx              # Release notes
```

---

## Priority Phases

### Phase 0: Launch Blockers (Target: 1 week)

| Document | Est. Hours | Owner | Notes |
|----------|------------|-------|-------|
| `quickstart/react.mdx` | 4h | TBD | Must use Stack API |
| `installation.mdx` | 2h | TBD | CLI install, auth setup |
| `concepts/overview.mdx` | 3h | TBD | Mental model, value prop |
| `concepts/stack-api.mdx` | 4h | TBD | Core API explanation |
| `cli/commands.mdx` | 2h | TBD | Already well-documented |
| **Total** | **~15h** | | |

**Exit Criteria**: A developer can go from zero to deployed stack in under 15 minutes.

### Phase 1: First Week Post-Launch

| Document | Est. Hours | Owner | Notes |
|----------|------------|-------|-------|
| `spec-dsl/macros.mdx` | 6h | TBD | Core macro reference |
| `spec-dsl/strategies.mdx` | 4h | TBD | Include decision tree |
| `sdks/typescript/react-hooks.mdx` | 4h | TBD | Full hook reference |
| `sdks/typescript/migration.mdx` | 2h | TBD | Legacy → Stack API |
| `concepts/data-strategies.mdx` | 3h | TBD | When to use each |
| **Total** | **~19h** | | |

**Exit Criteria**: Developers can author basic specs and understand strategy selection.

### Phase 2: First Month

| Document | Est. Hours | Owner | Notes |
|----------|------------|-------|-------|
| `guides/defining-specs/*` | 12h | TBD | Deep dives |
| `spec-dsl/examples/*` | 8h | TBD | Annotated real specs |
| `why-hyperstack/*` | 6h | TBD | Differentiation content |
| `self-hosting/*` | 6h | TBD | OSS setup guides |
| `guides/patterns/*` | 8h | TBD | Real-world use cases |
| **Total** | **~40h** | | |

**Exit Criteria**: Full documentation coverage for core use cases.

### Phase 3: Ongoing

| Document | Trigger | Notes |
|----------|---------|-------|
| Rust SDK docs | SDK completion | Currently in progress |
| Python SDK docs | SDK completion | Async patterns |
| Case studies | Customer wins | Real usage stories |
| Advanced guides | User feedback | Based on support questions |

---

## Critical Content Pieces

### 1. Strategy Selection Decision Tree

This is **essential** to prevent wrong-strategy bugs. Include in `concepts/data-strategies.mdx` and `spec-dsl/strategies.mdx`.

```
┌─────────────────────────────────────────────────────────────┐
│                  Strategy Selection Guide                    │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Is this field set once and never changes?                  │
│  ├── YES → SetOnce                                          │
│  └── NO → Does it need to track the latest value?           │
│           ├── YES → LastWrite                               │
│           └── NO → Is it a numeric aggregation?             │
│                    ├── YES → Max / Min / Sum / Count        │
│                    └── NO → Is it a collection of events?   │
│                             ├── YES → Append                │
│                             └── NO → UniqueCount            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 2. One-Sentence Positioning

For `why-hyperstack/vs-helius.mdx`:

> "Helius gives you raw WebSocket data. Hyperstack gives you transformed, typed, app-ready data."

Then immediately show the code difference.

### 3. Annotated Example Spec

Turn `flip-atom/src/spec.rs` into an interactive learning resource with inline comments explaining each section, decision, and pattern.

---

## Technical Decisions

### Documentation Location

| Content Type | Location | Rationale |
|--------------|----------|-----------|
| User-facing docs | `hyperstack-oss` repo | What OSS users see first |
| Internal/contributor docs | `hyperstack-platform` repo | Architecture, decisions |
| Generated API docs | Both repos | rustdoc/typedoc output |

### Framework Choice

**Recommendation**: Continue with **Docusaurus** (already set up in `docusaurus-repo/`).

| Option | Pros | Cons | Verdict |
|--------|------|------|---------|
| Docusaurus | Already set up, React ecosystem, plugin system | Heavier than alternatives | **Use this** |
| Fumadocs | Modern, MDX-first, Anchor uses it | Migration effort | Consider later |
| mdBook | Fast, Rust-native | Less interactive | Internal docs only |

### API Documentation Generation

| SDK | Tool | Integration |
|-----|------|-------------|
| Rust | rustdoc | Publish to docs.rs, link from main docs |
| TypeScript | TypeDoc | Generate and embed in docs site |
| Python | Sphinx/pdoc | Generate and embed |

**Key Principle**: Single source of truth. API docs generated from code comments, not manually maintained.

### Versioning Strategy

**Initial**: Single version, no version selector.

**Later**: Add versioning when breaking changes require it. Docusaurus supports version dropdown.

---

## DX Best Practices to Follow

### Do This

| Practice | Example |
|----------|---------|
| **Show value before installation** | Lead with what Hyperstack does, then how to install |
| **5-minute time-to-value** | Quickstart ends with working deployed stack |
| **Real Solana addresses** | Use recognizable addresses (like Toly's) in examples |
| **Error handling in every example** | Include try/catch, show error states |
| **Copy button on all code blocks** | One click to clipboard |
| **Run in browser option** | Consider Solana Playground integration |

### Don't Do This

| Anti-Pattern | Why It's Bad | Instead |
|--------------|--------------|---------|
| Hub page with links | No time-to-value, user bounces | Linear 5-min path |
| Legacy API in examples | Teaches deprecated patterns | Stack API everywhere |
| Placeholder values | Feels fake, unclear | Real Solana addresses |
| Installation-first | User doesn't know if they care | Show value prop first |
| Missing error handling | Code fails in production | Always show error cases |
| "Coming soon" sections | Erodes trust | Don't add until ready |

---

## Competitive Positioning in Docs

### Where "Why Hyperstack" Content Lives

| Page | Content | Tone |
|------|---------|------|
| Homepage | One-liner value prop | Bold, confident |
| `why-hyperstack/overview.mdx` | Full comparison | Educational, factual |
| `why-hyperstack/vs-raw-rpcs.mdx` | Code comparison | Show, don't tell |
| `why-hyperstack/vs-helius.mdx` | Differentiation | Respectful, clear |

### Key Differentiators to Highlight

1. **Declarative vs Imperative**: Define what you want, not how to get it
2. **Type Safety End-to-End**: From chain to client, types flow through
3. **Real-time, Not Batch**: Live streaming, not historical queries
4. **App-layer, Not RPC-layer**: We understand your data model

---

## Support Integration

### Discord Connection

- Link prominently in docs sidebar
- "Ask in Discord" button on every page
- Channel for doc feedback (`#docs-feedback`)

### FAQ Strategy

- Start with top 10 support questions
- Update weekly based on Discord/support volume
- Each FAQ links to relevant doc section

### Error Documentation

Create `common-errors.mdx` with:
- Error code
- What it means
- How to fix
- Link to relevant concept doc

---

## Open Questions

1. **Docs repo location**: Confirm `hyperstack-oss` is correct home for user docs
2. **Framework migration**: Any reason to move from Docusaurus to Fumadocs?
3. **Browser playground**: Should we invest in Solana Playground integration?
4. **AI search**: Helius has this - is it worth adding?
5. **Internationalization**: Any plans for non-English docs?

---

## Next Steps

- [ ] Validate this structure with team
- [ ] Assign owners to Phase 0 documents
- [ ] Set target date for Phase 0 completion
- [ ] Create doc issues in Linear
- [ ] Begin writing quickstart

---

## Appendix: Research Sources

### Agents Consulted
- **Principal Engineer**: Technical documentation architecture, API decisions
- **Dev-Rel**: Developer experience, onboarding optimization
- **Librarian**: Competitive research, framework analysis

### Repositories Explored
- `hyperstack-platform`: Full platform codebase
- `hyperstack-oss`: Open source components

### External Research
- Anchor documentation (Fumadocs)
- Foundry documentation (mdBook)
- Convex documentation (Docusaurus)
- Helius documentation patterns
