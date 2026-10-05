---
name: arete-stack-authoring
description: Design and compile custom Arete stack artifacts from Solana program IDLs using the Rust DSL, or compose stacks from published live views and program SDKs. Use for app-facing read models, entity keys, cross-account join proof, mappings, aggregations, views, resolvers, stack composition, and ProgramSpec/LiveSpec/StackManifest generation. Do not deploy or mutate hosted resources; use arete-deploy for that.
metadata:
  version: "1.1.1"
  min-cli: ">=0.25.1"
---

# Author Arete Stack Artifacts

A good Arete stack is a small, app-facing read model. Do not begin by mirroring every IDL account. Start from what the application needs to read together, choose one canonical entity key, and prove every field and update route back to that key.

This skill ends with validated local artifacts. Hosted publication and deployment are separate, externally mutating work handled by `arete-deploy`.

## Compose From Published Parts First

A stack is a group of live views and program SDKs. When published stacks already serve the views the application needs, and program packages provide the program SDKs, compose a stack from them instead of authoring a new read model. Composition needs no Rust toolchain:

```bash
a4 stack compose --name ore-plus-token \
  --live ore \
  --program spl-token \
  --selected-view ore=OreRound/latest \
  --selected-view ore=OreMiner/state
```

The command resolves the parts, checks that they compose, and writes `[authoring.stacks.<name>]` in `arete.toml`. You can also write the entry yourself:

```toml
[authoring.stacks.ore-plus-token]
live.ore = { stack = "ore", version = "^1", views = ["OreRound/latest", "OreMiner/state"] }
programs = [{ package = "spl-token", version = "^1" }]
```

- `--live` takes a published stack (`ore`, `ore@^1`, `alias=stack:ore@^1`, or `stack:<stack>#<live alias>` for one of several) or a LiveSpec file (`alias=path`). `--program` takes a program package (`spl-token`, `spl-token@^1`) or a ProgramSpec file.
- The programs the views index come with them, as the program SDKs their stack includes. A `--program` for the same program replaces that SDK; any other `--program` adds an independent program.
- Without `--selected-view`, each alias selects every view its stack serves. With it, the `alias=view_id` values are the exact ordered allowlist.
- A registry part without a version is saved at `^<resolved version>`.
- `--install` also declares `[dependencies.stacks.<name>]` with `source = { workspace = "<name>" }` and installs it. Pass the SDK target with it (`--install --ts`, `--rust`, or `--python`); it is recorded as the dependency's `targets`, as `a4 install <package> --ts` records it. Without one, the dependency generates every target in `[sdk].targets` (TypeScript and Rust by default). `a4 install` pins every part in `arete.lock` and generates the composed stack's SDK, with each program SDK at `arete.programs.<program>` (take the key from the generated types).
- A composed stack does not carry a source stack's stack extension, such as its reads. The install output notes each one it leaves out.

A live alias taken from a stack with hosted delivery reads that stack's deployment, so the composed SDK connects without a new deployment. An alias from a definition-only stack or a LiveSpec file has no endpoints until `a4 up <name>` deploys the composed stack to the user's account and records the endpoints. `a4 up <name>` deploys exactly what `arete.lock` pins, and a production deployment lists the program SDK each program carries. Deploying is an external mutation: hand it to `arete-deploy` and require the user's authorization.

Publishing a composition so that other projects can install it by name is not available yet. To reuse a composition built from registry parts in another project, copy its `[authoring.stacks]` entry there.

Author a new LiveSpec only when no published stack serves the read model the application needs.

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
