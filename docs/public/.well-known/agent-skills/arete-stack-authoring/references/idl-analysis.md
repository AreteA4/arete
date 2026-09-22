# IDL Analysis

Read this reference when translating a program IDL into an Arete read model.

## Acquire the Correct IDL

Prefer a protocol-owned, deployment-matched IDL. Common sources are the program repository's `target/idl` or `idl` directory, a verified on-chain Anchor IDL, or a protocol SDK package. Record the program ID and source revision.

Do not silently combine an IDL from one deployment with a different program ID. Normalize it to a ProgramSpec before publication or composition.

## Survey

Use the CLI's current `a4 idl --help` surface and machine-readable output. A useful initial survey is:

```bash
a4 idl summary <idl> --json
a4 idl relations <idl> --json
a4 idl types <idl> --json
a4 idl events <idl> --json
```

`relations` helps classify app-facing entities, infrastructure accounts, and role accounts. It is a starting hypothesis, not a substitute for application requirements.

## Trace Each Required Field

For each proposed output field, record:

| Output | Source kind | IDL path | Update trigger | Entity-key route |
| --- | --- | --- | --- | --- |
| `state.value` | account | `Position::value` | account update | direct `owner` |
| `metrics.count` | instruction | `OpenPosition` | instruction | `accounts::position` |

Use targeted commands:

```bash
a4 idl search <idl> '<field or concept>' --json
a4 idl type <idl> <name> --json
a4 idl instruction <idl> <name> --json
a4 idl account-usage <idl> <account> --json
```

Confirm optionality, integer width, enum variants, nested types, signer/writable roles, and instruction account names exactly.

## Prove Relationships

```bash
a4 idl links <idl> <AccountA> <AccountB> --json
a4 idl connect <idl> <NewAccount> --existing <a,b> --suggest-a4 --json
```

Inspect the actual instructions in every proposed route. Similar account names on different instructions are not interchangeable.

If a relationship remains ambiguous, leave the field out, split the entity, or request missing protocol evidence. Do not invent a join because it would make the desired UI convenient.
