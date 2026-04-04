# Changelog

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.5...hyperstack-idl-v0.1.6) (2026-04-04)


### Bug Fixes

* restore case-insensitive IDL lookups and per-event validation ([636fd6d](https://github.com/HyperTekOrg/hyperstack/commit/636fd6d0c6b166c0be50e3cb4b8a9ada43c52050))
* sort derive_from attrs and align field lookup semantics ([eea1af0](https://github.com/HyperTekOrg/hyperstack/commit/eea1af028bfec5fc52668f2279f7eb2a7c834c92))
* surface hyperstack macro validation failures during expansion ([7928539](https://github.com/HyperTekOrg/hyperstack/commit/7928539d0a9a4e53db546f4f65d35f26e2e95560))
* tighten IDL lookup casing and derive_from diagnostics ([de5706d](https://github.com/HyperTekOrg/hyperstack/commit/de5706d59be3942a2f1612426f2b1ee5cb0ce817))

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.4...hyperstack-idl-v0.1.5) (2026-03-19)


### Bug Fixes

* handle null discriminant in snapshot deserialization ([b373cde](https://github.com/HyperTekOrg/hyperstack/commit/b373cdeca7d87e79c01c5bcdc639160c77ed953a))
* preserve explicit discriminant_size values in IDL snapshot ([9e60c87](https://github.com/HyperTekOrg/hyperstack/commit/9e60c87ebafb1903db8e085e848ed4b29d3d7c85))
* preserve explicit discriminant_size values in IDL snapshot ([0ccdd91](https://github.com/HyperTekOrg/hyperstack/commit/0ccdd9175a5e0accfcae5973f1ae74a1b5dfbc1f))

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.3...hyperstack-idl-v0.1.4) (2026-03-19)


### Bug Fixes

* Improve Steel IDL detection for 1-byte discriminator arrays ([3fcd1ee](https://github.com/HyperTekOrg/hyperstack/commit/3fcd1ee31bf5b01353c69c5630c122667557ddf6))
* Improve Steel IDL detection for 1-byte discriminator arrays ([f3a7f9c](https://github.com/HyperTekOrg/hyperstack/commit/f3a7f9c628b4a66d195cc510707bb721a2908fad))

## [0.1.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.2...hyperstack-idl-v0.1.3) (2026-03-19)


### Bug Fixes

* align Steel discriminant size with get_discriminator return value ([4d283f2](https://github.com/HyperTekOrg/hyperstack/commit/4d283f2bd749690eb7b79e1c80e0447c79b35d8d))
* change any() to all() for Steel-style IDL detection ([77d2566](https://github.com/HyperTekOrg/hyperstack/commit/77d2566db00255cc5460e3ea4490302d0e530a25))
* Core interpreter and server improvements ([b05ae9b](https://github.com/HyperTekOrg/hyperstack/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* implement discriminant size inference and fix test failures ([c8b26d5](https://github.com/HyperTekOrg/hyperstack/commit/c8b26d58f62041cb7c5fe1624f5313f63a9ef9d9))
* prevent empty instruction arrays from being misclassified as Steel-style ([abad594](https://github.com/HyperTekOrg/hyperstack/commit/abad594108d0f9b1f795d7f320e7f69b1027fce6))
* replace panicking expect with graceful fallback in get_discriminator ([fa09ef9](https://github.com/HyperTekOrg/hyperstack/commit/fa09ef9cce75b64df295971c35ade31a724522c5))
* replace silent u8 truncation of Steel discriminant with try_from ([f4de6ef](https://github.com/HyperTekOrg/hyperstack/commit/f4de6ef1a835f204a8af39b4610481356bd62410))
* replace unnecessary unwrap with if let pattern ([b3e6dac](https://github.com/HyperTekOrg/hyperstack/commit/b3e6dac0cda3428471f2d648200c3b866f26e108))
* **tests:** replace diagnostic println with assertion for discriminant_size ([f391581](https://github.com/HyperTekOrg/hyperstack/commit/f39158107e551a544bb610ca4a8d7a59e81f6460))

## [0.1.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.1...hyperstack-idl-v0.1.2) (2026-03-14)


### Features

* Bump hyperstack idl version ([857f819](https://github.com/HyperTekOrg/hyperstack/commit/857f819ad97dbb8296e33765094d42a452aaf91f))

## [0.1.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-idl-v0.1.0...hyperstack-idl-v0.1.1) (2026-03-14)


### Features

* **idl:** add compute_discriminator public API ([48de0ae](https://github.com/HyperTekOrg/hyperstack/commit/48de0ae92ac45747208260befa977ac2363225fd))
* **idl:** add connect analysis module for path finding ([75ea359](https://github.com/HyperTekOrg/hyperstack/commit/75ea35917112ce5717125bd43bdc29be147d44be))
* **idl:** add constants field support to IdlSpec ([d9af98f](https://github.com/HyperTekOrg/hyperstack/commit/d9af98fbd101a8821614326ac8b4976d25a4c2d4))
* **idl:** add packed representation support to IdlRepr ([04486af](https://github.com/HyperTekOrg/hyperstack/commit/04486af36571ab761ec4804fefcf3dadd23db7a9))
* **idl:** add PDA graph analysis ([db9db3c](https://github.com/HyperTekOrg/hyperstack/commit/db9db3c793b5d0096bf500c0c16a22a7ca2e6427))
* **idl:** add relations analysis module ([bdc52eb](https://github.com/HyperTekOrg/hyperstack/commit/bdc52ebbb273c6ec52f6752e4eb40e35b98f8848))
* **idl:** add search module with fuzzy matching and structured errors ([541a8a5](https://github.com/HyperTekOrg/hyperstack/commit/541a8a51e91f2e64e2c4d6a1fe02e0f69c243284))
* **idl:** add snake_case/pascal_case utilities ([1eba345](https://github.com/HyperTekOrg/hyperstack/commit/1eba345f699892fd96916efa33b665b6cf39b002))
* **idl:** add type graph analysis + release-please independent versioning ([a64a26d](https://github.com/HyperTekOrg/hyperstack/commit/a64a26df62d6db190dd7a7f0762bf8144aa0e6a2))
* **idl:** create hyperstack-idl crate skeleton ([90712df](https://github.com/HyperTekOrg/hyperstack/commit/90712df6ece12a8bda63941417ff96361eaf59c1))
* **idl:** extract core IDL parsing types into hyperstack-idl ([1b1ea56](https://github.com/HyperTekOrg/hyperstack/commit/1b1ea5616e2d7ea3ea904c026491aa61000dd8b2))
* **idl:** extract snapshot types with backwards-compatible HashMap handling ([fe882e7](https://github.com/HyperTekOrg/hyperstack/commit/fe882e739ac5162d9e281ce385cc7e5de7729f02))
* misc compiler, VM, and IDL improvements ([2d6aea3](https://github.com/HyperTekOrg/hyperstack/commit/2d6aea373e43c84e3a07ecef7d9dab004a0b8c1c))


### Bug Fixes

* **idl:** remove redundant closure in pda_graph (clippy) ([4566a9d](https://github.com/HyperTekOrg/hyperstack/commit/4566a9db588199bf080eb77083052ba7a2bdcaad))
