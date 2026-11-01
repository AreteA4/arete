# Protocol design patterns

Common designs, each with a snippet from a complete protocol in
`evals/patterns/` of this repository. CI type-checks every one and runs
`arete-protocol check --remote` on it (`scripts/check-patterns.mjs`), so
they are what the server accepts, not only the local checker. Copy the
shape, then rename steps, states and guards for your protocol.

Every snippet uses only builtin programs (`builtins(...)`), so it needs no
`a4 install`. With an installed program the shapes are the same.

## An allow-list: `in`

The deployment sets the list; a guard refuses anything outside it. Declare
the list as an array resource with a `maxItems`, and set it per deployment
(`[protocol.resources] allowedMints = "mints.json"`, a JSON array).

<!-- snippet: evals/patterns/allow-list.protocol.ts -->
```ts
  resources: {
    allowedMints: {
      schema: { type: "array", items: { type: "string", format: "pubkey" }, maxItems: 8 },
      description: "The mints this deployment accepts.",
    },
  },
```

<!-- snippet: evals/patterns/allow-list.protocol.ts -->
```ts
      guards: {
        mint_not_allowed: "args.mint in resources.allowedMints",
        amount_zero: "args.amount > 0",
```

A rejection names the guard (`mint_not_allowed`). For a short fixed list,
`args.side in ["bid", "ask"]` needs no resource. Element types must match;
a string literal stands for a public key.

## One of several venues: literal indexes plus `in`

List the venues as an array of objects in a fixed order. A step that always
uses one venue indexes it with a literal; the field after the index is
typed from the item schema. A step where the caller picks takes the venue
as an argument and checks it against the listed entries with `in`.

<!-- snippet: evals/patterns/venues.protocol.ts -->
```ts
    venues: {
      schema: {
        type: "array",
        items: {
          type: "object",
          properties: { vault: { type: "string", format: "pubkey" } },
          required: ["vault"],
          additionalProperties: false,
        },
        maxItems: 2,
      },
```

<!-- snippet: evals/patterns/venues.protocol.ts -->
```ts
          accounts: { fundingAccount: "wallet", recipientAccount: "resources.venues[0].vault" },
```

<!-- snippet: evals/patterns/venues.protocol.ts -->
```ts
        unknown_venue: "args.vault in [resources.venues[0].vault, resources.venues[1].vault]",
```

An index computed from args (`resources.venues[args.i]`) is a check error:
the addresses a step can reach must be reviewable from the spec. An index at
or past `maxItems` is too; an index past the deployment's actual list is
`null`.

## Per-user accounts and PDAs

Each wallet that acts on an instance gets a participant record (the
`participant` machine), and its accounts are derived from the wallet, never
passed in: `ata(wallet, mint)` for token accounts, and
`pda("<alias>", "<pda name>", seeds...)` for a PDA the program's IDL names
(seeds such as `wallet` or `instance.accounts.<name>`). The generated module
of an installed program lists its PDAs.

<!-- snippet: evals/patterns/per-user.protocol.ts -->
```ts
        ix(splToken, "TransferChecked", {
          args: { amount: "args.amount", decimals: "params.decimals" },
          accounts: {
            source: "ata(wallet, params.mint)",
            mint: "params.mint",
            destination: "ata(instance.roles.recipient, params.mint)",
            owner: "wallet",
          },
        }),
```

Create an account another wallet needs with an idempotent instruction in
the step that first needs it (or `glue.atas: [{ owner, mint }]`):

<!-- snippet: evals/patterns/per-user.protocol.ts -->
```ts
        ix(associatedToken, "createIdempotent", {
          accounts: {
            fundingAddress: "wallet",
            associatedTokenAccountAddress: "ata(args.recipient, args.mint)",
            walletAddress: "args.recipient",
            tokenMintAddress: "args.mint",
```

A participant state that is terminal closes the wallet's participant steps;
a self-loop transition (`from: ["member"], to: "member"`) keeps a step
repeatable.

