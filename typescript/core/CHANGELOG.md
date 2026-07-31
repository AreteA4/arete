# Changelog

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-typescript-v0.3.0...arete-typescript-v0.4.0) (2026-07-31)


### Features

* **core:** add release-addressed reads and hosted transports ([f784932](https://github.com/AreteA4/arete/commit/f784932716f9555e50cb042896a938c47d3354c2))
* introduce the versioned public artifact model ([9c6777a](https://github.com/AreteA4/arete/commit/9c6777a3fe1703cc7491b56afaaac0bc5940b321))
* mint anonymous hosted sessions when no publishable key is configured ([afcddda](https://github.com/AreteA4/arete/commit/afcdddae35c05fb98c98358665ce829b63d4073f))

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-typescript-v0.2.0...arete-typescript-v0.3.0) (2026-07-22)


### Features

* add safe ORE transaction workflows ([5923c14](https://github.com/AreteA4/arete/commit/5923c1454d56c8e6b1f3a5a7b8765b863a581e03))
* add TypeScript query-scoped subscriptions ([c23ac38](https://github.com/AreteA4/arete/commit/c23ac380485bf193cecb73ad31fac2600f3f4af5))
* add WebSocket v2 and reactive ORE stack workflows ([ce37702](https://github.com/AreteA4/arete/commit/ce3770249a0899dd0be37f47cac2deeaa4e58511))
* **sdk:** add authenticated transaction execution ([0f123fd](https://github.com/AreteA4/arete/commit/0f123fd7daa414a58e911905a9d4c2a4d19fd613))


### Bug Fixes

* distinguish program behavior in client cache ([d808abd](https://github.com/AreteA4/arete/commit/d808abd80d2574d258bd06b855e2dc633308ac54))
* preserve subscription options on reconnect ([af30e9f](https://github.com/AreteA4/arete/commit/af30e9fd5c81c2c1bf99e1e3e580736bda7a528e))
* **sdk:** bundle noble ed25519 for CommonJS ([b9c46b0](https://github.com/AreteA4/arete/commit/b9c46b01992810bb34bb3d8087c47ef2eb41e391))
* **sdk:** preserve stream identity and canonical entity values ([76d0a7a](https://github.com/AreteA4/arete/commit/76d0a7a6b1708ee9f144cf9f7fea0d4fed548b2d))
* stabilize ORE resolver and streaming updates ([1ebeb44](https://github.com/AreteA4/arete/commit/1ebeb44750e2d2745d5ec31016a6098a1ce04d2f))
* validate sparse frames with patch schemas ([654a708](https://github.com/AreteA4/arete/commit/654a7080da0f173eef8a2da40d81a44fba4d1369))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.5...arete-typescript-v0.2.0) (2026-07-13)


### Features

* add generated program SDKs, sessions, HTTP reads, and typed execution ([f1437f1](https://github.com/AreteA4/arete/commit/f1437f1cbb615e8451d4f381c8e497c227f40c04))
* add session-oriented program operations ([a01c3e1](https://github.com/AreteA4/arete/commit/a01c3e167ea35935a90d20ae2bc899536385f0ba))
* add typed instruction runtime to the TypeScript SDK ([5ac4202](https://github.com/AreteA4/arete/commit/5ac420230432c49e8e2ba927c0ae66a6fd0e5072))


### Bug Fixes

* fail closed when required signers are unavailable ([2b9406a](https://github.com/AreteA4/arete/commit/2b9406a506549c3754f03a1f438027193ee68857))
* normalize generated stream data shapes ([094c5ab](https://github.com/AreteA4/arete/commit/094c5abab496e62b87f1f64ccdbedd98d98e0437))
* reject operations with missing concrete signers ([912b1d2](https://github.com/AreteA4/arete/commit/912b1d23b30471dcedee33ccfbaec37c0bb60909))

## [0.1.5](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.4...arete-typescript-v0.1.5) (2026-06-18)


### Miscellaneous Chores

* **arete-typescript:** Synchronize arete versions

## [0.1.4](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.3...arete-typescript-v0.1.4) (2026-06-17)


### Bug Fixes

* add manual npm publish recovery tooling ([1563f85](https://github.com/AreteA4/arete/commit/1563f858e9c5c75a54275bbe0a923273ddbbb9ef))

## [0.1.3](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.2...arete-typescript-v0.1.3) (2026-05-30)


### Miscellaneous Chores

* **arete-typescript:** Synchronize arete versions

## [0.1.2](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.1...arete-typescript-v0.1.2) (2026-05-22)


### Miscellaneous Chores

* **arete-typescript:** Synchronize arete versions

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-typescript-v0.1.0...arete-typescript-v0.1.1) (2026-04-30)


### Miscellaneous Chores

* **arete-typescript:** Synchronize arete versions

## [0.1.0](https://github.com/AreteA4/arete/compare/arete-typescript-v0.0.1...arete-typescript-v0.1.0) (2026-04-21)


### ⚠ BREAKING CHANGES

* Authentication system with WebSocket integration, SSR support, and security enhancements
* Merge pull request #75 from HyperTekOrg/auth

### Features

* add InstructionHandler with build() for code-generated instruction builders ([d0efba1](https://github.com/AreteA4/arete/commit/d0efba157580f0443ac39bf5189d8b96160cf785))
* Add license to typescript core ([22628a1](https://github.com/AreteA4/arete/commit/22628a19355144eba8e0014f098cd8b1c533c98e))
* add runtime schema validation to TypeScript core SDK ([a277e2c](https://github.com/AreteA4/arete/commit/a277e2c72407bf0c415ed9985b0a9e55eaf37c9b))
* Add SSR support for Next.js, TanStack Start, and Vite ([70cb1ac](https://github.com/AreteA4/arete/commit/70cb1acbf5b6eedb40947f0accdffd5f722e23d1))
* Authentication system with WebSocket integration, SSR support, and security enhancements ([d9b90f9](https://github.com/AreteA4/arete/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* improve SDK auth recovery for websocket connections ([193e442](https://github.com/AreteA4/arete/commit/193e442666aa1cc992c8ee364bbd11175ef7128a))
* Make snapshots optional with cursor-based filtering (HYP-148) ([46be9aa](https://github.com/AreteA4/arete/commit/46be9aa235d28a5c1ebe3f32ca94068ada9b245f))
* Merge pull request [#75](https://github.com/AreteA4/arete/issues/75) from HyperTekOrg/auth ([d9b90f9](https://github.com/AreteA4/arete/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* **typescript-sdk:** Support optional snapshots and cursor-based resume ([3f239e9](https://github.com/AreteA4/arete/commit/3f239e9d9fa2f992b7d8eaa1c467c541af9a8a9a))


### Bug Fixes

* allow unauthenticated requests to /ws/sessions for hosted stacks ([cbb047b](https://github.com/AreteA4/arete/commit/cbb047b454c122ea46c8a5692275494a8955d13c))
* correct Vite SSR auth route base path handling ([fee273a](https://github.com/AreteA4/arete/commit/fee273aae241b1ef6fc993b7b5beb97bfa747f9f))
* harden SSR session token issuance ([bde6607](https://github.com/AreteA4/arete/commit/bde6607b6bfcc146e8e7667336860410e8eaefb4))
* require authentication for hosted Hyperstack connections ([7b18719](https://github.com/AreteA4/arete/commit/7b187190a9b2e49e2fa4d694530c59f4d3a224cf))
* resolve TypeScript errors in core SDK ([e912aad](https://github.com/AreteA4/arete/commit/e912aad5f138526e107943e613d5fecf8ae8f7d3))
* **ts:** settle connect() promise on ws error and early close ([aa47833](https://github.com/AreteA4/arete/commit/aa47833e912ce9f2413c0e142cc24db08508509c))
* **ts:** settle connect() promise on ws error and early close ([28e3297](https://github.com/AreteA4/arete/commit/28e329705565ac314217b1eb5f40501bb483e274))
* **ts:** updateState on early-close connect rejection ([ddc12df](https://github.com/AreteA4/arete/commit/ddc12dfaa6919cb0631159ad1107147dff391fcc))

## [0.6.9](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.8...arete-typescript-v0.6.9) (2026-04-15)


### Bug Fixes

* **ts:** settle connect() promise on ws error and early close ([aa47833](https://github.com/AreteA4/arete/commit/aa47833e912ce9f2413c0e142cc24db08508509c))
* **ts:** settle connect() promise on ws error and early close ([28e3297](https://github.com/AreteA4/arete/commit/28e329705565ac314217b1eb5f40501bb483e274))
* **ts:** updateState on early-close connect rejection ([ddc12df](https://github.com/AreteA4/arete/commit/ddc12dfaa6919cb0631159ad1107147dff391fcc))

## [0.6.8](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.7...arete-typescript-v0.6.8) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.7](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.6...arete-typescript-v0.6.7) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.6](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.5...arete-typescript-v0.6.6) (2026-04-05)


### Bug Fixes

* require authentication for hosted Arete connections ([7b18719](https://github.com/AreteA4/arete/commit/7b187190a9b2e49e2fa4d694530c59f4d3a224cf))

## [0.6.5](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.4...arete-typescript-v0.6.5) (2026-04-05)


### Bug Fixes

* allow unauthenticated requests to /ws/sessions for hosted stacks ([cbb047b](https://github.com/AreteA4/arete/commit/cbb047b454c122ea46c8a5692275494a8955d13c))

## [0.6.4](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.3...arete-typescript-v0.6.4) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.3](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.2...arete-typescript-v0.6.3) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.2](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.1...arete-typescript-v0.6.2) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.1](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.6.0...arete-typescript-v0.6.1) (2026-04-05)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.6.0](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.10...arete-typescript-v0.6.0) (2026-04-04)


### ⚠ BREAKING CHANGES

* Authentication system with WebSocket integration, SSR support, and security enhancements
* Merge pull request #75 from AreteA4/auth

### Features

* Add SSR support for Next.js, TanStack Start, and Vite ([70cb1ac](https://github.com/AreteA4/arete/commit/70cb1acbf5b6eedb40947f0accdffd5f722e23d1))
* Authentication system with WebSocket integration, SSR support, and security enhancements ([d9b90f9](https://github.com/AreteA4/arete/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* improve SDK auth recovery for websocket connections ([193e442](https://github.com/AreteA4/arete/commit/193e442666aa1cc992c8ee364bbd11175ef7128a))
* Make snapshots optional with cursor-based filtering (HYP-148) ([46be9aa](https://github.com/AreteA4/arete/commit/46be9aa235d28a5c1ebe3f32ca94068ada9b245f))
* Merge pull request [#75](https://github.com/AreteA4/arete/issues/75) from AreteA4/auth ([d9b90f9](https://github.com/AreteA4/arete/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* **typescript-sdk:** Support optional snapshots and cursor-based resume ([3f239e9](https://github.com/AreteA4/arete/commit/3f239e9d9fa2f992b7d8eaa1c467c541af9a8a9a))


### Bug Fixes

* correct Vite SSR auth route base path handling ([fee273a](https://github.com/AreteA4/arete/commit/fee273aae241b1ef6fc993b7b5beb97bfa747f9f))
* harden SSR session token issuance ([bde6607](https://github.com/AreteA4/arete/commit/bde6607b6bfcc146e8e7667336860410e8eaefb4))

## [0.5.10](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.9...arete-typescript-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.9](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.6...arete-typescript-v0.5.9) (2026-03-19)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.6](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.5...arete-typescript-v0.5.6) (2026-03-19)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.5](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.4...arete-typescript-v0.5.5) (2026-03-14)


### Features

* Add license to typescript core ([22628a1](https://github.com/AreteA4/arete/commit/22628a19355144eba8e0014f098cd8b1c533c98e))

## [0.5.4](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.3...arete-typescript-v0.5.4) (2026-03-14)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.3](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.2...arete-typescript-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.2](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.1...arete-typescript-v0.5.2) (2026-02-07)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.1](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.5.0...arete-typescript-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.4.3...arete-typescript-v0.5.0) (2026-02-06)


### ⚠ BREAKING CHANGES

* EntityStore removed from core exports, replaced by StorageAdapter interface

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/AreteA4/arete/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* add InstructionHandler with build() for code-generated instruction builders ([d0efba1](https://github.com/AreteA4/arete/commit/d0efba157580f0443ac39bf5189d8b96160cf785))
* add runtime schema validation to TypeScript core SDK ([a277e2c](https://github.com/AreteA4/arete/commit/a277e2c72407bf0c415ed9985b0a9e55eaf37c9b))
* **core:** Add instruction execution infrastructure ([057f05d](https://github.com/AreteA4/arete/commit/057f05d9e8660ae319eb20f1c45f6cafa7d33b67))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/AreteA4/arete/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* Pluggable storage adapter architecture for React SDK ([60dac5e](https://github.com/AreteA4/arete/commit/60dac5e2d22f2dc388fc229efdf4068a95aa756f))
* **react:** add configurable frame buffering to reduce render churn ([c4bdb13](https://github.com/AreteA4/arete/commit/c4bdb13bf8efa085b8105c1fbbdc1e19127e6590))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/AreteA4/arete/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/AreteA4/arete/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **typescript-sdk:** add WatchOptions and .use() method for streaming merged entities ([b5c68c1](https://github.com/AreteA4/arete/commit/b5c68c13b6c7e597539b67693cf294e6799c6845))


### Bug Fixes

* **ci:** add basic tests for core SDK and skip react until core is published ([89def14](https://github.com/AreteA4/arete/commit/89def14ec05fe9265059ee58a8d9b169f32e03ec))
* **core:** ensure sorted views work with any storage adapter via SortedStorageDecorator ([d3ae37f](https://github.com/AreteA4/arete/commit/d3ae37faa214a0a944e7cb256e2fd366b3d3efe0))
* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/AreteA4/arete/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* resolve TypeScript errors in core SDK ([e912aad](https://github.com/AreteA4/arete/commit/e912aad5f138526e107943e613d5fecf8ae8f7d3))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/AreteA4/arete/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))
* **typescript-sdk:** support arbitrary view names in TypedViewGroup ([499eaa4](https://github.com/AreteA4/arete/commit/499eaa401d524782d2f61479ae6451d54f4c9212))
* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/AreteA4/arete/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))

## [0.4.3](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.4.2...arete-typescript-v0.4.3) (2026-02-03)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.4.2](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.4.1...arete-typescript-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.4.1](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.4.0...arete-typescript-v0.4.1) (2026-02-01)


### Bug Fixes

* **core:** ensure sorted views work with any storage adapter via SortedStorageDecorator ([d3ae37f](https://github.com/AreteA4/arete/commit/d3ae37faa214a0a944e7cb256e2fd366b3d3efe0))

## [0.4.0](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.15...arete-typescript-v0.4.0) (2026-01-31)


### ⚠ BREAKING CHANGES

* EntityStore removed from core exports, replaced by StorageAdapter interface

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/AreteA4/arete/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* **core:** Add instruction execution infrastructure ([057f05d](https://github.com/AreteA4/arete/commit/057f05d9e8660ae319eb20f1c45f6cafa7d33b67))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/AreteA4/arete/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* Pluggable storage adapter architecture for React SDK ([60dac5e](https://github.com/AreteA4/arete/commit/60dac5e2d22f2dc388fc229efdf4068a95aa756f))
* **react:** add configurable frame buffering to reduce render churn ([c4bdb13](https://github.com/AreteA4/arete/commit/c4bdb13bf8efa085b8105c1fbbdc1e19127e6590))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/AreteA4/arete/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/AreteA4/arete/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **typescript-sdk:** add WatchOptions and .use() method for streaming merged entities ([b5c68c1](https://github.com/AreteA4/arete/commit/b5c68c13b6c7e597539b67693cf294e6799c6845))


### Bug Fixes

* **ci:** add basic tests for core SDK and skip react until core is published ([89def14](https://github.com/AreteA4/arete/commit/89def14ec05fe9265059ee58a8d9b169f32e03ec))
* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/AreteA4/arete/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/AreteA4/arete/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))
* **typescript-sdk:** support arbitrary view names in TypedViewGroup ([499eaa4](https://github.com/AreteA4/arete/commit/499eaa401d524782d2f61479ae6451d54f4c9212))
* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/AreteA4/arete/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))

## [0.3.15](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.14...arete-typescript-v0.3.15) (2026-01-31)


### Features

* **core:** Add instruction execution infrastructure ([057f05d](https://github.com/AreteA4/arete/commit/057f05d9e8660ae319eb20f1c45f6cafa7d33b67))
* **typescript-sdk:** add WatchOptions and .use() method for streaming merged entities ([b5c68c1](https://github.com/AreteA4/arete/commit/b5c68c13b6c7e597539b67693cf294e6799c6845))


### Bug Fixes

* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* **typescript-sdk:** support arbitrary view names in TypedViewGroup ([499eaa4](https://github.com/AreteA4/arete/commit/499eaa401d524782d2f61479ae6451d54f4c9212))

## [0.3.14](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.13...arete-typescript-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.13](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.12...arete-typescript-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.12](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.11...arete-typescript-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.11](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.10...arete-typescript-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.10](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.9...arete-typescript-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.9](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.8...arete-typescript-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.8](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.7...arete-typescript-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.7](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.6...arete-typescript-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.6](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.5...arete-typescript-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.5](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.4...arete-typescript-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.4](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.3...arete-typescript-v0.3.4) (2026-01-24)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.3](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.2...arete-typescript-v0.3.3) (2026-01-23)


### Features

* **react:** add configurable frame buffering to reduce render churn ([c4bdb13](https://github.com/AreteA4/arete/commit/c4bdb13bf8efa085b8105c1fbbdc1e19127e6590))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))

## [0.3.2](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.1...arete-typescript-v0.3.2) (2026-01-20)


### Bug Fixes

* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/AreteA4/arete/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/AreteA4/arete/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))
* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/AreteA4/arete/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))

## [0.3.1](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.3.0...arete-typescript-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.3.0](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.5...arete-typescript-v0.3.0) (2026-01-20)


### ⚠ BREAKING CHANGES

* EntityStore removed from core exports, replaced by StorageAdapter interface

### Features

* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/AreteA4/arete/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* Pluggable storage adapter architecture for React SDK ([60dac5e](https://github.com/AreteA4/arete/commit/60dac5e2d22f2dc388fc229efdf4068a95aa756f))

## [0.2.5](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.4...arete-typescript-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.2.4](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.3...arete-typescript-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.2.3](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.2...arete-typescript-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/AreteA4/arete/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/AreteA4/arete/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/AreteA4/arete/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))

## [0.2.2](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.1...arete-typescript-v0.2.2) (2026-01-16)


### Features

* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))

## [0.2.1](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.2.0...arete-typescript-v0.2.1) (2026-01-16)


### Miscellaneous Chores

* **@usearete/sdk:** Synchronize arete versions

## [0.2.0](https://github.com/AreteA4/arete/compare/@usearete/sdk-v0.1.11...arete-typescript-v0.2.0) (2026-01-15)


### Bug Fixes

* **ci:** add basic tests for core SDK and skip react until core is published ([89def14](https://github.com/AreteA4/arete/commit/89def14ec05fe9265059ee58a8d9b169f32e03ec))
