# What Privy's Solana policies can enforce

A delegated signer's provider policy is the hard limit if the backend that
runs automations is compromised: the provider enforces it in its enclave,
whatever the backend asks for. This note records which parts of the signing
policy Privy's Solana policy engine can express, so the Privy adapter can
generate a provider policy for those parts and say plainly which parts it
cannot.

Checked against Privy's policy documentation on 2026-10-22:

- [Policies overview](https://docs.privy.io/controls/policies/overview)
- [Solana example policies](https://docs.privy.io/controls/policies/example-policies/solana)

## How Privy evaluates a Solana transaction

- Every top-level instruction is evaluated against the policy's rules, and
  every instruction must be allowed. One denied instruction rejects the
  transaction. A request no rule matches is denied.
- Conditions read fields from a `field_source`:
  - `solana_program_instruction`: `programId`;
  - `solana_system_program_instruction`: `instructionName`, `Transfer.from`,
    `Transfer.to`, `Transfer.lamports`;
  - `solana_token_program_instruction`: `instructionName` and the accounts and
    amounts of `Transfer`, `TransferChecked` (including `mint`), `Burn`,
    `MintTo`, `CloseAccount` and `InitializeAccount3` only;
  - `solana_instruction_data`: arguments and accounts of a custom program's
    instruction, decoded with an Anchor (0.30+) IDL of up to 32 KB;
  - `system`: `current_unix_timestamp`.
- Operators: `eq`, `neq`, `lt`, `lte`, `gt`, `gte`, `in` (up to 100 values),
  `in_condition_set`, `contains`, `starts_with`, `ends_with`.
- Rules are ALLOW or DENY. A broad ALLOW (for example every System
  instruction) overrides a narrower capped ALLOW for the same instruction.
- Conditions on an address loaded from an address lookup table fail, and the
  transaction is rejected. Conditions on program ids, instruction names,
  amounts and time still work.

## The signing policy, rule by rule

| Rule | Privy can express it? | How, or what is missing |
|---|---|---|
| `deployments` | No | Privy has no notion of a deployment. The runtime enforces it. |
| `clusters` | No | Privy signs bytes; the cluster is not in the request. The runtime enforces it. |
| `programs` allow-list | Yes, top-level only | ALLOW `solana_program_instruction.programId in [...]`. Leave the System and Token programs out of this rule, or it overrides their capped rules. Programs reached by CPI are not seen. |
| Authority refusals (`approve`, `setAuthority`, System `assign`, nonce and stake authority changes, loader instructions) | Partly | By default deny: allow only the System and Token instruction names the step needs, and do not allow the loader or stake programs. Inner instructions are not seen, and Token-2022 is not mentioned, so a protocol program's CPI that approves or reassigns is not caught. |
| `maxSolPerTransaction` | Partly | A cap on each top-level System `Transfer.lamports`. It is per instruction, not per transaction, and does not see lamports a protocol program moves by CPI, wrapped SOL spent, or token accounts closed to someone else. |
| `maxTokenAmounts` | Partly | A cap on `TransferChecked.amount` with `TransferChecked.mint`, per instruction. `Transfer` has no mint field. Tokens a protocol program moves by CPI are not seen. |
| "Refuse what cannot be simulated" | No | Conditions cannot read simulated balance changes. |
| Consent `perExecution` limits | Partly | As the caps above, when the step's own instruction moves the funds; through `solana_instruction_data` when the protocol program has an Anchor IDL whose arguments carry the amount. |
| Consent `perWindow` limits | No | Cumulative limits over time are only supported for Ethereum methods. |
| Consent expiry | Yes | ALLOW only while `system.current_unix_timestamp` is before the grant's expiry. |
| Exact bytes | Not a policy | The runtime's `messageSha256` check covers it. The adapter must use transaction signing without broadcast (`signTransaction`), not `signAndSendTransaction`, so the runtime submits through Arete. |

## Instructions Privy cannot allow

Privy's token rules cover six instructions, and the generated policy allows
only System `Transfer` and Token `TransferChecked`, `CloseAccount` and
`InitializeAccount3` at the top level. A step with any other top-level
System or Token instruction, such as creating an account or `syncNative` to
wrap SOL, is refused by Privy's default deny. `privySigner` refuses such a
step itself, before asking Privy, with a reason naming the instruction
(`refuseUnsupported: false` turns this off if your own Privy policy allows
more). Instructions a protocol program makes by CPI are not affected.

The `in` operator takes at most 100 values, so `privyPolicyFor` splits a
long program allow-list across several rules.

## What this means

- Protocol steps move funds through the protocol's own program, by CPI.
  Privy cannot bound those amounts unless the program publishes an Anchor
  IDL whose instruction arguments carry them. For most steps, the provider
  policy can bound **which programs** the wallet may call and **when** it may
  sign, but not **how much** moves.
- So for a compromised backend, the real bounds are the program allow-list,
  the time bound, and the protocol's on-chain rules. The threat model must say
  this, and the consent UX must not suggest that consent limits are enforced
  by the wallet provider.
- `privyPolicyFor` is still worth shipping: it can generate the program
  allow-list, the System and Token instruction-name allow-lists with caps on
  direct transfers, the expiry, and a denial of key export. It must return
  the list of policy rules it could not express, so integrators see the gap
  instead of assuming full coverage.
