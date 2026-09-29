# Changelog

## [0.26.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.25.1...arete-adapter-web3js-v0.26.0) (2026-09-29)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.25.1 to ^0.26.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.25.1 to ^0.26.0

## [0.25.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.25.0...arete-adapter-web3js-v0.25.1) (2026-09-29)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.25.0 to ^0.25.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.25.0 to ^0.25.1

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.24.0...arete-adapter-web3js-v0.25.0) (2026-09-28)


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


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0

## [0.24.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.23.1...arete-adapter-web3js-v0.24.0) (2026-09-27)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.23.1 to ^0.24.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.23.1 to ^0.24.0

## [0.23.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.23.0...arete-adapter-web3js-v0.23.1) (2026-09-25)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.23.0 to ^0.23.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.23.0 to ^0.23.1

## [0.23.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.22.4...arete-adapter-web3js-v0.23.0) (2026-09-25)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.22.4 to ^0.23.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.22.4 to ^0.23.0

## [0.22.4](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.22.3...arete-adapter-web3js-v0.22.4) (2026-09-24)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.22.3 to ^0.22.4
  * peerDependencies
    * @usearete/sdk bumped from ^0.22.3 to ^0.22.4

## [0.22.3](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.22.2...arete-adapter-web3js-v0.22.3) (2026-09-24)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.22.2 to ^0.22.3
  * peerDependencies
    * @usearete/sdk bumped from ^0.22.2 to ^0.22.3

## [0.22.2](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.22.1...arete-adapter-web3js-v0.22.2) (2026-09-24)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.22.1 to ^0.22.2
  * peerDependencies
    * @usearete/sdk bumped from ^0.22.1 to ^0.22.2

## [0.22.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.22.0...arete-adapter-web3js-v0.22.1) (2026-09-23)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.22.0 to ^0.22.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.22.0 to ^0.22.1

## [0.22.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.21.0...arete-adapter-web3js-v0.22.0) (2026-09-22)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.21.0 to ^0.22.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.21.0 to ^0.22.0

## [0.21.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.20.4...arete-adapter-web3js-v0.21.0) (2026-09-21)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.20.4 to ^0.21.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.20.4 to ^0.21.0

## [0.20.4](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.20.3...arete-adapter-web3js-v0.20.4) (2026-09-20)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.20.3 to ^0.20.4
  * peerDependencies
    * @usearete/sdk bumped from ^0.20.3 to ^0.20.4

## [0.20.3](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.20.2...arete-adapter-web3js-v0.20.3) (2026-09-19)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.20.2 to ^0.20.3
  * peerDependencies
    * @usearete/sdk bumped from ^0.20.2 to ^0.20.3

## [0.20.2](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.20.1...arete-adapter-web3js-v0.20.2) (2026-09-19)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.20.1 to ^0.20.2
  * peerDependencies
    * @usearete/sdk bumped from ^0.20.1 to ^0.20.2

## [0.20.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.20.0...arete-adapter-web3js-v0.20.1) (2026-09-17)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.20.0 to ^0.20.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.20.0 to ^0.20.1

## [0.20.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.19.1...arete-adapter-web3js-v0.20.0) (2026-09-16)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.19.1 to ^0.20.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.19.1 to ^0.20.0

## [0.19.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.19.0...arete-adapter-web3js-v0.19.1) (2026-09-16)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.19.0 to ^0.19.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.19.0 to ^0.19.1

## [0.19.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.18.0...arete-adapter-web3js-v0.19.0) (2026-09-15)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.18.0 to ^0.19.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.18.0 to ^0.19.0

## [0.18.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.17.0...arete-adapter-web3js-v0.18.0) (2026-09-14)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.17.0 to ^0.18.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.17.0 to ^0.18.0

## [0.17.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.16.0...arete-adapter-web3js-v0.17.0) (2026-09-10)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.16.0 to ^0.17.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.16.0 to ^0.17.0

## [0.16.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.15.0...arete-adapter-web3js-v0.16.0) (2026-09-09)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.15.0 to ^0.16.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.15.0 to ^0.16.0

## [0.15.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.14.0...arete-adapter-web3js-v0.15.0) (2026-09-09)


### ⚠ BREAKING CHANGES

* **sdk:** public Rust structs gain fields (SendOptions transaction_version/resources, TransactionSimulationResult loaded_accounts_data_size), so struct-literal construction without ..Default::default() no longer compiles. The TypeScript adapter interfaces require supportedTransactionVersions, and resource keys that previously passed through the SendOptions index signature untouched are now typed and validated.

### Features

