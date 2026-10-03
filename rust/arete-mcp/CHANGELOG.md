# Changelog

## [0.28.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.27.0...arete-mcp-v0.28.0) (2026-09-30)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.27.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.26.0...arete-mcp-v0.27.0) (2026-09-29)


### Features

* **discovery:** guide agent trials to starter stacks ([97883dc](https://github.com/AreteA4/arete/commit/97883dc21ee328a662befe24c8c3c06aa6a36a52))
* isolate agent credentials and propagate recovery actions ([cf3716a](https://github.com/AreteA4/arete/commit/cf3716a5bb888a265b05e292bca4ae706844f893))


### Bug Fixes

* address agent credential recovery review ([c5cdd56](https://github.com/AreteA4/arete/commit/c5cdd569037701bdfb4a252401c826c9a842f1f4))
* preserve MCP account status fields ([3134bed](https://github.com/AreteA4/arete/commit/3134beda351828c978423dfb7ba0bafc236f7a6d))

## [0.26.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.25.1...arete-mcp-v0.26.0) (2026-09-29)


### Features

* **explore:** attach curated field descriptions from catalog knowledge ([eaf6313](https://github.com/AreteA4/arete/commit/eaf63137cd5019f777900956df671e520c5ac567))
* **explore:** attach curated field descriptions from catalog knowledge ([9c38a3a](https://github.com/AreteA4/arete/commit/9c38a3a6a4c6af7bc64b2170d09149fa2c5d7dd9))


### Bug Fixes

* **explore:** skip the knowledge lookup where it cannot apply and bound it ([b2666d0](https://github.com/AreteA4/arete/commit/b2666d0f51dc039daef203d281dc67106acd373b))
* **mcp:** attach schema guidance only for the StackManifest it describes ([cd6e011](https://github.com/AreteA4/arete/commit/cd6e0115add59e92f58acd981673b0f888054e5d))

## [0.25.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.25.0...arete-mcp-v0.25.1) (2026-09-29)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.24.0...arete-mcp-v0.25.0) (2026-09-28)


### ⚠ BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))

## [0.24.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.23.1...arete-mcp-v0.24.0) (2026-09-27)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.23.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.23.0...arete-mcp-v0.23.1) (2026-09-25)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.23.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.22.4...arete-mcp-v0.23.0) (2026-09-25)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.22.4](https://github.com/AreteA4/arete/compare/arete-mcp-v0.22.3...arete-mcp-v0.22.4) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.22.3](https://github.com/AreteA4/arete/compare/arete-mcp-v0.22.2...arete-mcp-v0.22.3) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.22.2](https://github.com/AreteA4/arete/compare/arete-mcp-v0.22.1...arete-mcp-v0.22.2) (2026-09-24)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.22.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.22.0...arete-mcp-v0.22.1) (2026-09-23)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.22.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.21.0...arete-mcp-v0.22.0) (2026-09-22)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.21.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.20.4...arete-mcp-v0.21.0) (2026-09-21)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.20.4](https://github.com/AreteA4/arete/compare/arete-mcp-v0.20.3...arete-mcp-v0.20.4) (2026-09-20)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.20.3](https://github.com/AreteA4/arete/compare/arete-mcp-v0.20.2...arete-mcp-v0.20.3) (2026-09-19)


### Bug Fixes

* declare IDL types shared by several entities once per stack SDK ([d1151f7](https://github.com/AreteA4/arete/commit/d1151f7dea1783588a525ee432621b56f20740b8))

## [0.20.2](https://github.com/AreteA4/arete/compare/arete-mcp-v0.20.1...arete-mcp-v0.20.2) (2026-09-19)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.20.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.20.0...arete-mcp-v0.20.1) (2026-09-17)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.20.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.19.1...arete-mcp-v0.20.0) (2026-09-16)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.19.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.19.0...arete-mcp-v0.19.1) (2026-09-16)


### Bug Fixes

* **sdk,cli:** let a client recognise a configured hosted suffix ([6becadc](https://github.com/AreteA4/arete/commit/6becadcb9ff89157087b80eb6ec76c05c37aa281))
* **sdk,cli:** let a client recognise a configured hosted suffix ([dbf7139](https://github.com/AreteA4/arete/commit/dbf7139585b481db87d8890e352047e6ab11e6cf))

## [0.19.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.18.0...arete-mcp-v0.19.0) (2026-09-15)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.18.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.17.0...arete-mcp-v0.18.0) (2026-09-14)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.17.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.16.0...arete-mcp-v0.17.0) (2026-09-10)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.16.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.15.0...arete-mcp-v0.16.0) (2026-09-09)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.15.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.14.0...arete-mcp-v0.15.0) (2026-09-09)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.14.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.13.0...arete-mcp-v0.14.0) (2026-09-06)


### Features

* **cli,mcp:** add catalog discovery clients ([ef7616f](https://github.com/AreteA4/arete/commit/ef7616ff15e944425a4964448a5f69e9a9e19b39))
* register catalog bundle identities and add catalog discovery clients (Plan 039) ([146b9b9](https://github.com/AreteA4/arete/commit/146b9b9ab9516f7d3e2555b79c11a489fd6b3be8))


### Bug Fixes

* **cli,mcp:** paginate catalog search, pin install refs, harden slugs ([c862623](https://github.com/AreteA4/arete/commit/c8626233d27c22736a3209634ab10aecd228217c))

## [0.13.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.12.0...arete-mcp-v0.13.0) (2026-09-03)


### Features

* agent-first onboarding (self install/update, init, doctor, mcp, signed installers) ([fb58c5b](https://github.com/AreteA4/arete/commit/fb58c5b023ffa7d52b1188d8a21c392440ed7aa8))
* **cli:** agent-first onboarding: self install/update, init, doctor, mcp, auth signup ([838a878](https://github.com/AreteA4/arete/commit/838a878e8bbe25fb841ae59bae06b99b763b52a3))

## [0.12.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.11.0...arete-mcp-v0.12.0) (2026-09-02)


### Features

* **cli:** add Arete project manifest dependencies ([729dd97](https://github.com/AreteA4/arete/commit/729dd97284335f4879a65eace78f98e69cc9fd18))

## [0.11.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.10.0...arete-mcp-v0.11.0) (2026-09-01)


### Features

* expose knowledge layer tools in the MCP server ([7886a9f](https://github.com/AreteA4/arete/commit/7886a9f28faea14182fd09cb15828c845f749a41))
* expose the curated knowledge layer via CLI, MCP, and operation brands ([3a6bc01](https://github.com/AreteA4/arete/commit/3a6bc01a6ce78f66609573656db3344a123b3e44))


### Bug Fixes

* fall back to the default API URL when ARETE_API_URL normalizes to empty ([540f328](https://github.com/AreteA4/arete/commit/540f32849f73c0b545c062bfffcdd24cf2d1f0d7))
* make normalized credential key lookup deterministic ([41c9f81](https://github.com/AreteA4/arete/commit/41c9f8129e5737313bff67097ad77c1a5bcdcfb4))
* **mcp:** match credential lookup hosts case-insensitively ([efcf666](https://github.com/AreteA4/arete/commit/efcf6664280418efff2b071d568a5fdf901833b8))
* **mcp:** strip FQDN trailing dot in credential URL lookup ([3a3ed81](https://github.com/AreteA4/arete/commit/3a3ed81d0924006effdd245ccbe325b5ea0d07a2))
* normalize API URLs when matching credential keys ([f6c90ee](https://github.com/AreteA4/arete/commit/f6c90ee46ee74af8f858efd2d4ee8b9c2d3bed43))

## [0.10.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.9.1...arete-mcp-v0.10.0) (2026-08-22)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.9.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.9.0...arete-mcp-v0.9.1) (2026-08-20)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.9.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.8.2...arete-mcp-v0.9.0) (2026-08-18)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.8.2](https://github.com/AreteA4/arete/compare/arete-mcp-v0.8.1...arete-mcp-v0.8.2) (2026-08-17)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.8.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.8.0...arete-mcp-v0.8.1) (2026-08-16)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.8.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.7.2...arete-mcp-v0.8.0) (2026-08-16)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.7.2](https://github.com/AreteA4/arete/compare/arete-mcp-v0.7.1...arete-mcp-v0.7.2) (2026-08-15)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.7.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.7.0...arete-mcp-v0.7.1) (2026-08-15)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.7.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.6.0...arete-mcp-v0.7.0) (2026-08-14)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.5.0...arete-mcp-v0.6.0) (2026-08-13)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.4.1...arete-mcp-v0.5.0) (2026-08-12)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.4.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.4.0...arete-mcp-v0.4.1) (2026-08-02)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.3.0...arete-mcp-v0.4.0) (2026-07-31)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.2.0...arete-mcp-v0.3.0) (2026-07-22)


### Features

* add WebSocket v2 and reactive ORE stack workflows ([ce37702](https://github.com/AreteA4/arete/commit/ce3770249a0899dd0be37f47cac2deeaa4e58511))


### Bug Fixes

* scope MCP cached reads to exact subscriptions ([4b1e51a](https://github.com/AreteA4/arete/commit/4b1e51a80ea72714f619d00fc1325c5ad9fa6d33))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.5...arete-mcp-v0.2.0) (2026-07-13)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.5](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.4...arete-mcp-v0.1.5) (2026-06-18)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.4](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.3...arete-mcp-v0.1.4) (2026-06-17)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.3](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.2...arete-mcp-v0.1.3) (2026-05-30)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.2](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.1...arete-mcp-v0.1.2) (2026-05-22)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-mcp-v0.1.0...arete-mcp-v0.1.1) (2026-04-30)


### Miscellaneous Chores

* **arete-mcp:** Synchronize arete versions

## [0.1.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.0.1...arete-mcp-v0.1.0) (2026-04-21)


### Features

* add npm wrapper for a4-mcp ([43107e8](https://github.com/AreteA4/arete/commit/43107e8a85ce93e0f18316a2cbd14c6cd94bb991))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-sdk bumped from 0.0.1 to 0.1.0
