# Changelog

## [0.30.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.29.1...arete-mcp-npm-v0.30.0) (2026-10-07)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.29.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.29.0...arete-mcp-npm-v0.29.1) (2026-10-06)

### Dependencies

* Align all linked Arete packages at 0.29.1 after the hash dependency update.

## [0.29.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.28.0...arete-mcp-npm-v0.29.0) (2026-10-05)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.28.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.27.0...arete-mcp-npm-v0.28.0) (2026-09-30)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.27.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.26.0...arete-mcp-npm-v0.27.0) (2026-09-29)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.26.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.25.1...arete-mcp-npm-v0.26.0) (2026-09-29)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.25.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.25.0...arete-mcp-npm-v0.25.1) (2026-09-29)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.24.0...arete-mcp-npm-v0.25.0) (2026-09-28)


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

## [0.24.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.23.1...arete-mcp-npm-v0.24.0) (2026-09-27)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.23.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.23.0...arete-mcp-npm-v0.23.1) (2026-09-25)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.23.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.22.4...arete-mcp-npm-v0.23.0) (2026-09-25)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.22.4](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.22.3...arete-mcp-npm-v0.22.4) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.22.3](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.22.2...arete-mcp-npm-v0.22.3) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.22.2](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.22.1...arete-mcp-npm-v0.22.2) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.22.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.22.0...arete-mcp-npm-v0.22.1) (2026-09-23)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.22.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.21.0...arete-mcp-npm-v0.22.0) (2026-09-22)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.21.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.20.4...arete-mcp-npm-v0.21.0) (2026-09-21)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.20.4](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.20.3...arete-mcp-npm-v0.20.4) (2026-09-20)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.20.3](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.20.2...arete-mcp-npm-v0.20.3) (2026-09-19)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.20.2](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.20.1...arete-mcp-npm-v0.20.2) (2026-09-19)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.20.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.20.0...arete-mcp-npm-v0.20.1) (2026-09-17)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.20.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.19.1...arete-mcp-npm-v0.20.0) (2026-09-16)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.19.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.19.0...arete-mcp-npm-v0.19.1) (2026-09-16)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.19.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.18.0...arete-mcp-npm-v0.19.0) (2026-09-15)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.18.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.17.0...arete-mcp-npm-v0.18.0) (2026-09-14)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.17.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.16.0...arete-mcp-npm-v0.17.0) (2026-09-10)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.16.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.15.0...arete-mcp-npm-v0.16.0) (2026-09-09)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.15.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.14.0...arete-mcp-npm-v0.15.0) (2026-09-09)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.14.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.13.0...arete-mcp-npm-v0.14.0) (2026-09-06)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.13.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.12.0...arete-mcp-npm-v0.13.0) (2026-09-03)


### Features

* agent-first onboarding (self install/update, init, doctor, mcp, signed installers) ([fb58c5b](https://github.com/AreteA4/arete/commit/fb58c5b023ffa7d52b1188d8a21c392440ed7aa8))
* **cli:** agent-first onboarding: self install/update, init, doctor, mcp, auth signup ([838a878](https://github.com/AreteA4/arete/commit/838a878e8bbe25fb841ae59bae06b99b763b52a3))

## [0.12.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.11.0...arete-mcp-npm-v0.12.0) (2026-09-02)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.11.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.10.0...arete-mcp-npm-v0.11.0) (2026-09-01)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.10.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.9.1...arete-mcp-npm-v0.10.0) (2026-08-22)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.9.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.9.0...arete-mcp-npm-v0.9.1) (2026-08-20)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.9.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.8.2...arete-mcp-npm-v0.9.0) (2026-08-18)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.8.2](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.8.1...arete-mcp-npm-v0.8.2) (2026-08-17)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.8.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.8.0...arete-mcp-npm-v0.8.1) (2026-08-16)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.8.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.7.2...arete-mcp-npm-v0.8.0) (2026-08-16)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.7.2](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.7.1...arete-mcp-npm-v0.7.2) (2026-08-15)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.7.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.7.0...arete-mcp-npm-v0.7.1) (2026-08-15)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.7.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.6.0...arete-mcp-npm-v0.7.0) (2026-08-14)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.5.0...arete-mcp-npm-v0.6.0) (2026-08-13)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.4.1...arete-mcp-npm-v0.5.0) (2026-08-12)


### Bug Fixes

* normalize npm wrapper bin paths ([41cdfe3](https://github.com/AreteA4/arete/commit/41cdfe3a856b87fb98d64262c97f76c1c9657fa8))

## [0.4.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.4.0...arete-mcp-npm-v0.4.1) (2026-08-02)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.3.0...arete-mcp-npm-v0.4.0) (2026-07-31)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.2.0...arete-mcp-npm-v0.3.0) (2026-07-22)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.5...arete-mcp-npm-v0.2.0) (2026-07-13)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.1.5](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.4...arete-mcp-npm-v0.1.5) (2026-06-18)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.1.4](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.3...arete-mcp-npm-v0.1.4) (2026-06-17)


### Bug Fixes

* add manual npm publish recovery tooling ([1563f85](https://github.com/AreteA4/arete/commit/1563f858e9c5c75a54275bbe0a923273ddbbb9ef))

## [0.1.3](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.2...arete-mcp-npm-v0.1.3) (2026-05-30)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.1.2](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.1...arete-mcp-npm-v0.1.2) (2026-05-22)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.1.0...arete-mcp-npm-v0.1.1) (2026-04-30)


### Miscellaneous Chores

* **arete-mcp-npm:** Synchronize arete versions

## [0.1.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.0.1...arete-mcp-npm-v0.1.0) (2026-04-21)


### Features

* add npm wrapper for a4-mcp ([43107e8](https://github.com/AreteA4/arete/commit/43107e8a85ce93e0f18316a2cbd14c6cd94bb991))


### Bug Fixes

* harden mcp postinstall downloads ([f6feb89](https://github.com/AreteA4/arete/commit/f6feb89dbc9172a7370e47ee200c7234e2e518a8))
