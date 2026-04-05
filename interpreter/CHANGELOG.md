# Changelog

## [0.6.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.6.3...hyperstack-interpreter-v0.6.4) (2026-04-05)


### Features

* RuntimeResolver abstraction with enhanced caching ([9166434](https://github.com/HyperTekOrg/hyperstack/commit/9166434e52468f0d152781a4d81bb0db0fd9be21))


### Bug Fixes

* address resolver cache review issues ([900c08c](https://github.com/HyperTekOrg/hyperstack/commit/900c08c0244c9ae1fa7822a76dec15516e5faaf5))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.6.3 to 0.6.4

## [0.6.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.6.2...hyperstack-interpreter-v0.6.3) (2026-04-05)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.6.2 to 0.6.3

## [0.6.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.6.1...hyperstack-interpreter-v0.6.2) (2026-04-05)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.6.1 to 0.6.2

## [0.6.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.6.0...hyperstack-interpreter-v0.6.1) (2026-04-05)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.6.0 to 0.6.1

## [0.6.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.10...hyperstack-interpreter-v0.6.0) (2026-04-04)


### Features

* Add AST versioning system with automatic migration support ([997706b](https://github.com/HyperTekOrg/hyperstack/commit/997706b2854fc2e95427ef6b67b710db35ad86ac))
* Add AST versioning system with automatic migration support ([b62d08d](https://github.com/HyperTekOrg/hyperstack/commit/b62d08d99a579f323ea3f4a052fa90b83b269942))


### Bug Fixes

* Address code review feedback on AST versioning ([1791986](https://github.com/HyperTekOrg/hyperstack/commit/1791986fa0e37f5762a57c166ecbcf6be26bcb0b))
* Address code review feedback on into_latest, test assertions, and parsing ([da49bf9](https://github.com/HyperTekOrg/hyperstack/commit/da49bf9358362d27c307c8cc958a60b872ae2a2f))
* Clarify UnsupportedVersion error message to mention migration support ([51fde69](https://github.com/HyperTekOrg/hyperstack/commit/51fde6960103340b0fd18158f1e6d3a5a2b398ea))
* Make sync tests fail explicitly when source file not found ([c824e3d](https://github.com/HyperTekOrg/hyperstack/commit/c824e3dc74e102d57eb96485d9dad0c925d0bfc3))
* Remove Serialize derive from Versioned*Spec enums to prevent duplicate keys ([73ad7b4](https://github.com/HyperTekOrg/hyperstack/commit/73ad7b4b8519a30b7dcfca66badd05daf1eee085))
* tighten IDL lookup casing and derive_from diagnostics ([de5706d](https://github.com/HyperTekOrg/hyperstack/commit/de5706d59be3942a2f1612426f2b1ee5cb0ce817))
* Use CURRENT_AST_VERSION constant instead of hardcoded version ([5df9efe](https://github.com/HyperTekOrg/hyperstack/commit/5df9efe883d5880f6c440ae8e0577004041d53ca))
* Use CURRENT_AST_VERSION in test assertions instead of hardcoded string ([6809f1a](https://github.com/HyperTekOrg/hyperstack/commit/6809f1a632f464e2c1c6b175458fd51a24712acd))
* validate handler key resolution paths during macro expansion ([a069d1a](https://github.com/HyperTekOrg/hyperstack/commit/a069d1a2a6d0888e44bc9a9e0fbd1f6ba850b11d))
* validate join_on fields before IDL resolution ([6236f4b](https://github.com/HyperTekOrg/hyperstack/commit/6236f4b3e2c3fe438551a9bddc8613ddf5075f2c))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.10 to 0.6.0
    * hyperstack-idl bumped from 0.1.5 to 0.1.6

## [0.5.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.9...hyperstack-interpreter-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.9 to 0.5.10

## [0.5.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.8...hyperstack-interpreter-v0.5.9) (2026-03-19)


### Bug Fixes

* generate correct deserializers for i32/u32 fields ([86438e2](https://github.com/HyperTekOrg/hyperstack/commit/86438e22d3b8bb228a5a9485079211f92b000087))
* handle u64 integer precision loss across Rust-JS boundary ([e96e7fa](https://github.com/HyperTekOrg/hyperstack/commit/e96e7fa7172f520bd7ee88ed7582eda899c9f65b))
* handle u64 integer precision loss across Rust-JS boundary ([c3a3c69](https://github.com/HyperTekOrg/hyperstack/commit/c3a3c69587d9e6215aa5dfe4102739eef0ba8662))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.8 to 0.5.9

## [0.5.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.7...hyperstack-interpreter-v0.5.8) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.7 to 0.5.8
    * hyperstack-idl bumped from 0.1.4 to 0.1.5

## [0.5.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.6...hyperstack-interpreter-v0.5.7) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.6 to 0.5.7
    * hyperstack-idl bumped from 0.1.3 to 0.1.4

## [0.5.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.5...hyperstack-interpreter-v0.5.6) (2026-03-19)


### Features

* add Keccak256 hashing and slot hash caching ([768c407](https://github.com/HyperTekOrg/hyperstack/commit/768c407adce786003c30c634432279456baa5102))
* Add rt-multi-thread ([a426edb](https://github.com/HyperTekOrg/hyperstack/commit/a426edbd4fa8f32b031038aaf61d7f4e1b94c8f7))
* add skip_resolvers mechanism for stale data reprocessing ([a11c669](https://github.com/HyperTekOrg/hyperstack/commit/a11c6695f66f8951e6aa5635649de0047536be00))


### Bug Fixes

* add missing return to AbortIfNullKey opcode ([8d98359](https://github.com/HyperTekOrg/hyperstack/commit/8d983597d021c0617f6b8efb27e010d5d03cd277))
* align Steel discriminant size with get_discriminator return value ([4d283f2](https://github.com/HyperTekOrg/hyperstack/commit/4d283f2bd749690eb7b79e1c80e0447c79b35d8d))
* Core interpreter and server improvements ([b05ae9b](https://github.com/HyperTekOrg/hyperstack/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* correct resolver output types and schema generation in TypeScript emitter ([b37ef43](https://github.com/HyperTekOrg/hyperstack/commit/b37ef43e93d5c20903b01831424d94f6bb86bd72))
* correct SlotHashResolver TypeScript types to match actual return values ([358d9bd](https://github.com/HyperTekOrg/hyperstack/commit/358d9bdc5f53ae1e888df9630885f1baeaaa22d1))
* derive state_id from bytecode routing for PDA cache ([927b364](https://github.com/HyperTekOrg/hyperstack/commit/927b364a996ce9047641a35a66d346485feac4d8))
* prevent panic in SlotHash resolver when using current_thread runtime ([3525397](https://github.com/HyperTekOrg/hyperstack/commit/3525397a8adec127ac261b1c6e6cab2617f1217b))
* prevent silent byte truncation in json_array_to_bytes ([10ff8aa](https://github.com/HyperTekOrg/hyperstack/commit/10ff8aa10cec3f31707ebb39fd5a0fd3b9e06737))
* reduce MAX_CACHE_SIZE from 50k to 1k ([462230f](https://github.com/HyperTekOrg/hyperstack/commit/462230f814d08acd985fc13b3f17820ff6e1ac94))
* resolve clippy warnings across workspace ([c19d1ec](https://github.com/HyperTekOrg/hyperstack/commit/c19d1ec5926ee9099c6ab4254bde30b2c794e27f))
* restore cross-account lookup resolution at round boundaries ([0af0835](https://github.com/HyperTekOrg/hyperstack/commit/0af0835ea6c7d35c5c1efd6f63899706dd85ab91))
* serialize pre_reveal_rng as string to avoid JS precision loss ([9f07692](https://github.com/HyperTekOrg/hyperstack/commit/9f0769209f382291d3fd8119d8fc39549d0314d1))
* serialize u64-from-bytes computed fields as strings to avoid JS precision loss ([1f67a7a](https://github.com/HyperTekOrg/hyperstack/commit/1f67a7a9589823262a972ea01c71ad4e04e24ffe))
* TypeScript generator now correctly types computed fields using resolver output types ([5d28937](https://github.com/HyperTekOrg/hyperstack/commit/5d28937796b8c7672cff02c1f648ac0f37f48c44))
* use BTreeMap for slot hash cache to ensure oldest entries are evicted ([94249ba](https://github.com/HyperTekOrg/hyperstack/commit/94249badb4a1a1d0af860048c8ca1bc758818c7b))
* use BTreeMap in ResolverRegistry to ensure deterministic SDK output ([0129f75](https://github.com/HyperTekOrg/hyperstack/commit/0129f75ad443b9340a38f8da256ab6f8c977d1e2))
* wrap slot hash bytes in object to match SlotHashBytes schema ([247ffa3](https://github.com/HyperTekOrg/hyperstack/commit/247ffa37b3656677e4ede419c22ab99c2b8bf077))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.5 to 0.5.6
    * hyperstack-idl bumped from 0.1.2 to 0.1.3

## [0.5.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.4...hyperstack-interpreter-v0.5.5) (2026-03-14)


### Bug Fixes

* Add version constraint to hyperstack-idl dependency ([917bf5a](https://github.com/HyperTekOrg/hyperstack/commit/917bf5abe6242048ba9f7af0055d999ccfcb8692))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.4 to 0.5.5
    * hyperstack-idl bumped from 0.1 to 0.1.2

## [0.5.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.3...hyperstack-interpreter-v0.5.4) (2026-03-14)


### Features

* **interpreter:** various compiler and VM improvements ([23503ac](https://github.com/HyperTekOrg/hyperstack/commit/23503acaa69736d7487d5a9cdd01239d8f86776d))
* misc compiler, VM, and IDL improvements ([2d6aea3](https://github.com/HyperTekOrg/hyperstack/commit/2d6aea373e43c84e3a07ecef7d9dab004a0b8c1c))


### Bug Fixes

* add hashMap IDL type variant to support Metaplex token metadata SDK generation ([21a81ad](https://github.com/HyperTekOrg/hyperstack/commit/21a81ada3b8325295428de6c0cb5eaaedcc4f215))
* address code review issues in interpreter VM ([f8a5223](https://github.com/HyperTekOrg/hyperstack/commit/f8a5223807df4b5dec0fbf0456c06ab67bfeb852))
* improve error handling in UrlResolverClient for out-of-bounds access ([540aeef](https://github.com/HyperTekOrg/hyperstack/commit/540aeefed35be873067f41fc14daa205c4e959d9))
* **interpreter:** zero-variant enum dedup guard escape ([c9fb961](https://github.com/HyperTekOrg/hyperstack/commit/c9fb961f380002516ae845eaf743e15ea8e47c3c))
* track only actually emitted enum types to prevent over-eager deduplication ([0bdd7d4](https://github.com/HyperTekOrg/hyperstack/commit/0bdd7d4d0ab2c1c3b31a50ada22205fe0e95f9f7))
* track only actually emitted enum types to prevent over-eager deduplication ([a8a62cf](https://github.com/HyperTekOrg/hyperstack/commit/a8a62cfc4726b2be7c64b346a1d42f7307d977cb))


### Performance Improvements

* parallelize URL batch resolution using join_all ([e252a8e](https://github.com/HyperTekOrg/hyperstack/commit/e252a8ea2f91b7b7e0dc266586d670e98bae0cfb))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.3 to 0.5.4

## [0.5.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.2...hyperstack-interpreter-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.2 to 0.5.3

## [0.5.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.1...hyperstack-interpreter-v0.5.2) (2026-02-07)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.1 to 0.5.2

## [0.5.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.5.0...hyperstack-interpreter-v0.5.1) (2026-02-06)


### Bug Fixes

* **ci:** switch interpreter reqwest to rustls-tls for aarch64 cross-build ([ffe792d](https://github.com/HyperTekOrg/hyperstack/commit/ffe792d7d0b8eb6c09ea068afc0bc31965e5049e))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.5.0 to 0.5.1

## [0.5.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.4.3...hyperstack-interpreter-v0.5.0) (2026-02-06)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/HyperTekOrg/hyperstack/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))
* add CompletedSchema with required fields and TokenMetadata builtin type ([ba48532](https://github.com/HyperTekOrg/hyperstack/commit/ba485328e142a00bb2b01b9ae6ab838ab53cbd25))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add PDA and Instruction type definitions for SDK code generation ([6a74067](https://github.com/HyperTekOrg/hyperstack/commit/6a7406751d865b9e4caef3749779217e305636a5))
* add PDA DSL imports to generated TypeScript SDK ([5f033b6](https://github.com/HyperTekOrg/hyperstack/commit/5f033b62c7335b423ebff6aafca937e8bde81942))
* add resolver support with resolve attribute and computed field methods ([aed45c8](https://github.com/HyperTekOrg/hyperstack/commit/aed45c81477267cb6a005d439ee30400c1e24e5c))
* add resolver-provided computed methods (ui_amount, raw_amount) ([3729710](https://github.com/HyperTekOrg/hyperstack/commit/372971072f597625905a1365da4ec2e5e8d9d978))
* Add Rust codegen module for SDK generation ([24fac1c](https://github.com/HyperTekOrg/hyperstack/commit/24fac1cc894729ec44596ddadb969fce79dafbd4))
* add stop attribute for conditional field population ([441cfef](https://github.com/HyperTekOrg/hyperstack/commit/441cfef39760af9b3af78b992d1269eaeecd2f99))
* add token metadata resolver with DAS API integration ([ced55fe](https://github.com/HyperTekOrg/hyperstack/commit/ced55fe4981e0f36abbebe277438eb17ea01b519))
* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))
* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **computed:** add __slot and __timestamp context access in computed fields ([b707f95](https://github.com/HyperTekOrg/hyperstack/commit/b707f95058f74ae52c6003bc2d68e948e657e70e))
* **computed:** support resolver computed fields inside array .map() closures ([1f571c0](https://github.com/HyperTekOrg/hyperstack/commit/1f571c08caa2da41192c3b45399f1abe747dda10))
* **interpreter:** add canonical logging and OpenTelemetry metrics ([e07de40](https://github.com/HyperTekOrg/hyperstack/commit/e07de40b0a4523dea4958b485b493aed8bbc20b6))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **interpreter:** implement granular dirty tracking for field emissions ([c490c9c](https://github.com/HyperTekOrg/hyperstack/commit/c490c9ccb912f872ab92fadbfab674fc3ba56090))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* **sdk:** add default export to generated TypeScript SDK ([b24f39f](https://github.com/HyperTekOrg/hyperstack/commit/b24f39f0899bfe53d4307f5b0fa06733178006e2))
* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* always generate list views in multi-entity SDK codegen ([bc81e77](https://github.com/HyperTekOrg/hyperstack/commit/bc81e776088c9d18580a4f35d7a2a14a8e003e27))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/HyperTekOrg/hyperstack/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* emit canonical log data as structured field for OTEL/Axiom parsing ([247e807](https://github.com/HyperTekOrg/hyperstack/commit/247e807019b793a2194d3c9d670c4ab2a01615ac))
* flush queued account updates when lookup index is populated ([8a09279](https://github.com/HyperTekOrg/hyperstack/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))
* Handle root section case-insensitively and flatten fields ([1cf7110](https://github.com/HyperTekOrg/hyperstack/commit/1cf7110a28450a63b607007237ab46a9a6125bf5))
* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* **interpreter:** add bounded LRU caches to prevent unbounded memory growth ([4d9042e](https://github.com/HyperTekOrg/hyperstack/commit/4d9042e2ca115fe41827fcdeac037bea8a1b5589))
* **interpreter:** make all TypeScript interface fields optional for patch semantics ([d2d959c](https://github.com/HyperTekOrg/hyperstack/commit/d2d959c2d02ceff4c2cf0c76d147df770222cf25))
* **interpreter:** prevent duplicate unmapped fields in TypeScript generation ([a7b2870](https://github.com/HyperTekOrg/hyperstack/commit/a7b28709c67994b09eef973e0a14e9be965e3367))
* **interpreter:** queue instruction events when PDA lookup fails ([c5779b4](https://github.com/HyperTekOrg/hyperstack/commit/c5779b48a6ed31670fdbf2884e2748488adadc0c))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* remove needless borrow in rust codegen ([398047b](https://github.com/HyperTekOrg/hyperstack/commit/398047b5c8308ccb05c4426ffecbdd1daf6d6f7b))
* remove unused protobuf compilation that broke Windows CI ([284a2fc](https://github.com/HyperTekOrg/hyperstack/commit/284a2fc2d4e587610205dd510327092dac73a115))
* remove unused serde_helpers module from Rust SDK generator ([57e2d13](https://github.com/HyperTekOrg/hyperstack/commit/57e2d13dbcd9bbceaa5cd5bbaf8e1d37f7df99a7))
* reorder temporal index update after ReadOrInitState and handle missing state tables ([b086676](https://github.com/HyperTekOrg/hyperstack/commit/b0866765a01f21d75de860902899b8e3613eb96e))
* resolve clippy warnings across workspace ([565d92b](https://github.com/HyperTekOrg/hyperstack/commit/565d92b91552d92262cfaeca9674d0ad4d3f6b5d))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/HyperTekOrg/hyperstack/commit/10c1611282babb70bbe70d19fb599c83654caa6c))
* separate recency handling for account vs instruction updates ([e0d6a6e](https://github.com/HyperTekOrg/hyperstack/commit/e0d6a6ef756a4f1446b4b82d7bb78b166cb08264))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))
* use derive macro for LogLevel Default impl ([c2e30ed](https://github.com/HyperTekOrg/hyperstack/commit/c2e30ed0e1968e00f1b789e7d2cfb04dd4cb4867))
* use field init shorthand in generated Rust SDK code ([ac7a5b1](https://github.com/HyperTekOrg/hyperstack/commit/ac7a5b1d963b5b133d5cc1486b77e73d1e4ac350))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.4.3 to 0.5.0

## [0.4.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.4.2...hyperstack-interpreter-v0.4.3) (2026-02-03)


### Features

* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* always generate list views in multi-entity SDK codegen ([bc81e77](https://github.com/HyperTekOrg/hyperstack/commit/bc81e776088c9d18580a4f35d7a2a14a8e003e27))
* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/HyperTekOrg/hyperstack/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* reorder temporal index update after ReadOrInitState and handle missing state tables ([b086676](https://github.com/HyperTekOrg/hyperstack/commit/b0866765a01f21d75de860902899b8e3613eb96e))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/HyperTekOrg/hyperstack/commit/10c1611282babb70bbe70d19fb599c83654caa6c))
* use field init shorthand in generated Rust SDK code ([ac7a5b1](https://github.com/HyperTekOrg/hyperstack/commit/ac7a5b1d963b5b133d5cc1486b77e73d1e4ac350))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.4.2 to 0.4.3

## [0.4.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.4.1...hyperstack-interpreter-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.4.1 to 0.4.2

## [0.4.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.4.0...hyperstack-interpreter-v0.4.1) (2026-02-01)


### Features

* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/HyperTekOrg/hyperstack/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))


### Bug Fixes

* flush queued account updates when lookup index is populated ([8a09279](https://github.com/HyperTekOrg/hyperstack/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.4.0 to 0.4.1

## [0.4.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.15...hyperstack-interpreter-v0.4.0) (2026-01-31)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add Rust codegen module for SDK generation ([24fac1c](https://github.com/HyperTekOrg/hyperstack/commit/24fac1cc894729ec44596ddadb969fce79dafbd4))
* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))
* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **interpreter:** add canonical logging and OpenTelemetry metrics ([e07de40](https://github.com/HyperTekOrg/hyperstack/commit/e07de40b0a4523dea4958b485b493aed8bbc20b6))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **interpreter:** implement granular dirty tracking for field emissions ([c490c9c](https://github.com/HyperTekOrg/hyperstack/commit/c490c9ccb912f872ab92fadbfab674fc3ba56090))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* **sdk:** add default export to generated TypeScript SDK ([b24f39f](https://github.com/HyperTekOrg/hyperstack/commit/b24f39f0899bfe53d4307f5b0fa06733178006e2))
* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* emit canonical log data as structured field for OTEL/Axiom parsing ([247e807](https://github.com/HyperTekOrg/hyperstack/commit/247e807019b793a2194d3c9d670c4ab2a01615ac))
* Handle root section case-insensitively and flatten fields ([1cf7110](https://github.com/HyperTekOrg/hyperstack/commit/1cf7110a28450a63b607007237ab46a9a6125bf5))
* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* **interpreter:** add bounded LRU caches to prevent unbounded memory growth ([4d9042e](https://github.com/HyperTekOrg/hyperstack/commit/4d9042e2ca115fe41827fcdeac037bea8a1b5589))
* **interpreter:** make all TypeScript interface fields optional for patch semantics ([d2d959c](https://github.com/HyperTekOrg/hyperstack/commit/d2d959c2d02ceff4c2cf0c76d147df770222cf25))
* **interpreter:** prevent duplicate unmapped fields in TypeScript generation ([a7b2870](https://github.com/HyperTekOrg/hyperstack/commit/a7b28709c67994b09eef973e0a14e9be965e3367))
* **interpreter:** queue instruction events when PDA lookup fails ([c5779b4](https://github.com/HyperTekOrg/hyperstack/commit/c5779b48a6ed31670fdbf2884e2748488adadc0c))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))
* remove needless borrow in rust codegen ([398047b](https://github.com/HyperTekOrg/hyperstack/commit/398047b5c8308ccb05c4426ffecbdd1daf6d6f7b))
* remove unused protobuf compilation that broke Windows CI ([284a2fc](https://github.com/HyperTekOrg/hyperstack/commit/284a2fc2d4e587610205dd510327092dac73a115))
* remove unused serde_helpers module from Rust SDK generator ([57e2d13](https://github.com/HyperTekOrg/hyperstack/commit/57e2d13dbcd9bbceaa5cd5bbaf8e1d37f7df99a7))
* separate recency handling for account vs instruction updates ([e0d6a6e](https://github.com/HyperTekOrg/hyperstack/commit/e0d6a6ef756a4f1446b4b82d7bb78b166cb08264))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))
* use derive macro for LogLevel Default impl ([c2e30ed](https://github.com/HyperTekOrg/hyperstack/commit/c2e30ed0e1968e00f1b789e7d2cfb04dd4cb4867))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.15 to 0.4.0

## [0.3.15](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.14...hyperstack-interpreter-v0.3.15) (2026-01-31)


### Features

* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))


### Bug Fixes

* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* separate recency handling for account vs instruction updates ([e0d6a6e](https://github.com/HyperTekOrg/hyperstack/commit/e0d6a6ef756a4f1446b4b82d7bb78b166cb08264))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.14 to 0.3.15

## [0.3.14](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.13...hyperstack-interpreter-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.13 to 0.3.14

## [0.3.13](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.12...hyperstack-interpreter-v0.3.13) (2026-01-28)


### Bug Fixes

* remove unused protobuf compilation that broke Windows CI ([284a2fc](https://github.com/HyperTekOrg/hyperstack/commit/284a2fc2d4e587610205dd510327092dac73a115))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.12 to 0.3.13

## [0.3.12](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.11...hyperstack-interpreter-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.11 to 0.3.12

## [0.3.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.10...hyperstack-interpreter-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.10 to 0.3.11

## [0.3.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.9...hyperstack-interpreter-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.9 to 0.3.10

## [0.3.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.8...hyperstack-interpreter-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.8 to 0.3.9

## [0.3.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.7...hyperstack-interpreter-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.7 to 0.3.8

## [0.3.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.6...hyperstack-interpreter-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.6 to 0.3.7

## [0.3.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.5...hyperstack-interpreter-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.5 to 0.3.6

## [0.3.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.4...hyperstack-interpreter-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.4 to 0.3.5

## [0.3.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.3...hyperstack-interpreter-v0.3.4) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.3 to 0.3.4

## [0.3.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.2...hyperstack-interpreter-v0.3.3) (2026-01-23)


### Features

* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* add unified Views API to Rust SDK ([97afb97](https://github.com/HyperTekOrg/hyperstack/commit/97afb97f1f9d21030ba400fef5a7727d674a93e0))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))


### Bug Fixes

* **interpreter:** prevent duplicate unmapped fields in TypeScript generation ([a7b2870](https://github.com/HyperTekOrg/hyperstack/commit/a7b28709c67994b09eef973e0a14e9be965e3367))
* **interpreter:** queue instruction events when PDA lookup fails ([c5779b4](https://github.com/HyperTekOrg/hyperstack/commit/c5779b48a6ed31670fdbf2884e2748488adadc0c))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.2 to 0.3.3

## [0.3.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.1...hyperstack-interpreter-v0.3.2) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.1 to 0.3.2

## [0.3.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.3.0...hyperstack-interpreter-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.3.0 to 0.3.1

## [0.3.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.5...hyperstack-interpreter-v0.3.0) (2026-01-20)


### Features

* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **sdk:** add default export to generated TypeScript SDK ([b24f39f](https://github.com/HyperTekOrg/hyperstack/commit/b24f39f0899bfe53d4307f5b0fa06733178006e2))


### Bug Fixes

* remove needless borrow in rust codegen ([398047b](https://github.com/HyperTekOrg/hyperstack/commit/398047b5c8308ccb05c4426ffecbdd1daf6d6f7b))
* remove unused serde_helpers module from Rust SDK generator ([57e2d13](https://github.com/HyperTekOrg/hyperstack/commit/57e2d13dbcd9bbceaa5cd5bbaf8e1d37f7df99a7))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.5 to 0.3.0

## [0.2.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.4...hyperstack-interpreter-v0.2.5) (2026-01-19)


### Bug Fixes

* emit canonical log data as structured field for OTEL/Axiom parsing ([247e807](https://github.com/HyperTekOrg/hyperstack/commit/247e807019b793a2194d3c9d670c4ab2a01615ac))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.4 to 0.2.5

## [0.2.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.3...hyperstack-interpreter-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.3 to 0.2.4

## [0.2.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.2...hyperstack-interpreter-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* **interpreter:** add canonical logging and OpenTelemetry metrics ([e07de40](https://github.com/HyperTekOrg/hyperstack/commit/e07de40b0a4523dea4958b485b493aed8bbc20b6))
* **interpreter:** implement granular dirty tracking for field emissions ([c490c9c](https://github.com/HyperTekOrg/hyperstack/commit/c490c9ccb912f872ab92fadbfab674fc3ba56090))


### Bug Fixes

* increase cache limits 5x to reduce eviction pressure ([49ed3c4](https://github.com/HyperTekOrg/hyperstack/commit/49ed3c4148fbbdc8ad61817ebf31d5989552b181))
* **interpreter:** add bounded LRU caches to prevent unbounded memory growth ([4d9042e](https://github.com/HyperTekOrg/hyperstack/commit/4d9042e2ca115fe41827fcdeac037bea8a1b5589))
* reduce memory allocations in VM and projector ([d265a4f](https://github.com/HyperTekOrg/hyperstack/commit/d265a4fc358799f33d549412932cce9919b5dc56))
* use derive macro for LogLevel Default impl ([c2e30ed](https://github.com/HyperTekOrg/hyperstack/commit/c2e30ed0e1968e00f1b789e7d2cfb04dd4cb4867))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.2 to 0.2.3

## [0.2.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.1...hyperstack-interpreter-v0.2.2) (2026-01-16)


### Features

* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))


### Bug Fixes

* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.1 to 0.2.2

## [0.2.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.2.0...hyperstack-interpreter-v0.2.1) (2026-01-16)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.2.0 to 0.2.1

## [0.2.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.11...hyperstack-interpreter-v0.2.0) (2026-01-15)


### Bug Fixes

* **interpreter:** make all TypeScript interface fields optional for patch semantics ([d2d959c](https://github.com/HyperTekOrg/hyperstack/commit/d2d959c2d02ceff4c2cf0c76d147df770222cf25))
* remove deprecated kv mode from SDKs and documentation ([2097af0](https://github.com/HyperTekOrg/hyperstack/commit/2097af05165eed4a7d9b6ef4ede1b5722ab90215))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.11 to 0.2.0

## [0.1.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.10...hyperstack-interpreter-v0.1.11) (2026-01-14)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.10 to 0.1.11

## [0.1.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.9...hyperstack-interpreter-v0.1.10) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.9 to 0.1.10

## [0.1.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.8...hyperstack-interpreter-v0.1.9) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.8 to 0.1.9

## [0.1.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.6...hyperstack-interpreter-v0.1.8) (2026-01-13)


### Features

* Add Rust codegen module for SDK generation ([24fac1c](https://github.com/HyperTekOrg/hyperstack/commit/24fac1cc894729ec44596ddadb969fce79dafbd4))
* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Handle root section case-insensitively and flatten fields ([1cf7110](https://github.com/HyperTekOrg/hyperstack/commit/1cf7110a28450a63b607007237ab46a9a6125bf5))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.6 to 0.1.8

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.5...v0.1.6) (2026-01-13)


### Features

* Add Rust codegen module for SDK generation ([24fac1c](https://github.com/HyperTekOrg/hyperstack/commit/24fac1cc894729ec44596ddadb969fce79dafbd4))
* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Handle root section case-insensitively and flatten fields ([1cf7110](https://github.com/HyperTekOrg/hyperstack/commit/1cf7110a28450a63b607007237ab46a9a6125bf5))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.5 to 0.1.6

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.4...hyperstack-interpreter-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.4 to 0.1.5

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.2...hyperstack-interpreter-v0.1.4) (2026-01-09)


### Miscellaneous Chores

* **hyperstack-interpreter:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.2 to 0.1.4

## [0.1.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.1...hyperstack-interpreter-v0.1.2) (2026-01-09)


### Bug Fixes

* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-macros bumped from 0.1.1 to 0.1.2

## [0.1.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-interpreter-v0.1.0...hyperstack-interpreter-v0.1.1) (2026-01-09)


### Features

* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))


### Bug Fixes

* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-spec-macros bumped from 0.1.0 to 0.1.1
