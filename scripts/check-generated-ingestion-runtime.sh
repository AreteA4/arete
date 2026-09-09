#!/usr/bin/env bash
# Compile and execute a standalone #[arete] ingestion consumer. Registry mode
# is a post-publication gate; it never substitutes workspace or Git packages.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MODE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode) MODE="${2:-}"; shift 2 ;;
        --mode=*) MODE="${1#--mode=}"; shift ;;
        *) echo "Usage: $0 --mode <local|registry>" >&2; exit 2 ;;
    esac
done
if [[ "$MODE" != local && "$MODE" != registry ]]; then
    echo "Usage: $0 --mode <local|registry>" >&2
    exit 2
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-ingestion-runtime.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT
mkdir -p "$WORK_DIR/src"
cp "$SCRIPT_DIR/fixtures/ingestion-runtime/main.rs" "$WORK_DIR/src/main.rs"
cp "$ROOT_DIR/stacks/ore/idl/ore.json" "$WORK_DIR/ore.json"

# JSON strings are valid TOML strings; use Python to quote filesystem paths.
python3 - "$ROOT_DIR" "$WORK_DIR" "$MODE" <<'PY'
import json, pathlib, re, sys
root, work, mode = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
manifest = (root / 'arete/Cargo.toml').read_text()
version = re.search(r'^version = "([^"]+)"', manifest, re.M).group(1)
dependency = f'arete = {{ version = "={version}"'
if mode == 'local':
    dependency += ', path = ' + json.dumps(str(root / 'arete'))
dependency += ' }'
(work / 'Cargo.toml').write_text('''[package]
name = "arete-ingestion-consumer-check"
version = "0.0.0"
edition = "2021"

[workspace]

[dependencies]
''' + dependency + '''
serde = { version = "1", features = ["derive"] }
borsh = { version = "1.5", features = ["derive"] }
solana-pubkey = { version = "2.2", features = ["serde", "borsh"] }
''')
print(f'Checking generated ingestion consumer against arete ={version} ({mode})')
PY

# Keep registry mode independent of repository Cargo configuration.
(cd "$WORK_DIR" && cargo generate-lockfile --quiet && cargo metadata --locked --format-version 1 > graph.json)
python3 - "$WORK_DIR/graph.json" "$MODE" <<'PY'
import json, sys
packages = json.load(open(sys.argv[1]))['packages']
expected = {'shipstern': '0.9.0', 'shipstern-core': '0.9.0',
            'shipstern-yellowstone-grpc-source': '0.9.0',
            'yellowstone-grpc-client': '13.2.1', 'yellowstone-grpc-proto': '12.7.0'}
for name, version in expected.items():
    matches = [p for p in packages if p['name'] == name]
    assert len(matches) == 1 and matches[0]['version'] == version, (name, matches)
    assert matches[0]['source'].startswith('registry+'), matches[0]
    print(f'{name} ={version} (registry)')
assert not any(p['name'].startswith('yellowstone-vixen') for p in packages)
if sys.argv[2] == 'registry':
    for p in packages:
        if p['name'] != 'arete-ingestion-consumer-check':
            assert (p['source'] or '').startswith('registry+'), f'Non-registry dependency: {p["name"]}: {p["source"]}'
PY
(cd "$WORK_DIR" && cargo run --quiet --locked)
echo "Generated ingestion runtime compiled and executed ($MODE)."
