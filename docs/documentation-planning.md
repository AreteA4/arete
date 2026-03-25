# Documentation Planning: Information Audit

> **Created**: January 6, 2026  
> **Purpose**: Identify what information is available vs. missing before writing Phase 0 docs  
> **Status**: Ready for review

---

## Summary

Based on exploration of both **hyperstack-oss** and **hyperstack-platform** repositories, this document catalogs what's ready to document and what gaps need to be filled before writing the Phase 0 documentation.

---

## Information FOUND (Ready to Document)

| Area | Status | Source Files |
|------|--------|--------------|
| **TypeScript Stack API** | Complete | `typescript/src/stack.ts`, `view-factory.ts`, `types.ts` |
| **CLI Commands & Config** | Complete | `cli/README.md`, `cli/src/commands/*.rs` |
| **Legacy Hooks → Stack API Migration** | Complete | `typescript/src/hooks.ts` (has deprecation warning!) |
| **HyperstackProvider setup** | Complete | `typescript/src/provider.tsx` |
| **Example Stack definitions** | Complete | `hyperflip-stack.ts`, `pumpfun-stack.ts` |
| **Spec macro attributes** | Complete | `spec-macros/src/lib.rs`, `parse/attributes.rs` |
| **Population strategies** | Complete | `ast/types.rs` - SetOnce, LastWrite, Append, Max, Min, Sum, Count, UniqueCount |
| **View modes** | Complete | `state` and `list` modes documented |

---

## Information MISSING or INCOMPLETE

| Area | Issue | What's Needed |
|------|-------|---------------|
| **Real example specs** | flip-atom, pumpfun-atom, ore-atom specs **not in OSS repo** | Access to private repos or create synthetic examples |
| **WebSocket URL format** | Only examples are `ws://localhost:8080` and `wss://mainnet.hyperstack.xyz` | Confirm production URL pattern (e.g., `{spec-name}.stack.hypertek.app`) |
| **Auth flow details** | CLI has auth commands, but server-side flow unclear | Document API key retrieval, where credentials stored |
| **Deployment URL pattern** | `--branch` creates `{spec-name}-{branch}.stack.hypertek.app` | Confirm this is accurate |
| **Error codes** | No centralized error code list found | Compile from source or document as encountered |
| **Pricing/quotas** | Mentioned in business docs but not technical docs | Link to pricing page or document limits |

---

## Critical Gaps for Phase 0

To write the **5-minute quickstart**, decisions needed:

### 1. Which program to use as the quickstart example?

| Option | Pros | Cons |
|--------|------|------|
| **A: Real deployed program** | Users can actually interact with it | More complex setup |
| **B: Synthetic example** | Simple and illustrative | Not "real" |
| **C: Use pumpfun** | Stack definition already exists | May be too complex for quickstart |

**Recommendation**: Create a minimal "hello world" spec that tracks a simple, well-known program.

### 2. Where do users get their WebSocket URL?

After `hs deploy`, what URL do they use in their React app?

- Is it `wss://{spec-name}.stack.hypertek.app`?
- Or do they get it from `hs deploy info <build-id>`?
- What's the exact output format?

### 3. What's the simplest end-to-end flow?

```bash
cargo new my-spec && cd my-spec
# Add hyperstack to Cargo.toml
# Define spec in src/lib.rs
cargo build  # Generates .hyperstack/*.ast.json
hs config init
hs auth login
hs up my-spec
# Now use in React...
```

**Need to verify**: Is this the actual flow? Any missing steps?

### 4. Do users need their own Anchor IDL?

- For quickstart, can they use a pre-existing IDL?
- Or must they compile their own program first?
- Can we provide a "starter" IDL for learning?

---

## Package Naming Confusion

The README and code show inconsistent package names:

| Location | Package Name |
|----------|--------------|
| OSS README | `@hyperstack/react` |
| typescript/package.json | `@hypertek/typescript` |
| Provider code | References both |

**Action needed**: Confirm canonical npm package name before documenting.

---

## Pre-Documentation Actions

| Action | Priority | Owner | Notes |
|--------|----------|-------|-------|
| Create "hello world" spec | High | TBD | Simpler than flip/pumpfun for quickstart |
| Deploy to staging | High | TBD | Get real WebSocket URL to document |
| Record exact CLI flow | High | TBD | From `cargo new` to seeing data in React |
| Confirm npm package name | High | TBD | `@hypertek/typescript` vs `@hyperstack/react` |
| Confirm mainnet WebSocket URL | Medium | TBD | Is `wss://mainnet.hyperstack.xyz` correct? |
| Compile error code list | Low | TBD | Can be added post-launch |

---

## Technical Details Gathered

### TypeScript Stack API

**defineStack function**:
```typescript
defineStack({
  name: string;           // Required: stack identifier
  views: TViews;          // Required: view definitions
  transactions?: TTxs;    // Optional: transaction builders
  helpers?: THelpers;     // Optional: utility functions
})
```

**useHyperstack hook**:
```typescript
const { views, tx, helpers, store, runtime } = useHyperstack(stack);

// Access views
const { data, isLoading, error, refresh } = views.games.state.use({ gameId: '123' });
const { data: list } = views.games.list.use({ limit: 10 });
```

**View factories**:
```typescript
createStateView<T>(viewPath: string, options?: { transform?: (data: any) => T })
createListView<T>(viewPath: string, options?: { transform?: (data: any) => T })
```

