# TypeScript View Clients

Use this reference for Node.js, browser, Deno, Bun, Vue, Svelte, and other non-React TypeScript applications.

## Dependencies and Connection

Generated TypeScript bindings import `@usearete/sdk` and Zod schemas. Install the packages actually imported by the generated output, normally:

```bash
npm install @usearete/sdk zod
```

Prefer a session when the application uses multiple stacks, standalone programs, shared chain reads, or shared execution:

```ts
import { createSession } from '@usearete/sdk';
import { MY_STACK } from './generated/my-stack';

const session = await createSession(
  { stacks: { app: MY_STACK } },
  { auth: { publishableKey: process.env.ARETE_PUBLISHABLE_KEY! } },
);

const current = await session.stacks.app.views.Position.state.get({ owner });

for await (const position of session.stacks.app.views.Position.list.use({ take: 10 })) {
  console.log(position);
  break;
}

session.close();
```

Use `Arete.connect(MY_STACK, options)` for one direct stack client. The generated stack contains default endpoints; override `url` or `httpUrl` only for an intentional local or alternate binding.

## Type Rules

- Generated field and argument names are camelCase.
- `u64`, `u128`, `i64`, and `i128` values are `bigint`.
- State keys use the generated object shape, even for a single key field.
- Prefer generated entity, key, and schema exports over parallel hand-written interfaces.

## Streaming

Use `.use()` for merged values, `.watch()` for raw operations, and `.watchRich()` for diffs. Bound scripts with a condition, timeout, or abort signal, then close the session/client.

Use server query options exposed by the generated method instead of downloading the full view and filtering locally. Let TypeScript reveal the exact option shape for the installed version.

## Errors

Handle initial connection failure separately from later reconnecting state. Preserve structured socket and validation errors in diagnostics. Do not silently discard schema validation failures just because a loop produced no values.

Current reference: `https://docs.arete.run/sdks/typescript/`.
