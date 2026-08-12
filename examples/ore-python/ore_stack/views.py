"""Generated typed views for the `OreStream` stack. Do not edit.

`VIEWS` feeds `StackDef.views`; group keys are snake_case entity names
(`a4.views.<entity>.<view>`). State views declare typed key fields consumed
as keyword arguments (`.use(<key_field>=...)`).
"""

from __future__ import annotations

from typing import Dict

from arete.views import ViewDef

from . import models

__all__ = [
    "OreRoundViews",
    "OreBoardViews",
    "OreTreasuryViews",
    "OreMinerViews",
    "VIEWS",
]


class OreRoundViews:
    """Typed views of the `OreRound` entity."""

    state = ViewDef(
        mode="state",
        view="OreRound/state",
        key_fields=("round_id",),
        parser=models.ore_round_from_wire,
    )
    list = ViewDef(mode="list", view="OreRound/list", parser=models.ore_round_from_wire)
    latest = ViewDef(mode="list", view="OreRound/latest", parser=models.ore_round_from_wire)


class OreBoardViews:
    """Typed views of the `OreBoard` entity."""

    state = ViewDef(
        mode="state",
        view="OreBoard/state",
        key_fields=("address",),
        parser=models.ore_board_from_wire,
    )
    list = ViewDef(mode="list", view="OreBoard/list", parser=models.ore_board_from_wire)


class OreTreasuryViews:
    """Typed views of the `OreTreasury` entity."""

    state = ViewDef(
        mode="state",
        view="OreTreasury/state",
        key_fields=("address",),
        parser=models.ore_treasury_from_wire,
    )
    list = ViewDef(mode="list", view="OreTreasury/list", parser=models.ore_treasury_from_wire)


class OreMinerViews:
    """Typed views of the `OreMiner` entity."""

    state = ViewDef(
        mode="state",
        view="OreMiner/state",
        key_fields=("authority",),
        parser=models.ore_miner_from_wire,
    )
    list = ViewDef(mode="list", view="OreMiner/list", parser=models.ore_miner_from_wire)


VIEWS: Dict[str, Dict[str, ViewDef]] = {
    "ore_round": {
        "state": OreRoundViews.state,
        "list": OreRoundViews.list,
        "latest": OreRoundViews.latest,
    },
    "ore_board": {
        "state": OreBoardViews.state,
        "list": OreBoardViews.list,
    },
    "ore_treasury": {
        "state": OreTreasuryViews.state,
        "list": OreTreasuryViews.list,
    },
    "ore_miner": {
        "state": OreMinerViews.state,
        "list": OreMinerViews.list,
    },
}