### CLI Commands (Complete Reference)

#### Top-Level Commands
- `hs init` - Initialize new Hyperstack project
- `hs up [spec-name]` - Deploy spec (push + build + watch)
  - `--branch <branch>` - Deploy to specific branch
  - `--preview` - Create preview deployment
- `hs status` - Show overview of specs, builds, deployments
- `hs push [spec-name]` - Push specs to remote
- `hs logs [build-id]` - View build logs

#### Auth Commands
- `hs auth register` - Register new account
- `hs auth login` - Login to existing account
- `hs auth logout` - Remove stored credentials
- `hs auth status` - Check auth status (local)
- `hs auth whoami` - Verify with server

#### Spec Commands
- `hs spec push [spec-name]` - Push specs with AST
- `hs spec pull` - Pull remote specs
- `hs spec list` - List remote specs
- `hs spec versions <name>` - Show version history
- `hs spec show <name>` - Show spec details
- `hs spec delete <name>` - Delete spec

#### Build Commands
- `hs build create <spec-name>` - Create build
- `hs build list` - List builds
- `hs build status <id>` - Get build status
- `hs build logs <id>` - View build logs

#### Deploy Commands
- `hs deploy info <build-id>` - Show deployment info
- `hs deploy list` - List deployments
- `hs deploy stop <id>` - Stop deployment
- `hs deploy rollback <spec>` - Rollback deployment

#### SDK Commands
- `hs sdk create typescript <spec>` - Generate TS SDK
- `hs sdk list` - List available specs

### Spec Macros Reference

**Main macro**:
```rust
#[stream_spec(idl = "path/to/idl.json")]
// or
#[stream_spec(proto = ["file.proto"])]
```

**Entity definition**:
```rust
#[entity(name = "MyEntity")]
pub struct MyEntity { ... }
```

**Field mapping macros**:
- `#[map(path, strategy = Strategy)]` - Map from account fields
- `#[map_instruction(...)]` - Map from instruction fields
- `#[event(from = Type, fields = [...])]` - Capture events
- `#[capture(from = Type)]` - Capture entire accounts
- `#[aggregate(from = [...], strategy = Sum)]` - Aggregations
- `#[computed(expression)]` - Computed fields
- `#[track_from(from = [...], field = "name")]` - Track values

**Population strategies**:
- `SetOnce` - Set once, ignore updates
- `LastWrite` - Overwrite with latest (default)
- `Append` - Append to array
- `Max` - Track maximum
- `Min` - Track minimum
- `Sum` - Sum values
- `Count` - Count occurrences
- `UniqueCount` - Count unique values

### Configuration File Format

```toml
[project]
name = "my-project"

[sdk]
output_dir = "./generated"
typescript_package = "@myorg/package"

[build]
watch_by_default = true

[[specs]]
name = "my-spec"
ast = "MyEntity"
description = "Optional description"
```

---

## Docs Directory Structure

Target structure for documentation files:

```
docs/
├── getting-started/
│   ├── quickstart-react.mdx       # Phase 0 - 5 min tutorial
│   ├── installation.mdx           # Phase 0 - CLI + SDK setup
│   └── your-first-stack.mdx       # Phase 0 - 15 min tutorial
├── concepts/
│   ├── overview.mdx               # Phase 0 - Mental model
│   └── stack-api.mdx              # Phase 0 - Stack API deep dive
├── cli/
│   └── commands.mdx               # Phase 0 - Full CLI reference
├── guides/
│   ├── defining-specs/
│   ├── deployment/
│   └── patterns/
├── sdks/
│   ├── typescript/
│   ├── rust/
│   └── python/
├── spec-dsl/
│   ├── macros.mdx
│   ├── strategies.mdx
│   └── examples/
├── self-hosting/
└── why-hyperstack/
```

---

## Next Steps

1. [ ] Resolve package naming (`@hypertek/typescript` vs `@hyperstack/react`)
2. [ ] Create minimal quickstart example spec
3. [ ] Deploy example and capture real WebSocket URL
4. [ ] Record complete CLI flow with actual outputs
5. [ ] Begin writing Phase 0 docs with gathered information
6. [ ] Mark placeholders for missing information

---

## Appendix: Source Files Explored

### hyperstack-oss Repository

| File | Content |
|------|---------|
| `typescript/src/stack.ts` | defineStack, useHyperstack |
| `typescript/src/view-factory.ts` | createStateView, createListView |
| `typescript/src/types.ts` | All TypeScript interfaces |
| `typescript/src/provider.tsx` | HyperstackProvider |
| `typescript/src/hooks.ts` | Legacy hooks (deprecated) |
| `typescript/src/view-hooks.ts` | New view hook implementations |
| `typescript/src/hyperflip-stack.ts` | Example stack definition |
| `typescript/src/pumpfun-stack.ts` | Example stack definition |
| `cli/README.md` | Full CLI documentation |
| `cli/src/commands/*.rs` | Command implementations |
| `spec-macros/src/lib.rs` | Macro definitions |
| `spec-macros/README.md` | Macro overview |
| `hyperstack/README.md` | Umbrella crate docs |
| `README.md` | Main repo README |

### hyperstack-platform Repository

| File | Content |
|------|---------|
| Root directory | Project structure overview |
| `oss/` | OSS packages directory |
