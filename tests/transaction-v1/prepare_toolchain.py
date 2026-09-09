"""Install the pinned validator/Geyser pair in an isolated directory."""

import argparse
import hashlib
import json
import pathlib
import platform
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def command(args, **kwargs):
    return subprocess.run(args, check=True, timeout=1200, **kwargs)


def download(url, path, expected):
    if not path.exists() or digest(path) != expected:
        temporary = path.with_suffix(path.suffix + ".download")
        try:
            command(["curl", "--fail", "--location", "--connect-timeout", "15",
                     "--max-time", "180", "--output", str(temporary), url])
            if digest(temporary) != expected:
                raise RuntimeError(f"SHA-256 mismatch for {path.name}")
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=pathlib.Path, default=HERE / ".toolchain")
    args = parser.parse_args()
    destination = args.directory.resolve()
    pins = json.loads((HERE / "toolchain.json").read_text())
    host = f"{platform.system()}-{platform.machine()}"
    if host not in pins["agaveArchives"]:
        raise RuntimeError(f"No pinned toolchain for {host}; supported: {list(pins['agaveArchives'])}")
    destination.mkdir(parents=True, exist_ok=True)
    archive = pins["agaveArchives"][host]
    archive_path = destination / archive["name"]
    download(f"https://github.com/anza-xyz/agave/releases/download/v{pins['agaveVersion']}/{archive['name']}",
             archive_path, archive["sha256"])
    # Only extract an archive after matching the release's pinned SHA-256.
    command(["tar", "-xjf", str(archive_path), "-C", str(destination)])
    if host == "Linux-x86_64":
        plugin = destination / "libyellowstone_grpc_geyser.so"
        download(f"https://github.com/rpcpool/yellowstone-grpc/releases/download/{pins['geyserTag']}/libyellowstone_grpc_geyser.so",
                 plugin, pins["linuxGeyserSha256"])
        compiler = "upstream release binary"
    else:
        source = destination / "yellowstone-grpc"
        if not source.exists():
            command(["git", "clone", "--depth", "1", "--branch", pins["geyserTag"],
                     "https://github.com/rpcpool/yellowstone-grpc", str(source)])
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True, timeout=10).strip()
        dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=source, text=True, timeout=10).strip()
        if commit != pins["geyserCommit"] or dirty:
            raise RuntimeError("Geyser source must be clean at the pinned commit")
        if digest(source / "Cargo.lock") != pins["geyserCargoLockSha256"]:
            raise RuntimeError("Geyser Cargo.lock differs from the pinned release")
        # Geyser exposes Rust trait objects across a dynamic-library boundary.
        # Building with the host's default rustc can crash the validator before
        # it reports a useful error, even when the crate versions match.
        command(["rustup", "toolchain", "install", pins["rustVersion"], "--profile", "minimal"])
        command(["cargo", f"+{pins['rustVersion']}", "build", "--locked", "--release",
                 "-p", "yellowstone-grpc-geyser", "--manifest-path", str(source / "Cargo.toml"),
                 "--target-dir", str(source / "target")])
        plugin = destination / "libyellowstone_grpc_geyser.dylib"
        shutil.copyfile(source / "target/release/libyellowstone_grpc_geyser.dylib", plugin)
        compiler = subprocess.check_output(["rustc", f"+{pins['rustVersion']}", "--version"], text=True, timeout=10).strip()
    binary_dir = destination / "solana-release/bin"
    binaries = {name: digest(binary_dir / name) for name in ["solana-test-validator", "solana", "solana-keygen", "cargo-build-sbf"]}
    receipt = {
        "schemaVersion": 1, "host": host,
        "pinsSha256": digest(HERE / "toolchain.json"),
        "agaveVersion": pins["agaveVersion"], "geyserTag": pins["geyserTag"],
        "geyserCommit": pins["geyserCommit"], "pluginCompiler": compiler,
        "archiveSha256": archive["sha256"], "binarySha256": binaries,
        "plugin": plugin.name, "pluginSha256": digest(plugin),
    }
    (destination / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"status": "prepared", "receipt": str(destination / "receipt.json")}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({"status": "failed", "error": str(error)}), file=sys.stderr)
        sys.exit(1)
