# Changelog

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
