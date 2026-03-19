# Changelog

## [0.5.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.9...hyperstack-macros-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.5.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.8...hyperstack-macros-v0.5.9) (2026-03-19)


### Bug Fixes

* handle u64 integer precision loss across Rust-JS boundary ([e96e7fa](https://github.com/HyperTekOrg/hyperstack/commit/e96e7fa7172f520bd7ee88ed7582eda899c9f65b))

## [0.5.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.7...hyperstack-macros-v0.5.8) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-idl bumped from 0.1.4 to 0.1.5

## [0.5.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.6...hyperstack-macros-v0.5.7) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-idl bumped from 0.1.3 to 0.1.4

## [0.5.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.5...hyperstack-macros-v0.5.6) (2026-03-19)


### Features

* add Keccak256 hashing and slot hash caching ([768c407](https://github.com/HyperTekOrg/hyperstack/commit/768c407adce786003c30c634432279456baa5102))
* improve slot scheduler with notification-based waiting and enhanced logging ([0b8681e](https://github.com/HyperTekOrg/hyperstack/commit/0b8681eb8cf399e09140495c749872dcae91b1c1))


### Bug Fixes

* cache account data for all entity states that route an event type ([1b59633](https://github.com/HyperTekOrg/hyperstack/commit/1b59633f542dad2e140e914ff3e787019dd75944))
* conditionally enable TLS only for https/grpcs endpoints in slot subscription ([8c8875b](https://github.com/HyperTekOrg/hyperstack/commit/8c8875b4c587f282e6f3993c1651aebe816004c1))
* Core interpreter and server improvements ([b05ae9b](https://github.com/HyperTekOrg/hyperstack/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* correct resolver output types and schema generation in TypeScript emitter ([b37ef43](https://github.com/HyperTekOrg/hyperstack/commit/b37ef43e93d5c20903b01831424d94f6bb86bd72))
* derive state_id from bytecode routing for PDA cache ([927b364](https://github.com/HyperTekOrg/hyperstack/commit/927b364a996ce9047641a35a66d346485feac4d8))
* keep gRPC subscription sender alive to prevent stream termination ([42460d2](https://github.com/HyperTekOrg/hyperstack/commit/42460d2c2cf5dbb582da78e73100ab42b6213786))
* prevent integer overflow in SlotHashes sysvar parsing ([99427f6](https://github.com/HyperTekOrg/hyperstack/commit/99427f6d6ca03c691c9556f1dc69065ca5b2b1e0))
* prevent panic in SlotHash resolver when using current_thread runtime ([3525397](https://github.com/HyperTekOrg/hyperstack/commit/3525397a8adec127ac261b1c6e6cab2617f1217b))
* remove unnecessary async from parse_and_cache_slot_hashes ([03e00aa](https://github.com/HyperTekOrg/hyperstack/commit/03e00aa4248be85146e60c133cc88130c240dcc1))
* resolve clippy warnings across workspace ([c19d1ec](https://github.com/HyperTekOrg/hyperstack/commit/c19d1ec5926ee9099c6ab4254bde30b2c794e27f))
* restore cross-account lookup resolution at round boundaries ([0af0835](https://github.com/HyperTekOrg/hyperstack/commit/0af0835ea6c7d35c5c1efd6f63899706dd85ab91))
* serialize u64-from-bytes computed fields as strings to avoid JS precision loss ([1f67a7a](https://github.com/HyperTekOrg/hyperstack/commit/1f67a7a9589823262a972ea01c71ad4e04e24ffe))
* use dynamic-length discriminator slice in generated code ([d67b727](https://github.com/HyperTekOrg/hyperstack/commit/d67b727e3cb34f2f9eb66d75f15add86bc8dbbd6))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-idl bumped from 0.1.2 to 0.1.3

## [0.5.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.4...hyperstack-macros-v0.5.5) (2026-03-14)


### Bug Fixes

* Add version constraint to hyperstack-idl dependency ([917bf5a](https://github.com/HyperTekOrg/hyperstack/commit/917bf5abe6242048ba9f7af0055d999ccfcb8692))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-idl bumped from 0.1 to 0.1.2

## [0.5.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.3...hyperstack-macros-v0.5.4) (2026-03-14)


### Features

* add HashMap type support, auto-derive discriminators, and to_json_value codegen ([7e81e3d](https://github.com/HyperTekOrg/hyperstack/commit/7e81e3d9802ad25987439841b7e4dd0f45232386))
* **macros:** add CPI event support with camelCase field handling ([cd856fa](https://github.com/HyperTekOrg/hyperstack/commit/cd856fa19606481e3f6775d8a8e591d4782f7913))
* **macros:** enhance Vixen runtime with improved tracing and queue handling ([d8b46ea](https://github.com/HyperTekOrg/hyperstack/commit/d8b46ea369ce5669f4cd357e78c83132c758a66d))
* **macros:** support packed structs, large arrays, and stream spec improvements ([b337d09](https://github.com/HyperTekOrg/hyperstack/commit/b337d09c1b45bd32604d65e05c96d0dc5dc8ea7a))
* misc compiler, VM, and IDL improvements ([2d6aea3](https://github.com/HyperTekOrg/hyperstack/commit/2d6aea373e43c84e3a07ecef7d9dab004a0b8c1c))


### Bug Fixes

* - Instantiate UrlResolverClient once at startup on VmHandler instead of per-request ([f346bfa](https://github.com/HyperTekOrg/hyperstack/commit/f346bfa2131d2cb1ae3ea98e03b25e9351f25eca))
* add serde derives to IDL-generated account and custom types ([cada9f5](https://github.com/HyperTekOrg/hyperstack/commit/cada9f53f00112b7048f62743dd9aea0ffb07baa))
* address code review issues in macro codegen ([908c9ff](https://github.com/HyperTekOrg/hyperstack/commit/908c9ff757976336483b89c24fd8f75640f58029))
* eliminate duplicate url_path qualification logic in sections.rs ([6ed8e94](https://github.com/HyperTekOrg/hyperstack/commit/6ed8e948f4490e2aa659c7c25812fdcc968ac64e))
* **macros:** prevent cross-entity instruction hook contamination ([ff65ed0](https://github.com/HyperTekOrg/hyperstack/commit/ff65ed0259a18425c15d250dcc30ad19ab093dc7))
* panic on missing event type definition instead of silent empty struct ([cdb0d39](https://github.com/HyperTekOrg/hyperstack/commit/cdb0d391903c1382eca2b81bd9867172cf028ca3))
* re-queue URL resolver requests on empty URL or failure ([dc204a2](https://github.com/HyperTekOrg/hyperstack/commit/dc204a2cfae505b7cf603b4eb56b39e0d8277f97))
* remove redundant is_cpi_event variable shadowing ([d338399](https://github.com/HyperTekOrg/hyperstack/commit/d338399339c231ff150bc9b829e2ea9296dd48ee))
* resolve compilation errors in hyperstack-macros ([acd5a31](https://github.com/HyperTekOrg/hyperstack/commit/acd5a31d63997426f23b9cd42f26fcef7ff59f97))
* resolve instruction field prefix for camelCase IDL instruction names ([df75acf](https://github.com/HyperTekOrg/hyperstack/commit/df75acfca07371ffbda8f410bcc777a42f62627a))
* silence unused variable warning in stream_spec ([3fee1b1](https://github.com/HyperTekOrg/hyperstack/commit/3fee1b124e54a55c886844e9fd0c3bc82bfa1995))
* use transaction_accounts_include in prefilter builder ([2df88f8](https://github.com/HyperTekOrg/hyperstack/commit/2df88f890e7549a2d5dadee9703e2cb31cd866ef))
* validate HTTP method input in ResolveAttributeArgs parser to support only 'GET' and 'POST' ([fb80a48](https://github.com/HyperTekOrg/hyperstack/commit/fb80a48ebf603a13d72d67cd6aedb602d5818cc3))

## [0.5.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.2...hyperstack-macros-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.5.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.1...hyperstack-macros-v0.5.2) (2026-02-07)


### Bug Fixes

* batch primary and resolver mutations to avoid duplicate frames ([160feee](https://github.com/HyperTekOrg/hyperstack/commit/160feee69c8e9429939f13cdf81359278750f241))

## [0.5.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.5.0...hyperstack-macros-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.5.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.4.3...hyperstack-macros-v0.5.0) (2026-02-06)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/HyperTekOrg/hyperstack/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add deterministic sorting ([e775f59](https://github.com/HyperTekOrg/hyperstack/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* Add PDA and Instruction type definitions for SDK code generation ([6a74067](https://github.com/HyperTekOrg/hyperstack/commit/6a7406751d865b9e4caef3749779217e305636a5))
* add PDA resolution hooks and multi-program stack support ([f4210a0](https://github.com/HyperTekOrg/hyperstack/commit/f4210a0a665f75f60fce09634e9ddbdde3f6898c))
* add resolver support with resolve attribute and computed field methods ([aed45c8](https://github.com/HyperTekOrg/hyperstack/commit/aed45c81477267cb6a005d439ee30400c1e24e5c))
* add resolver-declared transforms to #[map] attribute ([8f35bff](https://github.com/HyperTekOrg/hyperstack/commit/8f35bff8c3fa811e4bfb50a1c431ecc09822e2d2))
* add resolver-provided computed methods (ui_amount, raw_amount) ([3729710](https://github.com/HyperTekOrg/hyperstack/commit/372971072f597625905a1365da4ec2e5e8d9d978))
* add stop attribute for conditional field population ([441cfef](https://github.com/HyperTekOrg/hyperstack/commit/441cfef39760af9b3af78b992d1269eaeecd2f99))
* add token metadata resolver with DAS API integration ([ced55fe](https://github.com/HyperTekOrg/hyperstack/commit/ced55fe4981e0f36abbebe277438eb17ea01b519))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **computed:** add __slot and __timestamp context access in computed fields ([b707f95](https://github.com/HyperTekOrg/hyperstack/commit/b707f95058f74ae52c6003bc2d68e948e657e70e))
* **computed:** support resolver computed fields inside array .map() closures ([1f571c0](https://github.com/HyperTekOrg/hyperstack/commit/1f571c08caa2da41192c3b45399f1abe747dda10))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/HyperTekOrg/hyperstack/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/HyperTekOrg/hyperstack/commit/bdf8b5564619695575503e817507c0c8238cecac))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* Codegen correct path for bytemuck ([ae1d6b5](https://github.com/HyperTekOrg/hyperstack/commit/ae1d6b52a6c1c216542888a7516fcd33fb619054))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/HyperTekOrg/hyperstack/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/HyperTekOrg/hyperstack/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* flush queued account updates when lookup index is populated ([8a09279](https://github.com/HyperTekOrg/hyperstack/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))
* Logging ([b727602](https://github.com/HyperTekOrg/hyperstack/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* Logs ([163b5ff](https://github.com/HyperTekOrg/hyperstack/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))
* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/HyperTekOrg/hyperstack/commit/76a124ef623c6aabd4d88ae688701403105a80dd))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/HyperTekOrg/hyperstack/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** recover from poisoned mutex and support block expressions in computed fields ([1106e77](https://github.com/HyperTekOrg/hyperstack/commit/1106e7774dd43639444b79c1d2fc2d8099e7fa5c))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/HyperTekOrg/hyperstack/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))
* Map syntax ([8a5eaad](https://github.com/HyperTekOrg/hyperstack/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/HyperTekOrg/hyperstack/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/HyperTekOrg/hyperstack/commit/06f1819193721f48edd96723c4cc833745732070))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/HyperTekOrg/hyperstack/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))
* resolve clippy warnings across workspace ([565d92b](https://github.com/HyperTekOrg/hyperstack/commit/565d92b91552d92262cfaeca9674d0ad4d3f6b5d))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/HyperTekOrg/hyperstack/commit/10c1611282babb70bbe70d19fb599c83654caa6c))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* wire auto-generated lookup resolvers into IDL codegen path ([1175753](https://github.com/HyperTekOrg/hyperstack/commit/1175753d45a0cdf512e0f173df0964d9fddd889b))

## [0.4.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.4.2...hyperstack-macros-v0.4.3) (2026-02-03)


### Features

* add PDA resolution hooks and multi-program stack support ([f4210a0](https://github.com/HyperTekOrg/hyperstack/commit/f4210a0a665f75f60fce09634e9ddbdde3f6898c))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/HyperTekOrg/hyperstack/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/HyperTekOrg/hyperstack/commit/10c1611282babb70bbe70d19fb599c83654caa6c))

## [0.4.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.4.1...hyperstack-macros-v0.4.2) (2026-02-01)


### Bug Fixes

* Codegen correct path for bytemuck ([ae1d6b5](https://github.com/HyperTekOrg/hyperstack/commit/ae1d6b52a6c1c216542888a7516fcd33fb619054))

## [0.4.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.4.0...hyperstack-macros-v0.4.1) (2026-02-01)


### Features

* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/HyperTekOrg/hyperstack/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))


### Bug Fixes

* flush queued account updates when lookup index is populated ([8a09279](https://github.com/HyperTekOrg/hyperstack/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))
* wire auto-generated lookup resolvers into IDL codegen path ([1175753](https://github.com/HyperTekOrg/hyperstack/commit/1175753d45a0cdf512e0f173df0964d9fddd889b))

## [0.4.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.15...hyperstack-macros-v0.4.0) (2026-01-31)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add deterministic sorting ([e775f59](https://github.com/HyperTekOrg/hyperstack/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/HyperTekOrg/hyperstack/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/HyperTekOrg/hyperstack/commit/bdf8b5564619695575503e817507c0c8238cecac))
* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/HyperTekOrg/hyperstack/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* Logging ([b727602](https://github.com/HyperTekOrg/hyperstack/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* Logs ([163b5ff](https://github.com/HyperTekOrg/hyperstack/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))
* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/HyperTekOrg/hyperstack/commit/76a124ef623c6aabd4d88ae688701403105a80dd))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/HyperTekOrg/hyperstack/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/HyperTekOrg/hyperstack/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))
* Map syntax ([8a5eaad](https://github.com/HyperTekOrg/hyperstack/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/HyperTekOrg/hyperstack/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/HyperTekOrg/hyperstack/commit/06f1819193721f48edd96723c4cc833745732070))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/HyperTekOrg/hyperstack/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.3.15](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.14...hyperstack-macros-v0.3.15) (2026-01-31)


### Bug Fixes

* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/HyperTekOrg/hyperstack/commit/06f1819193721f48edd96723c4cc833745732070))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/HyperTekOrg/hyperstack/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/HyperTekOrg/hyperstack/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))

## [0.3.14](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.13...hyperstack-macros-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.13](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.12...hyperstack-macros-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.12](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.11...hyperstack-macros-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.10...hyperstack-macros-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.9...hyperstack-macros-v0.3.10) (2026-01-28)


### Bug Fixes

* Logs ([163b5ff](https://github.com/HyperTekOrg/hyperstack/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))

## [0.3.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.8...hyperstack-macros-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.7...hyperstack-macros-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.6...hyperstack-macros-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.5...hyperstack-macros-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.4...hyperstack-macros-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.3...hyperstack-macros-v0.3.4) (2026-01-24)


### Bug Fixes

* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/HyperTekOrg/hyperstack/commit/76a124ef623c6aabd4d88ae688701403105a80dd))

## [0.3.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.2...hyperstack-macros-v0.3.3) (2026-01-23)


### Features

* add derived view support to React SDK and macros ([5f6414f](https://github.com/HyperTekOrg/hyperstack/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/HyperTekOrg/hyperstack/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/HyperTekOrg/hyperstack/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/HyperTekOrg/hyperstack/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/HyperTekOrg/hyperstack/commit/892c3d526c71df4c4d848142908ce511e302e082))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/HyperTekOrg/hyperstack/commit/bdf8b5564619695575503e817507c0c8238cecac))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/HyperTekOrg/hyperstack/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* Logging ([b727602](https://github.com/HyperTekOrg/hyperstack/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/HyperTekOrg/hyperstack/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/HyperTekOrg/hyperstack/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))

## [0.3.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.1...hyperstack-macros-v0.3.2) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.3.0...hyperstack-macros-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.3.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.5...hyperstack-macros-v0.3.0) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.2.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.4...hyperstack-macros-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.2.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.3...hyperstack-macros-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.2.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.2...hyperstack-macros-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/HyperTekOrg/hyperstack/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/HyperTekOrg/hyperstack/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/HyperTekOrg/hyperstack/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))

## [0.2.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.1...hyperstack-macros-v0.2.2) (2026-01-16)


### Features

* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/HyperTekOrg/hyperstack/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/HyperTekOrg/hyperstack/commit/d693f421742258bbbd3528ffbbd4731d638c992b))


### Bug Fixes

* Clippy errors ([d6a9f4d](https://github.com/HyperTekOrg/hyperstack/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))

## [0.2.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.2.0...hyperstack-macros-v0.2.1) (2026-01-16)


### Features

* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/HyperTekOrg/hyperstack/commit/48e3ec7a952399135a84323da78cdc499804bce9))

## [0.2.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.11...hyperstack-macros-v0.2.0) (2026-01-15)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.1.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.10...hyperstack-macros-v0.1.11) (2026-01-14)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.1.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.9...hyperstack-macros-v0.1.10) (2026-01-13)


### Features

* Add deterministic sorting ([e775f59](https://github.com/HyperTekOrg/hyperstack/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))

## [0.1.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.8...hyperstack-macros-v0.1.9) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.1.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.6...hyperstack-macros-v0.1.8) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/HyperTekOrg/hyperstack/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/HyperTekOrg/hyperstack/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.5...v0.1.6) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/HyperTekOrg/hyperstack/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/HyperTekOrg/hyperstack/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* Preserve integer types in computed field expressions ([616f042](https://github.com/HyperTekOrg/hyperstack/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.4...hyperstack-macros-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/HyperTekOrg/hyperstack/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/HyperTekOrg/hyperstack/commit/72348a42ee3988e94873db0d24317f3e661e093d))

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.2...hyperstack-macros-v0.1.4) (2026-01-09)


### Miscellaneous Chores

* **hyperstack-macros:** Synchronize hyperstack versions

## [0.1.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-macros-v0.1.1...hyperstack-macros-v0.1.2) (2026-01-09)


### Bug Fixes

* Update naming ([4381946](https://github.com/HyperTekOrg/hyperstack/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-spec-macros-v0.1.0...hyperstack-spec-macros-v0.1.1) (2026-01-09)


### Features

* Better sdk types during generation ([f9555ef](https://github.com/HyperTekOrg/hyperstack/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))


### Bug Fixes

* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
