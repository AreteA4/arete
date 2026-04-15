import asyncio
from hyperstack import HyperStackClient
from dataclasses import dataclass
from typing import Optional, Any, Dict


@dataclass
class GameId:
    global_count: int
    account_id: int


@dataclass
class GameStatus:
    current: str
    created_at: int
    activated_at: Optional[int] = None
    settled_at: Optional[int] = None


@dataclass
class GameMetrics:
    bet_count: int
    total_volume: int
    total_ev: int
    total_fees_collected: int
    claim_rate: Optional[float] = None
    house_profit_loss: Optional[int] = None
    total_payouts_distributed: Optional[int] = None
    unique_players: Optional[int] = None


@dataclass
class Game:
    id: GameId
    status: GameStatus
    metrics: GameMetrics
    events: Optional[Dict[str, Any]] = None


class SettlementGame:
    NAME = "SettlementGame"

    @staticmethod
    def state_view() -> str:
        return "SettlementGame/state"

    @staticmethod
    def list_view() -> str:
        return "SettlementGame/list"


def parse_game(data: Dict[str, Any]) -> Game:
    id_data = data.get("id", {})
    status_data = data.get("status", {})
    metrics_data = data.get("metrics", {})

    return Game(
        id=GameId(
            global_count=id_data.get("global_count", 0),
            account_id=id_data.get("account_id", 0),
        ),
        status=GameStatus(
            current=status_data.get("current", ""),
            created_at=status_data.get("created_at", 0),
            activated_at=status_data.get("activated_at"),
            settled_at=status_data.get("settled_at"),
        ),
        metrics=GameMetrics(
            bet_count=metrics_data.get("bet_count", 0),
            total_volume=metrics_data.get("total_volume", 0),
            total_ev=metrics_data.get("total_ev", 0),
            total_fees_collected=metrics_data.get("total_fees_collected", 0),
            claim_rate=metrics_data.get("claim_rate"),
            house_profit_loss=metrics_data.get("house_profit_loss"),
            total_payouts_distributed=metrics_data.get("total_payouts_distributed"),
            unique_players=metrics_data.get("unique_players"),
        ),
        events=data.get("events"),
    )


async def main():
    async with HyperStackClient("ws://localhost:8080") as client:
        game_store = client.watch(SettlementGame, parser=parse_game)

        print(f"connected, watching {game_store.view}\n")

        async for update in game_store:
            game = update.data

            print(f"\n=== Game {update.key} ===")
            print(f"Status: {game.status.current}")
            print(f"Bet count: {game.metrics.bet_count}")
            print(f"Total volume: {game.metrics.total_volume}")
            print(f"Claim rate: {game.metrics.claim_rate}")
            print(f"Created at: {game.status.created_at}")


if __name__ == "__main__":
    asyncio.run(main())
