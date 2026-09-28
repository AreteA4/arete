:robot: I have created a release *beep* *boop*
---


<details><summary>arete-python: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-python-v0.24.0...arete-python-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** order one key's frames by the server's version ([0b47eaf](https://github.com/AreteA4/arete/commit/0b47eafc0910c9006637a9759848a87203758c3b))
* **sdk:** order one key's frames by the server's version ([0c7fd21](https://github.com/AreteA4/arete/commit/0c7fd21bd76887b283a29dc1c892692b2f6ac17b))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Documentation

* **cli:** point agents at the starter templates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** point agents at the starter templates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** document identity, testing, wallets and mutation phases ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** document identity, testing, wallets and mutation phases ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>a4-cli: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/a4-cli-v0.24.0...a4-cli-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-interpreter bumped from 0.24.0 to 0.25.0
    * arete-mcp bumped from 0.24.0 to 0.25.0
</details>

<details><summary>arete: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-v0.24.0...arete-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-interpreter bumped from 0.24.0 to 0.25.0
    * arete-macros bumped from 0.24.0 to 0.25.0
    * arete-server bumped from 0.24.0 to 0.25.0
</details>

<details><summary>arete-sdk: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-sdk-v0.24.0...arete-sdk-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** order one key's frames by the server's version ([0b47eaf](https://github.com/AreteA4/arete/commit/0b47eafc0910c9006637a9759848a87203758c3b))
* **sdk:** order one key's frames by the server's version ([0c7fd21](https://github.com/AreteA4/arete/commit/0c7fd21bd76887b283a29dc1c892692b2f6ac17b))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>arete-interpreter: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-interpreter-v0.24.0...arete-interpreter-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-macros bumped from 0.24.0 to 0.25.0
</details>

<details><summary>arete-macros: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-macros-v0.24.0...arete-macros-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>arete-mcp: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-mcp-v0.24.0...arete-mcp-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>arete-server: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-server-v0.24.0...arete-server-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** stamp every frame with its version ([b7b770b](https://github.com/AreteA4/arete/commit/b7b770bd261d3b7bcefc0157af0b893110ec0659))
* **server:** stamp every frame with its version ([0134758](https://github.com/AreteA4/arete/commit/0134758ef1241c5385db4b016f000f61b4bceff8))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * arete-interpreter bumped from 0.24.0 to 0.25.0
</details>

<details><summary>a4-npm: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/a4-npm-v0.24.0...a4-npm-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>arete-adapter-kit: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-adapter-kit-v0.24.0...arete-adapter-kit-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
</details>

<details><summary>arete-adapter-web3js: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-adapter-web3js-v0.24.0...arete-adapter-web3js-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
</details>

<details><summary>arete-mcp-npm: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-mcp-npm-v0.24.0...arete-mcp-npm-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

<details><summary>arete-react: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-react-v0.24.0...arete-react-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Dependencies

* The following workspace dependencies were updated
  * devDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
  * peerDependencies
    * @usearete/sdk bumped from ^0.24.0 to ^0.25.0
</details>

<details><summary>arete-typescript: 0.25.0</summary>

## [0.25.0](https://github.com/AreteA4/arete/compare/arete-typescript-v0.24.0...arete-typescript-v0.25.0) (2026-09-28)


###   BREAKING CHANGES

* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.
* **sdk:** ServerFrame::Subscribed gains a whole_entities field.
* **sdk:** attaching a different program under a key a stack already provides throws PROGRAM_KEY_CONFLICT.
* **sdk:** attaching a different program under a key a stack already provides raises ProgramKeyConflictError instead of keeping the stack's program with a warning.

### Features

* **cli:** attach program identity after the package extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** attach program identity after the package extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** carry program SDKs in stacks ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** carry program SDKs in stacks ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** check runtime SDK compatibility ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check runtime SDK compatibility ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** compose stacks from registry parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** compose stacks from registry parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** explore one operation, a stack summary or selected views ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** explore one operation, a stack summary or selected views ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** make publishable key creation scriptable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** make publishable key creation scriptable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report account readiness in a4 doctor and a4 explore ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report account readiness in a4 doctor and a4 explore ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report requested, regenerated and shared dependencies on install ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report requested, regenerated and shared dependencies on install ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** report the program SDK each program gets on a4 up ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** report the program SDK each program gets on a4 up ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **docs:** rank documentation search by query terms ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **docs:** rank documentation search by query terms ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** let a consumer request a whole entity from the VM ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** let a consumer request a whole entity from the VM ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **mcp:** summarise explore results and look up one operation ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **mcp:** summarise explore results and look up one operation ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add inspect-only wallet adapters ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add inspect-only wallet adapters ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** add testing helpers ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** add testing helpers ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Rust client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Rust client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Python ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Python ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in Rust ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in Rust ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** identify program SDKs by package release in TypeScript ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** identify program SDKs by package release in TypeScript ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep runtime extensions through object spread ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep runtime extensions through object spread ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** order one key's frames by the server's version ([0b47eaf](https://github.com/AreteA4/arete/commit/0b47eafc0910c9006637a9759848a87203758c3b))
* **sdk:** order one key's frames by the server's version ([0c7fd21](https://github.com/AreteA4/arete/commit/0c7fd21bd76887b283a29dc1c892692b2f6ac17b))
* **sdk:** publish the extension API version ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** publish the extension API version ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))


### Bug Fixes

* **cli:** check the extensionApi the registry reports for an extension ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** check the extensionApi the registry reports for an extension ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** confine --env-file to the project and replace it atomically ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** confine --env-file to the project and replace it atomically ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep locks from older a4 releases installable ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep locks from older a4 releases installable ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** keep the previous composition artifacts until the replacement lands ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** keep the previous composition artifacts until the replacement lands ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** pin the program SDKs of locked composed parts ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** pin the program SDKs of locked composed parts ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **cli:** require an older lock to pin exactly the SDK extensions it generates ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **interpreter:** send a requested whole entity with the next batch ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **interpreter:** send a requested whole entity with the next batch ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **react:** peer-depend on @usearete/sdk ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **react:** peer-depend on @usearete/sdk ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the Python client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the Python client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** drop patches for keys the TypeScript client does not hold ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** drop patches for keys the TypeScript client does not hold ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** keep a stack's program types when no programs are attached ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** keep a stack's program types when no programs are attached ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **sdk:** share a same-release program only when it reads the same way ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **sdk:** share a same-release program only when it reads the same way ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** keep a resent entity's own position in recency order ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** keep a resent entity's own position in recency order ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** never serve partial entities and resend evicted entities whole ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** never serve partial entities and resend evicted entities whole ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** rank entities without a sort value last ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** rank entities without a sort value last ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** replace a state subscriber's copy when a missed frame replaced the entity ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
* **server:** tell created entities from changes instead of trusting eviction memory ([008860b](https://github.com/AreteA4/arete/commit/008860bf55884ba0222f6b6046c16b19337a37f4))
* **server:** tell created entities from changes instead of trusting eviction memory ([d1966c9](https://github.com/AreteA4/arete/commit/d1966c967bf8cec9249710b0def4aedf6ac01342))
</details>

---
This PR was generated with [Release Please](https://github.com/googleapis/release-please). See [documentation](https://github.com/googleapis/release-please#release-please).