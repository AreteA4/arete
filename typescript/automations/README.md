# @usearete/automations

Server-side automation of Arete Protocol steps on a user's behalf: when a
step needs the user's signature, a backend prepares it, checks it against a
signing policy and the user's consent, signs it with a delegated signer, and
submits it. Arete never holds a key; the signer runs in your backend.

> **Status:** in development and not yet published. This first part is the
> signing policy. The runtime, signer adapters, consent and stores follow.

## Use the least powerful mechanism

Reach for delegated signing only when nothing simpler works:

| Situation | Mechanism |
|---|---|
| The program lets anyone advance it | A keeper step |
| The money is committed now | Deposit straight into the pot or escrow |
| A fixed recurring payment from a wallet the user keeps using | An on-chain allowance |
| The user's signature is needed each time, and the wallet is where their funds already live | Delegated signing under consent (this package) |

## The signing policy

Every prepared transaction is checked before it is signed. A refusal names
the rule:

```ts
import { checkPrepared, checkTarget, definePolicy, rpcSimulator, summarizePrepared } from '@usearete/automations';

const policy = definePolicy({
  deployments: [DEPLOYMENT_ID],
  clusters: ['mainnet'],              // mainnet only when listed
  programs: [VAULT_PROGRAM_ID],       // optional; runtime programs are always allowed
  maxSolPerTransaction: '0.01',       // SOL, wrapped SOL and closed token accounts included
  maxTokenAmounts: { [USDC]: '5000000000' }, // base units; a mint without a cap may not leave
});

checkTarget(policy, DEPLOYMENT_ID, 'mainnet');           // before preparing; null when allowed

const summary = await summarizePrepared(prepared, wallet, rpcSimulator(RPC_URL));
const refusal = checkPrepared(policy, { deploymentId: DEPLOYMENT_ID, prepared, summary, simulates: true });
if (refusal) throw new Error(`${refusal.rule}: ${refusal.reason}`);
```

- Outflows are measured by simulating the exact bytes, per transaction. A
  transaction whose effect on the signer cannot be simulated is refused.
- Instructions that hand control of the signer's funds to someone else
  (token approvals, authority changes, System assignments, nonce, stake and
  loader authority changes) are always refused, top-level and inner.
- The rules match the Arete MCP server's local signer. Both run the shared
  vectors in [`test-vectors/signer-policy/`](../../test-vectors/signer-policy/).

What a wallet provider can enforce on its side is narrower; see
[docs/privy-policy-expressiveness.md](docs/privy-policy-expressiveness.md).

## Development

```bash
npm install
npm test         # unit tests and the shared signer-policy vectors
npm run typecheck
npm run lint
npm run build
```
