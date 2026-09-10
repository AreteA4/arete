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

Default CI runs these fast checks. The adapter jobs require real dependencies:

```sh
# Use the normal core build and adapter dependency installation first.
npm test --prefix typescript/adapters/kit
cargo test --locked -p arete-a4-sdk --features solana-adapter --test generated_solana
cd python/arete-sdk
python -m pip install -e '.[dev,solana]'
python -m pytest tests/test_generated_solders.py
```

Python CI preserves the base install on Python 3.9 and 3.11 (`.[dev]`). Without
`solders`, the shared collection rule excludes both `test_solders_adapter.py`
and `test_generated_solders.py`. The Python 3.10/3.11 Solana-extra jobs install
`.[dev,solana]`, require `import solders`, run the full suite, and explicitly run
each adapter module in a separate pytest invocation. Missing dependencies or an
uncollected generated module fail those jobs; neither test uses `importorskip`.

## One local TypeScript smoke

Use **Surfpool** as the local backend. The smoke requires an **already-running**
V1-capable backend supporting the 4096-byte limit, a compatible Yellowstone/Geyser
service, an Arete transaction relay pointing at that backend, and the ingestion
fixture below. An existing V1-capable Agave validator also works with the same
command; a second run against Agave is not an acceptance requirement. Provide a
funded disposable local keypair (at least 0.04 SOL plus fees). The smoke sends
32 transfers of 0.001 SOL in V1 and one equivalent v0 transfer to fresh recipients.
It does not download, build or start a validator, provision a toolchain, deploy
an on-chain program, or start the relay/runtime.

The existing ORE stack requires application state/setup, so the tiny fixture uses
the built-in System program. Its IDL describes transfer plus an empty System
wallet account. The fixture runs the normal generated instruction parser and
`VmHandler` through Shipstern. A public VM debugger prints the actual input
context and emitted transfer state as JSON to stdout; this exposes ingestion
metadata without an observation server or committed reports. The fixture subscribes
at processed commitment because Surfpool does not provide the full Agave slot
lifecycle used by newer Yellowstone confirmed-stream reconstruction. The smoke
separately requires successful transaction confirmation through Arete before
accepting the observed state and metadata.

### Start the local services

The sibling `arete-examples` repository provides a reference in
`scripts/localnet/start-squads-surfpool.sh` and
`config/localnet/yellowstone-grpc.json`. Its local plugin is
`.localnet/plugins/libyellowstone_grpc_geyser.dylib`. The inspected setup uses
Surfpool 1.4.0 and Yellowstone 13.3.0; it is a configuration reference, not a
validated V1 pairing.

The complete smoke passed on macOS ARM64 with this pairing:

