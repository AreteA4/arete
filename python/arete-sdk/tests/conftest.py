"""Shared collection rules.

``test_solders_adapter.py`` imports ``solders`` unconditionally: it tests the
optional ``solana`` extra against the real codec, and skipping individual cases
would hide a broken adapter. A base install has no solders, so the whole module
is left out of collection there; installing the extra brings it back.

Adapter environments must not rely on this: they check for the dependency
explicitly and run ``pytest tests/test_solders_adapter.py``, which fails (exit
code 5, "no tests ran") if the module was dropped.
"""

from importlib.util import find_spec

collect_ignore = [] if find_spec("solders") is not None else ["test_solders_adapter.py"]
