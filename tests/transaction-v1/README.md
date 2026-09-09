# A4-256 acceptance harness

**Status: scaffolding; full lifecycle acceptance is blocked.** The offline
fixture gate and disposable validator/Geyser startup probe are implemented.
This does not establish generated SDK, relay, ingestion/replay, or wallet V1
readiness. `--mode local` deliberately exits 1 and writes the remaining gates
to its JSON report. It must not become green until all those assertions run.

At the tested OSS baseline `bcbdb9ab`, A4-250/A4-251 are present in version
0.16.0. A4-253 is still in progress and the Kit adapter advertises only v0.
The optional Rust adapter (PR #200, `21c2550b2cf8f4217143859534074622d111d03f`)
and Python adapter (PR #201, `a02198390787f2d67edf127b15e42cd649f48341`)
are open prerequisites. Their changes are not incorporated in this harness PR.

[Recorded execution evidence](evidence.json) contains the actual offline,
startup, missing-toolchain, interruption and blocked-local reports, including
toolchain and source-input digests. The startup probe passed on macOS arm64;
the pinned Linux pair is configured but has not been executed locally.

## Offline gate

Requires Node 24 and Python 3.11 or newer. Run from the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix tests/transaction-v1
bash scripts/test-transaction-v1-e2e.sh --mode offline
```

Installation is separate; execution makes no network requests. CI runs this
gate and retains its JSON report. Reports default to
`tests/transaction-v1/.artifacts/report.json`; `--report PATH` selects another
destination. A missing dependency, timeout, malformed fixture or unsuccessful
self-test fails with exit 1. Reports include source revision, dirty status,
codec version, corpus provenance, transaction signatures and fixture digests.

The nine cases include legacy/v0 controls, V1 empty config, explicit zeros,
maximum u64 fee, multiple signers, a 1574-byte transaction and correctly signed
4096/4097-byte codec outputs. All signatures are verified with Node crypto;
Kit decodes and round-trips the original wire bytes. These fixtures have an
expired blockhash and are not executable live transactions. The 4097 case
proves that the negative input is otherwise valid; it does not yet prove
adapter/relay rejection. See the [fixture provenance](../fixtures/transaction-v1/README.md).

To intentionally regenerate the corpus and digests:

```sh
node tests/transaction-v1/regenerate.mjs
```

The command preserves the original five relay fixtures byte for byte. The
acceptance runner never regenerates its own expectations.

## Disposable validator prerequisite probe

```sh
python3 tests/transaction-v1/prepare_toolchain.py
python3 tests/transaction-v1/validator.py
```

Preparation downloads into `.toolchain/`, verifies pinned release checksums,
and records binary checksums in `receipt.json`. It supports Linux x86_64 and
macOS arm64. It does not change the active Solana installation. Use
`--directory PATH` when preparing and `--toolchain PATH` when probing to select
another isolated installation. Download and build commands have deadlines.

The pinned tuple is Agave **4.2.2**, Yellowstone
**v15.2.0+solana.4.2.2** at
`b7e7557c0cf6206a175a35910b339dd60e35c5c9`, and Rust **1.96.1** for the
macOS plugin build. Linux uses the checksummed upstream `.so`; macOS builds
the clean pinned source with its checked-in Cargo.lock. Compiling the plugin
with the host's default Rust 1.98 caused a `capacity overflow` panic during
`setup_logger_for_plugin`; matching the upstream compiler resolved it.
The Agave archive includes `cargo-build-sbf 4.1.0`, platform-tools `v1.54`;
the fixture program has not yet been built with them.

The probe reserves loopback ports, creates a new temporary ledger, starts its
own validator, waits for health with a deadline, verifies the RPC identity and
version, checks the V1 feature's activated slot and probes the plugin's TCP
listener. Process exit, missing feature and timeout fail. It terminates only
the process group it started and removes its ledger, including on interruption.
Logs and a report remain under `.artifacts/validator/` (override with
`--artifacts PATH`). No existing validator, public cluster, funded key or saved
Solana CLI configuration is used. A listening Geyser port is startup evidence;
it does not establish transaction delivery or metadata preservation.

## Remaining acceptance work

- Integrate A4-253–A4-255 and build the minimal outer/CPI program and Arete stack.
- Implement `scripts/check-generated-v1-sdks.sh --mode local|registry` for
  program and stack bindings in all three languages. Generate into temporary
  directories; compile/import the emitted builders and exercise the adapters.
- Complete `--mode local`: generated instruction → inspect without signing →
  simulate → sign final config → Arete relay → reconcile by signature → JSON
  with maximum version 1 → Geyser → Arete events/state and duplicate replay.
- Require real large V1, multi-signer and outer/CPI cases. V1 must retain its
  exact observed config/version. Legacy/v0 must retain explicit codec/RPC
  versions and equivalent application behavior; unknown ingestion
  classification is accepted and must never be guessed.
- Assert adapter/relay size, budget, signer and version rejection, and no
  resubmission after ambiguous sends. Missing expected events must fail.
- Add required local lifecycle CI and post-publication registry jobs once the
  actual consumers exist; record the passing release version before A4-257.
- Update public usage documentation against the completed APIs. Record
  browser wallet compatibility separately: local-keypair acceptance is not
  Phantom acceptance, and sign-and-send-only wallets cannot silently transfer
  broadcast ownership away from the Arete relay.

No full-support release or browser-wallet compatibility is claimed here.

References: [A4-256](https://linear.app/arete-a4/issue/A4-256),
[official runnable V1 examples](https://github.com/solana-foundation/transaction-v1-examples),
[Agave 4.2.2](https://github.com/anza-xyz/agave/releases/tag/v4.2.2),
[Yellowstone 15.2.0](https://github.com/rpcpool/yellowstone-grpc/releases/tag/v15.2.0%2Bsolana.4.2.2).
