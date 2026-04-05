# Changelog

## [0.6.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.6.0...hyperstack-server-v0.6.1) (2026-04-05)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.6.0 to 0.6.1
    * hyperstack-auth bumped from 0.1.0 to 0.2.0

## [0.6.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.10...hyperstack-server-v0.6.0) (2026-04-04)


### ⚠ BREAKING CHANGES

* Authentication system with WebSocket integration, SSR support, and security enhancements
* Merge pull request #75 from HyperTekOrg/auth

### Features

* Add authentication system with WebSocket integration ([bd7f8ad](https://github.com/HyperTekOrg/hyperstack/commit/bd7f8adca65e2c0222aab32146faa8d57d357735))
* Authentication system with WebSocket integration, SSR support, and security enhancements ([d9b90f9](https://github.com/HyperTekOrg/hyperstack/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* enforce websocket auth and rate limits on the server ([71a0c04](https://github.com/HyperTekOrg/hyperstack/commit/71a0c04540fa75544eedb250e06aacf5b3709c03))
* Make snapshots optional with cursor-based filtering (HYP-148) ([46be9aa](https://github.com/HyperTekOrg/hyperstack/commit/46be9aa235d28a5c1ebe3f32ca94068ada9b245f))
* Merge pull request [#75](https://github.com/HyperTekOrg/hyperstack/issues/75) from HyperTekOrg/auth ([d9b90f9](https://github.com/HyperTekOrg/hyperstack/commit/d9b90f9bbae6cf3a70273c7fc30230cdb58198df))
* **server:** Add optional snapshot and cursor-based filtering to WebSocket protocol ([da7b486](https://github.com/HyperTekOrg/hyperstack/commit/da7b4862375de7a25c0e254d6e90aa2c1e64c546))


### Bug Fixes

* add camelCase serde rename to Subscription struct ([fc68ea5](https://github.com/HyperTekOrg/hyperstack/commit/fc68ea51cf1074e4442f7264322908901f1e041e))
* add truncate after sorting by _seq to respect snapshot_limit ([0782e66](https://github.com/HyperTekOrg/hyperstack/commit/0782e666a33ac9326f53c0b9f5556d66a663badc))
* allow large error variants in WebSocket handshake closures ([4a5aeec](https://github.com/HyperTekOrg/hyperstack/commit/4a5aeec8e901d3e22e6a3510e65d68560d6e6aa2))
* apply snapshot_limit after key filter in websocket subscriptions ([12f8f75](https://github.com/HyperTekOrg/hyperstack/commit/12f8f755d2a125f4a255c4da79e699dab842cdd4))
* apply snapshot_limit when no after cursor is provided ([b3850e6](https://github.com/HyperTekOrg/hyperstack/commit/b3850e6e411576c31bb846508230dd7afda91cb1))
* correct snapshot ordering when using cursor with limit ([6f4a4d3](https://github.com/HyperTekOrg/hyperstack/commit/6f4a4d36132809ec91cd4a542e4239b82ca83eca))
* Fix _seq numeric comparison and missing borrow_and_update in cache and WebSocket handlers ([7b2c06c](https://github.com/HyperTekOrg/hyperstack/commit/7b2c06cb699b1f8bc1503a31a190e08a91996158))
* make snapshot_limit deterministic by sorting before truncation ([1d62917](https://github.com/HyperTekOrg/hyperstack/commit/1d629172fc023aff7576bd79c13351ce12535df7))
* prune stale rate limiter buckets ([8579fa8](https://github.com/HyperTekOrg/hyperstack/commit/8579fa856b3527b367c9fb06c533848d7847e66d))
* resolve clippy warnings in SDK and server ([ee68b63](https://github.com/HyperTekOrg/hyperstack/commit/ee68b63aaaf24228114dac6afcb78cb01ae92c25))
* sort entities by _seq before applying snapshot_limit ([6abba2a](https://github.com/HyperTekOrg/hyperstack/commit/6abba2a923bb725465f579e6f4ee25e2d68ec03e))
* tighten Rust auth verification and cleanup ([1413a7a](https://github.com/HyperTekOrg/hyperstack/commit/1413a7aff6fbf2fd91976c48657d0a929482c276))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.10 to 0.6.0

## [0.5.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.9...hyperstack-server-v0.5.10) (2026-03-19)


### Bug Fixes

* Reduce cache size for slot hashes ([ec9cba3](https://github.com/HyperTekOrg/hyperstack/commit/ec9cba3507b1caae01cdfed961e2c3e3ec7d5481))
* Reduce cache size for slot hashes ([5d140a4](https://github.com/HyperTekOrg/hyperstack/commit/5d140a4e7d35088c9ad87eb8dcae627485c0c035))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.9 to 0.5.10

## [0.5.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.8...hyperstack-server-v0.5.9) (2026-03-19)


### Bug Fixes

* apply u64 string transform to live bus updates in projector ([c179f06](https://github.com/HyperTekOrg/hyperstack/commit/c179f06d0faa7ea9cf96d058c5d3a2d354165c99))
* convert negative i64 values outside MIN_SAFE_INTEGER to strings in WebSocket frames ([de89478](https://github.com/HyperTekOrg/hyperstack/commit/de89478d97906bbf03c106287f54c609486a259a))
* handle u64 integer precision loss across Rust-JS boundary ([e96e7fa](https://github.com/HyperTekOrg/hyperstack/commit/e96e7fa7172f520bd7ee88ed7582eda899c9f65b))
* handle u64 integer precision loss across Rust-JS boundary ([c3a3c69](https://github.com/HyperTekOrg/hyperstack/commit/c3a3c69587d9e6215aa5dfe4102739eef0ba8662))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.8 to 0.5.9

## [0.5.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.7...hyperstack-server-v0.5.8) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.7 to 0.5.8

## [0.5.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.6...hyperstack-server-v0.5.7) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.6 to 0.5.7

## [0.5.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.5...hyperstack-server-v0.5.6) (2026-03-19)


### Features

* add Yellowstone gRPC dependencies and update Ore examples ([2f23205](https://github.com/HyperTekOrg/hyperstack/commit/2f23205577c814572e57e5628bee9225d4631f4b))


### Bug Fixes

* Core interpreter and server improvements ([b05ae9b](https://github.com/HyperTekOrg/hyperstack/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* resolve clippy warnings across workspace ([c19d1ec](https://github.com/HyperTekOrg/hyperstack/commit/c19d1ec5926ee9099c6ab4254bde30b2c794e27f))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.5 to 0.5.6

## [0.5.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.4...hyperstack-server-v0.5.5) (2026-03-14)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.4 to 0.5.5

## [0.5.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.3...hyperstack-server-v0.5.4) (2026-03-14)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.3 to 0.5.4

## [0.5.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.2...hyperstack-server-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.2 to 0.5.3

## [0.5.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.1...hyperstack-server-v0.5.2) (2026-02-07)


### Bug Fixes

* exclude derived views from projector export index ([53e2ea9](https://github.com/HyperTekOrg/hyperstack/commit/53e2ea905c59303ef9126b969d3f7168082cf076))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.1 to 0.5.2

## [0.5.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.5.0...hyperstack-server-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.0 to 0.5.1

## [0.5.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.4.3...hyperstack-server-v0.5.0) (2026-02-06)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* add resolver support with resolve attribute and computed field methods ([aed45c8](https://github.com/HyperTekOrg/hyperstack/commit/aed45c81477267cb6a005d439ee30400c1e24e5c))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add entity cache for snapshot-on-subscribe ([5342720](https://github.com/HyperTekOrg/hyperstack/commit/53427201fb1dbd69f918b07dfc5355c89c2a7694))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add SortedViewCache for incremental sorted view maintenance ([6660b90](https://github.com/HyperTekOrg/hyperstack/commit/6660b9072250bcbbdda64db21a547391fa7456f6))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Feature flag import ([a5c3160](https://github.com/HyperTekOrg/hyperstack/commit/a5c3160cedff5c678c23f266627d2fa7bebc890c))
* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* Missing import ([dc9a92d](https://github.com/HyperTekOrg/hyperstack/commit/dc9a92d7f685692746d8c6349da440b2e21028aa))
* Otel feature flag on timing ([42626fb](https://github.com/HyperTekOrg/hyperstack/commit/42626fbc9db0e48f5ab966741c5e71bc34718059))
* prefix unused stats variables to satisfy clippy ([70741c6](https://github.com/HyperTekOrg/hyperstack/commit/70741c680297a84a8136be5104f1dc42d03e2d99))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/HyperTekOrg/hyperstack/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* resolve clippy warnings across workspace ([565d92b](https://github.com/HyperTekOrg/hyperstack/commit/565d92b91552d92262cfaeca9674d0ad4d3f6b5d))
* send compressed frames as raw binary gzip instead of base64-wrapped JSON ([2433daf](https://github.com/HyperTekOrg/hyperstack/commit/2433dafdd321151cf5185aaad639871c57777cf4))
* send delete/upsert for derived single view updates ([0577afd](https://github.com/HyperTekOrg/hyperstack/commit/0577afde6640a04be2d8429ba89f149f7a583ae5))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/HyperTekOrg/hyperstack/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))
* **server:** isolate HTTP health server on dedicated OS thread ([4695ec9](https://github.com/HyperTekOrg/hyperstack/commit/4695ec90bae65094f77a421b409f02fdd14be702))
* **server:** preserve sort order when entity updates lack the sort field ([7978edc](https://github.com/HyperTekOrg/hyperstack/commit/7978edc2f25eb2d4aa538e351beca2cc2542ee96))
* **server:** resolve WebSocket connection hang from lock contention ([0596c47](https://github.com/HyperTekOrg/hyperstack/commit/0596c47b932075ab3c513b8cf11da3d1b0190778))
* **server:** stream live updates for all items in windowed subscriptions ([ddd4aae](https://github.com/HyperTekOrg/hyperstack/commit/ddd4aaea3f304ae04207f8539e747aa66f331c0a))
* use entity cache for state view initial data and increase default cache limit ([393ea9d](https://github.com/HyperTekOrg/hyperstack/commit/393ea9d065d647c5a814a1180b967e1d867ad179))
* Variable incorrectly named ([826fb82](https://github.com/HyperTekOrg/hyperstack/commit/826fb825f12dd17f96be439aa99d59d3a60bb96c))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.3 to 0.5.0

## [0.4.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.4.2...hyperstack-server-v0.4.3) (2026-02-03)


### Features

* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))


### Bug Fixes

* use entity cache for state view initial data and increase default cache limit ([393ea9d](https://github.com/HyperTekOrg/hyperstack/commit/393ea9d065d647c5a814a1180b967e1d867ad179))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.2 to 0.4.3

## [0.4.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.4.1...hyperstack-server-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.1 to 0.4.2

## [0.4.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.4.0...hyperstack-server-v0.4.1) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.0 to 0.4.1

## [0.4.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.15...hyperstack-server-v0.4.0) (2026-01-31)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add entity cache for snapshot-on-subscribe ([5342720](https://github.com/HyperTekOrg/hyperstack/commit/53427201fb1dbd69f918b07dfc5355c89c2a7694))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add SortedViewCache for incremental sorted view maintenance ([6660b90](https://github.com/HyperTekOrg/hyperstack/commit/6660b9072250bcbbdda64db21a547391fa7456f6))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Feature flag import ([a5c3160](https://github.com/HyperTekOrg/hyperstack/commit/a5c3160cedff5c678c23f266627d2fa7bebc890c))
* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* Missing import ([dc9a92d](https://github.com/HyperTekOrg/hyperstack/commit/dc9a92d7f685692746d8c6349da440b2e21028aa))
* Otel feature flag on timing ([42626fb](https://github.com/HyperTekOrg/hyperstack/commit/42626fbc9db0e48f5ab966741c5e71bc34718059))
* prefix unused stats variables to satisfy clippy ([70741c6](https://github.com/HyperTekOrg/hyperstack/commit/70741c680297a84a8136be5104f1dc42d03e2d99))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/HyperTekOrg/hyperstack/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* send compressed frames as raw binary gzip instead of base64-wrapped JSON ([2433daf](https://github.com/HyperTekOrg/hyperstack/commit/2433dafdd321151cf5185aaad639871c57777cf4))
* send delete/upsert for derived single view updates ([0577afd](https://github.com/HyperTekOrg/hyperstack/commit/0577afde6640a04be2d8429ba89f149f7a583ae5))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/HyperTekOrg/hyperstack/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))
* **server:** isolate HTTP health server on dedicated OS thread ([4695ec9](https://github.com/HyperTekOrg/hyperstack/commit/4695ec90bae65094f77a421b409f02fdd14be702))
* **server:** preserve sort order when entity updates lack the sort field ([7978edc](https://github.com/HyperTekOrg/hyperstack/commit/7978edc2f25eb2d4aa538e351beca2cc2542ee96))
* **server:** resolve WebSocket connection hang from lock contention ([0596c47](https://github.com/HyperTekOrg/hyperstack/commit/0596c47b932075ab3c513b8cf11da3d1b0190778))
* **server:** stream live updates for all items in windowed subscriptions ([ddd4aae](https://github.com/HyperTekOrg/hyperstack/commit/ddd4aaea3f304ae04207f8539e747aa66f331c0a))
* Variable incorrectly named ([826fb82](https://github.com/HyperTekOrg/hyperstack/commit/826fb825f12dd17f96be439aa99d59d3a60bb96c))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.15 to 0.4.0

## [0.3.15](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.14...hyperstack-server-v0.3.15) (2026-01-31)


### Bug Fixes

* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.14 to 0.3.15

## [0.3.14](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.13...hyperstack-server-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.13 to 0.3.14

## [0.3.13](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.12...hyperstack-server-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.12 to 0.3.13

## [0.3.12](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.11...hyperstack-server-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.11 to 0.3.12

## [0.3.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.10...hyperstack-server-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.10 to 0.3.11

## [0.3.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.9...hyperstack-server-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.9 to 0.3.10

## [0.3.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.8...hyperstack-server-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.8 to 0.3.9

## [0.3.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.7...hyperstack-server-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.7 to 0.3.8

## [0.3.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.6...hyperstack-server-v0.3.7) (2026-01-26)


### Bug Fixes

* **server:** isolate HTTP health server on dedicated OS thread ([4695ec9](https://github.com/HyperTekOrg/hyperstack/commit/4695ec90bae65094f77a421b409f02fdd14be702))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.6 to 0.3.7

## [0.3.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.5...hyperstack-server-v0.3.6) (2026-01-26)


### Bug Fixes

* **server:** preserve sort order when entity updates lack the sort field ([7978edc](https://github.com/HyperTekOrg/hyperstack/commit/7978edc2f25eb2d4aa538e351beca2cc2542ee96))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.5 to 0.3.6

## [0.3.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.4...hyperstack-server-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.4 to 0.3.5

## [0.3.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.3...hyperstack-server-v0.3.4) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.3 to 0.3.4

## [0.3.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.2...hyperstack-server-v0.3.3) (2026-01-23)


### Features

* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add SortedViewCache for incremental sorted view maintenance ([6660b90](https://github.com/HyperTekOrg/hyperstack/commit/6660b9072250bcbbdda64db21a547391fa7456f6))


### Bug Fixes

* send delete/upsert for derived single view updates ([0577afd](https://github.com/HyperTekOrg/hyperstack/commit/0577afde6640a04be2d8429ba89f149f7a583ae5))
* **server:** stream live updates for all items in windowed subscriptions ([ddd4aae](https://github.com/HyperTekOrg/hyperstack/commit/ddd4aaea3f304ae04207f8539e747aa66f331c0a))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.2 to 0.3.3

## [0.3.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.1...hyperstack-server-v0.3.2) (2026-01-20)


### Bug Fixes

* prevent duplicate WebSocket subscriptions from same client ([8135fdf](https://github.com/HyperTekOrg/hyperstack/commit/8135fdf28461b1906a03fe78c3f9ae50362ccb96))
* send compressed frames as raw binary gzip instead of base64-wrapped JSON ([2433daf](https://github.com/HyperTekOrg/hyperstack/commit/2433dafdd321151cf5185aaad639871c57777cf4))
* send snapshots in batches for faster initial page loads ([d4a8c40](https://github.com/HyperTekOrg/hyperstack/commit/d4a8c405bbd5859f40825d99a3b044c64ede6985))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.1 to 0.3.2

## [0.3.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.3.0...hyperstack-server-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.0 to 0.3.1

## [0.3.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.5...hyperstack-server-v0.3.0) (2026-01-20)


### Features

* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.5 to 0.3.0

## [0.2.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.4...hyperstack-server-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.4 to 0.2.5

## [0.2.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.3...hyperstack-server-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.3 to 0.2.4

## [0.2.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.2...hyperstack-server-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* prefix unused stats variables to satisfy clippy ([70741c6](https://github.com/HyperTekOrg/hyperstack/commit/70741c680297a84a8136be5104f1dc42d03e2d99))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* **server:** resolve WebSocket connection hang from lock contention ([0596c47](https://github.com/HyperTekOrg/hyperstack/commit/0596c47b932075ab3c513b8cf11da3d1b0190778))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.2 to 0.2.3

## [0.2.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.1...hyperstack-server-v0.2.2) (2026-01-16)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.1 to 0.2.2

## [0.2.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.2.0...hyperstack-server-v0.2.1) (2026-01-16)


### Features

* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.0 to 0.2.1

## [0.2.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.11...hyperstack-server-v0.2.0) (2026-01-15)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* **server:** add entity cache for snapshot-on-subscribe ([5342720](https://github.com/HyperTekOrg/hyperstack/commit/53427201fb1dbd69f918b07dfc5355c89c2a7694))


### Bug Fixes

* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.11 to 0.2.0

## [0.1.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.10...hyperstack-server-v0.1.11) (2026-01-14)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.10 to 0.1.11

## [0.1.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.9...hyperstack-server-v0.1.10) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.9 to 0.1.10

## [0.1.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.8...hyperstack-server-v0.1.9) (2026-01-13)


### Bug Fixes

* Otel feature flag on timing ([42626fb](https://github.com/HyperTekOrg/hyperstack/commit/42626fbc9db0e48f5ab966741c5e71bc34718059))
* Variable incorrectly named ([826fb82](https://github.com/HyperTekOrg/hyperstack/commit/826fb825f12dd17f96be439aa99d59d3a60bb96c))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.8 to 0.1.9

## [0.1.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.7...hyperstack-server-v0.1.8) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Feature flag import ([a5c3160](https://github.com/HyperTekOrg/hyperstack/commit/a5c3160cedff5c678c23f266627d2fa7bebc890c))
* Missing import ([dc9a92d](https://github.com/HyperTekOrg/hyperstack/commit/dc9a92d7f685692746d8c6349da440b2e21028aa))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.6 to 0.1.8

## [0.1.7](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.6...v0.1.7) (2026-01-13)


### Bug Fixes

* Feature flag import ([a5c3160](https://github.com/HyperTekOrg/hyperstack/commit/a5c3160cedff5c678c23f266627d2fa7bebc890c))
* Missing import ([dc9a92d](https://github.com/HyperTekOrg/hyperstack/commit/dc9a92d7f685692746d8c6349da440b2e21028aa))

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.5...v0.1.6) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.5 to 0.1.6

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.4...hyperstack-server-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.4 to 0.1.5

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.2...hyperstack-server-v0.1.4) (2026-01-09)


### Miscellaneous Chores

* **hyperstack-server:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.2 to 0.1.4

## [0.1.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.1...hyperstack-server-v0.1.2) (2026-01-09)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.1 to 0.1.2

## [0.1.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-server-v0.1.0...hyperstack-server-v0.1.1) (2026-01-09)


### Bug Fixes

* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.0 to 0.1.1