| Component | Tested build |
| --- | --- |
| Surfpool | Commit `30242a4fe3b87557f3d6f1629f548e2515718242`, macOS ARM64 artifact from [upstream run 33936070302](https://github.com/solana-foundation/surfpool/actions/runs/33936070302) |
| Host dependencies | Agave 4.2.1, Rust 1.95.0, `solana-message` 4.3.0, `solana-transaction` 4.1.5 |
| Yellowstone plugin | [15.1.2+solana.4.2.0 source](https://github.com/rpcpool/yellowstone-grpc/tree/v15.1.2%2Bsolana.4.2.0), rebuilt with Rust 1.95.0, its Agave 4.2.0 pins changed to 4.2.1, and the message/transaction versions above, using the host lockfile as the starting point |

The Surfpool binary reports 1.5.0, but the release-tagged 1.5.0 binary uses
Agave 4.1.2: the version label alone does not identify a compatible plugin.
Match the host's Geyser interface, shared Solana types and Rust ABI. The plugin
must preserve V1 `Message.config`; upstream Yellowstone added that conversion
in 15.1.1. An older plugin can drop metadata even when RPC execution succeeds.
Supply compatible binaries before starting the smoke. The build above was local
environment preparation; this PR contains no downloader, plugin build scripts
or toolchain provisioning.

1. Copy the reference plugin config to a local file, such as
   `/tmp/a4-v1-geyser.json`. Set `libpath` to the **absolute path of your compatible
   plugin**, `grpc.listen[0].address` to `127.0.0.1:10009`, and
   `prometheus.address` to `127.0.0.1:18999`. Keep the reference's unauthenticated
   loopback gRPC listener for this local fixture. On Linux the library uses `.so`.
2. Start Surfpool in its own terminal using a disposable keypair. This fixture
   needs only the built-in System program, so offline mode is sufficient:

```sh
surfpool --version
surfpool start --offline --no-deploy --yes --no-tui --no-studio \
  --host 127.0.0.1 --port 8899 --ws-port 8900 \
  --block-production-mode clock \
  --features-all \
  --geyser-plugin-config /tmp/a4-v1-geyser.json \
  --airdrop-keypair-path /absolute/path/to/local-keypair.json \
  --airdrop-amount 1000000000
```

Keep signature verification and blockhash checks enabled. The Jurassic demo's
impersonation launcher passes `--skip-signature-verification`; use the command
above for this smoke. `--features-all` includes V1 without hardcoding a feature
identifier that differs between Agave builds. Flags are documented in the
[Surfpool CLI reference](https://solana.com/docs/tools/surfpool/toolchain/cli).

3. Point your existing Arete relay at `http://127.0.0.1:8899`, or start the
   standalone gateway example in another terminal. It uses the normal public
   server API, binds to loopback, and reads the existing transaction settings:

```sh
ARETE_TRANSACTIONS_ENABLED=true \
ARETE_TRANSACTIONS_ALLOW_UNAUTHENTICATED=true \
ARETE_TRANSACTION_RPC_URL=http://127.0.0.1:8899 \
  cargo run --locked -p arete-server --example solana_gateway
```

The smoke below assumes its HTTP endpoint is `http://127.0.0.1:8081`. The local
example explicitly allows unauthenticated transactions; use `A4_V1_TOKEN` with
your existing authenticated relay instead when applicable.

4. From this repository root, start the ingestion runtime in another terminal
   and leave it running. Use the same gRPC port as the plugin config:

```sh
YELLOWSTONE_ENDPOINT=http://127.0.0.1:10009 \
  cargo run --locked -p arete --example transaction_v1 > /tmp/a4-v1-ingestion.log
```

### Run the smoke

Prepare the SDK using the same core build and Kit dependency installation as
CI (Node >=20.18):

```sh
npm ci --prefix typescript/core
npm run build --prefix typescript/core
npm ci --prefix typescript/adapters/kit
(cd typescript/adapters/kit && npm install ../../core --no-save --package-lock=false)
```

With the services above ready, explicitly invoke from this repository root:

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

## Release verification and completion

Reuse `check-generated-rust-crates.sh` in local/registry modes, adding an optional
adapter import to its existing consumers. Reuse the Kit ESM/CJS/Vite package
smoke with `--registry` after publication. Python adds a clean-environment import
of `arete-sdk[solana]` after publication. No live lifecycle is repeated per
language or after publication. The existing ingestion registry check is unchanged.

This branch is rebased onto main `fd7ce0e0`, including **A4-253 / PR #205**
(TypeScript), **A4-254 / PR #200** (Rust), and **A4-255 / PR #201** (Python).
All three real adapter implementations are now integrated; no adapter
implementation dependency remains. Python's required aggregate CI check and
base/extra matrix are preserved. The generated Rust test runs in the existing
Solana adapter job alongside its regression suite and V1 example.

The live smoke passed on 2026-09-10 with the pairing above: **1790-byte V1**
and **273-byte v0**, both simulated, signed, submitted through Arete, confirmed
and ingested. It checked all 32 V1 transfer states and their inline config, plus
the v0 control state without inherited V1 metadata. Signature verification and
fresh-blockhash checks remained enabled. The fixture IDL declares System's
four-byte tag as a `u32` discriminant so both generated clients and the generated
ingestion parser use the correct wire layout.

The three adapter dependencies are resolved. PR CI remains required for merge;
existing publication checks remain in the release workflow. No extra
per-language live runs are required.

The old toolchain downloader/compiler, validator probe, offline codec/signature
matrix, evidence/provenance files, report runner and standalone test package are
removed. Pre-existing shared transaction fixtures, necessary dependency locks,
and focused adapter, relay and ingestion regression tests are preserved.
