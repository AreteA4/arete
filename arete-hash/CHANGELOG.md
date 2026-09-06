# Changelog

## Unreleased

### Features

* add strict hosted-private Program Release V3 identity and shared vectors
* register the catalog bundle publication identities (`knowledge-document`,
  `knowledge-snapshot`, `extension-surface`, `sdk-install-target`,
  `catalog-bundle`, `catalog-publication-set`) with shared Rust/TypeScript
  vectors; no existing identity changes

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
