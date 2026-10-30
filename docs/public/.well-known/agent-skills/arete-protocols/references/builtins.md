# Builtin programs

`builtins(...)` adds runtime programs that every protocol may use without
installing them. They have no program spec, so:

| Builtin | Program | Registry package | What uses it |
|---|---|---|---|
| `system` | `11111111111111111111111111111111` | `system-program` | glue (account creation inside ATA creation) |
| `splToken` | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | `spl-token` | glue `wrapSol` / `unwrapSol` |
| `associatedToken` | `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | `spl-ata` | glue `atas`, and `ata(owner, mint)` addresses |
| `computeBudget` | `ComputeBudget111111111111111111111111111111` | | compute limits the runtime adds |
| `memo` | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | | the attested memo on every step transaction |

The same program has three names. The builtin is `associatedToken`, the
registry package is `spl-ata`, and the generated module is `splAta`. Errors
use the name of the alias you wrote.

## Templates cannot call builtins yet

A template instruction on a builtin alias (`ix(associatedToken, ...)`) has no
program spec to check against. The server rejects it with
`template_program_spec_missing`, and `arete-protocol check` warns about it.
Instead:

- **Token accounts:** use the step's glue. `glue.atas: [{ owner, mint }]`
  creates each associated token account idempotently before the step.
  `glue.wrapSol: { amount }` wraps SOL into the fee payer's wSOL account,
  and `glue.unwrapSol: true` closes it after.
- **Any other instruction:** install the registry package, for example
  `a4 install program spl-token --ts` (or `system-program`, `spl-ata`), and
  call it through that alias with `ix(...)`. Its program spec is pinned like any
  installed program, and the server checks the template against it.
- **Addresses only:** `ata(owner, mint)` derives an associated token
  account without any template.
