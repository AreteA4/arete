"""Generated typed views for the `9lives` stack. Do not edit.

`VIEWS` feeds `StackDef.views`; group keys are snake_case entity names
(`a4.views.<entity>.<view>`). State views declare typed key fields consumed
as keyword arguments (`.use(<key_field>=...)`).
"""

from __future__ import annotations

from typing import Dict

from arete.views import ViewDef

from . import models

__all__ = [
    "VaultViews",
    "VIEWS",
]


class VaultViews:
    """Typed views of the `Vault` entity."""

    state = ViewDef(
        mode="state",
        view="Vault/state",
        key_fields=("address",),
        parser=models.vault_from_wire,
    )
    list = ViewDef(mode="list", view="Vault/list", parser=models.vault_from_wire)


VIEWS: Dict[str, Dict[str, ViewDef]] = {
    "vault": {
        "state": VaultViews.state,
        "list": VaultViews.list,
    },
}