## State-only steps

A step whose template has no instructions sends only the attested memo. Its
transitions apply when that transaction confirms. Use it for pause, resume,
close or leave, instead of a placeholder transfer:

<!-- snippet: evals/patterns/state-only.protocol.ts -->
```ts
    pause: {
      scope: "instance",
      authority: { kind: "role", role: "owner" },
      guards: { not_owner: "wallet == instance.roles.owner" },
      transitions: [{ from: ["open"], to: "paused" }],
      build: [],
    },
```

A create step can be state-only too (it records the instance, its roles and
bound accounts). A Squads step can't: it proposes its instructions, so it
needs at least one.

## A PDA the catalog IDL doesn't name

`pda(...)` derives only PDAs the program's IDL names. When a catalog
program's IDL lacks one your protocol needs (a market authority, a vault
signer), the fix is a change to that program's catalog IDL: open an issue
or PR on [AreteA4/arete-artifacts](https://github.com/AreteA4/arete-artifacts)
naming the PDA, its seeds and where they come from (the program's code or
docs). Once the program is republished, `a4 install` the new version and
derive it with `pda(...)`.

Until then, take the address as an argument of the create step and bind it,
so later steps read it from the instance and the creator, not every caller,
supplies it:

<!-- snippet: evals/patterns/missing-pda.protocol.ts -->
```ts
      args: { mint: "pubkey", marketAuthority: "pubkey" },
      bind: {
        accounts: { creator: "wallet", marketAuthority: "args.marketAuthority" },
        roles: { owner: "wallet" },
      },
```

<!-- snippet: evals/patterns/missing-pda.protocol.ts -->
```ts
            delegate: "instance.accounts.marketAuthority",
```

Don't hand-derive it in a handler or an SDK wrapper. When the address is
the same for every instance of a deployment, a resource works too.

## Dev-only conveniences in an app

Fund buttons, dev-wallet pickers, "skip the deadline" controls and
`@usearete/automations/dev` belong to the dev environment only. Gate them on
the environment the protocol client targets:

```ts
export const isDev = client.environment === "dev";
```

```tsx
{isDev && <button onClick={() => fetch("/dev/v1/airdrop", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ pubkey, sol: 5 }) })}>Fund (dev)</button>}
```

`/dev/v1` is `a4 dev up`'s dev API, which accepts requests from its own
origin only: let the app's dev server proxy it (Vite's `server.proxy`, with
the `origin` header set to the dev API's), and never call it from a
production build.

- Gate on the client, never on `NODE_ENV` or a hostname: a production build
  can hold a dev client by mistake, and the gate must follow the client.
- On a server, the `@usearete/automations/dev` helpers throw for a
  production client, so a build that still calls one fails at startup.
  Keep the production signer, store and simulator beside them and choose by
  `isDev`.
- In a browser bundle, load dev-only modules with a dynamic `import()`
  inside the `isDev` branch. The bundler still emits them as a separate
  chunk (`isDev` is only known at run time), but a production client never
  loads or runs them; keep secrets and keys out of them regardless. The
  dev API itself (`/dev/v1`, beside `a4 dev up`) does not exist in
  production.

## Dev and production builds of one app

`a4 dev up` regenerates the protocol client (and the protocol stack's SDK)
for the dev environment's deployment, under `generated/`.
`a4 install --locked` restores the published ones that `arete.lock` pins,
which target mainnet. Before a production build or deploy:

```sh
a4 install --locked     # the published clients, exactly as arete.lock pins them
a4 install --check      # fails if generated/ differs from arete.lock
```

A production build must not contain:

- a client generated for a dev environment (a host adapter with
  `ARETE_CLUSTER=mainnet` refuses it at startup);
- dev wallet key files (`.arete/` is git-ignored; keep it out of images and
  bundles too);
- the dev API URL or an environment token;
- dev-only UI or `@usearete/automations/dev` reached outside an `isDev`
  branch.

Run `a4 install --locked && a4 install --check` in CI before building.
