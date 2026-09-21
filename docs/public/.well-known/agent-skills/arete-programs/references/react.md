# React Program Operations

Generated program operations exposed by `useArete` provide fluent `.useMutation()` hooks.

```tsx
const arete = useArete(MY_STACK);
const deposit = arete.programs.myProgram.transactions.deposit.useMutation();

async function submit() {
  await deposit.submit(
    { owner, amount: { ui: amount } },
    { reconcile: { refresh: [position] } },
  );
}
```

Call the hook unconditionally. Trigger `submit` only from an authorized user action. Render distinct phases such as preparing, awaiting wallet, submitted, confirmed, confirmed-unreconciled, and error using the exact result union exposed by the installed SDK.

`AreteProvider` must receive:

- the generated stack;
- origin-bound `auth={{ publishableKey }}` for hosted access;
- the application's wallet adapter for execution.

Read-only program account hooks do not require a wallet. A disconnected mutation should remain an error; do not bypass it with an untracked RPC send.

When reconciliation matters, refresh the specific view/read results affected by the operation. Confirmation plus a processed-slot watermark does not prove that every expected entity changed.

Enable the Arete fluent-hooks ESLint rule so nested `.useMutation()` calls remain subject to Hooks rules.

Current references: `https://docs.arete.run/sdks/react/` and `https://docs.arete.run/using-stacks/transactions/`.
