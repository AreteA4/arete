# Changelog

## Unreleased

### Features

* add strict hosted-private Program Release V3 identity and shared vectors
* register the catalog bundle publication identities (`knowledge-document`,
  `knowledge-snapshot`, `extension-surface`, `sdk-install-target`,
  `catalog-bundle`, `catalog-publication-set`) with shared Rust/TypeScript
  vectors; no existing identity changes

## [0.8.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.7.0...arete-hash-v0.8.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* **protocol:** arete-protocol check and build refuse Squads steps whose multisig reads the instance, args, params or wallet. Declare the multisig as a resource and use resources.<name>.multisig.

### Features

* **protocol:** a Squads step's multisig comes from a resource ([b10b64f](https://github.com/AreteA4/arete/commit/b10b64f641edc7e193aae71d3a17eb4dec1bc9ad))

## [0.7.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.6.2...arete-hash-v0.7.0) (2026-10-24)


### Features

* **protocol:** let a squadsProposer step choose its Squads vault ([fc98944](https://github.com/AreteA4/arete/commit/fc98944680600f8430a165d83c1709f46f239a04))
* **protocol:** let a squadsProposer step choose its Squads vault ([598787b](https://github.com/AreteA4/arete/commit/598787b302ef9c2005bb656b18f92abeeab0d0b4))

## [0.6.2](https://github.com/AreteA4/arete/compare/arete-hash-v0.6.1...arete-hash-v0.6.2) (2026-10-22)


### Bug Fixes

* exclude injected runtime legs from protocol activity ([56e3baa](https://github.com/AreteA4/arete/commit/56e3baa2d3235ae4e4ce0279b5f3e7b85745dfff))
* scope protocol activity joins to attested operation instructions ([119048b](https://github.com/AreteA4/arete/commit/119048bf5caefdfcef5e097d443a5a6f80ab0473))

## [0.6.1](https://github.com/AreteA4/arete/compare/arete-hash-v0.6.0...arete-hash-v0.6.1) (2026-10-20)


### Bug Fixes

* **protocol:** allow builtin program stack activity ([751d5a7](https://github.com/AreteA4/arete/commit/751d5a73979d92bc74153011d6da7eb47959ba4a))
* **protocol:** allow declared builtin programs in stack activity ([221d0ea](https://github.com/AreteA4/arete/commit/221d0ea3327552c443752baa5b7d871493d554f0))

## [0.6.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.5.3...arete-hash-v0.6.0) (2026-10-18)


### Features

* **hash:** add protocol artifact contracts and native builtin identities ([6eb21da](https://github.com/AreteA4/arete/commit/6eb21da937fc1a617112be4f0ca8898611a5f44c))
* **hash:** add protocol artifact contracts and native builtin identities ([34a802b](https://github.com/AreteA4/arete/commit/34a802be4c35ee7c78d057aae15072586aa23b02))


### Bug Fixes

* **hash:** align IDL layouts and reject unsafe contract values ([8ed312f](https://github.com/AreteA4/arete/commit/8ed312f282d9f3419cb097b71a14aff961d11e55))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.8.0 to 0.9.0

## [0.5.3](https://github.com/AreteA4/arete/compare/arete-hash-v0.5.2...arete-hash-v0.5.3) (2026-10-11)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.7.0 to 0.8.0

## [0.5.2](https://github.com/AreteA4/arete/compare/arete-hash-v0.5.1...arete-hash-v0.5.2) (2026-10-06)


### Bug Fixes

* **hash:** honor explicitly untagged IDL accounts ([2f97c77](https://github.com/AreteA4/arete/commit/2f97c778203706faa348ba034f98c928a2637292))

## [0.5.1](https://github.com/AreteA4/arete/compare/arete-hash-v0.5.0...arete-hash-v0.5.1) (2026-10-05)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.6.0 to 0.7.0

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.4.0...arete-hash-v0.5.0) (2026-09-14)


### Features

* **sdk:** separate generated content identity from compiler provenance ([2f6c3bc](https://github.com/AreteA4/arete/commit/2f6c3bc24de462b22d8577ecb9b4d2f6e35ed21f))
* **sdk:** separate generated content identity from compiler provenance ([466be84](https://github.com/AreteA4/arete/commit/466be841bc1c73c67bc248cf47ca78b215479cbd))

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.3.2...arete-hash-v0.4.0) (2026-09-06)


### ⚠ BREAKING CHANGES

* **hash:** arete_interpreter::program_sdk no longer exports extract_pdas_from_idl or extract_instructions_from_idl. They were an unused second copy of the IDL converters that had drifted from the canonical implementation and reproduced the PDA misclassification this change fixes. No deprecated shim is provided: the corrected converters live in arete-hash, are private, and return arete-hash types, so a wrapper would mean re-exporting internals and maintaining a type bridge between two representations, which is the duplication being removed. Callers should use arete-hash's ProgramSpec projection.

### Features

* **hash:** register catalog bundle publication identities ([dcb1b4f](https://github.com/AreteA4/arete/commit/dcb1b4f6ac4751abf07cd9e2cb49d0e972aaa775))
* register catalog bundle identities and add catalog discovery clients (Plan 039) ([146b9b9](https://github.com/AreteA4/arete/commit/146b9b9ab9516f7d3e2555b79c11a489fd6b3be8))


### Bug Fixes

* **hash:** preserve instruction-local PDA provenance ([56278d4](https://github.com/AreteA4/arete/commit/56278d452c436d36f2b9b4ee4dd89ef8ad25aeec))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.5.0 to 0.6.0

## [0.3.2](https://github.com/AreteA4/arete/compare/arete-hash-v0.3.1...arete-hash-v0.3.2) (2026-09-03)


### Bug Fixes

* **hash:** preserve NUL PDA seeds as bytes ([d0798df](https://github.com/AreteA4/arete/commit/d0798df26d0d20a1531ff5d1cde736be4cda1e52))
* **hash:** preserve NUL PDA seeds as bytes ([5a2fa54](https://github.com/AreteA4/arete/commit/5a2fa54e3a6c80dd77b263fb8df411c72a7eeaa7))

## [0.3.1](https://github.com/AreteA4/arete/compare/arete-hash-v0.3.0...arete-hash-v0.3.1) (2026-09-02)


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.4.0 to 0.5.0

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.2.1...arete-hash-v0.3.0) (2026-09-01)


### ⚠ BREAKING CHANGES

* `normalizedIdlHash` changes for any IDL declaring an instruction discriminant whose type is not `u8`. Anchor IDLs and every Steel IDL in the catalog are unaffected.

### Features

* **cli:** support owner-private program uploads ([2351c39](https://github.com/AreteA4/arete/commit/2351c39e8bf67c1cab53f1cd43368c410df88678))
* derive the instruction discriminator width from the IDL ([#170](https://github.com/AreteA4/arete/issues/170)) ([9996dd5](https://github.com/AreteA4/arete/commit/9996dd5ee78a081726ce3de829f151c852b6751f))
* expose the curated knowledge layer via CLI, MCP, and operation brands ([3a6bc01](https://github.com/AreteA4/arete/commit/3a6bc01a6ce78f66609573656db3344a123b3e44))
* **hash:** define hosted-private program release v3 ([a1bd633](https://github.com/AreteA4/arete/commit/a1bd63320331e744af72c4ef1d26af67e375ab07))
* support u64-length-prefixed sequences in IDL types ([#168](https://github.com/AreteA4/arete/issues/168)) ([a37f07a](https://github.com/AreteA4/arete/commit/a37f07a662f9d6e04afb97aa906174582b5d825c))


### Bug Fixes

* **idl:** align cross-language ProgramSpec hashing ([7b42b11](https://github.com/AreteA4/arete/commit/7b42b1163a1c1cc54a770c63b03557492e6cbbb6))
* **idl:** parse inline tuple type nodes in legacy Anchor and Codama IDLs ([b717559](https://github.com/AreteA4/arete/commit/b7175597913aadede18c3ac4c865e07b884cdbaf))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.3.0 to 0.4.0

## [0.2.1](https://github.com/AreteA4/arete/compare/arete-hash-v0.2.0...arete-hash-v0.2.1) (2026-08-14)


### Bug Fixes

* preserve safe instruction account metadata ([3cb4ad5](https://github.com/AreteA4/arete/commit/3cb4ad56095af56e3e28e574d1301e90e6a25c7f))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-hash-v0.1.1...arete-hash-v0.2.0) (2026-08-12)


### Features

* add managed release v2 contracts ([8d86427](https://github.com/AreteA4/arete/commit/8d864276f0267898ec70912b89e8fd2147a26b12))
* add managed release v2 contracts ([16db61a](https://github.com/AreteA4/arete/commit/16db61aea9658ceabff66cb672b195fe09848f5e))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.2.0 to 0.3.0

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-hash-v0.1.0...arete-hash-v0.1.1) (2026-08-02)


### Bug Fixes

* align decoder fixture vectors with v1 ([f6ca2c3](https://github.com/AreteA4/arete/commit/f6ca2c3516e4728a2638fb09c8c1886d0aed542e))
* align decoder fixture vectors with v1 ([9104a09](https://github.com/AreteA4/arete/commit/9104a09400331ce1a9b9020d9f327b35cbe7759f))

## 0.1.0 (2026-07-31)


### Features

* add arete-h1 typed artifact identity and public artifact schemas ([f0e9bca](https://github.com/AreteA4/arete/commit/f0e9bca50846abf9eaa5db0c8ac13def39c51947))
* introduce the versioned public artifact model ([9c6777a](https://github.com/AreteA4/arete/commit/9c6777a3fe1703cc7491b56afaaac0bc5940b321))


### Bug Fixes

* satisfy Rust 1.97 checks and refresh generated SDKs ([f70238b](https://github.com/AreteA4/arete/commit/f70238b4a5369e13afc9de3866e9275b3efa0558))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-idl bumped from 0.1.0 to 0.2.0
