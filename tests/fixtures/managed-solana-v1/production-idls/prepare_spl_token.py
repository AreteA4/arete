"""Adapt the pinned public Anchor SPL IDL to explicit native binary layouts.

Account COptions have a u32 tag and always-present padded value. Instruction
COptions use a u8 option tag. Account names, fields and instruction interfaces
come from the public source; this script adds the native layout declarations.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
idl = json.loads((ROOT / "spl-token.source.json").read_text())
idl["address"] = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
idl["metadata"] = {"address": idl["address"]}


def convert(value, account):
    if isinstance(value, dict):
        if value.get("defined") == "&'astr":
            return "string"
        if value.get("defined") in ("COption<Pubkey>", "COption<u64>"):
            pubkey = value["defined"] == "COption<Pubkey>"
            return {"defined": "COptionPubkey" if pubkey else "COptionU64"} if account else {"option": "publicKey" if pubkey else "u64"}
        return {key: convert(item, account) for key, item in value.items()}
    if isinstance(value, list):
        return [convert(item, account) for item in value]
    return value


for account in idl["accounts"]:
    account["docs"] = ["arete.account_untagged=true"]
    account["discriminator"] = []
    account["type"] = convert(account["type"], True)
for opcode, instruction in enumerate(idl["instructions"]):
    instruction["discriminant"] = {"type": "u8", "value": opcode}
    instruction["args"] = convert(instruction["args"], False)
for name, value in [("COptionPubkey", "publicKey"), ("COptionU64", "u64")]:
    idl["types"].append({"name": name, "type": {"kind": "struct", "fields": [
        {"name": "tag", "type": "u32"}, {"name": "value", "type": value}
    ]}})
(ROOT / "spl-token.idl.json").write_text(json.dumps(idl, indent=2) + "\n")
