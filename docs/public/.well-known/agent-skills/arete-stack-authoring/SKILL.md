---
name: arete-stack-authoring
description: Design and compile custom Arete stack artifacts from Solana program IDLs using the Rust DSL. Use for app-facing read models, entity keys, cross-account join proof, mappings, aggregations, views, resolvers, and ProgramSpec/LiveSpec/StackManifest generation. Do not deploy or mutate hosted resources; use arete-deploy for that.
metadata:
  version: "1.0.0"
  min-cli: ">=0.13.0"
---

# Author Arete Stack Artifacts

A good Arete stack is a small, app-facing read model. Do not begin by mirroring every IDL account. Start from what the application needs to read together, choose one canonical entity key, and prove every field and update route back to that key.

This skill ends with validated local artifacts. Hosted publication and deployment are separate, externally mutating work handled by `arete-deploy`.

## Establish the Local Toolchain

When environment health is relevant, run `a4 doctor --json`. Stack authoring additionally needs a working Rust toolchain because the DSL is compiled by Rust macros.

Use the project's pinned toolchain and dependency policy. For a new crate, obtain the current compatible `arete` dependency through Cargo or current Arete documentation; do not copy a version from an old example.

## Define the Product Read Model

Before writing Rust, record:

- the application questions and UI/API outputs;
- entity candidates and one canonical key per entity;
- point-in-time fields versus accumulated metrics or event history;
- required state and list/custom views;
- field provenance: account field, instruction argument/account, event, resolver, or computation;
- retention/window expectations and any deliberately unsupported history.

If requirements are exploratory, propose a small first model and make uncertainty visible. Do not turn every account type into an entity by default.

## Inspect the IDL

Use machine-readable IDL analysis before writing macros:

```bash
a4 idl summary idl/<program>.json --json
a4 idl relations idl/<program>.json --json
a4 idl types idl/<program>.json --json
a4 idl events idl/<program>.json --json
```

Then drill into only the accounts, instructions, and relationships needed by the model:

```bash
a4 idl search idl/<program>.json '<intent>' --json
a4 idl type idl/<program>.json <AccountOrType> --json
a4 idl instruction idl/<program>.json <Instruction> --json
a4 idl account-usage idl/<program>.json <Account> --json
a4 idl links idl/<program>.json <AccountA> <AccountB> --json
a4 idl connect idl/<program>.json <NewAccount> --existing <a,b> --suggest-a4 --json
```

Run `a4 idl --help` and the relevant subcommand help if the local CLI surface differs. For a disciplined inspection sequence, read [references/idl-analysis.md](references/idl-analysis.md).

## Prove Joins Before Macros

For every mapping sourced from a different account or an instruction:

1. Identify the source account/instruction and exact IDL name.
2. Confirm that any `lookup_by` account is actually present on that instruction.
3. Prove how that address resolves to the entity's canonical key.
4. Add a `lookup_index(register_from = [...])` only when the registration instruction and PDA/account relationship support it.
5. Reject or redesign fields whose route cannot be proved.

A macro compiling does not prove a semantically correct join. Read [references/read-models-and-joins.md](references/read-models-and-joins.md) before implementing a multi-account entity.

## Write the DSL

An authored module uses `#[arete]`, one or more `#[entity]` structs, generated IDL SDK paths, nested `Stream` sections, and optional custom `#[view]` declarations.

```rust
use arete::prelude::*;

#[arete(idl = ["idl/program.json"])]
pub mod app_stack {
    use arete::macros::Stream;
    use serde::{Deserialize, Serialize};

    #[entity(name = "Position")]
    #[view(name = "largest", sort_by = "state.value", order = "desc")]
    pub struct Position {
        pub id: PositionId,
        pub state: PositionState,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PositionId {
        #[map(program_sdk::accounts::Position::owner, primary_key, strategy = SetOnce)]
        pub owner: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Stream)]
    pub struct PositionState {
        #[map(program_sdk::accounts::Position::value, strategy = LastWrite)]
        pub value: Option<u64>,
    }
}
```

The generated module name comes from the IDL metadata; inspect compiler output rather than assuming `program_sdk`. Add mappings in increasing complexity: direct fields, computed values, then proven cross-account/instruction routes.

For macro selection and authoring constraints, read [references/dsl.md](references/dsl.md).

## Compile in Tight Loops

Use the compiler as part of design:

```bash
cargo check
cargo build
```

After each coherent change, fix the first real compiler or macro diagnostic before layering more mappings. Do not suppress errors or replace typed paths with guessed strings.

Compilation emits content-addressed artifacts under `.arete/`. Inspect and validate the ProgramSpec, LiveSpec, and StackManifest closure before handing it to deployment:

```bash
a4 config validate
a4 sdk create --manifest .arete/<Stack>.stack-manifest.json --ts
```

Generate a local client as a contract test: the requested entities, views, programs, and field types should appear exactly as intended.

Read [references/artifacts.md](references/artifacts.md) for artifact roles, composition, and local validation.

## Completion Criteria

- Each entity answers an explicit application need.
- Every entity has one canonical primary key.
- Every cross-account update route is documented and proved against the IDL.
- `cargo check` and `cargo build` pass.
- The artifact closure is complete and hashes are stable on a clean rebuild.
- A generated client exposes the intended selected views.
- No hosted publish, deploy, stop, or delete action was taken as part of authoring.
