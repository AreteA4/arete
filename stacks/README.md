# Arete Stacks

Protocol stack source projects and the generated SDK outputs they feed.

## Structure

```
stacks/
└── ore/                       # Stack definition (deployable)
    ├── Cargo.toml             # Rust crate with #[arete] macro
    ├── arete.toml             # CLI config for local SDK generation
    ├── src/spec.rs            # Stack definition
    └── idl/ore.json           # Anchor IDL

examples/
├── ore-react/src/generated/ore-stack.ts
├── ore-typescript/src/generated/ore-stack.ts
└── ore-rust/src/generated/ore/
```

The generated example SDKs are checked into the repo. Regenerate them with `./scripts/generate-example-sdks.sh`.

## Deploy a Stack

```bash
cd ore
cargo build                      # Generates .arete/*.ast.json
a4 up                            # Deploy to Arete
a4 sdk create typescript ore --output ../../my-app/src/generated/ore-stack.ts
a4 sdk create rust ore --output ../../my-app/src/generated/ore --module
```

## Use Generated Stacks

```typescript
import { ORE_STREAM_STACK } from './generated/ore-stack';
```

```rust
mod generated;

use generated::ore::OreStreamStack;
```
