# Python Program SDKs

Use the exact generated module and the Python runtime installation documented by that output.

```python
program = client.programs.my_program

vault, bump = program.pdas.vault.derive(owner=owner)
account = await program.accounts.vault.fetch(vault)

prepared = await program.transactions.deposit.prepare(
    owner=owner,
    amount={"ui": "10.5"},
)

# Execute only when signing and submission are authorized.
receipt = await client.execute(prepared)
```

Raw builders retain generated IDL names and are pure:

```python
instruction = program.raw.deposit.build(
    owner=owner,
    vault=vault,
    amount=10_500_000,
)
```

Generated code may also export standalone builder functions and typed dataclasses. Inspect the generated package rather than assuming its export layout.

Python rules:

- Large integers are native `int`.
- Program/account names use generated snake_case idiom.
- Unknown raw arguments and missing required values fail closed.
- Wallets implement the `arete.WalletAdapter` protocol.
- Use async context management or close the client/session explicitly.
- Preserve structured outcomes and never retry `submitted-unknown` automatically.

Current OSS reference: `https://github.com/AreteA4/arete/tree/main/python/arete-sdk`.
