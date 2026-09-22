# Read Models and Join Proof

Read this reference for multi-account entities, instruction aggregates, lookup indexes, or ambiguous keys.

## Start From Application Questions

A stack is not an IDL mirror. Group data only when the application needs to read it together and the chain provides a durable way to route updates to the same entity.

Prefer:

- a stable domain identifier as the primary key;
- sections that communicate provenance or lifecycle (`id`, `state`, `metrics`, `results`);
- nullable fields when hydration is genuinely asynchronous;
- explicit unsupported capabilities instead of synthetic history or guessed joins.

Avoid:

- one entity per account type without a consumer use case;
- keys chosen only because they are easy to map;
- floats for raw token amounts unless the product explicitly accepts precision loss;
- unbounded append history without a retention decision.

## Direct Route

A direct account mapping is strongest when the same account contains the primary key and the mapped field. Account updates can then resolve the entity without a secondary index.

## Instruction Route

For an instruction aggregate or derived field:

1. The `from` instruction must exist.
2. The `lookup_by` account name must exist on that instruction.
3. That account address must be the entity key or map through an established lookup index.
4. The update strategy must match the business meaning.

Example reasoning:

```text
OpenPosition.accounts.position
  -> Position account address
  -> Position.owner captured as primary key or registered lookup
  -> Position entity keyed by owner
```

If step two or three is unsupported by the IDL/instruction graph, the aggregate cannot be routed reliably.

## Lookup Index Route

Use `lookup_index(register_from = [...])` to register an address-to-entity-key relationship only when the referenced registration instruction exposes the required accounts and the lifecycle is correct.

Check creation, closure, reuse, and PDA derivation. A registration that happens after relevant updates or can become stale may produce silently missing or misrouted metrics.

## Review Checklist

- Can every field name its authoritative source?
- Can every update identify one entity key?
- Are instruction account names verified independently per instruction?
- Are optional fields and hydration timing represented honestly?
- Is each aggregate reversible or intentionally monotonic?
- Does a custom view sort/filter a stable populated field?
- Would the model remain correct through account closure, recreation, and program upgrade?
