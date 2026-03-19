# Changelog

## [0.5.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.9...hyperstack-sdk-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.6...hyperstack-sdk-v0.5.9) (2026-03-19)


### Bug Fixes

* generate correct deserializers for i32/u32 fields ([86438e2](https://github.com/HyperTekOrg/hyperstack/commit/86438e22d3b8bb228a5a9485079211f92b000087))
* handle u64 integer precision loss across Rust-JS boundary ([e96e7fa](https://github.com/HyperTekOrg/hyperstack/commit/e96e7fa7172f520bd7ee88ed7582eda899c9f65b))
* handle u64 integer precision loss across Rust-JS boundary ([c3a3c69](https://github.com/HyperTekOrg/hyperstack/commit/c3a3c69587d9e6215aa5dfe4102739eef0ba8662))
* harden serde_utils integer deserialization and add missing i64 vec deserializers ([4afc52e](https://github.com/HyperTekOrg/hyperstack/commit/4afc52ee97020988cf8492c1c4922adc1db5a16c))

## [0.5.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.5...hyperstack-sdk-v0.5.6) (2026-03-19)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.4...hyperstack-sdk-v0.5.5) (2026-03-14)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.3...hyperstack-sdk-v0.5.4) (2026-03-14)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.2...hyperstack-sdk-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.1...hyperstack-sdk-v0.5.2) (2026-02-07)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.5.0...hyperstack-sdk-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.5.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.4.3...hyperstack-sdk-v0.5.0) (2026-02-06)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* Refactor API with Entity trait and connection management ([d5c010c](https://github.com/HyperTekOrg/hyperstack/commit/d5c010ca96e5a125943f742af424e2693e2afe47))
* **rust-sdk:** add deep merge for patches and expose raw patch data ([000f2a2](https://github.com/HyperTekOrg/hyperstack/commit/000f2a2092872aca689e0c00b17f069704f506f6))
* **rust-sdk:** add lazy streams with chainable filter/map operators ([e7fd71a](https://github.com/HyperTekOrg/hyperstack/commit/e7fd71a1a430a6e28db77fa79942c27aba29bf28))
* **rust-sdk:** add prelude module, rich updates, batch key watching, and DX improvements ([ac8d513](https://github.com/HyperTekOrg/hyperstack/commit/ac8d513b234544c165a24def685533c9cc31f8be))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/HyperTekOrg/hyperstack/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/HyperTekOrg/hyperstack/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **sdk:** add take/skip subscription filtering to Rust SDK ([8b55dad](https://github.com/HyperTekOrg/hyperstack/commit/8b55dad1615db82346eb60aec6990e2dd4ed1359))


### Bug Fixes

* Add hyperstack-sdk to CI ([e5f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/e5f3f4c7ac144000683297ec79efd946bf626b07))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Extract entity name from view path and unwrap item data ([289ce66](https://github.com/HyperTekOrg/hyperstack/commit/289ce667c1f28bd67c8179e7113c130a347d7f2a))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* **sdk:** derive Clone for ViewBuilder ([5212150](https://github.com/HyperTekOrg/hyperstack/commit/5212150c6386ec07d078ea49085290054fee6973))
* **sdk:** respect sort order for string fields in Rust SDK ([75f49b0](https://github.com/HyperTekOrg/hyperstack/commit/75f49b041f26bb1771bc688f9215fe740fac77b0))
* **sdk:** subscribe to broadcast before server subscription ([0ecb0f1](https://github.com/HyperTekOrg/hyperstack/commit/0ecb0f139032bf241225a0a3c6a8f63a6a0833e1))
* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/HyperTekOrg/hyperstack/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))
* Use BroadcastStream for proper async polling ([9dbbc42](https://github.com/HyperTekOrg/hyperstack/commit/9dbbc4251d788d889adcbdb31f4e22987b48e05b))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))

## [0.4.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.4.2...hyperstack-sdk-v0.4.3) (2026-02-03)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.4.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.4.1...hyperstack-sdk-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.4.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.4.0...hyperstack-sdk-v0.4.1) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.4.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.15...hyperstack-sdk-v0.4.0) (2026-01-31)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))
* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* Refactor API with Entity trait and connection management ([d5c010c](https://github.com/HyperTekOrg/hyperstack/commit/d5c010ca96e5a125943f742af424e2693e2afe47))
* **rust-sdk:** add deep merge for patches and expose raw patch data ([000f2a2](https://github.com/HyperTekOrg/hyperstack/commit/000f2a2092872aca689e0c00b17f069704f506f6))
* **rust-sdk:** add lazy streams with chainable filter/map operators ([e7fd71a](https://github.com/HyperTekOrg/hyperstack/commit/e7fd71a1a430a6e28db77fa79942c27aba29bf28))
* **rust-sdk:** add prelude module, rich updates, batch key watching, and DX improvements ([ac8d513](https://github.com/HyperTekOrg/hyperstack/commit/ac8d513b234544c165a24def685533c9cc31f8be))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/HyperTekOrg/hyperstack/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/HyperTekOrg/hyperstack/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **sdk:** add take/skip subscription filtering to Rust SDK ([8b55dad](https://github.com/HyperTekOrg/hyperstack/commit/8b55dad1615db82346eb60aec6990e2dd4ed1359))


### Bug Fixes

* Add hyperstack-sdk to CI ([e5f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/e5f3f4c7ac144000683297ec79efd946bf626b07))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Extract entity name from view path and unwrap item data ([289ce66](https://github.com/HyperTekOrg/hyperstack/commit/289ce667c1f28bd67c8179e7113c130a347d7f2a))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* **sdk:** respect sort order for string fields in Rust SDK ([75f49b0](https://github.com/HyperTekOrg/hyperstack/commit/75f49b041f26bb1771bc688f9215fe740fac77b0))
* **sdk:** subscribe to broadcast before server subscription ([0ecb0f1](https://github.com/HyperTekOrg/hyperstack/commit/0ecb0f139032bf241225a0a3c6a8f63a6a0833e1))
* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/HyperTekOrg/hyperstack/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))
* Use BroadcastStream for proper async polling ([9dbbc42](https://github.com/HyperTekOrg/hyperstack/commit/9dbbc4251d788d889adcbdb31f4e22987b48e05b))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))

## [0.3.15](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.14...hyperstack-sdk-v0.3.15) (2026-01-31)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.14](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.13...hyperstack-sdk-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.13](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.12...hyperstack-sdk-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.12](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.11...hyperstack-sdk-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.10...hyperstack-sdk-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.9...hyperstack-sdk-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.8...hyperstack-sdk-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.7...hyperstack-sdk-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.6...hyperstack-sdk-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.5...hyperstack-sdk-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.4...hyperstack-sdk-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.3...hyperstack-sdk-v0.3.4) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.2...hyperstack-sdk-v0.3.3) (2026-01-23)


### Features

* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **sdk:** add take/skip subscription filtering to Rust SDK ([8b55dad](https://github.com/HyperTekOrg/hyperstack/commit/8b55dad1615db82346eb60aec6990e2dd4ed1359))


### Bug Fixes

* **sdk:** respect sort order for string fields in Rust SDK ([75f49b0](https://github.com/HyperTekOrg/hyperstack/commit/75f49b041f26bb1771bc688f9215fe740fac77b0))
* **sdk:** subscribe to broadcast before server subscription ([0ecb0f1](https://github.com/HyperTekOrg/hyperstack/commit/0ecb0f139032bf241225a0a3c6a8f63a6a0833e1))

## [0.3.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.1...hyperstack-sdk-v0.3.2) (2026-01-20)


### Bug Fixes

* update SDKs to detect and decompress raw gzip binary frames ([2441b54](https://github.com/HyperTekOrg/hyperstack/commit/2441b54e7f3dbf53cea428e0aa6bcd81b9a06e60))

## [0.3.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.3.0...hyperstack-sdk-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.3.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.5...hyperstack-sdk-v0.3.0) (2026-01-20)


### Features

* add gzip compression for large WebSocket payloads ([cb694e9](https://github.com/HyperTekOrg/hyperstack/commit/cb694e9ef74ff99345e5f054820207f743d55e1d))

## [0.2.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.4...hyperstack-sdk-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.2.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.3...hyperstack-sdk-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.2.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.2...hyperstack-sdk-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* implement proper unsubscribe support across server and all SDKs ([81118cb](https://github.com/HyperTekOrg/hyperstack/commit/81118cb103720bdf8424cb71aab63d24d26e434c))
* **sdk:** add configurable store size limits with LRU eviction ([3e91148](https://github.com/HyperTekOrg/hyperstack/commit/3e91148b68c02b97da60dc9d12f1a45369895e7d))
* **sdk:** add snapshot frame support for batched initial data ([bf7cafe](https://github.com/HyperTekOrg/hyperstack/commit/bf7cafe9bcd0b8f255cd710b622d412476acb6a9))

## [0.2.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.1...hyperstack-sdk-v0.2.2) (2026-01-16)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.2.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.2.0...hyperstack-sdk-v0.2.1) (2026-01-16)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.2.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.11...hyperstack-sdk-v0.2.0) (2026-01-15)


### ⚠ BREAKING CHANGES

* Wire protocol simplified - list frames no longer wrap data in {id, order, item}. Data is now sent directly.

### Features

* **rust-sdk:** add deep merge for patches and expose raw patch data ([000f2a2](https://github.com/HyperTekOrg/hyperstack/commit/000f2a2092872aca689e0c00b17f069704f506f6))
* **rust-sdk:** add lazy streams with chainable filter/map operators ([e7fd71a](https://github.com/HyperTekOrg/hyperstack/commit/e7fd71a1a430a6e28db77fa79942c27aba29bf28))
* **rust-sdk:** add prelude module, rich updates, batch key watching, and DX improvements ([ac8d513](https://github.com/HyperTekOrg/hyperstack/commit/ac8d513b234544c165a24def685533c9cc31f8be))


### Bug Fixes

* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))


### Code Refactoring

* remove Mode::Kv and simplify websocket frame structure ([f1a2b81](https://github.com/HyperTekOrg/hyperstack/commit/f1a2b81f24eeda9a81b5fc0738ef78a5741b687b))

## [0.1.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.10...hyperstack-sdk-v0.1.11) (2026-01-14)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.1.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.9...hyperstack-sdk-v0.1.10) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.1.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.8...hyperstack-sdk-v0.1.9) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-sdk:** Synchronize hyperstack versions

## [0.1.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.6...hyperstack-sdk-v0.1.8) (2026-01-13)


### Features

* Refactor API with Entity trait and connection management ([d5c010c](https://github.com/HyperTekOrg/hyperstack/commit/d5c010ca96e5a125943f742af424e2693e2afe47))


### Bug Fixes

* Add hyperstack-sdk to CI ([e5f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/e5f3f4c7ac144000683297ec79efd946bf626b07))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Extract entity name from view path and unwrap item data ([289ce66](https://github.com/HyperTekOrg/hyperstack/commit/289ce667c1f28bd67c8179e7113c130a347d7f2a))
* Use BroadcastStream for proper async polling ([9dbbc42](https://github.com/HyperTekOrg/hyperstack/commit/9dbbc4251d788d889adcbdb31f4e22987b48e05b))

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.5...v0.1.6) (2026-01-13)


### Features

* Refactor API with Entity trait and connection management ([d5c010c](https://github.com/HyperTekOrg/hyperstack/commit/d5c010ca96e5a125943f742af424e2693e2afe47))


### Bug Fixes

* Add hyperstack-sdk to CI ([e5f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/e5f3f4c7ac144000683297ec79efd946bf626b07))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Extract entity name from view path and unwrap item data ([289ce66](https://github.com/HyperTekOrg/hyperstack/commit/289ce667c1f28bd67c8179e7113c130a347d7f2a))
* Use BroadcastStream for proper async polling ([9dbbc42](https://github.com/HyperTekOrg/hyperstack/commit/9dbbc4251d788d889adcbdb31f4e22987b48e05b))

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.4...hyperstack-sdk-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-sdk-v0.1.2...hyperstack-sdk-v0.1.4) (2026-01-09)


### Bug Fixes

* Add hyperstack-sdk to CI ([e5f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/e5f3f4c7ac144000683297ec79efd946bf626b07))
