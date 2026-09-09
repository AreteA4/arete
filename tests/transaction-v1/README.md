# Transaction V1 critical-path integration (A4-256)

This is the reduced scope for PR #204. It replaces the earlier acceptance plan.
Removed requirements are cancelled here, not deferred release gates or automatic
follow-up tickets.

## Deterministic integration

There is one generated-client test in each existing adapter/SDK suite:

- TypeScript: `typescript/adapters/kit/src/generated-client.test.ts`.
- Rust: `rust/arete-a4-sdk/tests/generated_solana.rs`.
- Python: `python/arete-sdk/tests/test_generated_solders.py`.

Each uses the checked-in generated ORE log instruction, the real public adapter,
a recording relay transport, and the upstream transaction decoder. It asserts
V1, the ORE instruction/accounts and the chosen inline resource config. Adapter
edge-case matrices remain in the existing focused adapter suites.

Default CI runs these fast checks. Missing adapters are failures, never skips:

```sh
# Use the normal core build and adapter dependency installation first.
npm test --prefix typescript/adapters/kit
cargo test --locked -p arete-a4-sdk --features solana-adapter --test generated_solana
cd python/arete-sdk
python -m pip install -e '.[dev,solana]'
python -m pytest tests/test_generated_solders.py
```

CI installs the optional Solana extra and runs the generated-client test in the
existing Python suite. Running that test without its adapter is a collection failure.

## One local TypeScript smoke

Prerequisites: an **already-running** local validator supporting V1 and its
4096-byte limit, a compatible Yellowstone/Geyser service, an Arete transaction
relay pointing at that validator, and the ingestion fixture below. Provide a
funded disposable local keypair (at least 0.04 SOL plus fees). The smoke sends
32 transfers of 0.001 SOL in V1 and one equivalent v0 transfer to fresh recipients.
It does not download, build or start a validator, provision a toolchain, deploy
an on-chain program, or start the relay/runtime.

The existing ORE stack requires application state/setup, so the tiny fixture uses
the built-in System program. Its IDL describes transfer plus an empty System
wallet account. The fixture runs the normal generated instruction parser and
`VmHandler` through Shipstern. A public VM debugger prints the actual input
context and emitted transfer state as JSON to stdout; this exposes ingestion
metadata without an observation server or committed reports.

Start that runtime separately against your local Geyser service:

```sh
YELLOWSTONE_ENDPOINT=http://127.0.0.1:10000 \
  cargo run --locked -p arete --example transaction_v1 > /tmp/a4-v1-ingestion.log
```

Configure your existing Arete relay with `ARETE_TRANSACTIONS_ENABLED=true`,
`ARETE_TRANSACTION_RPC_URL` pointing at the local validator, and appropriate
transaction authorization. Then explicitly invoke:

```sh
A4_V1_RELAY_URL=http://127.0.0.1:8081 \
A4_V1_KEYPAIR=/absolute/path/to/local-keypair.json \
A4_V1_INGESTION_LOG=/tmp/a4-v1-ingestion.log \
  npm run smoke:v1 --prefix typescript/adapters/kit
```

`A4_V1_TOKEN` optionally supplies an existing relay bearer token. Missing required
configuration, missing V1 adapter support, failed simulation/execution, unexpected
size, or absent/incorrect ingestion state or metadata fails the command.

The script uses the generated System instruction and public Kit adapter to
inspect/simulate, sign, send through Arete, and confirm. Both adapter operations
fetch fresh blockhashes. It decodes submitted bytes with Kit and requires V1
size **>1232 and <=4096**. Every fresh recipient must appear in decoded transfer
state with the submitted signature and expected amount. The V1 input context
must retain the selected inline config. The v0 control checks the same transfer
behavior without inheriting V1 metadata; Yellowstone's absence of a V1 config
is treated as unknown, not proof of v0. The submitted transaction is independently
decoded as v0.

The checked-in `smoke/generated/system-core.ts` is unmodified output from the
current CLI's `sdk create --idl arete/examples/transaction-v1/system.json
--program-only --ts` command. There is no extra regeneration package or CI job.

## Release verification and current dependencies

Reuse `check-generated-rust-crates.sh` in local/registry modes, adding an optional
adapter import to its existing consumers. Reuse the Kit ESM/CJS/Vite package
smoke with `--registry` after publication. Python adds a clean-environment import
of `arete-sdk[solana]` after publication. No live lifecycle is repeated per
language or after publication. The existing ingestion registry check is unchanged.

As of this revision:

- **A4-253:** TypeScript V1 adapter is not integrated; current Kit adapter supports
  v0 only. Its generated-client test fails explicitly at the capability check.
- **A4-254 / PR #200:** Rust adapter is open. The new integration test passes
  against that PR's head; this branch lacks its feature and implementation.
- **A4-255 / PR #201:** Python adapter is open. The new integration test passes
  against that PR's head with solders 0.29/Python 3.12; this branch lacks the extra
  and implementation.
- **Live smoke has not passed.** No configured running local stack was supplied,
  and A4-253 prevents the required TypeScript V1 send. A4-256 stays In Progress
  and PR #204 stays draft until integration and the live smoke pass.

The old toolchain downloader/compiler, validator probe, offline codec/signature
matrix, evidence/provenance files, report runner and standalone test package are
removed. Pre-existing shared transaction fixtures, necessary dependency locks,
and focused adapter, relay and ingestion regression tests are preserved.
