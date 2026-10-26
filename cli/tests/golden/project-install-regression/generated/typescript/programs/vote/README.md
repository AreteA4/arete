# `vote` TypeScript program SDK reference

Generated from program package `vote` 0.1.0, installed as `vote`. `a4 install` regenerates this folder, so do not edit it.
This file lists only what is specific to this SDK. For how to use any Arete SDK (`createSession`, wallets, preparing and executing operations), see the `arete-programs` agent skill or https://docs.arete.run/using-stacks/transactions/.
Look parts up with `a4 sdk describe vote [--view <Entity/view>] [--read <name>] [--program <key>] [--json]` or the MCP tool `describe_sdk`.

## Import

```ts
import { VOTE_PROGRAM } from './generated/typescript/programs/vote/vote.js'; // from the project root
```

The path is relative to the importing file: from `src/`, import `'../generated/typescript/programs/vote/vote.js'`. TypeScript resolves the `.js` to `vote.ts`.

Register it as `vote`: `createSession({ programs: { vote: VOTE_PROGRAM } })` gives `session.programs.vote`. Paths below start there.

Program `Vote111111111111111111111111111111111111111`.

## Accounts and instructions

- Raw instructions (`raw.<name>`): `ping`

