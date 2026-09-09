# Transaction V1 fixture corpus

Real signed transactions in all three wire versions, for tests that must exercise the actual codec
rather than hand-assembled bytes. Hand-assembly is how a test ends up asserting the parser against
itself; every payload here was produced by a released Solana codec and can be re-derived.

## Provenance

| | |
|---|---|
| Codec | [`@solana/kit`](https://github.com/anza-xyz/kit) `8.2.0` (MIT) |
| Generator | `generate.mjs` in this directory |
| Regenerate | `npm install @solana/kit@8.2.0 && node generate.mjs > transactions.json` |
| Specification | [SIMD-0385 transaction V1](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md) |

`8.2.0` is the first line we verified encodes version 1 — `getTransactionVersionEncoder().encode(1)`
yields `0x81`, the same version byte `arete-server` recognizes. Record the codec version whenever
these are regenerated: a fixture whose provenance is unknown proves nothing.

## Keys

Deterministic test-only Ed25519 keys, seeded with a repeated byte (`0x01` payer, `0x02` cosigner).
They hold nothing and are not valid anywhere:

| Role | Address |
|---|---|
| payer | `AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9` |
| cosigner | `9hSR6S7WPtxmTojgo6GG3k4yDPecgJY292j7xrsUGWBu` |

The lifetime blockhash is the all-ones system address, so nothing here is replayable.

## Fixtures

Each entry carries `version`, `signatureCount`, `firstSignature` (base58), `bytes` and `base64`.

| Key | Version | Bytes | Why it exists |
|---|---|---|---|
| `legacy` | `legacy` | 172 | The unversioned control |
| `v0` | 0 | 174 | Version byte `0x80`, the current default |
| `v1` | 1 | 177 | Version byte `0x81`, minimal payload |
| `v1_oversize` | 1 | 1574 | Past the 1232-byte legacy/v0 ceiling, under V1's 4096 |
| `v1_two_signatures` | 1 | 273 | Two required signatures, both present |

`v1_oversize` is the one that matters most: at 1574 bytes it is rejected by any path still applying
the legacy limit, and accepted by one that knows V1's. A test that only uses the 177-byte `v1`
payload passes either way and proves nothing about the limit.
