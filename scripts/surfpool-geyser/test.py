#!/usr/bin/env python3
"""Exercise build path handling and the workflow's actual staging shell offline."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PIN = json.loads((ROOT / 'scripts/surfpool-geyser/pins.json').read_text())['pins']['1.6.0']
WORKFLOW = (ROOT / '.github/workflows/surfpool-geyser-release.yml').read_text()
STAGE = WORKFLOW.split('      - name: Stage the release assets\n', 1)[1]
STAGE = STAGE.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
STAGE = '\n'.join(line[10:] for line in STAGE.splitlines())


def executable(path, body):
    path.write_text('#!/usr/bin/env python3\n' + body)
    path.chmod(0o755)


class ReleaseTests(unittest.TestCase):
    def check_platform(self, system, machine, platform, relative):
        with tempfile.TemporaryDirectory(prefix='geyser-release-') as temporary:
            root = Path(temporary)
            tools = root / 'tools'
            tools.mkdir()
            out = root / 'geyser output'
            work = out / 'src' / PIN['yellowstoneCommit']
            (work / '.git').mkdir(parents=True)
            (work / 'Cargo.lock').write_text('')
            (work / 'Cargo.toml').write_text('version = "=4.2.2"\n')
            (work / 'path with space').mkdir()
            (work / 'path with space/Cargo.toml').write_text('version = "=4.2.2"\n')
            executable(tools / 'git', '''import os, sys
args = sys.argv[1:]
if 'rev-parse' in args:
    print(os.environ['TEST_COMMIT'])
elif 'ls-files' in args:
    sys.stdout.write('Cargo.toml\\0path with space/Cargo.toml\\0' if '-z' in args else 'Cargo.toml\\npath with space/Cargo.toml\\n')
''')
            executable(tools / 'rustup', '''import os
print(os.environ['TEST_RUSTC'] + '-test')
''')
            executable(tools / 'uname', '''import os, sys
print(os.environ['TEST_SYSTEM'] if sys.argv[1] == '-s' else os.environ['TEST_MACHINE'])
''')
            executable(tools / 'cargo', '''import os, pathlib, sys
if 'build' in sys.argv:
    target = pathlib.Path(os.environ['CARGO_TARGET_DIR'])
    assert target.is_absolute(), 'Cargo target directory must be absolute'
    (target / 'release').mkdir(parents=True, exist_ok=True)
    (target / 'release' / os.environ['TEST_LIB']).write_bytes(b'fresh plugin')
''')
            extension = 'dylib' if system == 'Darwin' else 'so'
            library = f'libyellowstone_grpc_geyser.{extension}'
            (out / 'target/release').mkdir(parents=True)
            (out / 'target/release' / library).write_bytes(b'stale plugin')
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                       TEST_COMMIT=PIN['yellowstoneCommit'], TEST_RUSTC=PIN['rustc'],
                       TEST_SYSTEM=system, TEST_MACHINE=machine, TEST_LIB=library)
            argument = './geyser output' if relative else str(out)
            result = subprocess.run(['bash', str(ROOT / 'scripts/surfpool-geyser/build.sh'),
                                     '--surfpool', '1.6.0', '--out', argument],
                                    cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            built = out / '1.6.0' / platform
            self.assertEqual((built / library).read_bytes(), b'fresh plugin')
            manifest = json.loads((built / 'manifest.json').read_text())
            self.assertEqual(manifest['library']['sha256'], hashlib.sha256(b'fresh plugin').hexdigest())
            self.assertEqual((work / 'path with space/Cargo.toml').read_text(), 'version = "=4.2.1"\n')
            runner = root / 'runner'
            (runner / 'geyser/1.6.0').mkdir(parents=True)
            (runner / 'geyser/1.6.0' / platform).symlink_to(built, target_is_directory=True)
            env.update(RUNNER_TEMP=str(runner), PLATFORM=platform, SURFPOOL_VERSION='1.6.0')
            staged = subprocess.run(['bash', '-euo', 'pipefail', '-c', STAGE],
                                    cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(staged.returncode, 0, staged.stderr)
            published = json.loads((root / 'assets' / f'manifest-{platform}.json').read_text())
            asset = root / 'assets' / published['library']['file']
            self.assertEqual(asset.name, f'libyellowstone_grpc_geyser-{platform}.{extension}')
            self.assertEqual(hashlib.sha256(asset.read_bytes()).hexdigest(), published['library']['sha256'])
            config = json.loads((root / 'assets' / f'yellowstone-grpc.{platform}.json').read_text())
            self.assertNotIn('libpath', config)

    def test_build_and_stage_all_platforms(self):
        platforms = [('Darwin', 'arm64', 'darwin-arm64'), ('Darwin', 'x86_64', 'darwin-x64'),
                     ('Linux', 'x86_64', 'linux-x64'), ('Linux', 'aarch64', 'linux-arm64')]
        for system, machine, platform in platforms:
            for relative in (True, False):
                with self.subTest(platform=platform, relative=relative):
                    self.check_platform(system, machine, platform, relative)


if __name__ == '__main__':
    unittest.main()
