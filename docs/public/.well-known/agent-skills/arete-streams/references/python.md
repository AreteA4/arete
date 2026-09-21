# Python View Clients

Generate a Python stack package or module only when the descriptor lists Python support:

```bash
a4 install stack <stack-ref> --python
```

Follow the generated package metadata for installation. If the Arete Python runtime is not published for the selected release, use the OSS source or workspace dependency documented by the project rather than inventing a PyPI version.

```python
import arete
from my_generated_stack import MY_STACK

async def read_positions(publishable_key: str, owner: str) -> None:
    async with await arete.Arete.connect(
        MY_STACK,
        auth=arete.AuthConfig(publishable_key=publishable_key),
    ) as client:
        current = await client.views.position.state.get(owner=owner)
        print(current)

        async for position in client.views.position.list.use(take=10):
            print(position)
            break
```

Inspect the generated module for exact snake_case names and key keyword arguments.

Python uses:

- `.use(**options)` for merged async iteration.
- `.watch(**options)` and `.watch_rich(**options)` for update streams.
- `await .get(...)`, `.get_sync(...)`, and `await .get_one(...)` for reads.
- native `int` for large integers.
- an unset state distinct from `None` and an empty list.

Use `async with` or close clients and sessions explicitly. Apply `asyncio.timeout`/`wait_for` or another bounded stopping condition in probes and tests.

Current OSS reference: `https://github.com/AreteA4/arete/tree/main/python/arete-sdk`.
