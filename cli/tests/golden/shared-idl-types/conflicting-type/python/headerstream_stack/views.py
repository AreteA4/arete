"""Generated typed views for the `HeaderStream` stack. Do not edit.

`VIEWS` feeds `StackDef.views`; group keys are snake_case entity names
(`a4.views.<entity>.<view>`). State views declare typed key fields consumed
as keyword arguments (`.use(<key_field>=...)`).
"""

from __future__ import annotations

from typing import Dict

from arete.views import ViewDef

from . import models

__all__ = [
    "AlphaVaultViews",
    "BetaVaultViews",
    "VIEWS",
]


class AlphaVaultViews:
    """Typed views of the `AlphaVault` entity."""

    state = ViewDef(
        mode="state",
        view="AlphaVault/state",
        key_fields=("address",),
        parser=models.alpha_vault_from_wire,
    )
    list = ViewDef(mode="list", view="AlphaVault/list", parser=models.alpha_vault_from_wire)


class BetaVaultViews:
    """Typed views of the `BetaVault` entity."""

    state = ViewDef(
        mode="state",
        view="BetaVault/state",
        key_fields=("address",),
        parser=models.beta_vault_from_wire,
    )
    list = ViewDef(mode="list", view="BetaVault/list", parser=models.beta_vault_from_wire)


VIEWS: Dict[str, Dict[str, ViewDef]] = {
    "alpha_vault": {
        "state": AlphaVaultViews.state,
        "list": AlphaVaultViews.list,
    },
    "beta_vault": {
        "state": BetaVaultViews.state,
        "list": BetaVaultViews.list,
    },
}
