# Signer policy vectors

Shared test vectors for the signing policy that decides whether a wallet
signs a prepared protocol step. Two implementations run them:

- the MCP server's opt-in local signer (`rust/arete-mcp/src/signer.rs`),
  which generates them;
- `@usearete/automations` (`typescript/automations`).

The cases are defined in `rust/arete-mcp/src/signer_vectors.rs`, and every
`expect` is what the Rust signer computes. A Rust test fails when these files
differ from that output. After changing a rule or adding a case, regenerate:

```bash
ARETE_WRITE_SIGNER_POLICY_VECTORS=1 cargo test -p arete-mcp --lib signer_vectors
```

Then run the TypeScript suite (`npm test` in `typescript/automations`). A
rule that must behave differently in one implementation is a change to the
policy, and is made in both.

## Files

| File | What it covers |
|---|---|
| `config.json` | Policy fields as JSON, and the normalized policy or an error. |
| `target.json` | The deployment and cluster check before preparing. |
| `decode.json` | Legacy, v0 and v1 wire transactions and what decoding yields. |
| `authority.json` | Instructions that hand control of the signer's funds to someone else, top-level and inner. |
| `summary.json` | The summary of a prepared step over a fixture chain. |
| `check.json` | The policy decision over that summary. |

Each file is `{ "version": 1, "description": "...", "cases": [{ "name", "input", "expect" }] }`.
A refusal is `{ "rule", "reason" }` and no refusal is `null`. Compare
`reason` exactly unless the case sets `"compareReason": false`; those cases
have no simulator, and the hint in the reason names each implementation's
own setup.

## The chain fixture

`summary.json` and `check.json` describe chain state instead of calling a
node. A case's `input.chain` is `null` (no simulator) or:

```json
{
  "slot": 100,
  "accounts": { "<address>": { "lamports": 1, "owner": "...", "data": ["<base64>", "base64"] } },
  "simulation": { "slot": 101, "err": null, "logs": [], "innerInstructions": [], "accounts": { "<address>": {} } }
}
```

A fixture simulator answers:

- `getMultipleAccounts([addresses, config])` with
  `{ "context": { "slot": chain.slot }, "value": [chain.accounts[address] or null, ...] }`;
- `simulateTransaction([transaction, config])`: when `simulation.rpcError`
  is set, a JSON-RPC error with that message; otherwise
  `{ "context": { "slot": simulation.slot }, "value": { "err", "logs", "accounts", "innerInstructions"? } }`,
  where `accounts` lists `simulation.accounts[address]` (or `null`) for each
  address in `config.accounts.addresses`, and `innerInstructions` is present
  only when the fixture has it.

Error text from the RPC is the implementation's own, so summaries replace
`signerEffect.error` with `"rpc_error"` before comparing.
