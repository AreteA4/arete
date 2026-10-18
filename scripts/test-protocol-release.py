#!/usr/bin/env python3
"""Exercise release metadata and auth selection without publishing or using real tokens."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class ProtocolReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ["publish-protocol-package.sh", "check-protocol-release-dependency.mjs"]:
            shutil.copy2(ROOT / "scripts" / name, scripts / name)
        (scripts / "publish-npm-package.sh").write_text(
            '#!/bin/bash\nset -eu\n'
            'if [[ -n "${NPM_TOKEN:-}" ]]; then\n'
            '  test "${NODE_AUTH_TOKEN:-}" = "$NPM_TOKEN"\n'
            '  echo bootstrap\nelse\n'
            '  test -z "${NODE_AUTH_TOKEN:-}"\n  echo oidc\nfi\n'
        )
        (scripts / "publish-npm-package.sh").chmod(0o755)
        self.package = self.root / "typescript" / "protocol"
        self.package.mkdir(parents=True)
        self.hash = self.root / "typescript" / "hash"
        self.hash.mkdir()
        self.metadata()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.bin / "curl").write_text('#!/bin/bash\nif [[ "${@: -1}" == *%2fhash/* ]]; then printf "%s" "$TEST_HASH_STATUS"; else printf "%s" "$TEST_REGISTRY_STATUS"; fi\n')
        (self.bin / "curl").chmod(0o755)

    def metadata(self, version="0.7.0", dependency="^0.7.0"):
        (self.hash / "package.json").write_text(json.dumps({"name": "@usearete/hash", "version": version}))
        (self.package / "package.json").write_text(json.dumps({
            "name": "@usearete/protocol", "dependencies": {"@usearete/hash": dependency}
        }))

    def run_publish(self, status="200", token="test-only-token", hash_status="200"):
        env = os.environ.copy()
        env.update(PATH=f"{self.bin}:{env['PATH']}", TEST_REGISTRY_STATUS=status, TEST_HASH_STATUS=hash_status)
        env.pop("NPM_TOKEN", None)
        env.pop("NODE_AUTH_TOKEN", None)
        if token:
            env.update(NPM_TOKEN=token, NODE_AUTH_TOKEN=token)
        return subprocess.run(["bash", str(self.root / "scripts/publish-protocol-package.sh"),
                               str(self.package), "--dry-run"], env=env, capture_output=True, text=True)

    def test_existing_package_uses_oidc_without_exposing_bootstrap_token(self):
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "oidc")

    def test_first_publish_receives_bootstrap_credentials(self):
        result = self.run_publish("404")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "bootstrap")

    def test_missing_bootstrap_credentials_refuse_publish(self):
        result = self.run_publish("404", token=None)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("NPM_TOKEN", result.stderr)

    def test_registry_outage_never_falls_back_to_bootstrap(self):
        result = self.run_publish("503")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("bootstrap", result.stdout)

    def test_missing_hash_release_refuses_protocol_publish(self):
        result = self.run_publish("404", hash_status="404")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("bootstrap", result.stdout)

    def test_old_hash_release_is_rejected(self):
        self.metadata("0.6.1", "^0.6.1")
        self.assertNotEqual(self.run_publish().returncode, 0)

    def test_old_range_against_new_hash_is_rejected(self):
        self.metadata("0.7.0", "^0.6.1")
        self.assertNotEqual(self.run_publish().returncode, 0)

    def test_prerelease_hash_is_rejected(self):
        self.metadata("0.7.0-beta.1", "^0.7.0-beta.1")
        self.assertNotEqual(self.run_publish().returncode, 0)

    def test_later_stable_release_remains_supported(self):
        self.metadata("1.2.3", "^1.2.3")
        self.assertEqual(self.run_publish().returncode, 0)


if __name__ == "__main__":
    unittest.main()
