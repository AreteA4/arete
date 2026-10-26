# Changelog

## [0.5.0](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.4.0...arete-protocol-npm-v0.5.0) (2026-10-26)


### Features

* **protocol:** accept list indexing and in in the local expression check ([19d3a66](https://github.com/AreteA4/arete/commit/19d3a6677fa21c363f662fea2ffc364336004f48))
* **protocol:** accept list indexing and in in the local expression check ([60a9889](https://github.com/AreteA4/arete/commit/60a988905b91fd78345675a78d984436ac7f8408))
* **protocol:** allow state-only template steps ([273d0d5](https://github.com/AreteA4/arete/commit/273d0d51b9d4ec783af9dced3401ce40b4dac409))
* **protocol:** allow state-only template steps ([75ea8aa](https://github.com/AreteA4/arete/commit/75ea8aa773f20493cc58de01e91ad41c0ed5f198))
* **protocol:** check a release against the server before deploying ([443c48b](https://github.com/AreteA4/arete/commit/443c48b3161a4b3f32aa70aa711c3d60dce5bce6))
* **protocol:** check a release against the server before deploying ([7f2d37a](https://github.com/AreteA4/arete/commit/7f2d37a1f612dd8860296ba357ec68f1129e6add))


### Bug Fixes

* **protocol:** a remote check passes only on a check result and gives up in time ([53617e7](https://github.com/AreteA4/arete/commit/53617e75d4f59105e4a49fed594c39e64c8858ca))
* **protocol:** own only binds that purely derive from an ephemeral key ([7273e02](https://github.com/AreteA4/arete/commit/7273e0299dda1c053f7b838589017e7c88008cc8))
* **protocol:** own only binds that purely derive from an ephemeral key ([83740d1](https://github.com/AreteA4/arete/commit/83740d131e40bfb0d8dd4a4f8215e712b4961e65))
* **protocol:** the local checker stops rejecting what the server accepts ([0577c53](https://github.com/AreteA4/arete/commit/0577c53b2a125834ae669ae22c5715974ee89788))
* **protocol:** the local checker stops rejecting what the server accepts ([7c19655](https://github.com/AreteA4/arete/commit/7c19655d1c58d84c20347809179a5527abeefb48))

## [0.4.0](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.3.0...arete-protocol-npm-v0.4.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* **protocol:** arete-protocol check and build refuse Squads steps whose multisig reads the instance, args, params or wallet. Declare the multisig as a resource and use resources.<name>.multisig.

### Features

* **protocol:** a Squads step's multisig comes from a resource ([b10b64f](https://github.com/AreteA4/arete/commit/b10b64f641edc7e193aae71d3a17eb4dec1bc9ad))


### Bug Fixes

* **protocol:** keep the error for a bare resources multisig ([14b4724](https://github.com/AreteA4/arete/commit/14b47247ff1ff069b849ab4ca946be53743b2990))

## [0.3.0](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.2.2...arete-protocol-npm-v0.3.0) (2026-10-24)


### Features

* **protocol:** let a squadsProposer step choose its Squads vault ([fc98944](https://github.com/AreteA4/arete/commit/fc98944680600f8430a165d83c1709f46f239a04))
* **protocol:** let a squadsProposer step choose its Squads vault ([598787b](https://github.com/AreteA4/arete/commit/598787b302ef9c2005bb656b18f92abeeab0d0b4))
* **sdk:** export program extension helpers from @usearete/sdk/program ([8e1e234](https://github.com/AreteA4/arete/commit/8e1e234f446daff3942a09f917bf7029ed329d08))
* **sdk:** export program extension helpers from @usearete/sdk/program ([9f64a35](https://github.com/AreteA4/arete/commit/9f64a3529ed741e8bdd306aaff4abf8c5491688d))


### Bug Fixes

* **protocol:** keep unknown Squads authority fields so typos are rejected ([d9da14f](https://github.com/AreteA4/arete/commit/d9da14f7153e16d97980edeef12dab5f6e8dd189))

## [0.2.2](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.2.1...arete-protocol-npm-v0.2.2) (2026-10-22)


### Bug Fixes

* exclude injected runtime legs from protocol activity ([56e3baa](https://github.com/AreteA4/arete/commit/56e3baa2d3235ae4e4ce0279b5f3e7b85745dfff))
* scope protocol activity joins to attested operation instructions ([119048b](https://github.com/AreteA4/arete/commit/119048bf5caefdfcef5e097d443a5a6f80ab0473))

## [0.2.1](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.2.0...arete-protocol-npm-v0.2.1) (2026-10-20)


### Bug Fixes

* **protocol:** allow builtin program stack activity ([751d5a7](https://github.com/AreteA4/arete/commit/751d5a73979d92bc74153011d6da7eb47959ba4a))
* **protocol:** allow declared builtin programs in stack activity ([221d0ea](https://github.com/AreteA4/arete/commit/221d0ea3327552c443752baa5b7d871493d554f0))

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-protocol-npm-v0.1.0...arete-protocol-npm-v0.2.0) (2026-10-18)


### Features

* **protocol:** add protocol authoring and compiler package ([a6fd271](https://github.com/AreteA4/arete/commit/a6fd271e108472f0762dc5c6997615d3812bfbd3))
* **protocol:** add protocol authoring and compiler package ([bb13503](https://github.com/AreteA4/arete/commit/bb1350387b4059c3099568dddb4bf9e75e907bd8))


### Bug Fixes

* **protocol:** harden handler builds replay and account validation ([3bc50d3](https://github.com/AreteA4/arete/commit/3bc50d3c34ea15bf54939b66a47637f28e2c1aa7))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * @usearete/hash bumped from ^0.6.1 to ^0.7.0
