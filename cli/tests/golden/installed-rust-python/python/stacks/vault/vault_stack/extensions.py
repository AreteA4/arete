"""Vault stack extension (Python)."""


def create_read(client):
    async def vault(key):
        return await client.views.vault.state.get(key)

    return {"vault": vault}


STACK_EXTENSIONS = {
    "defaults": {"limits": lambda: {"max_deposit": 1_000_000}},
    "read_arg_counts": {"vault": 1},
    "create_read": create_read,
}
