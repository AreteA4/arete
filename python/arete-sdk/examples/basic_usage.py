import asyncio
from arete import AreteClient


async def basic_subscribe():
    view = "SettlementGame/list"

    async with AreteClient("wss://flip.stack.hypertek.app") as client:
        store = client.subscribe(view=view)

        print(f"Subscribed to {view}, waiting for updates...\n")

        async for update in store:
            print(f"Update received for key '{update.key}':")
            print(f"  Data: {update.data}\n")


async def multiple_subscriptions():
    async with AreteClient("wss://flip.stack.hypertek.app") as client:
        games_store = client.subscribe("SettlementGame/list")
        games_store_state = client.subscribe("SettlementGame/state")

        print("Subscribed to multiple views\n")

        async def handle_games():
            async for update in games_store:
                print(f"[GAME LIST] {update.key} updated")

        async def handle_games_state():
            async for update in games_store_state:
                print(f"[GAME STATE] {update.key} updated")

        await asyncio.gather(handle_games(), handle_games_state())


if __name__ == "__main__":
    try:
        asyncio.run(basic_subscribe())
        # asyncio.run(multiple_subscriptions())
    except KeyboardInterrupt:
        print("Keyboard interrupt received. Exiting gracefully.")