* **sdk:** preserve simulation budgets and define the V1 option contract ([#198](https://github.com/AreteA4/arete/issues/198)) ([38d9537](https://github.com/AreteA4/arete/commit/38d9537f77a37a7ffa89b9f23ba08da641e8fe99))


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.14.0 to ^0.15.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.14.0 to ^0.15.0

## [0.14.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.13.0...arete-adapter-web3js-v0.14.0) (2026-09-06)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.13.0 to ^0.14.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.13.0 to ^0.14.0

## [0.13.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.12.0...arete-adapter-web3js-v0.13.0) (2026-09-03)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.12.0 to ^0.13.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.12.0 to ^0.13.0

## [0.12.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.11.0...arete-adapter-web3js-v0.12.0) (2026-09-02)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.11.0 to ^0.12.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.11.0 to ^0.12.0

## [0.11.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.10.0...arete-adapter-web3js-v0.11.0) (2026-09-01)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.10.0 to ^0.11.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.10.0 to ^0.11.0

## [0.10.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.9.1...arete-adapter-web3js-v0.10.0) (2026-08-22)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.9.1 to ^0.10.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.9.1 to ^0.10.0

## [0.9.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.9.0...arete-adapter-web3js-v0.9.1) (2026-08-20)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.9.0 to ^0.9.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.9.0 to ^0.9.1

## [0.9.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.8.2...arete-adapter-web3js-v0.9.0) (2026-08-18)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.8.2 to ^0.9.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.8.2 to ^0.9.0

## [0.8.2](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.8.1...arete-adapter-web3js-v0.8.2) (2026-08-17)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.8.1 to ^0.8.2
  * peerDependencies
    * @usearete/sdk bumped from ^0.8.1 to ^0.8.2

## [0.8.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.8.0...arete-adapter-web3js-v0.8.1) (2026-08-16)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.8.0 to ^0.8.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.8.0 to ^0.8.1

## [0.8.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.7.2...arete-adapter-web3js-v0.8.0) (2026-08-16)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.7.2 to ^0.8.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.7.2 to ^0.8.0

## [0.7.2](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.7.1...arete-adapter-web3js-v0.7.2) (2026-08-15)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.7.1 to ^0.7.2
  * peerDependencies
    * @usearete/sdk bumped from ^0.7.1 to ^0.7.2

## [0.7.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.7.0...arete-adapter-web3js-v0.7.1) (2026-08-15)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.7.0 to ^0.7.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.7.0 to ^0.7.1

## [0.7.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.6.0...arete-adapter-web3js-v0.7.0) (2026-08-14)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.6.0 to ^0.7.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.6.0 to ^0.7.0

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.5.0...arete-adapter-web3js-v0.6.0) (2026-08-13)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.5.0 to ^0.6.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.5.0 to ^0.6.0

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.4.1...arete-adapter-web3js-v0.5.0) (2026-08-12)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.4.1 to ^0.5.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.4.1 to ^0.5.0

## [0.4.1](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.4.0...arete-adapter-web3js-v0.4.1) (2026-08-02)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.4.0 to ^0.4.1
  * peerDependencies
    * @usearete/sdk bumped from ^0.4.0 to ^0.4.1

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.3.0...arete-adapter-web3js-v0.4.0) (2026-07-31)


### Miscellaneous Chores

* **arete-adapter-web3js:** Synchronize arete versions


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.3.0 to ^0.4.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.3.0 to ^0.4.0

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.1.0...arete-adapter-web3js-v0.3.0) (2026-07-22)


### Features

* **adapters:** support Arete transaction transport ([5e45cee](https://github.com/AreteA4/arete/commit/5e45cee23ffd8a343f911429cc4ff0b325861c45))
* add generated program SDKs, sessions, HTTP reads, and typed execution ([f1437f1](https://github.com/AreteA4/arete/commit/f1437f1cbb615e8451d4f381c8e497c227f40c04))
* add React wallet bridge to web3.js adapter ([70fe0ae](https://github.com/AreteA4/arete/commit/70fe0ae47dc3262ae87e29b5cd0db231a3d06aaf))
* add reference wallet adapters for instruction execution ([ae481cb](https://github.com/AreteA4/arete/commit/ae481cb0361f26ac59e71e75c51780f70571fd28))
* add safe ORE transaction workflows ([5923c14](https://github.com/AreteA4/arete/commit/5923c1454d56c8e6b1f3a5a7b8765b863a581e03))
* add WebSocket v2 and reactive ORE stack workflows ([ce37702](https://github.com/AreteA4/arete/commit/ce3770249a0899dd0be37f47cac2deeaa4e58511))
* support multi-signer wallet adapters ([e8dacfe](https://github.com/AreteA4/arete/commit/e8dacfeca9a9592b34a1c0600c8c8b48932399fc))


### Bug Fixes

* **adapter:** pin CommonJS-compatible websocket dependency ([1bab683](https://github.com/AreteA4/arete/commit/1bab6832037335b612f2765855a602346f1f1beb))
* **adapter:** publish compatible websocket dependency ([b89a7c7](https://github.com/AreteA4/arete/commit/b89a7c7b86395ea23574aa42009683db815a7962))
* **adapter:** require Node 20-compatible websocket release ([cb1d4eb](https://github.com/AreteA4/arete/commit/cb1d4eb6414f6304adbc8e339dd61a6c58241842))


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.2.0 || ^0.3.0 to ^0.3.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.2.0 || ^0.3.0 to ^0.3.0
