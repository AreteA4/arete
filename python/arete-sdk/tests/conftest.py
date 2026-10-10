"""Keep optional Solana suites out of base-install collection.

Both modules import the real solders codec and adapter unconditionally. When
the extra is installed, normal collection includes them. Solana-extra CI also
requires solders explicitly and invokes each module separately, so a missing
dependency or an uncollected module cannot produce a successful adapter check.
"""

from importlib.util import find_spec

collect_ignore = [] if find_spec("solders") is not None else [
    "test_solders_adapter.py",
    "test_generated_solders.py",
]


import pytest


@pytest.fixture(autouse=True)
def _no_developer_a4_login(monkeypatch, tmp_path):
    """Keep the developer's own ``a4`` login out of tests: the SDK falls back
    to it when no key is configured."""
    monkeypatch.setenv("ARETE_CREDENTIALS_PATH", str(tmp_path / "no-credentials.toml"))
