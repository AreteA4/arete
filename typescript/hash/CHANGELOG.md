# Changelog

## Unreleased

### Features

* add strict hosted-private Program Release V3 identity and shared vectors

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-hash-npm-v0.3.1...arete-hash-npm-v0.4.0) (2026-09-01)


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

## [0.3.1](https://github.com/AreteA4/arete/compare/arete-hash-npm-v0.3.0...arete-hash-npm-v0.3.1) (2026-08-14)


### Bug Fixes

* preserve safe instruction account metadata ([3cb4ad5](https://github.com/AreteA4/arete/commit/3cb4ad56095af56e3e28e574d1301e90e6a25c7f))

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-hash-npm-v0.2.0...arete-hash-npm-v0.3.0) (2026-08-12)


### Features

* add managed release v2 contracts ([8d86427](https://github.com/AreteA4/arete/commit/8d864276f0267898ec70912b89e8fd2147a26b12))
* add managed release v2 contracts ([16db61a](https://github.com/AreteA4/arete/commit/16db61aea9658ceabff66cb672b195fe09848f5e))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-hash-npm-v0.1.0...arete-hash-npm-v0.2.0) (2026-07-31)


### Features

* add arete-h1 typed artifact identity and public artifact schemas ([f0e9bca](https://github.com/AreteA4/arete/commit/f0e9bca50846abf9eaa5db0c8ac13def39c51947))
* introduce the versioned public artifact model ([9c6777a](https://github.com/AreteA4/arete/commit/9c6777a3fe1703cc7491b56afaaac0bc5940b321))
