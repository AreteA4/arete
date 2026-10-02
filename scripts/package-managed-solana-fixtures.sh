#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT_DIR/target"
python3 - "$ROOT_DIR" <<'PY'
import gzip
import hashlib
import io
from pathlib import Path
import sys
import tarfile

root = Path(sys.argv[1])
fixtures = root / "tests/fixtures/managed-solana-v1"
with (root / "target/managed-solana-v1.tar.gz").open("wb") as output:
    with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for path in sorted(fixtures.rglob("*")):
                if not path.is_file():
                    continue
                content = path.read_bytes()
                entry = tarfile.TarInfo(path.relative_to(fixtures.parent).as_posix())
                entry.size = len(content)
                entry.mode = 0o644
                archive.addfile(entry, io.BytesIO(content))
asset = root / "target/managed-solana-v1.tar.gz"
asset.with_suffix(asset.suffix + ".sha256").write_text(
    hashlib.sha256(asset.read_bytes()).hexdigest() + "  " + asset.name + "\n"
)
PY
echo "$ROOT_DIR/target/managed-solana-v1.tar.gz"
