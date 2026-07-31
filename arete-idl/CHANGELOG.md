# Changelog

## [0.2.0](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.0...arete-idl-v0.2.0) (2026-07-31)


### Features

* add arete-h1 typed artifact identity and public artifact schemas ([f0e9bca](https://github.com/AreteA4/arete/commit/f0e9bca50846abf9eaa5db0c8ac13def39c51947))
* introduce the versioned public artifact model ([9c6777a](https://github.com/AreteA4/arete/commit/9c6777a3fe1703cc7491b56afaaac0bc5940b321))

## [0.1.0](https://github.com/AreteA4/arete/compare/arete-idl-v0.0.3...arete-idl-v0.1.0) (2026-07-13)


### Features

* add generated program SDKs, sessions, HTTP reads, and typed execution ([f1437f1](https://github.com/AreteA4/arete/commit/f1437f1cbb615e8451d4f381c8e497c227f40c04))
* improve idl event discovery and validation ([970aed3](https://github.com/AreteA4/arete/commit/970aed3eeb8efe7ce0a4cb76f0310b58fb957d8a))
* preserve event fields in parsed IDLs ([f7fc0c6](https://github.com/AreteA4/arete/commit/f7fc0c60ebec9040d67e2f15b57bc5d777dcdc36))
* preserve richer IDL metadata in stack specs ([ba5a16b](https://github.com/AreteA4/arete/commit/ba5a16b8ab2dab2b6e493cf6950798dfd98b2dd6))
* remove external IDL fixture dependency ([284d263](https://github.com/AreteA4/arete/commit/284d263751f26b5bef1f7479c62ec33f64ed5f18))


### Bug Fixes

* normalize generated stream data shapes ([094c5ab](https://github.com/AreteA4/arete/commit/094c5abab496e62b87f1f64ccdbedd98d98e0437))
* remove external IDL fixture dependency ([9a054a2](https://github.com/AreteA4/arete/commit/9a054a2f1c25cf376463c97db096b9a82727c75b))

## [0.0.3](https://github.com/AreteA4/arete/compare/arete-idl-v0.0.2...arete-idl-v0.0.3) (2026-06-18)


### Features

* preserve richer IDL metadata in stack specs ([5c42827](https://github.com/AreteA4/arete/commit/5c42827bfcb19e32a1a99a19882c42e89733eaf0))
* preserve richer IDL metadata in stack specs ([b603a9e](https://github.com/AreteA4/arete/commit/b603a9e368ba828e64f8987cd5cd2cc343f1036d))


### Bug Fixes

* handle signed numeric PDA seed values in Codama IDL ([9c2cfef](https://github.com/AreteA4/arete/commit/9c2cfefc0a722745f2483a3e70a1d19c3b517efb))
* log silent PDA resolution failures for diagnosability ([60ecdfc](https://github.com/AreteA4/arete/commit/60ecdfc761b0e4426d86fda2e91b546ec20ebad7))
* surface unrecognised isSigner tags in Codama account docs ([c4a0809](https://github.com/AreteA4/arete/commit/c4a0809811994a43a4cd8e2392d1c2cbb8c33a59))

## [0.0.2](https://github.com/AreteA4/arete/compare/arete-idl-v0.0.1...arete-idl-v0.0.2) (2026-06-17)


### Features

* support Codama root-node IDLs and variable discriminators ([1ffcf84](https://github.com/AreteA4/arete/commit/1ffcf84e23363b06d83f07a22bf5b85e7d45b1f0))
* support Codama root-node IDLs in arete-idl ([5317df4](https://github.com/AreteA4/arete/commit/5317df447064d1ed8a4d5cd71f1cd300403f2e61))


### Bug Fixes

* surface errors for unsupported Codama IDL features instead of silent fallbacks ([f6fd10c](https://github.com/AreteA4/arete/commit/f6fd10c28d08fe71cff3cb090aecaa5150cf3e24))
* Update ore idls ([108a66f](https://github.com/AreteA4/arete/commit/108a66ffc40c39334a6a4d85ce6f1c82e3461bd0))

## [0.1.6](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.5...arete-idl-v0.1.6) (2026-04-04)


### Bug Fixes

* restore case-insensitive IDL lookups and per-event validation ([636fd6d](https://github.com/AreteA4/arete/commit/636fd6d0c6b166c0be50e3cb4b8a9ada43c52050))
* sort derive_from attrs and align field lookup semantics ([eea1af0](https://github.com/AreteA4/arete/commit/eea1af028bfec5fc52668f2279f7eb2a7c834c92))
* surface arete macro validation failures during expansion ([7928539](https://github.com/AreteA4/arete/commit/7928539d0a9a4e53db546f4f65d35f26e2e95560))
* tighten IDL lookup casing and derive_from diagnostics ([de5706d](https://github.com/AreteA4/arete/commit/de5706d59be3942a2f1612426f2b1ee5cb0ce817))

## [0.1.5](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.4...arete-idl-v0.1.5) (2026-03-19)


### Bug Fixes

* handle null discriminant in snapshot deserialization ([b373cde](https://github.com/AreteA4/arete/commit/b373cdeca7d87e79c01c5bcdc639160c77ed953a))
* preserve explicit discriminant_size values in IDL snapshot ([9e60c87](https://github.com/AreteA4/arete/commit/9e60c87ebafb1903db8e085e848ed4b29d3d7c85))
* preserve explicit discriminant_size values in IDL snapshot ([0ccdd91](https://github.com/AreteA4/arete/commit/0ccdd9175a5e0accfcae5973f1ae74a1b5dfbc1f))

## [0.1.4](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.3...arete-idl-v0.1.4) (2026-03-19)


### Bug Fixes

* Improve Steel IDL detection for 1-byte discriminator arrays ([3fcd1ee](https://github.com/AreteA4/arete/commit/3fcd1ee31bf5b01353c69c5630c122667557ddf6))
* Improve Steel IDL detection for 1-byte discriminator arrays ([f3a7f9c](https://github.com/AreteA4/arete/commit/f3a7f9c628b4a66d195cc510707bb721a2908fad))

## [0.1.3](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.2...arete-idl-v0.1.3) (2026-03-19)


### Bug Fixes

* align Steel discriminant size with get_discriminator return value ([4d283f2](https://github.com/AreteA4/arete/commit/4d283f2bd749690eb7b79e1c80e0447c79b35d8d))
* change any() to all() for Steel-style IDL detection ([77d2566](https://github.com/AreteA4/arete/commit/77d2566db00255cc5460e3ea4490302d0e530a25))
* Core interpreter and server improvements ([b05ae9b](https://github.com/AreteA4/arete/commit/b05ae9bd169f48c2cfd1222d8fa4adc882d96adc))
* implement discriminant size inference and fix test failures ([c8b26d5](https://github.com/AreteA4/arete/commit/c8b26d58f62041cb7c5fe1624f5313f63a9ef9d9))
* prevent empty instruction arrays from being misclassified as Steel-style ([abad594](https://github.com/AreteA4/arete/commit/abad594108d0f9b1f795d7f320e7f69b1027fce6))
* replace panicking expect with graceful fallback in get_discriminator ([fa09ef9](https://github.com/AreteA4/arete/commit/fa09ef9cce75b64df295971c35ade31a724522c5))
* replace silent u8 truncation of Steel discriminant with try_from ([f4de6ef](https://github.com/AreteA4/arete/commit/f4de6ef1a835f204a8af39b4610481356bd62410))
* replace unnecessary unwrap with if let pattern ([b3e6dac](https://github.com/AreteA4/arete/commit/b3e6dac0cda3428471f2d648200c3b866f26e108))
* **tests:** replace diagnostic println with assertion for discriminant_size ([f391581](https://github.com/AreteA4/arete/commit/f39158107e551a544bb610ca4a8d7a59e81f6460))

## [0.1.2](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.1...arete-idl-v0.1.2) (2026-03-14)


### Features

* Bump arete idl version ([857f819](https://github.com/AreteA4/arete/commit/857f819ad97dbb8296e33765094d42a452aaf91f))

## [0.1.1](https://github.com/AreteA4/arete/compare/arete-idl-v0.1.0...arete-idl-v0.1.1) (2026-03-14)


### Features

* **idl:** add compute_discriminator public API ([48de0ae](https://github.com/AreteA4/arete/commit/48de0ae92ac45747208260befa977ac2363225fd))
* **idl:** add connect analysis module for path finding ([75ea359](https://github.com/AreteA4/arete/commit/75ea35917112ce5717125bd43bdc29be147d44be))
* **idl:** add constants field support to IdlSpec ([d9af98f](https://github.com/AreteA4/arete/commit/d9af98fbd101a8821614326ac8b4976d25a4c2d4))
* **idl:** add packed representation support to IdlRepr ([04486af](https://github.com/AreteA4/arete/commit/04486af36571ab761ec4804fefcf3dadd23db7a9))
* **idl:** add PDA graph analysis ([db9db3c](https://github.com/AreteA4/arete/commit/db9db3c793b5d0096bf500c0c16a22a7ca2e6427))
* **idl:** add relations analysis module ([bdc52eb](https://github.com/AreteA4/arete/commit/bdc52ebbb273c6ec52f6752e4eb40e35b98f8848))
* **idl:** add search module with fuzzy matching and structured errors ([541a8a5](https://github.com/AreteA4/arete/commit/541a8a51e91f2e64e2c4d6a1fe02e0f69c243284))
* **idl:** add snake_case/pascal_case utilities ([1eba345](https://github.com/AreteA4/arete/commit/1eba345f699892fd96916efa33b665b6cf39b002))
* **idl:** add type graph analysis + release-please independent versioning ([a64a26d](https://github.com/AreteA4/arete/commit/a64a26df62d6db190dd7a7f0762bf8144aa0e6a2))
* **idl:** create arete-idl crate skeleton ([90712df](https://github.com/AreteA4/arete/commit/90712df6ece12a8bda63941417ff96361eaf59c1))
* **idl:** extract core IDL parsing types into arete-idl ([1b1ea56](https://github.com/AreteA4/arete/commit/1b1ea5616e2d7ea3ea904c026491aa61000dd8b2))
* **idl:** extract snapshot types with backwards-compatible HashMap handling ([fe882e7](https://github.com/AreteA4/arete/commit/fe882e739ac5162d9e281ce385cc7e5de7729f02))
* misc compiler, VM, and IDL improvements ([2d6aea3](https://github.com/AreteA4/arete/commit/2d6aea373e43c84e3a07ecef7d9dab004a0b8c1c))


### Bug Fixes

* **idl:** remove redundant closure in pda_graph (clippy) ([4566a9d](https://github.com/AreteA4/arete/commit/4566a9db588199bf080eb77083052ba7a2bdcaad))
