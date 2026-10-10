# Changelog

## [0.34.0](https://github.com/AreteA4/arete/compare/arete-python-v0.33.0...arete-python-v0.34.0) (2026-10-10)


### Features

* **sdk:** fall back to the a4 CLI login for server-side auth ([76d821d](https://github.com/AreteA4/arete/commit/76d821d150b1ea0c4d7349eb6a2f2c2552000bf6))
* **sdk:** fall back to the a4 CLI login for server-side auth ([6963931](https://github.com/AreteA4/arete/commit/696393133f7c7a377151c111e4f24610743d7140))


### Bug Fixes

* **sdk:** keep a4 login keys on the Arete API and tighten profile checks ([9a287b6](https://github.com/AreteA4/arete/commit/9a287b63017bb3a7415641360db8632e95ac34e0))
* **sdk:** scope the a4 login key restriction to its own auth config ([37fb91c](https://github.com/AreteA4/arete/commit/37fb91cd9dd6fc6d2567c166cb1f94e6dfb36874))

## [0.33.0](https://github.com/AreteA4/arete/compare/arete-python-v0.32.0...arete-python-v0.33.0) (2026-10-09)


### Features

* **sdk:** add server-only secretKey auth ([61e6766](https://github.com/AreteA4/arete/commit/61e6766c59bd4bfcb0e7df23c53e5906b070d4ed))
* **sdk:** add server-only secretKey auth ([b65b5ff](https://github.com/AreteA4/arete/commit/b65b5ff0b2d119b9afbb08920c448c063928e82d))


### Bug Fixes

* **sdk:** validate keys in low-level constructors and hide credentials from repr ([d1293a9](https://github.com/AreteA4/arete/commit/d1293a9a5bb955dfc184416602f0baa5b1b7f3e2))

## [0.32.0](https://github.com/AreteA4/arete/compare/arete-python-v0.31.0...arete-python-v0.32.0) (2026-10-08)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.31.0](https://github.com/AreteA4/arete/compare/arete-python-v0.30.0...arete-python-v0.31.0) (2026-10-07)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.30.0](https://github.com/AreteA4/arete/compare/arete-python-v0.29.1...arete-python-v0.30.0) (2026-10-07)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.29.1](https://github.com/AreteA4/arete/compare/arete-python-v0.29.0...arete-python-v0.29.1) (2026-10-06)

### Dependencies

* Align all linked Arete packages at 0.29.1 after the hash dependency update.

## [0.29.0](https://github.com/AreteA4/arete/compare/arete-python-v0.28.0...arete-python-v0.29.0) (2026-10-05)


### Features

* **contracts:** freeze public managed Solana v1 lifecycle contracts ([e14af8a](https://github.com/AreteA4/arete/commit/e14af8a7fc1282c33d90a29cc9aa838a08ff64cf))
* **python-sdk:** support managed Solana capability contracts ([c97379d](https://github.com/AreteA4/arete/commit/c97379dc650ae8ae11ad1d9d7ad198a5e944f268))
* **python:** add BuiltInstruction.to_artifact for TypeScript instruction artifacts ([f8a1143](https://github.com/AreteA4/arete/commit/f8a1143aeda23e4d513aa7b4f451e414c821edf6))
* **python:** add keccak256, sha256 and base58 helpers to arete ([b8e8545](https://github.com/AreteA4/arete/commit/b8e8545e41a43d975c520f34c46c9280dc1af5d7))
* **python:** apply PROGRAM_EXTENSIONS and STACK_EXTENSIONS, add program create_read ([da23d51](https://github.com/AreteA4/arete/commit/da23d510098691a9ee9badebf430e3f50ebde736))
* **python:** carry signer material on prepared transactions ([c8007fe](https://github.com/AreteA4/arete/commit/c8007fe9a31223b868cca040dd2a92fbcb2a4e9d))
* **python:** decode TypeScript AmountInput shapes ([586bb23](https://github.com/AreteA4/arete/commit/586bb23762ef56f4807f59fa67f4d036cf27069a))
* Rust and Python SDK extensions with full TypeScript parity ([f70a9f4](https://github.com/AreteA4/arete/commit/f70a9f4a124182a3c9503f8c878c12cb641bc325))
* **solana:** add contextual reads, discovery, and account lifecycle support ([4dbffb7](https://github.com/AreteA4/arete/commit/4dbffb777db78fac5ff5b321b2b84d7e3e073eb9))


### Bug Fixes

* **python:** default the ATA token program on None and raise TypeScript's messages ([572f41f](https://github.com/AreteA4/arete/commit/572f41ffefeab782d4810efe2bc9faa112fe945e))
* **python:** fill only wallet signers from the build's wallet ([6c101a8](https://github.com/AreteA4/arete/commit/6c101a8100dfc28ce5f39bcc99c5097842c65cf1))
* **python:** honour an explicit address for every instruction account ([c200c4b](https://github.com/AreteA4/arete/commit/c200c4bc591cf70d028a5df46a6951fa1e3dd846))

## [0.28.0](https://github.com/AreteA4/arete/compare/arete-python-v0.27.0...arete-python-v0.28.0) (2026-09-30)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.27.0](https://github.com/AreteA4/arete/compare/arete-python-v0.26.0...arete-python-v0.27.0) (2026-09-29)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.26.0](https://github.com/AreteA4/arete/compare/arete-python-v0.25.1...arete-python-v0.26.0) (2026-09-29)


### Bug Fixes

* **python:** keep a server's connection error message as sent ([1b1a37e](https://github.com/AreteA4/arete/commit/1b1a37edf8b4b53a62da50e321b38420f1a8abfe))
* **python:** make get and get_one fail fast when the connection fails ([09a67d6](https://github.com/AreteA4/arete/commit/09a67d6a056543938aea800555417b4f0f7c4ef1))
* **python:** make get and get_one fail fast when the connection fails ([e98a863](https://github.com/AreteA4/arete/commit/e98a86354d3973619edcad4ed3158d51dd63a2b6))

## [0.25.1](https://github.com/AreteA4/arete/compare/arete-python-v0.25.0...arete-python-v0.25.1) (2026-09-29)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-python-v0.24.0...arete-python-v0.25.0) (2026-09-28)


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
* **sdk:** order one key's frames by the server's version ([0b47eaf](https://github.com/AreteA4/arete/commit/0b47eafc0910c9006637a9759848a87203758c3b))
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


### Documentation

* **cli:** point agents at the starter templates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** document identity, testing, wallets and mutation phases ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))

## [0.24.0](https://github.com/AreteA4/arete/compare/arete-python-v0.23.1...arete-python-v0.24.0) (2026-09-27)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.23.1](https://github.com/AreteA4/arete/compare/arete-python-v0.23.0...arete-python-v0.23.1) (2026-09-25)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.23.0](https://github.com/AreteA4/arete/compare/arete-python-v0.22.4...arete-python-v0.23.0) (2026-09-25)


### ⚠ BREAKING CHANGES

* **sdk:** in the Rust SDK, AuthErrorCode gains the StackVersionRetired and StackVersionUnknown variants, and AreteError::AuthRequestFailed gains a stack_version field. Exhaustive matches on AuthErrorCode, and patterns or constructors of AuthRequestFailed without `..`, need updating.

### Features

* **sdk:** name the served stack version when minting sessions ([aa06836](https://github.com/AreteA4/arete/commit/aa068362d434fb38f2998677fca763f3dba53714))
* **sdk:** name the served stack version when minting sessions ([b4d3587](https://github.com/AreteA4/arete/commit/b4d3587f6adb1cc1722537c572be6f779822c20a))


### Bug Fixes

* **sdk:** keep socket issue guidance, unknown codes and close reasons ([8246496](https://github.com/AreteA4/arete/commit/8246496d7e9d1f7966edac4096a361b296c766ad))

## [0.22.4](https://github.com/AreteA4/arete/compare/arete-python-v0.22.3...arete-python-v0.22.4) (2026-09-24)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.22.3](https://github.com/AreteA4/arete/compare/arete-python-v0.22.2...arete-python-v0.22.3) (2026-09-24)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.22.2](https://github.com/AreteA4/arete/compare/arete-python-v0.22.1...arete-python-v0.22.2) (2026-09-24)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.22.1](https://github.com/AreteA4/arete/compare/arete-python-v0.22.0...arete-python-v0.22.1) (2026-09-23)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.22.0](https://github.com/AreteA4/arete/compare/arete-python-v0.21.0...arete-python-v0.22.0) (2026-09-22)


### Features

* **sdk:** expose replay cursors and fail closed on data gaps ([#243](https://github.com/AreteA4/arete/issues/243)) ([129919d](https://github.com/AreteA4/arete/commit/129919d28dbf7f87781fcc398e36059c0963c2fd))

## [0.21.0](https://github.com/AreteA4/arete/compare/arete-python-v0.20.4...arete-python-v0.21.0) (2026-09-21)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.20.4](https://github.com/AreteA4/arete/compare/arete-python-v0.20.3...arete-python-v0.20.4) (2026-09-20)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.20.3](https://github.com/AreteA4/arete/compare/arete-python-v0.20.2...arete-python-v0.20.3) (2026-09-19)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.20.2](https://github.com/AreteA4/arete/compare/arete-python-v0.20.1...arete-python-v0.20.2) (2026-09-19)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.20.1](https://github.com/AreteA4/arete/compare/arete-python-v0.20.0...arete-python-v0.20.1) (2026-09-17)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.20.0](https://github.com/AreteA4/arete/compare/arete-python-v0.19.1...arete-python-v0.20.0) (2026-09-16)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.19.1](https://github.com/AreteA4/arete/compare/arete-python-v0.19.0...arete-python-v0.19.1) (2026-09-16)


### Bug Fixes

* **sdk,cli:** let a client recognise a configured hosted suffix ([6becadc](https://github.com/AreteA4/arete/commit/6becadcb9ff89157087b80eb6ec76c05c37aa281))
* **sdk:** reach the suffix configuration from Python and the browser ([9a54556](https://github.com/AreteA4/arete/commit/9a545561527548d26cca056b779e1d1dbda81eda))

## [0.19.0](https://github.com/AreteA4/arete/compare/arete-python-v0.18.0...arete-python-v0.19.0) (2026-09-15)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.18.0](https://github.com/AreteA4/arete/compare/arete-python-v0.17.0...arete-python-v0.18.0) (2026-09-14)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.17.0](https://github.com/AreteA4/arete/compare/arete-python-v0.16.0...arete-python-v0.17.0) (2026-09-10)


### Features

* **sdk:** provide an optional Python solders transaction adapter ([#201](https://github.com/AreteA4/arete/issues/201)) ([fd7ce0e](https://github.com/AreteA4/arete/commit/fd7ce0e0c67fc1c0488a54dbe3189787a590a2c5))

## [0.16.0](https://github.com/AreteA4/arete/compare/arete-python-v0.15.0...arete-python-v0.16.0) (2026-09-09)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.15.0](https://github.com/AreteA4/arete/compare/arete-python-v0.14.0...arete-python-v0.15.0) (2026-09-09)


### ⚠ BREAKING CHANGES

* **sdk:** public Rust structs gain fields (SendOptions transaction_version/resources, TransactionSimulationResult loaded_accounts_data_size), so struct-literal construction without ..Default::default() no longer compiles. The TypeScript adapter interfaces require supportedTransactionVersions, and resource keys that previously passed through the SendOptions index signature untouched are now typed and validated.

### Features

* **sdk:** preserve simulation budgets and define the V1 option contract ([#198](https://github.com/AreteA4/arete/issues/198)) ([38d9537](https://github.com/AreteA4/arete/commit/38d9537f77a37a7ffa89b9f23ba08da641e8fe99))

## [0.14.0](https://github.com/AreteA4/arete/compare/arete-python-v0.13.0...arete-python-v0.14.0) (2026-09-06)


### ⚠ BREAKING CHANGES

* **hash:** arete_interpreter::program_sdk no longer exports extract_pdas_from_idl or extract_instructions_from_idl. They were an unused second copy of the IDL converters that had drifted from the canonical implementation and reproduced the PDA misclassification this change fixes. No deprecated shim is provided: the corrected converters live in arete-hash, are private, and return arete-hash types, so a wrapper would mean re-exporting internals and maintaining a type bridge between two representations, which is the duplication being removed. Callers should use arete-hash's ProgramSpec projection.

### Bug Fixes

* **hash:** preserve instruction-local PDA provenance ([56278d4](https://github.com/AreteA4/arete/commit/56278d452c436d36f2b9b4ee4dd89ef8ad25aeec))

## [0.13.0](https://github.com/AreteA4/arete/compare/arete-python-v0.12.0...arete-python-v0.13.0) (2026-09-03)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.12.0](https://github.com/AreteA4/arete/compare/arete-python-v0.11.0...arete-python-v0.12.0) (2026-09-02)


### Miscellaneous Chores

* **arete-python:** Synchronize arete versions

## [0.11.0](https://github.com/AreteA4/arete/compare/arete-python-v0.10.0...arete-python-v0.11.0) (2026-09-01)


### Features

* **cli:** support owner-private program uploads ([2351c39](https://github.com/AreteA4/arete/commit/2351c39e8bf67c1cab53f1cd43368c410df88678))
* expose the curated knowledge layer via CLI, MCP, and operation brands ([3a6bc01](https://github.com/AreteA4/arete/commit/3a6bc01a6ce78f66609573656db3344a123b3e44))
* generate Python SDKs and align cross-language workflows ([e12471b](https://github.com/AreteA4/arete/commit/e12471b0ef149cd80c4d2b6fcd6c6530803d998f))
* generate Python SDKs and align cross-language workflows ([ed7938b](https://github.com/AreteA4/arete/commit/ed7938b18c519214c471015cb11e20cd7ab2f319))
* **sdk:** auto-wire managed Solana gateway transports ([ccab1b0](https://github.com/AreteA4/arete/commit/ccab1b0e1d011ef582918cbd789710538683bc99))
* support u64-length-prefixed sequences in IDL types ([#168](https://github.com/AreteA4/arete/issues/168)) ([a37f07a](https://github.com/AreteA4/arete/commit/a37f07a662f9d6e04afb97aa906174582b5d825c))


### Bug Fixes

* **interpreter:** encode tuple arguments across runtimes ([67e273b](https://github.com/AreteA4/arete/commit/67e273bf6e730533c36984f86cbd0afa56e46b6a))
* **release:** link Python SDK version ([daf809b](https://github.com/AreteA4/arete/commit/daf809bf4ab1f300189cb5ab47a85ed51c823e5c))
* wire managed gateways and stabilize ORE entropy routing ([55b50bb](https://github.com/AreteA4/arete/commit/55b50bb15e92d2d2dbabc8dddb4fed835e4c8e15))


### Documentation

* align API key examples with arete_/aretepk_ prefixes ([2bc7ab4](https://github.com/AreteA4/arete/commit/2bc7ab4860d3141f56bb2619912e1cc9c54d6d71))
* Arete API key prefix examples (a4_sk_/a4_pk_) ([ae62482](https://github.com/AreteA4/arete/commit/ae624829974a0e5ef6b462f2545a8966fecddcbe))
* rename API key prefixes to a4_sk_/a4_pk_ ([d274b20](https://github.com/AreteA4/arete/commit/d274b2066a0dc61c73baf43219cefbf53fdb92fe))
* rename API key prefixes to a4-sk_/a4-pub_ ([7252a7f](https://github.com/AreteA4/arete/commit/7252a7f0a9e5653e53756b7f2a46b5eb774facfa))
