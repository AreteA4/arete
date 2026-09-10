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
