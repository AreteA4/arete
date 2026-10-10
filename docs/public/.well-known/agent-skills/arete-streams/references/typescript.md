# TypeScript View Clients

Use this reference for Node.js, browser, Deno, Bun, Vue, Svelte, and other non-React TypeScript applications.

## Dependencies and Connection

Generated TypeScript bindings import `@usearete/sdk` and Zod schemas. Install the packages actually imported by the generated output, normally:

```bash
npm install @usearete/sdk zod
```

In a directory with no `package.json`, `a4 install stack <stack-ref> --ts --setup` creates an ES module `package.json` and `tsconfig.json` and installs these plus `typescript`, `tsx`, and `@types/node`.

Prefer a session when the application uses multiple stacks, standalone programs, shared chain reads, or shared execution:

```ts
import { createSession } from '@usearete/sdk';
import { MY_STACK } from './generated/my-stack';

// No auth option: server-side, the SDK uses ARETE_API_KEY if set, else your a4 CLI login.
const session = await createSession({ stacks: { app: MY_STACK } });

const current = await session.stacks.app.views.Position.state.get({ owner });

for await (const position of session.stacks.app.views.Position.list.use({ take: 10 })) {
  console.log(position);
  break;
}

session.close();
```

Pick the key by where the code runs:

- Node.js, Bun, Deno, workers, SSR, agents, and scripts: no `auth` option. The SDK uses `ARETE_API_KEY` if set, and otherwise the agent or secret key from the active `a4` login. The login fallback needs `@usearete/sdk` 0.34.0 or later on Node.js 20.16+ or 22.3+, Bun, or Deno; on older SDKs or Node.js versions, set `ARETE_API_KEY`. If several `a4` profiles hold a key, set `ARETE_PROFILE`. To pass a key explicitly, use `auth: { secretKey }` with an agent key (`a4_ak_...`) or secret key (`a4_sk_...`) read from the environment.
- Browser code (Vue, Svelte, plain pages): `auth: { publishableKey }` with an origin-bound publishable key (`a4_pk_...`), for example from `import.meta.env.VITE_ARETE_PUBLISHABLE_KEY`. Create one with `a4 auth keys create-publishable --origin <scheme://host[:port]>`.

`secretKey` throws in a browser, as does a secret or agent key passed as `publishableKey`. A publishable key passed as `secretKey` is refused everywhere.

Use `Arete.connect(MY_STACK, options)` for one direct stack client. The generated stack contains default endpoints; override `url` or `httpUrl` only for an intentional local or alternate binding.

## Stack Reads for Derived State

For derived "current X" state, call the stack's reads first. A connected stack client exposes the stack extension's reads under `read`, and they combine views, program accounts, and chain state into one value. ORE's `read.currentRound()`, for example, returns the current board and round with the round's phase:

```ts
// The ORE stack inserted into the session as `ore`
const current = await session.stacks.ore.read.currentRound();
```

Take the read names, arguments, and result shapes from the generated stack. Each call is one read, not a subscription; call it again after the views it depends on change. A stack without a stack extension has no reads, and a composed stack does not carry its source stack's reads.

## Type Rules

- Generated field and argument names are camelCase.
- `u64`, `u128`, `i64`, and `i128` values are `bigint`.
- State keys use the generated object shape, even for a single key field.
- Prefer generated entity, key, and schema exports over parallel hand-written interfaces.

## Streaming

Use `.use()` for merged values, `.watch()` for raw operations, and `.watchRich()` for diffs. Bound scripts with a condition, timeout, or abort signal, then close the session/client.

Use server query options exposed by the generated method instead of downloading the full view and filtering locally. Let TypeScript reveal the exact option shape for the installed version.

## One-Shot Reads

`get()` opens (or reuses) the equivalent subscription, waits for its initial snapshot, and releases it. List views also have `getOne()`, a `take: 1` read that resolves `null` for an empty list. Both reject with `InitialDataTimeoutError` after `timeoutMs` (5000 by default; `null` waits forever). `getSync()` never subscribes: it reads an already active subscription and returns `undefined` when there is none.

With `@usearete/sdk` 0.25.0 or earlier, `get()` only reads an already active subscription, so a cold read resolves empty, and `getOne()` does not exist. On those versions, await the first value from `.use()` instead, or upgrade.

## Errors

Handle initial connection failure separately from later reconnecting state. Preserve structured socket and validation errors in diagnostics. Do not silently discard schema validation failures just because a loop produced no values.

Current reference: `https://docs.arete.run/sdks/typescript/`.
