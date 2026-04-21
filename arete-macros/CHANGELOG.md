# Changelog

## [0.1.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.0.1...arete-macros-v0.1.0) (2026-04-21)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.9](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.8...arete-macros-v0.6.9) (2026-04-15)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.8](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.7...arete-macros-v0.6.8) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.7](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.6...arete-macros-v0.6.7) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.6](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.5...arete-macros-v0.6.6) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.5](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.4...arete-macros-v0.6.5) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.4](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.3...arete-macros-v0.6.4) (2026-04-05)


### Features

* RuntimeResolver abstraction with enhanced caching ([9166434](https://github.com/AreteA4/arete/commit/9166434e52468f0d152781a4d81bb0db0fd9be21))

## [0.6.3](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.2...arete-macros-v0.6.3) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.1...arete-macros-v0.6.2) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.1](https://github.com/AreteA4/arete/compare/arete-macros-v0.6.0...arete-macros-v0.6.1) (2026-04-05)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.10...arete-macros-v0.6.0) (2026-04-04)


### Features

* Add AST versioning system with automatic migration support ([997706b](https://github.com/AreteA4/arete/commit/997706b2854fc2e95427ef6b67b710db35ad86ac))
* Add AST versioning system with automatic migration support ([b62d08d](https://github.com/AreteA4/arete/commit/b62d08d99a579f323ea3f4a052fa90b83b269942))
* add span field to ConditionExpr for better error reporting ([ec6eadd](https://github.com/AreteA4/arete/commit/ec6eadd3862ad418be8c4b767d9c1e8ab819e0c2))


### Bug Fixes

* accept key fields captured by event handlers ([de38610](https://github.com/AreteA4/arete/commit/de38610a578b9afde54af1775f2f6b1e7bf802dd))
* add compile-time validation and improve lookup_by warnings ([6cb45b4](https://github.com/AreteA4/arete/commit/6cb45b49181d8edb9475cdba13369734e382950f))
* add debug warnings and deduplicate validation errors ([ca324a4](https://github.com/AreteA4/arete/commit/ca324a4277439462fc2238dd7c49cf20cbff7cef))
* Address clippy warnings in AST versioning module ([1d6540f](https://github.com/AreteA4/arete/commit/1d6540fd71a66e76a211de35e81a89cfe584dc66))
* Address code review feedback on AST versioning ([1791986](https://github.com/AreteA4/arete/commit/1791986fa0e37f5762a57c166ecbcf6be26bcb0b))
* address code review feedback on error handling and diagnostics ([e11a539](https://github.com/AreteA4/arete/commit/e11a5398ec287ecea959993c873364c1c5e6f4cb))
* Address code review feedback on into_latest, test assertions, and parsing ([da49bf9](https://github.com/AreteA4/arete/commit/da49bf9358362d27c307c8cc958a60b872ae2a2f))
* address code review issues for dead code, resolver conditions, and event source documentation ([0e0f5be](https://github.com/AreteA4/arete/commit/0e0f5be7ef57fa275948543f8cad4ad27b788347))
* address code review issues for validation logic ([0ab5734](https://github.com/AreteA4/arete/commit/0ab5734f2ff1454201efc86ddd94054cf44b465d))
* address code review issues in validation and condition parsing ([49ee76b](https://github.com/AreteA4/arete/commit/49ee76ba26485f535ff35d2c76c39c224dbc4178))
* address review feedback on macro diagnostics ([c1d6e33](https://github.com/AreteA4/arete/commit/c1d6e33a71e1444a63a91865ac5fa8e327051cd2))
* align null literal handling and prevent Ident panic on invalid leaves ([6d2935f](https://github.com/AreteA4/arete/commit/6d2935fb51148365f8df4b9134472afc4330019a))
* align proto UI tests with warning-only behavior ([6b89861](https://github.com/AreteA4/arete/commit/6b89861bcc74ec1819bb8683f6cd3d1c05ca3650))
* align resolver condition parsing with strict conditions ([a2be077](https://github.com/AreteA4/arete/commit/a2be077ebe3a39d9313cf83318d23adc5a9bccb6))
* avoid allocating string for comparison in __account_address check ([28cb740](https://github.com/AreteA4/arete/commit/28cb7402dc750036f571754a77900d9b2ac4f0da))
* Clarify UnsupportedVersion error message to mention migration support ([51fde69](https://github.com/AreteA4/arete/commit/51fde6960103340b0fd18158f1e6d3a5a2b398ea))
* correct debug_assert logic and add sentinel field filtering ([8bb3559](https://github.com/AreteA4/arete/commit/8bb3559b2f9833561219ada64e91ce9085c06001))
* correct escape sequence handling and add ZERO constants to resolver conditions ([e85c3ec](https://github.com/AreteA4/arete/commit/e85c3ec2ea709b5ef4c152134af3d0c6e7ad2116))
* deduplicate condition-leaf errors and validate event-backed aggregates ([aa1bb53](https://github.com/AreteA4/arete/commit/aa1bb53e9e49e3fb65f6a28ecceffebaecfdd6d2))
* deduplicate derive_from IDL errors ([3af2189](https://github.com/AreteA4/arete/commit/3af2189b984d3b39762c86b855da6fc979e5dc61))
* deduplicate event join_on errors per instruction group ([cfee9be](https://github.com/AreteA4/arete/commit/cfee9be66346a75321f881acb3f4108eb3e304be))
* deduplicate invalid source diagnostics ([5dc9621](https://github.com/AreteA4/arete/commit/5dc96217f809c6e05633d75fe6c82bcf14b51b76))
* deduplicate join_on entity-field errors per source group ([c98efdd](https://github.com/AreteA4/arete/commit/c98efddb98399ad5ff76865218bddb3cda54bf24))
* deterministic sort order and validate join_on before IDL resolution ([2cad643](https://github.com/AreteA4/arete/commit/2cad643b58b9a920c3cce909ad0bffe26fea3ebf))
* ensure deterministic AST JSON output by using BTreeMap ([a84b7af](https://github.com/AreteA4/arete/commit/a84b7afbd2956a980a5f5d8cc3b6f5beaceddc99))
* handle derive_from key fields and non-finite conditions ([117590f](https://github.com/AreteA4/arete/commit/117590fc5f0074dc338cb6121599580a57513797))
* improve error messages and handle ambiguous mapping sources ([6fa25c7](https://github.com/AreteA4/arete/commit/6fa25c7903470a0bfd5bbdfbc46813e7d8bc95c6))
* improve error messages and handle ambiguous mapping sources ([fed3b5b](https://github.com/AreteA4/arete/commit/fed3b5b0d2217c2064240ada415f385dfc8fe243))
* improve validation determinism and error handling for aggregate conditions ([9022603](https://github.com/AreteA4/arete/commit/9022603c39beb8ba6ae4a54bb56a029753c52f56))
* improve validation robustness and diagnostics ([66cdef1](https://github.com/AreteA4/arete/commit/66cdef19c42f21bf12b56eb7da0e2a2f59a296fd))
* Make sync tests fail explicitly when source file not found ([c824e3d](https://github.com/AreteA4/arete/commit/c824e3dc74e102d57eb96485d9dad0c925d0bfc3))
* normalize instruction names to snake_case before IDL lookup ([7814a42](https://github.com/AreteA4/arete/commit/7814a42012650efda2b0f27bbcbc96b3416751c1))
* only reject two-character operator sequences in condition parser ([fcca1b4](https://github.com/AreteA4/arete/commit/fcca1b42faff1963150a2cea22534b16db907e5b))
* prevent join_on panic on dotted paths and validate account aggregate conditions ([b4cc12d](https://github.com/AreteA4/arete/commit/b4cc12d9ba1dc53f3016f2fec378b709d6216760))
* reject invalid legacy event IDL lookups ([a70e9da](https://github.com/AreteA4/arete/commit/a70e9dafd6c3c277f297fbcb475b56ca620dddb5))
* remove dead code and false-positive validation for event-backed aggregates ([6dc43ed](https://github.com/AreteA4/arete/commit/6dc43edf5593f90673c35dcd1f512849a41759f3))
* remove incorrect debug_assert for event sources in validation ([6549eb5](https://github.com/AreteA4/arete/commit/6549eb567d350b7c8435181fd45cf3ae016bb8ee))
* Remove Serialize derive from Versioned*Spec enums to prevent duplicate keys ([73ad7b4](https://github.com/AreteA4/arete/commit/73ad7b4b8519a30b7dcfca66badd05daf1eee085))
* remove spurious warning for valid event-backed aggregates ([8e66dff](https://github.com/AreteA4/arete/commit/8e66dff9bde3cf36d6ebe55658aeb4c9734dc099))
* restore case-insensitive IDL lookups and per-event validation ([636fd6d](https://github.com/AreteA4/arete/commit/636fd6d0c6b166c0be50e3cb4b8a9ada43c52050))
* restore is_event_source guard with corrected comment ([05106ba](https://github.com/AreteA4/arete/commit/05106ba153917e3001875558f3f3a149ef310849))
* sort derive_from attrs and align field lookup semantics ([eea1af0](https://github.com/AreteA4/arete/commit/eea1af028bfec5fc52668f2279f7eb2a7c834c92))
* sort filter and computed field refs for deterministic errors ([f8d4a70](https://github.com/AreteA4/arete/commit/f8d4a70f6d68c7322def87903fb3bc793bcaf517))
* stabilize derive validation diagnostics ([e1d3e57](https://github.com/AreteA4/arete/commit/e1d3e570284dd84414df33f60a523ad48dfc230f))
* stabilize dynamic macro test harnesses ([9715c2b](https://github.com/AreteA4/arete/commit/9715c2b88772dd5c4f1972a4f1e6abced8dc5d93))
* stabilize event key validation and resolver parsing ([d026256](https://github.com/AreteA4/arete/commit/d026256085c7433f95c30504d78c82d271874a95))
* stabilize event lookup handling across validation and codegen ([0911d10](https://github.com/AreteA4/arete/commit/0911d10d900f39394acdbf776b213bab020942ec))
* strip entity prefix when matching aggregate condition target fields ([07ef86a](https://github.com/AreteA4/arete/commit/07ef86a2cc7d4b66ac320a18bb0c929015b1e534))
* surface arete macro validation failures during expansion ([7928539](https://github.com/AreteA4/arete/commit/7928539d0a9a4e53db546f4f65d35f26e2e95560))
* tighten IDL lookup casing and derive_from diagnostics ([de5706d](https://github.com/AreteA4/arete/commit/de5706d59be3942a2f1612426f2b1ee5cb0ce817))
* tighten macro validation follow-up checks ([b00f678](https://github.com/AreteA4/arete/commit/b00f6785a5e10fcd89c2dc55aa4218e6bcb4e41b))
* Use CURRENT_AST_VERSION constant instead of hardcoded version ([5df9efe](https://github.com/AreteA4/arete/commit/5df9efe883d5880f6c440ae8e0577004041d53ca))
* Use CURRENT_AST_VERSION in test assertions instead of hardcoded string ([6809f1a](https://github.com/AreteA4/arete/commit/6809f1a632f464e2c1c6b175458fd51a24712acd))
* use find_top_level_operator for accurate logical operator detection ([456c5c0](https://github.com/AreteA4/arete/commit/456c5c0a184c9bd7e36f1635e96d8f883225fefc))
* use stable sort for mappings and remove dead is_event_source guard ([4a22666](https://github.com/AreteA4/arete/commit/4a2266658b3927acae061f2033f8c30c4ce94e33))
* use stable sort for mappings and remove dead is_event_source guard ([df51dd6](https://github.com/AreteA4/arete/commit/df51dd6e2d22028391f0d7247b1bcd07c4f4b4ed))
* validate __account_address transform to lookup-index field ([d2a1c45](https://github.com/AreteA4/arete/commit/d2a1c45a37b4c8366842b5d516b1c8d9cbb48a6a))
* validate account condition fields and sort aggregate source lookups ([bcac30c](https://github.com/AreteA4/arete/commit/bcac30c9d77db46d4f3acfb1e79764ddd38baf8d))
* validate account lookup_by fields and fix cycle detection ([08611a4](https://github.com/AreteA4/arete/commit/08611a4ea6ffd339f3cda1e7ab49d76863dcc60c))
* validate condition field paths against IDL sources ([1f63e08](https://github.com/AreteA4/arete/commit/1f63e08da39c386ff0460f19d6c9650457974012))
* validate derive_from hooks as instruction groups ([607289c](https://github.com/AreteA4/arete/commit/607289c69776d75a11793f3de1bbe45c3ba42492))
* validate event join_on and URL template references ([de1c775](https://github.com/AreteA4/arete/commit/de1c7751158f47987c800ff5029d8c00ef613e60))
* validate handler key resolution paths during macro expansion ([a069d1a](https://github.com/AreteA4/arete/commit/a069d1a2a6d0888e44bc9a9e0fbd1f6ba850b11d))
* validate join_on fields before IDL resolution ([6236f4b](https://github.com/AreteA4/arete/commit/6236f4b3e2c3fe438551a9bddc8613ddf5075f2c))
* validate legacy event fields and improve view transform error spans ([c70625d](https://github.com/AreteA4/arete/commit/c70625d868fc693dfa7d4d84ecb2dfed0225fbda))
* validate nested condition fields and prevent Ident panics on dotted paths ([4bb14cb](https://github.com/AreteA4/arete/commit/4bb14cbf0a9d5c17652df3bee0f1e2137737acec))
* validate view filter fields and source kinds ([18fc19e](https://github.com/AreteA4/arete/commit/18fc19eccec140d6fc7ff47cdac2f85c71d70597))
* validation and error handling improvements from code review ([d544e28](https://github.com/AreteA4/arete/commit/d544e280f7a9b1e8eb5af80e3edc96c85bc98a82))
* validation improvements from code review ([3bc7d59](https://github.com/AreteA4/arete/commit/3bc7d597647c7db21b626b7c53bfff9aebc6cf6f))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1.5 to 0.1.6

## [0.5.10](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.9...arete-macros-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.5.9](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.8...arete-macros-v0.5.9) (2026-03-19)


### Bug Fixes

* handle u64 integer precision loss across Rust-JS boundary ([e96e7fa](https://github.com/AreteA4/arete/commit/e96e7fa7172f520bd7ee88ed7582eda899c9f65b))

## [0.5.8](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.7...arete-macros-v0.5.8) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1.4 to 0.1.5

## [0.5.7](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.6...arete-macros-v0.5.7) (2026-03-19)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1.3 to 0.1.4

## [0.5.6](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.5...arete-macros-v0.5.6) (2026-03-19)


### Features

* add Keccak256 hashing and slot hash caching ([768c407](https://github.com/AreteA4/arete/commit/768c407adce786003c30c634432279456baa5102))
* improve slot scheduler with notification-based waiting and enhanced logging ([0b8681e](https://github.com/AreteA4/arete/commit/0b8681eb8cf399e09140495c749872dcae91b1c1))


### Bug Fixes

* cache account data for all entity states that route an event type ([1b59633](https://github.com/AreteA4/arete/commit/1b59633f542dad2e140e914ff3e787019dd75944))
* conditionally enable TLS only for https/grpcs endpoints in slot subscription ([8c8875b](https://github.com/AreteA4/arete/commit/8c8875b4c587f282e6f3993c1651aebe816004c1))
* Core interpreter and server improvements ([b05ae9b](https://github.com/AreteA4/arete/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* correct resolver output types and schema generation in TypeScript emitter ([b37ef43](https://github.com/AreteA4/arete/commit/b37ef43e93d5c20903b01831424d94f6bb86bd72))
* derive state_id from bytecode routing for PDA cache ([927b364](https://github.com/AreteA4/arete/commit/927b364a996ce9047641a35a66d346485feac4d8))
* keep gRPC subscription sender alive to prevent stream termination ([42460d2](https://github.com/AreteA4/arete/commit/42460d2c2cf5dbb582da78e73100ab42b6213786))
* prevent integer overflow in SlotHashes sysvar parsing ([99427f6](https://github.com/AreteA4/arete/commit/99427f6d6ca03c691c9556f1dc69065ca5b2b1e0))
* prevent panic in SlotHash resolver when using current_thread runtime ([3525397](https://github.com/AreteA4/arete/commit/3525397a8adec127ac261b1c6e6cab2617f1217b))
* remove unnecessary async from parse_and_cache_slot_hashes ([03e00aa](https://github.com/AreteA4/arete/commit/03e00aa4248be85146e60c133cc88130c240dcc1))
* resolve clippy warnings across workspace ([c19d1ec](https://github.com/AreteA4/arete/commit/c19d1ec5926ee9099c6ab4254bde30b2c794e27f))
* restore cross-account lookup resolution at round boundaries ([0af0835](https://github.com/AreteA4/arete/commit/0af0835ea6c7d35c5c1efd6f63899706dd85ab91))
* serialize u64-from-bytes computed fields as strings to avoid JS precision loss ([1f67a7a](https://github.com/AreteA4/arete/commit/1f67a7a9589823262a972ea01c71ad4e04e24ffe))
* use dynamic-length discriminator slice in generated code ([d67b727](https://github.com/AreteA4/arete/commit/d67b727e3cb34f2f9eb66d75f15add86bc8dbbd6))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1.2 to 0.1.3

## [0.5.5](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.4...arete-macros-v0.5.5) (2026-03-14)


### Bug Fixes

* Add version constraint to arete-idl dependency ([917bf5a](https://github.com/AreteA4/arete/commit/917bf5abe6242048ba9f7af0055d999ccfcb8692))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1 to 0.1.2

## [0.5.4](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.3...arete-macros-v0.5.4) (2026-03-14)


### Features

* add HashMap type support, auto-derive discriminators, and to_json_value codegen ([7e81e3d](https://github.com/AreteA4/arete/commit/7e81e3d9802ad25987439841b7e4dd0f45232386))
* **macros:** add CPI event support with camelCase field handling ([cd856fa](https://github.com/AreteA4/arete/commit/cd856fa19606481e3f6775d8a8e591d4782f7913))
* **macros:** enhance Vixen runtime with improved tracing and queue handling ([d8b46ea](https://github.com/AreteA4/arete/commit/d8b46ea369ce5669f4cd357e78c83132c758a66d))
* **macros:** support packed structs, large arrays, and stream spec improvements ([b337d09](https://github.com/AreteA4/arete/commit/b337d09c1b45bd32604d65e05c96d0dc5dc8ea7a))
* misc compiler, VM, and IDL improvements ([2d6aea3](https://github.com/AreteA4/arete/commit/2d6aea373e43c84e3a07ecef7d9dab004a0b8c1c))


### Bug Fixes

* - Instantiate UrlResolverClient once at startup on VmHandler instead of per-request ([f346bfa](https://github.com/AreteA4/arete/commit/f346bfa2131d2cb1ae3ea98e03b25e9351f25eca))
* add serde derives to IDL-generated account and custom types ([cada9f5](https://github.com/AreteA4/arete/commit/cada9f53f00112b7048f62743dd9aea0ffb07baa))
* address code review issues in macro codegen ([908c9ff](https://github.com/AreteA4/arete/commit/908c9ff757976336483b89c24fd8f75640f58029))
* eliminate duplicate url_path qualification logic in sections.rs ([6ed8e94](https://github.com/AreteA4/arete/commit/6ed8e948f4490e2aa659c7c25812fdcc968ac64e))
* **macros:** prevent cross-entity instruction hook contamination ([ff65ed0](https://github.com/AreteA4/arete/commit/ff65ed0259a18425c15d250dcc30ad19ab093dc7))
* panic on missing event type definition instead of silent empty struct ([cdb0d39](https://github.com/AreteA4/arete/commit/cdb0d391903c1382eca2b81bd9867172cf028ca3))
* re-queue URL resolver requests on empty URL or failure ([dc204a2](https://github.com/AreteA4/arete/commit/dc204a2cfae505b7cf603b4eb56b39e0d8277f97))
* remove redundant is_cpi_event variable shadowing ([d338399](https://github.com/AreteA4/arete/commit/d338399339c231ff150bc9b829e2ea9296dd48ee))
* resolve compilation errors in arete-macros ([acd5a31](https://github.com/AreteA4/arete/commit/acd5a31d63997426f23b9cd42f26fcef7ff59f97))
* resolve instruction field prefix for camelCase IDL instruction names ([df75acf](https://github.com/AreteA4/arete/commit/df75acfca07371ffbda8f410bcc777a42f62627a))
* silence unused variable warning in stream_spec ([3fee1b1](https://github.com/AreteA4/arete/commit/3fee1b124e54a55c886844e9fd0c3bc82bfa1995))
* use transaction_accounts_include in prefilter builder ([2df88f8](https://github.com/AreteA4/arete/commit/2df88f890e7549a2d5dadee9703e2cb31cd866ef))
* validate HTTP method input in ResolveAttributeArgs parser to support only 'GET' and 'POST' ([fb80a48](https://github.com/AreteA4/arete/commit/fb80a48ebf603a13d72d67cd6aedb602d5818cc3))

## [0.5.3](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.2...arete-macros-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.5.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.1...arete-macros-v0.5.2) (2026-02-07)


### Bug Fixes

* batch primary and resolver mutations to avoid duplicate frames ([160feee](https://github.com/AreteA4/arete/commit/160feee69c8e9429939f13cdf81359278750f241))

## [0.5.1](https://github.com/AreteA4/arete/compare/arete-macros-v0.5.0...arete-macros-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.4.3...arete-macros-v0.5.0) (2026-02-06)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/AreteA4/arete/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/AreteA4/arete/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add deterministic sorting ([e775f59](https://github.com/AreteA4/arete/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/AreteA4/arete/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* Add PDA and Instruction type definitions for SDK code generation ([6a74067](https://github.com/AreteA4/arete/commit/6a7406751d865b9e4caef3749779217e305636a5))
* add PDA resolution hooks and multi-program stack support ([f4210a0](https://github.com/AreteA4/arete/commit/f4210a0a665f75f60fce09634e9ddbdde3f6898c))
* add resolver support with resolve attribute and computed field methods ([aed45c8](https://github.com/AreteA4/arete/commit/aed45c81477267cb6a005d439ee30400c1e24e5c))
* add resolver-declared transforms to #[map] attribute ([8f35bff](https://github.com/AreteA4/arete/commit/8f35bff8c3fa811e4bfb50a1c431ecc09822e2d2))
* add resolver-provided computed methods (ui_amount, raw_amount) ([3729710](https://github.com/AreteA4/arete/commit/372971072f597625905a1365da4ec2e5e8d9d978))
* add stop attribute for conditional field population ([441cfef](https://github.com/AreteA4/arete/commit/441cfef39760af9b3af78b992d1269eaeecd2f99))
* add token metadata resolver with DAS API integration ([ced55fe](https://github.com/AreteA4/arete/commit/ced55fe4981e0f36abbebe277438eb17ea01b519))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/AreteA4/arete/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **computed:** add __slot and __timestamp context access in computed fields ([b707f95](https://github.com/AreteA4/arete/commit/b707f95058f74ae52c6003bc2d68e948e657e70e))
* **computed:** support resolver computed fields inside array .map() closures ([1f571c0](https://github.com/AreteA4/arete/commit/1f571c08caa2da41192c3b45399f1abe747dda10))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/AreteA4/arete/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/AreteA4/arete/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/AreteA4/arete/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/AreteA4/arete/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/AreteA4/arete/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/AreteA4/arete/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/AreteA4/arete/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/AreteA4/arete/commit/bdf8b5564619695575503e817507c0c8238cecac))
* Broken streams after naming refactor ([64437b4](https://github.com/AreteA4/arete/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/AreteA4/arete/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* Codegen correct path for bytemuck ([ae1d6b5](https://github.com/AreteA4/arete/commit/ae1d6b52a6c1c216542888a7516fcd33fb619054))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/AreteA4/arete/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/AreteA4/arete/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* flush queued account updates when lookup index is populated ([8a09279](https://github.com/AreteA4/arete/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))
* Logging ([b727602](https://github.com/AreteA4/arete/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* Logs ([163b5ff](https://github.com/AreteA4/arete/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))
* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/AreteA4/arete/commit/76a124ef623c6aabd4d88ae688701403105a80dd))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/AreteA4/arete/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** recover from poisoned mutex and support block expressions in computed fields ([1106e77](https://github.com/AreteA4/arete/commit/1106e7774dd43639444b79c1d2fc2d8099e7fa5c))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/AreteA4/arete/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))
* Map syntax ([8a5eaad](https://github.com/AreteA4/arete/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/AreteA4/arete/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/AreteA4/arete/commit/06f1819193721f48edd96723c4cc833745732070))
* Preserve integer types in computed field expressions ([616f042](https://github.com/AreteA4/arete/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/AreteA4/arete/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/AreteA4/arete/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))
* resolve clippy warnings across workspace ([565d92b](https://github.com/AreteA4/arete/commit/565d92b91552d92262cfaeca9674d0ad4d3f6b5d))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/AreteA4/arete/commit/10c1611282babb70bbe70d19fb599c83654caa6c))
* Update naming ([4381946](https://github.com/AreteA4/arete/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))
* wire auto-generated lookup resolvers into IDL codegen path ([1175753](https://github.com/AreteA4/arete/commit/1175753d45a0cdf512e0f173df0964d9fddd889b))

## [0.4.3](https://github.com/AreteA4/arete/compare/arete-macros-v0.4.2...arete-macros-v0.4.3) (2026-02-03)


### Features

* add PDA resolution hooks and multi-program stack support ([f4210a0](https://github.com/AreteA4/arete/commit/f4210a0a665f75f60fce09634e9ddbdde3f6898c))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/AreteA4/arete/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/AreteA4/arete/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* correct lookup resolution and conditional mappings ([0c1ac80](https://github.com/AreteA4/arete/commit/0c1ac809d65eb6139b1208e041817ca9d0f73724))
* resolve clippy warnings for Rust 1.91 ([10c1611](https://github.com/AreteA4/arete/commit/10c1611282babb70bbe70d19fb599c83654caa6c))

## [0.4.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.4.1...arete-macros-v0.4.2) (2026-02-01)


### Bug Fixes

* Codegen correct path for bytemuck ([ae1d6b5](https://github.com/AreteA4/arete/commit/ae1d6b52a6c1c216542888a7516fcd33fb619054))

## [0.4.1](https://github.com/AreteA4/arete/compare/arete-macros-v0.4.0...arete-macros-v0.4.1) (2026-02-01)


### Features

* add bytemuck serialization support and fix lookup index key resolution ([b2dcd1f](https://github.com/AreteA4/arete/commit/b2dcd1fa881644d7d4fdc909216fa1ef2995dd7b))


### Bug Fixes

* flush queued account updates when lookup index is populated ([8a09279](https://github.com/AreteA4/arete/commit/8a09279c06a369d513c2b1d2f0cbf1560187db6f))
* wire auto-generated lookup resolvers into IDL codegen path ([1175753](https://github.com/AreteA4/arete/commit/1175753d45a0cdf512e0f173df0964d9fddd889b))

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.15...arete-macros-v0.4.0) (2026-01-31)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* add derived view support to React SDK and macros ([5f6414f](https://github.com/AreteA4/arete/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* Add deterministic sorting ([e775f59](https://github.com/AreteA4/arete/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))
* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/AreteA4/arete/commit/48e3ec7a952399135a84323da78cdc499804bce9))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/AreteA4/arete/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/AreteA4/arete/commit/d693f421742258bbbd3528ffbbd4731d638c992b))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/AreteA4/arete/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/AreteA4/arete/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/AreteA4/arete/commit/892c3d526c71df4c4d848142908ce511e302e082))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/AreteA4/arete/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/AreteA4/arete/commit/bdf8b5564619695575503e817507c0c8238cecac))
* Broken streams after naming refactor ([64437b4](https://github.com/AreteA4/arete/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([d6a9f4d](https://github.com/AreteA4/arete/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/AreteA4/arete/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* Logging ([b727602](https://github.com/AreteA4/arete/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* Logs ([163b5ff](https://github.com/AreteA4/arete/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))
* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/AreteA4/arete/commit/76a124ef623c6aabd4d88ae688701403105a80dd))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/AreteA4/arete/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/AreteA4/arete/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))
* Map syntax ([8a5eaad](https://github.com/AreteA4/arete/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/AreteA4/arete/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/AreteA4/arete/commit/06f1819193721f48edd96723c4cc833745732070))
* Preserve integer types in computed field expressions ([616f042](https://github.com/AreteA4/arete/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/AreteA4/arete/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/AreteA4/arete/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))
* Update naming ([4381946](https://github.com/AreteA4/arete/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.3.15](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.14...arete-macros-v0.3.15) (2026-01-31)


### Bug Fixes

* prefix unused variable with underscore to satisfy clippy ([06f1819](https://github.com/AreteA4/arete/commit/06f1819193721f48edd96723c4cc833745732070))
* prevent entity field loss from partial patches in sorted cache ([1e3c8e6](https://github.com/AreteA4/arete/commit/1e3c8e6f25b2b7968e60754e8175c7a66f68c908))
* Remove ast build log output ([67ebb21](https://github.com/AreteA4/arete/commit/67ebb21746b9142ef52c46caae5019a925db3a2b))

## [0.3.14](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.13...arete-macros-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.13](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.12...arete-macros-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.12](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.11...arete-macros-v0.3.12) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.11](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.10...arete-macros-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.10](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.9...arete-macros-v0.3.10) (2026-01-28)


### Bug Fixes

* Logs ([163b5ff](https://github.com/AreteA4/arete/commit/163b5ff75dababdf17761b193b7f3b8d3d7bc30c))

## [0.3.9](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.8...arete-macros-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.8](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.7...arete-macros-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.7](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.6...arete-macros-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.6](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.5...arete-macros-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.5](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.4...arete-macros-v0.3.5) (2026-01-24)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.4](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.3...arete-macros-v0.3.4) (2026-01-24)


### Bug Fixes

* **macros:** add explicit type annotation to account_names vector ([76a124e](https://github.com/AreteA4/arete/commit/76a124ef623c6aabd4d88ae688701403105a80dd))

## [0.3.3](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.2...arete-macros-v0.3.3) (2026-01-23)


### Features

* add derived view support to React SDK and macros ([5f6414f](https://github.com/AreteA4/arete/commit/5f6414f879f2891be2d8ee5c16173cf83ddf2ea9))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/AreteA4/arete/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **macros:** add #[view] attribute for declarative view definitions on entities ([3f0bdc5](https://github.com/AreteA4/arete/commit/3f0bdc51d7945c32082ffa8997362328c7b26022))
* **macros:** support tuple structs, 8-byte discriminators, and optional error messages in IDL ([090b5d6](https://github.com/AreteA4/arete/commit/090b5d62999e7bbae2dfb577a0d028b6675def01))
* **sdk:** add sorted view support with server-side subscribed frame ([1a7d83f](https://github.com/AreteA4/arete/commit/1a7d83fe4000c26d282f2df9ce95f9d79414014d))
* **server:** add slot-based sequence ordering for list views ([892c3d5](https://github.com/AreteA4/arete/commit/892c3d526c71df4c4d848142908ce511e302e082))


### Bug Fixes

* Account lookup ([bdf8b55](https://github.com/AreteA4/arete/commit/bdf8b5564619695575503e817507c0c8238cecac))
* convert field names to camelCase for JSON serialization ([3dc05a4](https://github.com/AreteA4/arete/commit/3dc05a4eddebca9636827562f9793fdf1c16c1c9))
* Logging ([b727602](https://github.com/AreteA4/arete/commit/b727602bd0a232fcede8bfdeea4c9ec3d060483d))
* **macros:** convert lookup_indexes field_name to camelCase ([0c65c07](https://github.com/AreteA4/arete/commit/0c65c071adb36a768708eb53d05f3dcf0fb3c3b6))
* **macros:** simplify account mapping and warn on IDL mismatch ([597ea1e](https://github.com/AreteA4/arete/commit/597ea1e155e9fa19572f1d32a4c3089a7c7c57ca))

## [0.3.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.1...arete-macros-v0.3.2) (2026-01-20)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.1](https://github.com/AreteA4/arete/compare/arete-macros-v0.3.0...arete-macros-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.5...arete-macros-v0.3.0) (2026-01-20)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.2.5](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.4...arete-macros-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.2.4](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.3...arete-macros-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.2.3](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.2...arete-macros-v0.2.3) (2026-01-18)


### Features

* add append hints to frame protocol for granular array updates ([ce2213f](https://github.com/AreteA4/arete/commit/ce2213fc5a2c242cb4833ab417ff3d71f918812f))
* **server:** add trace context propagation and expanded telemetry ([0dbd8ed](https://github.com/AreteA4/arete/commit/0dbd8ed49780dd2f8f793b6af2425b47d9ccb151))


### Bug Fixes

* preserve VM state across reconnections and add memory management ([7fba770](https://github.com/AreteA4/arete/commit/7fba770df913dd0fbd06e43b402c6c288b25acbb))

## [0.2.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.1...arete-macros-v0.2.2) (2026-01-16)


### Features

* **interpreter:** add memory limits and LRU eviction to prevent unbounded growth ([33198a6](https://github.com/AreteA4/arete/commit/33198a69833de6e57f0c5fe568b0714a2105e987))
* **interpreter:** add staleness detection to reject out-of-order gRPC updates ([d693f42](https://github.com/AreteA4/arete/commit/d693f421742258bbbd3528ffbbd4731d638c992b))


### Bug Fixes

* Clippy errors ([d6a9f4d](https://github.com/AreteA4/arete/commit/d6a9f4d27f619d05189f421e214f6eacb8c19542))

## [0.2.1](https://github.com/AreteA4/arete/compare/arete-macros-v0.2.0...arete-macros-v0.2.1) (2026-01-16)


### Features

* add gRPC stream reconnection with exponential backoff ([48e3ec7](https://github.com/AreteA4/arete/commit/48e3ec7a952399135a84323da78cdc499804bce9))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.11...arete-macros-v0.2.0) (2026-01-15)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.1.11](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.10...arete-macros-v0.1.11) (2026-01-14)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.1.10](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.9...arete-macros-v0.1.10) (2026-01-13)


### Features

* Add deterministic sorting ([e775f59](https://github.com/AreteA4/arete/commit/e775f598a95165b2dd5504be67d24e7b1dabc766))

## [0.1.9](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.8...arete-macros-v0.1.9) (2026-01-13)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.1.8](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.6...arete-macros-v0.1.8) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/AreteA4/arete/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/AreteA4/arete/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/AreteA4/arete/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* Preserve integer types in computed field expressions ([616f042](https://github.com/AreteA4/arete/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/AreteA4/arete/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.6](https://github.com/AreteA4/arete/compare/v0.1.5...v0.1.6) (2026-01-13)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/AreteA4/arete/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/AreteA4/arete/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/AreteA4/arete/commit/72348a42ee3988e94873db0d24317f3e661e093d))
* Preserve integer types in computed field expressions ([616f042](https://github.com/AreteA4/arete/commit/616f04288637a84a4eed0febebf9867e06d134cb))
* Update naming ([4381946](https://github.com/AreteA4/arete/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.5](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.4...arete-macros-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/AreteA4/arete/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Map syntax ([8a5eaad](https://github.com/AreteA4/arete/commit/8a5eaadf5642dc7e569b2591e8e051c728a6eb9f))
* Module name snake case ([72348a4](https://github.com/AreteA4/arete/commit/72348a42ee3988e94873db0d24317f3e661e093d))

## [0.1.4](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.2...arete-macros-v0.1.4) (2026-01-09)


### Miscellaneous Chores

* **arete-macros:** Synchronize arete versions

## [0.1.2](https://github.com/AreteA4/arete/compare/arete-macros-v0.1.1...arete-macros-v0.1.2) (2026-01-09)


### Bug Fixes

* Update naming ([4381946](https://github.com/AreteA4/arete/commit/4381946147e9c51c7de0cb0e63a052c9e9379600))

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-spec-macros-v0.1.0...arete-spec-macros-v0.1.1) (2026-01-09)


### Features

* Better sdk types during generation ([f9555ef](https://github.com/AreteA4/arete/commit/f9555ef440eb9271a147d178d8b3554cf532b9c7))


### Bug Fixes

* Clippy errors/warnings ([e18fcd6](https://github.com/AreteA4/arete/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
