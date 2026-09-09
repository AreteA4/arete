"""Acceptance entry point. Never install dependencies or contact a cluster offline."""

import argparse
import datetime
import hashlib
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=["offline", "local"])
    parser.add_argument("--report", type=pathlib.Path, default=HERE / ".artifacts/report.json")
    args = parser.parse_args()
    report = {
        "schemaVersion": 1,
        "issue": "A4-256",
        "mode": args.mode,
        "status": "failed",
        "fullLifecycleAccepted": False,
        "startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "checks": [],
    }
    try:
        report["sourceCommit"] = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10
        ).strip()
        report["sourceDirty"] = bool(subprocess.check_output(
            ["git", "status", "--porcelain"], cwd=ROOT, text=True, timeout=10
        ).strip())
        report["fixtureProvenanceSha256"] = hashlib.sha256(
            (HERE.parent / "fixtures/transaction-v1/provenance.json").read_bytes()
        ).hexdigest()
        inputs = sorted([
            *HERE.glob("*.py"), *HERE.glob("*.mjs"),
            HERE / "package.json", HERE / "package-lock.json", HERE / "toolchain.json",
            *sorted((HERE.parent / "fixtures/transaction-v1").glob("*.json")),
            HERE.parent / "fixtures/transaction-v1/generate.mjs",
            ROOT / "scripts/test-transaction-v1-e2e.sh",
        ])
        report["inputSha256"] = {
            str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in inputs
        }
        # Deliberately separate installation from execution: npm ci is a setup
        # step, and --mode offline must work with networking disabled.
        result = subprocess.run(
            ["node", str(HERE / "offline.mjs")], cwd=ROOT,
            capture_output=True, text=True, timeout=60,
        )
        if result.returncode:
            raise RuntimeError(f"Offline fixture check failed: {result.stdout}\n{result.stderr}")
        check = json.loads(result.stdout)
        if check.get("status") != "passed":
            raise RuntimeError(f"Offline checker did not report success: {check}")
        report["checks"].append(check)
        if args.mode == "local":
            # This scaffolding must fail closed until joined generated-client
            # acceptance exists. Passing codec fixtures or validator startup
            # alone is not the local E2E gate described by A4-256.
            report["status"] = "blocked"
            report["pendingGates"] = [
                "A4-253 TypeScript V1 adapter and generated program/stack lifecycle",
                "A4-254 Rust V1 adapter and generated program/stack lifecycle",
                "A4-255 Python V1 adapter and generated program/stack lifecycle",
                "Real relay/status/JSON/Geyser/Arete events, CPI, state and replay",
                "Generated-language registry consumers after publication",
            ]
            raise RuntimeError(
                "Local E2E acceptance is not implemented yet: the prerequisite V1 adapters "
                "are not integrated in this checkout. See pendingGates. Validator startup "
                "can be diagnosed separately with tests/transaction-v1/validator.py."
            )
        report["status"] = "passed"
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
    finally:
        report["finishedAt"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "mode": args.mode,
                      "report": str(args.report), "error": report.get("error")}))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
