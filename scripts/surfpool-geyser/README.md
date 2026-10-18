# Yellowstone geyser plugin for Surfpool

A local Surfpool fork streams into a self-hosted stack through the
Yellowstone gRPC geyser plugin. Geyser plugins have no stable ABI, so the
plugin must be built against the exact agave version a Surfpool release
embeds, with the same rustc. A mismatched build loads, then crashes Surfpool
on the first transaction.

`pins.json` records each supported Surfpool release:

| Field | Meaning |
|---|---|
| `agave` | the agave version the Surfpool release embeds |
| `rustc` | the toolchain that built it |
| `yellowstoneTag`, `yellowstoneCommit` | the yellowstone-grpc release to build |
| `pinAgaveFrom` | when that release targets a different agave patch version, its agave crates are pinned down to `agave` |

```sh
scripts/surfpool-geyser/build.sh                 # the Surfpool on PATH
scripts/surfpool-geyser/build.sh --surfpool 1.6.0 --out /tmp/geyser
```

The output directory holds the library, a `yellowstone-grpc.json` plugin
config template (set `libpath` to the absolute library path and pick the gRPC
and Prometheus ports) and `manifest.json` with the versions and the
library's sha256. To add a Surfpool release, find its agave version and rustc
(`strings $(which surfpool) | grep -E 'agave-.*-[0-9]+\.[0-9]+\.[0-9]+|/rustc/'`),
add a pin, build, and run a transaction through the fork with the plugin
loaded before relying on it.

## Published builds

Builds are published as the `surfpool-geyser-v<surfpool version>` GitHub
release of this repository, verifiable with the release key (minisign
signature over `checksums.txt`, trusted comment = the tag, then each file's
sha256). The
signature authenticates the trusted comment, but existing installers do not
compare it with an expected tag; consumers requiring release binding must
also compare that comment with the requested release tag. The
`Surfpool Geyser Plugin Release` workflow
(`.github/workflows/surfpool-geyser-release.yml`, dispatch only) runs
`build.sh` for a pinned version on darwin-arm64, darwin-x64, linux-x64 and
linux-arm64 and publishes `libyellowstone_grpc_geyser-<platform>.{dylib,so}`,
`manifest-<platform>.json`, `yellowstone-grpc.json` (without `libpath`),
`checksums.txt` and `checksums.txt.minisig` with
`scripts/sign-and-upload-external-assets.sh`. Add the pin first, then
dispatch the workflow with that Surfpool version.

## Verified builds

| Surfpool | Platform | Result |
|---|---|---|
| 1.6.0 (agave 4.2.1, rustc 1.95.0) | darwin-arm64 | yellowstone-grpc v15.2.1+solana.4.2.2 pinned to agave 4.2.1: Surfpool stays healthy through airdrops and transfers, and a `Subscribe` stream delivers the transactions (2026-10-13) |
