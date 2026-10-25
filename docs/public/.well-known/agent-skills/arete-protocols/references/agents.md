# Agents acting on a protocol with a wallet

An agent can find what its wallet can do on a protocol, prepare a step, check
what the transactions do, sign them with the wallet it was given and submit
them, through the Arete MCP server (`a4 mcp`), the command line or a
protocol client. Arete never holds the agent's key. The MCP server signs only
when the user configures a signer for it.

## The flow

```text
protocol_actions  → what can this wallet do now?
protocol_prepare  → unsigned transactions + a summary
  check the summary against the user's intent
  sign every transaction with the wallet, outside MCP
protocol_submit   → submit the signed transactions, wait for the outcome
protocol_intent   → read the outcome later, if it had not settled
```

Every tool takes `deployment` (an id) or `protocol` (a package reference,
resolved to its install descriptor's deployment), or `dev: true` (or
`project`) for the running `a4 dev` session. The server uses the API key it
resolves itself (`ARETE_API_KEY` or the `a4 auth login` key); never put a key
in a tool argument.

## 1. What can this wallet do?

```bash
a4 protocol actions <protocol-or-deployment> --wallet <pubkey> --json
a4 protocol actions --dev --wallet <dev-wallet-name-or-pubkey>
```

MCP: `protocol_actions { protocol | deployment | dev, wallet, instance?, cursor? }`,
or `dev_actions { wallet, instance? }` in an `a4 dev` session (a dev wallet
name such as `creator` works there). Clients: `actions({ wallet })` in
TypeScript, Rust and Python, `useWalletActions` in React.

```json
{
  "createSteps": [
    { "step": "open", "scope": "create", "allowed": true, "argsDependent": ["amount_zero"],
      "args": [{ "name": "amount", "optional": false, "type": "string", "format": "u64" },
               { "name": "recipient", "optional": false, "type": "string", "format": "pubkey" }] }
  ],
  "instances": [
    { "instanceId": "…", "state": "open", "relation": ["participant"], "participantState": "chipped",
      "steps": [
        { "step": "close", "scope": "instance", "allowed": false, "reason": "not_recipient", "args": […] },
        { "step": "chip", "scope": "participant", "allowed": true, "argsDependent": ["amount_zero"], "args": […] },
        { "step": "leave", "scope": "participant", "allowed": true, "args": […] }
      ] }
  ],
  "nextCursor": null
}
```

- `createSteps` start a new instance; `instances` are the ones the wallet
  relates to (`participant`, `role:<name>`, `creator`), newest first. Page
  with `cursor: nextCursor`, or pass `instance` for one.
- More than one step can be allowed at once (chip in again or leave; close
  or start a new jar). When the user's intent does not pick one, ask; never
  choose a money-moving step for them.
- `reason` is the guard or access code, or `instance_terminal` /
  `participant_terminal` once the instance or the wallet's participant state
  is terminal (`detail: { state }`); explain it with `detail`.
- `args` is each argument's schema. Amounts and keys are strings with a
  `format` (`u64`, `pubkey`, `unixTimestamp`): pass `"1000000"`, not
  `1000000`.
- `argsDependent` guards are decided only by the args (and for create steps,
  by the instance they would make). `allowed: true` means every other guard
  passes now. Prepare is always the authority.

## 2. Prepare and check

`protocol_prepare { step, wallet, instance?, args, protocol | deployment | dev }`
returns `executionId`, `expiresAt`, the unsigned base64 `transactions`, and a
`summary` decoded from the transactions themselves:

- `programsInvoked` and per transaction `programs` (the step's `program`,
  `glue`, or `runtime` such as compute budget and memo),
- `accountsWritten`,
- `signerBalanceChanges` (SOL and per-mint tokens, simulated on the exact
  bytes; `available: false` means no RPC to simulate, never zero),
- `simulationErrors` with log tails,
- `signaturesStillRequired` (the wallet) and the signatures already present.

Check the summary against what the user asked for before signing: the
programs, the accounts written and the SOL and tokens leaving the wallet.
Stop and report when they do not match, when a simulation failed, or when
balance changes are unavailable and the user set a spending limit.

A guard or access rejection is a result, not an error:
`{ "state": "rejected", "rejected": "amount_zero", "guard": "args.amount > 0", … }`.
Explain it; do not retry with altered args unless the user agrees.

## 3. Sign outside the server, then submit

Sign every transaction with the wallet, keeping the signatures already
present and changing no other byte, then call
`protocol_submit { executionId, signedTransactions }` with them in the same
order before `expiresAt`. It refuses a transaction still missing a required
signature, submits, and waits up to 60 seconds (`timeoutSeconds` ≤ 120,
`wait: false` to return at once). The answer has `state` (`completed`,
`chain_failed`, `expired`, `rejected` or pending), `settled`, `instanceId`
and each transaction's signature. If it has not settled, read it later with
`protocol_intent { executionId }`. Never submit the same execution twice and
never resend after an unknown outcome.

## The optional local signer

Only when the user asks for it, the MCP server can sign with a keypair and a
policy. Without a signer `protocol_execute` is not listed. The config is a
`[signer]` table in `~/.arete/mcp.toml`, or in the TOML file
`ARETE_MCP_SIGNER_CONFIG` names:

```toml
[signer]
keypair = "~/.config/solana/agent.json"   # a Solana CLI keypair file
rpcUrl = "https://<your-rpc>"             # simulates balance changes

[signer.policy]
deployments = ["<deployment id>"]         # required allow-list
maxSolPerTransaction = 0.05               # required: SOL out of the signer per transaction
programs = ["<program id>"]               # optional allow-list
clusters = ["dev"]                        # optional; mainnet only when listed

[signer.policy.maxTokenAmounts]           # optional: base units out per mint; other mints refused
"<mint>" = "5000000"
```

`protocol_execute` takes `protocol_prepare`'s arguments without `wallet`,
prepares for the signer's wallet, checks the policy, then signs, submits and
waits. A refusal names the rule and nothing is signed:

```json
{ "state": "refused", "rule": "maxSolPerTransaction",
  "reason": "transaction 0 moves 0.50001 SOL out of the signer, over the 0.01 SOL cap" }
```

Rules: `deployments`, `clusters`, `programs`, `maxSolPerTransaction` (a
balance change that cannot be simulated is refused), `maxTokenAmounts`.
Report a refusal as it is; do not widen the policy to get past it.

- Write a signer config only on the user's request, with the narrowest
  policy that does the job: their deployment ids, a small SOL cap, and
  mainnet only if they ask for it by name.
- The keypair's secret never appears in tool output, errors or logs. Never
  read, print or copy the keypair file yourself.
- In an `a4 dev` session the signer's wallet gets fork SOL when it has none:
  try the policy there (`dev: true`, `clusters` including `dev`) before
  pointing it at a real deployment.

## Trying it locally

```bash
a4 dev up                                   # in the protocol project
a4 protocol actions --dev --wallet creator
a4 dev instance <id> --wallet recipient     # one instance's allowedSteps for a wallet
```

Then drive `dev_actions`, `protocol_prepare { dev: true, … }`,
`protocol_submit` and `protocol_intent` from the MCP client, signing with a
throwaway keypair airdropped fork SOL.
