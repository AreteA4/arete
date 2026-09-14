# Updating the Ore IDL and example SDKs

The checked-in artifact closure, source extension pins, and all three example
SDKs must be updated together:

1. Edit `idl/ore.json`, then run
   `cargo build --locked --manifest-path stacks/ore/Cargo.toml` from the repo root.
   IDL files are tracked build inputs; an IDL-only edit rebuilds the artifacts.
2. Review the TypeScript and Rust extensions against the new account and
   instruction interfaces. Copy `.arete/OreStream.stack-manifest.json`'s
   `artifactHash` into `inputHash` in both source extension manifests:
   - `stacks/ore/extensions/extensions.json`
   - `examples/ore-rust/src/generated/ore/extensions.json`
3. Run `bash scripts/generate-example-sdks.sh`. It checks both pins before
   generating the React, TypeScript, and Rust SDKs. Fix extension source files,
   then rerun generation; do not patch generated TypeScript files by hand.
4. Run the example builds and tests, commit the complete result, then rerun the
   generation script. The tracked artifact and SDK directories must stay clean.

The TypeScript checkpoint helper consumes the raw `Round.totalReturnedSol`
account field. The stream's projected `totalWinnings` field retains its public
name; it is a separate interface.

The `buyback`/`bury` correction was checked against
[Ore a3177af](https://github.com/regolith-labs/ore/tree/a3177afacf52e1711c9bfc77e651613d314f7c48):
`api/src/instruction.rs` declares `Buyback = 13` and `Bury = 24`, and
`program/src/buyback.rs` takes 13 fixed accounts followed by swap route accounts
and raw swap instruction data. This is source verification, not a verification
of the deployed binary or of every instruction in this IDL. Updating a local
ProgramSpec also changes its identity; managed catalog/release alignment must
be handled when publishing it.
