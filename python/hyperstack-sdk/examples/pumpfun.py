import asyncio
from dataclasses import dataclass
from typing import Any, Dict, Optional

from hyperstack import HyperStackClient


@dataclass
class PumpfunTokenData:
    mint: str
    name: str
    symbol: str
    creator: Optional[str]
    timestamp: Optional[int]


def parse_token(payload: Dict[str, Any]) -> Optional[PumpfunTokenData]:
    info = payload.get("info") if isinstance(payload.get("info"), dict) else {}
    token_id = payload.get("id") if isinstance(payload.get("id"), dict) else {}
    events = payload.get("events") if isinstance(payload.get("events"), dict) else {}

    name = info.get("name")
    symbol = info.get("symbol")
    mint = token_id.get("mint")

    creator = None
    timestamp = None
    create_event = events.get("create")
    if isinstance(create_event, dict):
        name = name or create_event.get("name")
        symbol = symbol or create_event.get("symbol")
        mint = mint or create_event.get("mint")
        creator = create_event.get("creator")
        timestamp = create_event.get("timestamp")

    if not mint or not name or not symbol:
        return None

    return PumpfunTokenData(
        mint=mint,
        name=name,
        symbol=symbol,
        creator=creator,
        timestamp=timestamp,
    )


async def main() -> None:
    print("Connecting to Solana via Hyperstack...\n")
    async with HyperStackClient(
        "wss://pumpfun-token-rfx6zp.stack.usehyperstack.com"
    ) as client:
        print("Connected! Streaming live pump.fun tokens:\n")
        async for update in client.subscribe("PumpfunToken/list"):
            if not isinstance(update.data, dict):
                continue
            token = parse_token(update.data)
            if not token:
                continue
            print(f"New token: {token.name} ({token.symbol})")
            print(f"  Mint: {token.mint}")
            if token.creator:
                creator_short = (
                    f"{token.creator[:8]}..."
                    if len(token.creator) > 8
                    else token.creator
                )
                print(f"  Creator: {creator_short}")
            print("")


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Keyboard interrupt received. Exiting gracefully.")

