#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
cargo test --locked -p arete-solana-contracts -p arete-a4-sdk -p arete-server
cargo test --locked -p arete-interpreter --test managed_liquidity
cargo test --locked -p arete-macros --test managed_liquidity
npm run typecheck --prefix typescript/core
npm test --prefix typescript/core -- --reporter=dot managed-solana.test.ts
npm run build --prefix typescript/core
node scripts/check-managed-liquidity-typescript.cjs
"${PYTHON:-python3}" -m pytest python/arete-sdk/tests/test_managed_solana.py -q
./scripts/package-managed-solana-fixtures.sh
