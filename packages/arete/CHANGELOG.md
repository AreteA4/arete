# Changelog

## [0.34.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.33.0...a4-npm-v0.34.0) (2026-10-10)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.33.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.32.0...a4-npm-v0.33.0) (2026-10-09)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.32.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.31.0...a4-npm-v0.32.0) (2026-10-08)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.31.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.30.0...a4-npm-v0.31.0) (2026-10-07)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.30.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.29.1...a4-npm-v0.30.0) (2026-10-07)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.29.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.29.0...a4-npm-v0.29.1) (2026-10-06)

### Dependencies

* Align all linked Arete packages at 0.29.1 after the hash dependency update.

## [0.29.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.28.0...a4-npm-v0.29.0) (2026-10-05)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.28.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.27.0...a4-npm-v0.28.0) (2026-09-30)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.27.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.26.0...a4-npm-v0.27.0) (2026-09-29)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.26.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.25.1...a4-npm-v0.26.0) (2026-09-29)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.25.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.25.0...a4-npm-v0.25.1) (2026-09-29)


### Bug Fixes

* **npm:** run project installs through the installed binary ([121b753](https://github.com/AreteA4/arete/commit/121b753bf2923ec86a32823e55fd1a0bc81e12c9))
* **npm:** run project installs through the installed binary ([7469180](https://github.com/AreteA4/arete/commit/7469180361016ec5593a86016de5a9a30403d588))

## [0.25.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.24.0...a4-npm-v0.25.0) (2026-09-28)


### ⚠ BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))

## [0.24.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.23.1...a4-npm-v0.24.0) (2026-09-27)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.23.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.23.0...a4-npm-v0.23.1) (2026-09-25)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.23.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.22.4...a4-npm-v0.23.0) (2026-09-25)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.22.4](https://github.com/AreteA4/arete/compare/a4-npm-v0.22.3...a4-npm-v0.22.4) (2026-09-24)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.22.3](https://github.com/AreteA4/arete/compare/a4-npm-v0.22.2...a4-npm-v0.22.3) (2026-09-24)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.22.2](https://github.com/AreteA4/arete/compare/a4-npm-v0.22.1...a4-npm-v0.22.2) (2026-09-24)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.22.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.22.0...a4-npm-v0.22.1) (2026-09-23)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.22.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.21.0...a4-npm-v0.22.0) (2026-09-22)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.21.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.20.4...a4-npm-v0.21.0) (2026-09-21)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.20.4](https://github.com/AreteA4/arete/compare/a4-npm-v0.20.3...a4-npm-v0.20.4) (2026-09-20)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.20.3](https://github.com/AreteA4/arete/compare/a4-npm-v0.20.2...a4-npm-v0.20.3) (2026-09-19)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.20.2](https://github.com/AreteA4/arete/compare/a4-npm-v0.20.1...a4-npm-v0.20.2) (2026-09-19)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.20.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.20.0...a4-npm-v0.20.1) (2026-09-17)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.20.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.19.1...a4-npm-v0.20.0) (2026-09-16)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.19.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.19.0...a4-npm-v0.19.1) (2026-09-16)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.19.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.18.0...a4-npm-v0.19.0) (2026-09-15)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.18.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.17.0...a4-npm-v0.18.0) (2026-09-14)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.17.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.16.0...a4-npm-v0.17.0) (2026-09-10)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.16.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.15.0...a4-npm-v0.16.0) (2026-09-09)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.15.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.14.0...a4-npm-v0.15.0) (2026-09-09)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.14.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.13.0...a4-npm-v0.14.0) (2026-09-06)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.13.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.12.0...a4-npm-v0.13.0) (2026-09-03)


### Features

* agent-first onboarding (self install/update, init, doctor, mcp, signed installers) ([fb58c5b](https://github.com/AreteA4/arete/commit/fb58c5b023ffa7d52b1188d8a21c392440ed7aa8))
* **release:** signed release artifacts, installers and scriptless npm bootstrapper ([0200f9d](https://github.com/AreteA4/arete/commit/0200f9d7810ce1a05916a95eaf20ec6e8273dca1))


### Bug Fixes

* **cli:** address review findings and CI ([7439939](https://github.com/AreteA4/arete/commit/74399393969d3d04c8ef69b8b18889d8d6f7e6ab))

## [0.12.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.11.0...a4-npm-v0.12.0) (2026-09-02)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.11.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.10.0...a4-npm-v0.11.0) (2026-09-01)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.10.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.9.1...a4-npm-v0.10.0) (2026-08-22)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.9.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.9.0...a4-npm-v0.9.1) (2026-08-20)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.9.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.8.2...a4-npm-v0.9.0) (2026-08-18)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.8.2](https://github.com/AreteA4/arete/compare/a4-npm-v0.8.1...a4-npm-v0.8.2) (2026-08-17)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.8.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.8.0...a4-npm-v0.8.1) (2026-08-16)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.8.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.7.2...a4-npm-v0.8.0) (2026-08-16)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.7.2](https://github.com/AreteA4/arete/compare/a4-npm-v0.7.1...a4-npm-v0.7.2) (2026-08-15)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.7.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.7.0...a4-npm-v0.7.1) (2026-08-15)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.7.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.6.0...a4-npm-v0.7.0) (2026-08-14)


### Bug Fixes

* preserve composed signers and binary fallback ([75991e0](https://github.com/AreteA4/arete/commit/75991e06bb89b423e7088311cadad65db936178f))
* prevent recursive npm CLI launches ([bb6398d](https://github.com/AreteA4/arete/commit/bb6398df59c0d337881d39adc31a7b5316eb75d9))

## [0.6.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.5.0...a4-npm-v0.6.0) (2026-08-13)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.4.1...a4-npm-v0.5.0) (2026-08-12)


### Bug Fixes

* normalize npm wrapper bin paths ([41cdfe3](https://github.com/AreteA4/arete/commit/41cdfe3a856b87fb98d64262c97f76c1c9657fa8))

## [0.4.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.4.0...a4-npm-v0.4.1) (2026-08-02)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.4.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.3.0...a4-npm-v0.4.0) (2026-07-31)


### Features

* introduce the versioned public artifact model ([9c6777a](https://github.com/AreteA4/arete/commit/9c6777a3fe1703cc7491b56afaaac0bc5940b321))

## [0.3.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.2.0...a4-npm-v0.3.0) (2026-07-22)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.2.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.5...a4-npm-v0.2.0) (2026-07-13)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.1.5](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.4...a4-npm-v0.1.5) (2026-06-18)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.1.4](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.3...a4-npm-v0.1.4) (2026-06-17)


### Bug Fixes

* add manual npm publish recovery tooling ([1563f85](https://github.com/AreteA4/arete/commit/1563f858e9c5c75a54275bbe0a923273ddbbb9ef))

## [0.1.3](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.2...a4-npm-v0.1.3) (2026-05-30)


### Bug Fixes

* Version bump ([8dde791](https://github.com/AreteA4/arete/commit/8dde7918b4a9e45efac51266ef6fe7c290c03c5e))
* Version bump ([5c9ae22](https://github.com/AreteA4/arete/commit/5c9ae22baf2764b3223f0a3174b9b7738631932c))

## [0.1.2](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.1...a4-npm-v0.1.2) (2026-05-22)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.1.1](https://github.com/AreteA4/arete/compare/a4-npm-v0.1.0...a4-npm-v0.1.1) (2026-04-30)


### Miscellaneous Chores

* **a4-npm:** Synchronize arete versions

## [0.1.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.0.1...a4-npm-v0.1.0) (2026-04-21)


### Bug Fixes

* harden cli postinstall downloads ([7f8dbfb](https://github.com/AreteA4/arete/commit/7f8dbfb1e6aa866ad13956d2ec5cc28e9456495c))

## [0.6.9](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.8...arete-npm-v0.6.9) (2026-04-15)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.8](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.7...arete-npm-v0.6.8) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.7](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.6...arete-npm-v0.6.7) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.6](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.5...arete-npm-v0.6.6) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.5](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.4...arete-npm-v0.6.5) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.4](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.3...arete-npm-v0.6.4) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.3](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.2...arete-npm-v0.6.3) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.2](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.1...arete-npm-v0.6.2) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.1](https://github.com/AreteA4/arete/compare/arete-npm-v0.6.0...arete-npm-v0.6.1) (2026-04-05)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.10...arete-npm-v0.6.0) (2026-04-04)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.10](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.9...arete-npm-v0.5.10) (2026-03-19)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.9](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.6...arete-npm-v0.5.9) (2026-03-19)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.6](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.5...arete-npm-v0.5.6) (2026-03-19)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.5](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.4...arete-npm-v0.5.5) (2026-03-14)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.4](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.3...arete-npm-v0.5.4) (2026-03-14)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.3](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.2...arete-npm-v0.5.3) (2026-02-20)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.2](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.1...arete-npm-v0.5.2) (2026-02-07)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.1](https://github.com/AreteA4/arete/compare/arete-npm-v0.5.0...arete-npm-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-npm-v0.4.3...arete-npm-v0.5.0) (2026-02-06)


### Features

* **cli:** add npm wrapper package for cross-platform distribution ([07e5080](https://github.com/AreteA4/arete/commit/07e5080b563e01dbcf8ba1879cc9ba10d708ea6f))


### Bug Fixes

* **cli:** register only a4-cli command to avoid overwriting a4 binary ([1a9a291](https://github.com/AreteA4/arete/commit/1a9a291ce83e03749593eda9c066b12696f887df))
* NPM CLI looks at PATH ([bb8f2eb](https://github.com/AreteA4/arete/commit/bb8f2eb2c3a29e9b8bfbf589eb522a154e119188))

## [0.4.3](https://github.com/AreteA4/arete/compare/arete-npm-v0.4.2...arete-npm-v0.4.3) (2026-02-03)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.4.2](https://github.com/AreteA4/arete/compare/arete-npm-v0.4.1...arete-npm-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.4.1](https://github.com/AreteA4/arete/compare/arete-npm-v0.4.0...arete-npm-v0.4.1) (2026-02-01)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.15...arete-npm-v0.4.0) (2026-01-31)


### Features

* **cli:** add npm wrapper package for cross-platform distribution ([07e5080](https://github.com/AreteA4/arete/commit/07e5080b563e01dbcf8ba1879cc9ba10d708ea6f))


### Bug Fixes

* **cli:** register only a4-cli command to avoid overwriting a4 binary ([1a9a291](https://github.com/AreteA4/arete/commit/1a9a291ce83e03749593eda9c066b12696f887df))
* NPM CLI looks at PATH ([bb8f2eb](https://github.com/AreteA4/arete/commit/bb8f2eb2c3a29e9b8bfbf589eb522a154e119188))

## [0.3.15](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.14...arete-npm-v0.3.15) (2026-01-31)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.3.14](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.13...arete-npm-v0.3.14) (2026-01-28)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.3.13](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.12...arete-npm-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.3.12](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.11...arete-npm-v0.3.12) (2026-01-28)


### Bug Fixes

* **cli:** register only a4-cli command to avoid overwriting a4 binary ([1a9a291](https://github.com/AreteA4/arete/commit/1a9a291ce83e03749593eda9c066b12696f887df))

## [0.3.11](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.10...arete-npm-v0.3.11) (2026-01-28)


### Bug Fixes

* NPM CLI looks at PATH ([bb8f2eb](https://github.com/AreteA4/arete/commit/bb8f2eb2c3a29e9b8bfbf589eb522a154e119188))

## [0.3.10](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.9...arete-npm-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.3.9](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.8...arete-npm-v0.3.9) (2026-01-28)


### Miscellaneous Chores

* **arete-npm:** Synchronize arete versions

## [0.3.8](https://github.com/AreteA4/arete/compare/arete-npm-v0.3.7...arete-npm-v0.3.8) (2026-01-28)


### Features

* **cli:** add npm wrapper package for cross-platform distribution ([07e5080](https://github.com/AreteA4/arete/commit/07e5080b563e01dbcf8ba1879cc9ba10d708ea6f))
