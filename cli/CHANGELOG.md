# Changelog

## [0.5.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.5.4...hyperstack-cli-v0.5.5) (2026-03-14)


### Bug Fixes

* Add version constraint to hyperstack-idl dependency ([917bf5a](https://github.com/HyperTekOrg/hyperstack/commit/917bf5abe6242048ba9f7af0055d999ccfcb8692))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.4 to 0.5.5
    * hyperstack-idl bumped from 0.1 to 0.1.2

## [0.5.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.5.3...hyperstack-cli-v0.5.4) (2026-03-14)


### Features

* **cli:** implement hs idl connect command with hyperstack suggestions ([6579a84](https://github.com/HyperTekOrg/hyperstack/commit/6579a84b1bd6018255e707fc3fb7cf2d16a050ee))
* **cli:** implement hs idl data commands (errors through discriminator) ([87267dd](https://github.com/HyperTekOrg/hyperstack/commit/87267ddb1df671c162ffdc0a3b2c539dadcb1660))
* **cli:** implement hs idl data commands (summary through type) ([60e3efe](https://github.com/HyperTekOrg/hyperstack/commit/60e3efe8dfddba45ffb17b0cbbd550322ad19b76))
* **cli:** implement hs idl relationship commands ([8fccdcb](https://github.com/HyperTekOrg/hyperstack/commit/8fccdcbdf6a198cbe1127182fc5efff3c47ccc3c))
* **cli:** scaffold hs idl subcommand structure + ci publish order ([64f3f4c](https://github.com/HyperTekOrg/hyperstack/commit/64f3f4cffd286092886496e2da920d353204de6f))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.3 to 0.5.4

## [0.5.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.5.2...hyperstack-cli-v0.5.3) (2026-02-20)


### Features

* **cli:** add explore command for stack and schema discovery ([17c3022](https://github.com/HyperTekOrg/hyperstack/commit/17c302249c0393200fc53a1eca79bd2cabc53ce2))


### Bug Fixes

* use next_back() instead of last() on DoubleEndedIterator ([a4aa568](https://github.com/HyperTekOrg/hyperstack/commit/a4aa5688959fb1eea5800b3764c0c9696a45e17e))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.2 to 0.5.3

## [0.5.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.5.1...hyperstack-cli-v0.5.2) (2026-02-07)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.1 to 0.5.2

## [0.5.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.5.0...hyperstack-cli-v0.5.1) (2026-02-06)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.5.0 to 0.5.1

## [0.5.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.4.3...hyperstack-cli-v0.5.0) (2026-02-06)


### Features

* Add 'hs sdk create rust' command for Rust SDK generation ([baadf84](https://github.com/HyperTekOrg/hyperstack/commit/baadf84ce36053acd07eb3743ccadbb28a3df8cb))
* Add per-language output directory configuration ([0f58344](https://github.com/HyperTekOrg/hyperstack/commit/0f58344e715b393c4d0bc14422a1baeec1dccad7))
* add stop command for HYP-145 ([247625e](https://github.com/HyperTekOrg/hyperstack/commit/247625e64c646f8e883f3b8ef7d4b884949f248f))
* add stop command for HYP-145 ([5838dff](https://github.com/HyperTekOrg/hyperstack/commit/5838dffdec4c56aa4c6fc311f3251c262435deed))
* Add typescript ore to cli templates ([f227b74](https://github.com/HyperTekOrg/hyperstack/commit/f227b74a18f051f208f37459a64d49f2b0567d03))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **cli:** add create command for scaffolding projects from templates ([385ab5a](https://github.com/HyperTekOrg/hyperstack/commit/385ab5abfa78783475d7b2031ce425d28d84ec41))
* **cli:** add per-stack output path overrides for SDK generation ([ebbabfd](https://github.com/HyperTekOrg/hyperstack/commit/ebbabfd241b1084f4800a037d6525e9fac2bb8fe))
* **cli:** add privacy-respecting telemetry ([5aebab5](https://github.com/HyperTekOrg/hyperstack/commit/5aebab5897b9b05ef3f116a0b06fba1bfb0c79ff))
* **cli:** auto-install dependencies in `hs create` ([da64ce2](https://github.com/HyperTekOrg/hyperstack/commit/da64ce2aa0219c91769cdb03b324aae41b1bf4c7))
* **cli:** replace pumpfun template with ore-rust in scaffolding ([df1a52a](https://github.com/HyperTekOrg/hyperstack/commit/df1a52a95120914a89878c026df6486a285c2a2d))
* improve CLI UI/UX with indicatif, dry-run, and shell completions ([8565eae](https://github.com/HyperTekOrg/hyperstack/commit/8565eaeae01206b128b522008b5f45d78e1242e1))
* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* remove logs commands from CLI ([d1418eb](https://github.com/HyperTekOrg/hyperstack/commit/d1418eb968bbdef3a21e5e370b89d2d9ff2c53f6))
* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Case matching in cli stack lookup ([b4da6a5](https://github.com/HyperTekOrg/hyperstack/commit/b4da6a5e7ca28e2d21ead0f0dd1e4023544f2f0d))
* Clippy errors ([e36224f](https://github.com/HyperTekOrg/hyperstack/commit/e36224fa861e65225c5b1f5a55bacfd1d23dc14d))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* **cli:** use rustls instead of native-tls for cross-compilation ([8bea03a](https://github.com/HyperTekOrg/hyperstack/commit/8bea03a28eecd68736ff7592c77b1de1127d03c8))
* Display url in cli ([4b18925](https://github.com/HyperTekOrg/hyperstack/commit/4b189253e8a182674048044b7303ee1f8822ff30))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Remove -c short flag from --crate-name to avoid conflict ([83e1e59](https://github.com/HyperTekOrg/hyperstack/commit/83e1e59255ce070b231ac662e7af5dab7814c625))
* Update api url ([12b1be2](https://github.com/HyperTekOrg/hyperstack/commit/12b1be27f9a5117037398fd30ed137df4c788159))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.3 to 0.5.0

## [0.4.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.4.2...hyperstack-cli-v0.4.3) (2026-02-03)


### Features

* multi-IDL support with always-scoped naming ([d752008](https://github.com/HyperTekOrg/hyperstack/commit/d752008c8662b8dd91a4b411e9f9ff4404630f81))
* unified multi-entity stack spec format with SDK generation ([00194c5](https://github.com/HyperTekOrg/hyperstack/commit/00194c58b1d1bfc5d7dc9f46506ebd9c35af7338))


### Bug Fixes

* Case matching in cli stack lookup ([b4da6a5](https://github.com/HyperTekOrg/hyperstack/commit/b4da6a5e7ca28e2d21ead0f0dd1e4023544f2f0d))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.2 to 0.4.3

## [0.4.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.4.1...hyperstack-cli-v0.4.2) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.1 to 0.4.2

## [0.4.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.4.0...hyperstack-cli-v0.4.1) (2026-02-01)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.4.0 to 0.4.1

## [0.4.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.15...hyperstack-cli-v0.4.0) (2026-01-31)


### Features

* Add 'hs sdk create rust' command for Rust SDK generation ([baadf84](https://github.com/HyperTekOrg/hyperstack/commit/baadf84ce36053acd07eb3743ccadbb28a3df8cb))
* Add per-language output directory configuration ([0f58344](https://github.com/HyperTekOrg/hyperstack/commit/0f58344e715b393c4d0bc14422a1baeec1dccad7))
* add stop command for HYP-145 ([247625e](https://github.com/HyperTekOrg/hyperstack/commit/247625e64c646f8e883f3b8ef7d4b884949f248f))
* add stop command for HYP-145 ([5838dff](https://github.com/HyperTekOrg/hyperstack/commit/5838dffdec4c56aa4c6fc311f3251c262435deed))
* Add typescript ore to cli templates ([f227b74](https://github.com/HyperTekOrg/hyperstack/commit/f227b74a18f051f208f37459a64d49f2b0567d03))
* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))
* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **cli:** add create command for scaffolding projects from templates ([385ab5a](https://github.com/HyperTekOrg/hyperstack/commit/385ab5abfa78783475d7b2031ce425d28d84ec41))
* **cli:** add per-stack output path overrides for SDK generation ([ebbabfd](https://github.com/HyperTekOrg/hyperstack/commit/ebbabfd241b1084f4800a037d6525e9fac2bb8fe))
* **cli:** add privacy-respecting telemetry ([5aebab5](https://github.com/HyperTekOrg/hyperstack/commit/5aebab5897b9b05ef3f116a0b06fba1bfb0c79ff))
* **cli:** auto-install dependencies in `hs create` ([da64ce2](https://github.com/HyperTekOrg/hyperstack/commit/da64ce2aa0219c91769cdb03b324aae41b1bf4c7))
* **cli:** replace pumpfun template with ore-rust in scaffolding ([df1a52a](https://github.com/HyperTekOrg/hyperstack/commit/df1a52a95120914a89878c026df6486a285c2a2d))
* improve CLI UI/UX with indicatif, dry-run, and shell completions ([8565eae](https://github.com/HyperTekOrg/hyperstack/commit/8565eaeae01206b128b522008b5f45d78e1242e1))
* remove logs commands from CLI ([d1418eb](https://github.com/HyperTekOrg/hyperstack/commit/d1418eb968bbdef3a21e5e370b89d2d9ff2c53f6))
* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors ([e36224f](https://github.com/HyperTekOrg/hyperstack/commit/e36224fa861e65225c5b1f5a55bacfd1d23dc14d))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* **cli:** use rustls instead of native-tls for cross-compilation ([8bea03a](https://github.com/HyperTekOrg/hyperstack/commit/8bea03a28eecd68736ff7592c77b1de1127d03c8))
* Display url in cli ([4b18925](https://github.com/HyperTekOrg/hyperstack/commit/4b189253e8a182674048044b7303ee1f8822ff30))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Remove -c short flag from --crate-name to avoid conflict ([83e1e59](https://github.com/HyperTekOrg/hyperstack/commit/83e1e59255ce070b231ac662e7af5dab7814c625))
* Update api url ([12b1be2](https://github.com/HyperTekOrg/hyperstack/commit/12b1be27f9a5117037398fd30ed137df4c788159))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.15 to 0.4.0

## [0.3.15](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.14...hyperstack-cli-v0.3.15) (2026-01-31)


### Features

* add stop command for HYP-145 ([247625e](https://github.com/HyperTekOrg/hyperstack/commit/247625e64c646f8e883f3b8ef7d4b884949f248f))
* **sdk:** update CLI SDK generation for new Stack trait pattern ([b71d8b2](https://github.com/HyperTekOrg/hyperstack/commit/b71d8b2575ec4ce13546f22dc8793827cfce2a22))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.14 to 0.3.15

## [0.3.14](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.13...hyperstack-cli-v0.3.14) (2026-01-28)


### Features

* Add typescript ore to cli templates ([f227b74](https://github.com/HyperTekOrg/hyperstack/commit/f227b74a18f051f208f37459a64d49f2b0567d03))


### Bug Fixes

* Clippy errors ([e36224f](https://github.com/HyperTekOrg/hyperstack/commit/e36224fa861e65225c5b1f5a55bacfd1d23dc14d))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.13 to 0.3.14

## [0.3.13](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.12...hyperstack-cli-v0.3.13) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.12 to 0.3.13

## [0.3.12](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.11...hyperstack-cli-v0.3.12) (2026-01-28)


### Features

* **cli:** replace pumpfun template with ore-rust in scaffolding ([df1a52a](https://github.com/HyperTekOrg/hyperstack/commit/df1a52a95120914a89878c026df6486a285c2a2d))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.11 to 0.3.12

## [0.3.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.10...hyperstack-cli-v0.3.11) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.10 to 0.3.11

## [0.3.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.9...hyperstack-cli-v0.3.10) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.9 to 0.3.10

## [0.3.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.8...hyperstack-cli-v0.3.9) (2026-01-28)


### Bug Fixes

* **cli:** use rustls instead of native-tls for cross-compilation ([8bea03a](https://github.com/HyperTekOrg/hyperstack/commit/8bea03a28eecd68736ff7592c77b1de1127d03c8))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3 to 0.3.9

## [0.3.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.7...hyperstack-cli-v0.3.8) (2026-01-28)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.7 to 0.3.8

## [0.3.7](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.6...hyperstack-cli-v0.3.7) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.6 to 0.3.7

## [0.3.6](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.5...hyperstack-cli-v0.3.6) (2026-01-26)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.5 to 0.3.6

## [0.3.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.4...hyperstack-cli-v0.3.5) (2026-01-24)


### Features

* **cli:** add privacy-respecting telemetry ([5aebab5](https://github.com/HyperTekOrg/hyperstack/commit/5aebab5897b9b05ef3f116a0b06fba1bfb0c79ff))
* **cli:** auto-install dependencies in `hs create` ([da64ce2](https://github.com/HyperTekOrg/hyperstack/commit/da64ce2aa0219c91769cdb03b324aae41b1bf4c7))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.4 to 0.3.5

## [0.3.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.3...hyperstack-cli-v0.3.4) (2026-01-24)


### Features

* **cli:** add create command for scaffolding projects from templates ([385ab5a](https://github.com/HyperTekOrg/hyperstack/commit/385ab5abfa78783475d7b2031ce425d28d84ec41))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.3 to 0.3.4

## [0.3.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.2...hyperstack-cli-v0.3.3) (2026-01-23)


### Features

* add view pipeline for derived views like `latest`, `top10` ([f2f72fa](https://github.com/HyperTekOrg/hyperstack/commit/f2f72fa8894d2a38a13e8ee05791b7e4124977ea))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.2 to 0.3.3

## [0.3.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.1...hyperstack-cli-v0.3.2) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.1 to 0.3.2

## [0.3.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.3.0...hyperstack-cli-v0.3.1) (2026-01-20)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.3.0 to 0.3.1

## [0.3.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.5...hyperstack-cli-v0.3.0) (2026-01-20)


### Features

* **cli:** add --module flag for Rust SDK generation ([42812e6](https://github.com/HyperTekOrg/hyperstack/commit/42812e673d5b763792b96937d8dd6dee20314253))
* **cli:** add per-stack output path overrides for SDK generation ([ebbabfd](https://github.com/HyperTekOrg/hyperstack/commit/ebbabfd241b1084f4800a037d6525e9fac2bb8fe))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.5 to 0.3.0

## [0.2.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.4...hyperstack-cli-v0.2.5) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.4 to 0.2.5

## [0.2.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.3...hyperstack-cli-v0.2.4) (2026-01-19)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.3 to 0.2.4

## [0.2.3](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.2...hyperstack-cli-v0.2.3) (2026-01-18)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.2 to 0.2.3

## [0.2.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.1...hyperstack-cli-v0.2.2) (2026-01-16)


### Features

* improve CLI UI/UX with indicatif, dry-run, and shell completions ([8565eae](https://github.com/HyperTekOrg/hyperstack/commit/8565eaeae01206b128b522008b5f45d78e1242e1))
* remove logs commands from CLI ([d1418eb](https://github.com/HyperTekOrg/hyperstack/commit/d1418eb968bbdef3a21e5e370b89d2d9ff2c53f6))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.1 to 0.2.2

## [0.2.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.2.0...hyperstack-cli-v0.2.1) (2026-01-16)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.2.0 to 0.2.1

## [0.2.0](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.11...hyperstack-cli-v0.2.0) (2026-01-15)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.11 to 0.2.0

## [0.1.11](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.10...hyperstack-cli-v0.1.11) (2026-01-14)


### Bug Fixes

* Display url in cli ([4b18925](https://github.com/HyperTekOrg/hyperstack/commit/4b189253e8a182674048044b7303ee1f8822ff30))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.10 to 0.1.11

## [0.1.10](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.9...hyperstack-cli-v0.1.10) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.9 to 0.1.10

## [0.1.9](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.8...hyperstack-cli-v0.1.9) (2026-01-13)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.8 to 0.1.9

## [0.1.8](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.7...hyperstack-cli-v0.1.8) (2026-01-13)


### Features

* Add 'hs sdk create rust' command for Rust SDK generation ([baadf84](https://github.com/HyperTekOrg/hyperstack/commit/baadf84ce36053acd07eb3743ccadbb28a3df8cb))
* Add per-language output directory configuration ([0f58344](https://github.com/HyperTekOrg/hyperstack/commit/0f58344e715b393c4d0bc14422a1baeec1dccad7))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Remove -c short flag from --crate-name to avoid conflict ([83e1e59](https://github.com/HyperTekOrg/hyperstack/commit/83e1e59255ce070b231ac662e7af5dab7814c625))
* Update api url ([12b1be2](https://github.com/HyperTekOrg/hyperstack/commit/12b1be27f9a5117037398fd30ed137df4c788159))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.6 to 0.1.8

## [0.1.7](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.6...v0.1.7) (2026-01-13)


### Bug Fixes

* Update api url ([12b1be2](https://github.com/HyperTekOrg/hyperstack/commit/12b1be27f9a5117037398fd30ed137df4c788159))

## [0.1.6](https://github.com/HyperTekOrg/hyperstack/compare/v0.1.5...v0.1.6) (2026-01-13)


### Features

* Add 'hs sdk create rust' command for Rust SDK generation ([baadf84](https://github.com/HyperTekOrg/hyperstack/commit/baadf84ce36053acd07eb3743ccadbb28a3df8cb))
* Add per-language output directory configuration ([0f58344](https://github.com/HyperTekOrg/hyperstack/commit/0f58344e715b393c4d0bc14422a1baeec1dccad7))


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))
* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))
* Remove -c short flag from --crate-name to avoid conflict ([83e1e59](https://github.com/HyperTekOrg/hyperstack/commit/83e1e59255ce070b231ac662e7af5dab7814c625))
* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.5 to 0.1.6

## [0.1.5](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.4...hyperstack-cli-v0.1.5) (2026-01-09)


### Bug Fixes

* Broken streams after naming refactor ([64437b4](https://github.com/HyperTekOrg/hyperstack/commit/64437b4d80c3b2ec68468ce11bbeaab49678aa8b))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.4 to 0.1.5

## [0.1.4](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.2...hyperstack-cli-v0.1.4) (2026-01-09)


### Miscellaneous Chores

* **hyperstack-cli:** Synchronize hyperstack versions


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.2 to 0.1.4

## [0.1.2](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.1...hyperstack-cli-v0.1.2) (2026-01-09)


### Bug Fixes

* Update typescript package name ([6267eae](https://github.com/HyperTekOrg/hyperstack/commit/6267eaeb19e00a3e1c1f76fca417f56170edafb9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.1 to 0.1.2

## [0.1.1](https://github.com/HyperTekOrg/hyperstack/compare/hyperstack-cli-v0.1.0...hyperstack-cli-v0.1.1) (2026-01-09)


### Bug Fixes

* Clippy errors/warnings ([e18fcd6](https://github.com/HyperTekOrg/hyperstack/commit/e18fcd66fb45ee33b0c6019ab65562d286c16eab))
* Naming issues in generated sdk ([179da1f](https://github.com/HyperTekOrg/hyperstack/commit/179da1f2f6c8c75f99c35c0fb90b38576ffc19e2))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * hyperstack-interpreter bumped from 0.1.0 to 0.1.1
