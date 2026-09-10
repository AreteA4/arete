"""Optional first-party wallet adapters.

Nothing is imported here: every adapter in this package depends on an optional
extra, so importing :mod:`arete.adapters` must stay free of third-party
imports. Import the adapter you want directly::

    from arete.adapters.solders import SoldersWalletAdapter  # pip install 'arete-sdk[solana]'
"""
