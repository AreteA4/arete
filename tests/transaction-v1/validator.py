"""Probe disposable validator/Geyser startup; this is NOT the SDK lifecycle gate."""

import argparse
import base64
import contextlib
import datetime
import json
import os
import pathlib
import random
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request

from prepare_toolchain import HERE, digest


def reserve_ports():
    # Reserve both protocols and all consecutive ports before spawning. A new
    # run never attaches to an existing validator or resets another ledger.
    for _ in range(30):
        ports = []
        start = random.randrange(20000, 55000)
        try:
            for port in range(start, start + 40):
                for protocol in [socket.SOCK_STREAM, socket.SOCK_DGRAM]:
                    sock = socket.socket(socket.AF_INET, protocol)
                    ports.append(sock)
                    sock.bind(("127.0.0.1", port))
            return start, ports
        except OSError:
            for sock in ports:
                sock.close()
    raise RuntimeError("Could not reserve an isolated local port range")


def rpc(url, method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}).encode()
    with urllib.request.urlopen(urllib.request.Request(url, body, {"Content-Type": "application/json"}), timeout=2) as response:
        payload = json.load(response)
    if "error" in payload:
        raise RuntimeError(f"{method}: {payload['error']}")
    return payload["result"]


@contextlib.contextmanager
def validator(toolchain, artifacts, timeout):
    pins = json.loads((HERE / "toolchain.json").read_text())
    receipt = json.loads((toolchain / "receipt.json").read_text())
    if receipt["pinsSha256"] != digest(HERE / "toolchain.json"):
        raise RuntimeError("Toolchain pins changed; run prepare_toolchain.py again")
    binaries = toolchain / "solana-release/bin"
    for name in ["solana-test-validator", "solana", "solana-keygen", "cargo-build-sbf"]:
        if digest(binaries / name) != receipt["binarySha256"][name]:
            raise RuntimeError(f"Toolchain binary digest mismatch: {name}")
    plugin = toolchain / receipt["plugin"]
    if digest(plugin) != receipt["pluginSha256"]:
        raise RuntimeError("Geyser plugin digest mismatch")
    version = subprocess.check_output([str(binaries / "solana-test-validator"), "--version"], text=True, timeout=10).strip()
    if version.split()[1] != pins["agaveVersion"]:
        raise RuntimeError(f"Wrong validator version: {version}")
    run = pathlib.Path(tempfile.mkdtemp(prefix="arete-v1-validator-"))
    process = None
    sockets = []
    artifacts.mkdir(parents=True, exist_ok=True)
    try:
        base, sockets = reserve_ports()
        url = f"http://127.0.0.1:{base}"
        grpc = f"127.0.0.1:{base + 2}"
        config = {"libpath": str(plugin), "log": {"level": "info"},
                  "grpc": {"listen": [{"address": grpc}]}}
        (run / "geyser.json").write_text(json.dumps(config))
        args = [str(binaries / "solana-test-validator"), "--ledger", str(run / "ledger"),
                "--bind-address", "127.0.0.1", "--rpc-port", str(base),
                "--faucet-port", str(base + 3), "--gossip-port", str(base + 4),
                "--dynamic-port-range", f"{base + 5}-{base + 40}",
                "--geyser-plugin-config", str(run / "geyser.json"), "--quiet"]
        for sock in sockets:
            sock.close()
        with (artifacts / "startup.log").open("w") as log:
            process = subprocess.Popen(args, cwd=run, stdout=log, stderr=log, start_new_session=True)
        deadline = time.monotonic() + timeout
        while True:
            if process.poll() is not None:
                raise RuntimeError(f"Validator exited {process.returncode}; see {artifacts / 'validator.log'}")
            try:
                if rpc(url, "getHealth") == "ok":
                    break
            except (OSError, ValueError, RuntimeError):
                pass
            if time.monotonic() >= deadline:
                raise RuntimeError("Validator startup deadline expired")
            time.sleep(0.2)  # Poll an observable condition, never assume readiness.
        identity = subprocess.check_output(
            [str(binaries / "solana-keygen"), "pubkey", str(run / "ledger/validator-keypair.json")],
            text=True, timeout=10,
        ).strip()
        if rpc(url, "getIdentity")["identity"] != identity:
            raise RuntimeError("RPC identity differs from the disposable validator")
        if rpc(url, "getVersion")["solana-core"] != pins["agaveVersion"]:
            raise RuntimeError("RPC validator version does not match toolchain")
        feature = rpc(url, "getAccountInfo", [pins["feature"], {"encoding": "base64"}])["value"]
        if not feature or feature["owner"] != "Feature111111111111111111111111111111111111":
            raise RuntimeError("V1 feature account is missing or has the wrong owner")
        data = base64.b64decode(feature["data"][0], validate=True)
        # solana_feature_gate_interface::Feature serializes activated_at as
        # bincode Option<Slot>: one-byte Some tag followed by a little-endian u64.
        if len(data) < 9 or data[0] != 1:
            raise RuntimeError("V1 feature is not active")
        activated_at = struct.unpack_from("<Q", data, 1)[0]
        if activated_at > rpc(url, "getSlot"):
            raise RuntimeError("V1 feature activation is in the future")
        with socket.create_connection(("127.0.0.1", base + 2), timeout=2):
            pass
        yield {"validator": version, "toolchain": receipt, "rpc": url,
               "geyser": f"http://{grpc}", "identity": identity,
               "feature": pins["feature"], "activatedAtSlot": activated_at}
    finally:
        for sock in sockets:
            sock.close()
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        validator_log = run / "ledger/validator.log"
        if validator_log.exists():
            shutil.copyfile(validator_log, artifacts / "validator.log")
        shutil.rmtree(run)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", type=pathlib.Path, default=HERE / ".toolchain")
    parser.add_argument("--artifacts", type=pathlib.Path, default=HERE / ".artifacts/validator")
    parser.add_argument("--startup-timeout", type=int, default=60)
    args = parser.parse_args()
    if not 1 <= args.startup_timeout <= 300:
        parser.error("--startup-timeout must be between 1 and 300 seconds")
    def interrupted(signum, _frame):
        raise InterruptedError(f"Validator probe interrupted by signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    artifacts = args.artifacts.resolve()
    report = {"scope": "validator-geyser-startup", "status": "failed",
              "fullLifecycleAccepted": False,
              "startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    try:
        with validator(args.toolchain.resolve(), artifacts, args.startup_timeout) as ready:
            report.update(ready)
        report["status"] = "passed"
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
    report["finishedAt"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
