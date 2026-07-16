# ORE React Example

A mainnet-only example of reading and writing ORE with the generated Arete stack SDK. Transactions use the authenticated Arete HTTP transport by default; explicit direct Solana RPC remains available.

## Architecture

- `src/App.tsx` wires the wallet to adapter `auto` mode without constructing a Solana `Connection`. Direct mode mounts `ConnectionProvider` separately.
- `src/components/OreDashboard.tsx` directly shows `useArete`, the live `OreRound/latest` view, wallet-keyed Miner subscriptions, generated quote/prepare calls, operation inspection, exact execution, and processed-slot reconciliation.
- `src/components/BlockGrid.tsx` and `StatsPanel.tsx` are small presentational components for the board and current-round summary.

The board renders from `OreRound/latest`, so a fresh client immediately subscribes to live round updates without waiting for a Board account change. Wallet miners subscribe through `OreMiner/state`; positions are overlaid only when the miner and live round IDs match. Transaction review and approval separately read the raw Board account and clock before using the prepared operation.

## Local packages

Development dependencies intentionally use:

```json
"@usearete/sdk": "file:../../typescript/core",
"@usearete/react": "file:../../typescript/react",
"@usearete/adapter-web3js": "file:../../typescript/adapters/web3js"
```

Build those packages before a clean install when their `dist/` directories are absent. Release automation rewrites these links to compatible semver versions.

## Configuration

Copy values from `.env.example` into an untracked `.env.local` as needed.

- The generated production Arete WS/HTTP pair is used when no override is present.
- `VITE_ARETE_WS_URL` and `VITE_ARETE_HTTP_URL` must be provided together.
- HTTP derivation is allowed only with `VITE_DERIVE_ARETE_HTTP=true` and a WS override.
- `VITE_TRANSACTION_TRANSPORT=auto` is the default and does not use a public Solana RPC.
- `VITE_TRANSACTION_TRANSPORT=direct` requires `VITE_SOLANA_RPC_URL`; devnet, testnet, localhost, and loopback endpoints are rejected.
- `VITE_ARETE_PUBLISHABLE_KEY` is optional and has no hardcoded fallback.
- `VITE_SOLANA_EXPLORER_URL` is the transaction URL prefix.

## Wallet compatibility

The app installs only `@solana/wallet-adapter-react` and `@solana/wallet-adapter-react-ui`, which declare React `*` peer support and work with React 19. It intentionally avoids the large `@solana/wallet-adapter-wallets` bundle. Compatible Wallet Standard extensions are discovered automatically. Transactions require v0 `VersionedTransaction` signing; unsupported wallets are rejected by `@usearete/adapter-web3js` before submission.

## Transaction safety

- One funded round prepares `deployWithCheckpoint`; the Board round is resolved again during preparation.
- The generated `read.quoteManualDeployment` fetches raw Miner and Automation accounts before a connected wallet can review.
- Active automation blocks manual deployment. Automation must be disabled and reconciled outside this canonical example.
- A prepared operation is inspected without signing. Execution uses that exact object; a Board rollover invalidates a reviewed deploy.
- No action retries or resubmits automatically.
- The current round and raw Automation account are revalidated immediately before execution.
- After confirmation, the app waits for Arete to process the confirmed slot and refreshes Board, Round, and Miner views.
- Playwright sets `VITE_AUTOMATED_TEST_MODE=true`; the execute path refuses writes even if a browser wallet is present.

The compact quote copy reports generated principal allocation, rounding remainder, and checkpoint reserve. Inspection separately displays the RPC network fee estimate; account rent is not invented or estimated by the example.

## Commands

```bash
npm install
npm run dev
npm run typecheck
npm test
npm run build
npm run test:e2e
npm ls @usearete/sdk
```

Unit/component tests use Vitest and React Testing Library. Playwright tests are disconnected/read-only and cover desktop, mobile, keyboard, reduced motion, touch targets, direct selection, and wallet prompting. Controlled mainnet smoke testing is intentionally manual and must use a disposable, tightly funded wallet.
