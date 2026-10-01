import json
from pathlib import Path
import httpx
import pytest
from arete.chain import HttpChainClient
from arete.http import HttpAuthClient
from arete.managed_solana import *
from arete.read import AccountReader
from arete.transactions import HttpTransactionTransport
from arete.rpc import RpcTransactionTransport

ROOT = Path(__file__).resolve().parents[3] / "tests/fixtures/managed-solana-v1"
def fixture(name): return json.loads((ROOT / f"{name}.json").read_text())
def auth(handler): return HttpAuthClient(http_client=httpx.AsyncClient(transport=httpx.MockTransport(handler)))

@pytest.mark.asyncio
async def test_owner_pages_exact_inventory_and_unknown_freshness():
    cases = fixture("owner-token-accounts")["cases"][:3]
    requests = []
    def handle(request):
        requests.append(json.loads(request.content))
        return httpx.Response(200, json=cases[len(requests)-1]["response"])
    chain = HttpChainClient("https://gateway.example", auth(handle))
    first = await chain.owner_token_accounts(OwnerTokenAccountsRequest(owner=cases[0]["request"]["owner"], mint=cases[0]["request"]["mint"], token_program=cases[0]["request"]["tokenProgram"], limit=1))
    assert first.items[0].amount == 18446744073709551615
    assert first.discovery.watermark is None
    next_page = await chain.owner_token_accounts(OwnerTokenAccountsRequest(owner=cases[1]["request"]["owner"], mint=cases[1]["request"]["mint"], token_program=cases[1]["request"]["tokenProgram"], limit=1, cursor=first.next_cursor))
    assert next_page.items == () and next_page.discovery.watermark == 9007199254740993
    assert requests[1] == cases[1]["request"]

@pytest.mark.asyncio
async def test_contextual_raw_and_typed_batch_preserves_context_and_errors():
    cases = fixture("contextual-accounts")["cases"]
    chain = HttpChainClient("https://gateway.example", auth(lambda request: httpx.Response(200, json=cases[0]["response"])))
    actual = await chain.account_with_context(cases[0]["request"]["address"], ReadOptions("finalized", 9007199254740993))
    assert actual.context.slot == 9007199254740994 and actual.value.lamports == 9007199254740993
    assert (await chain.accounts_with_context([])).context is None
    class Transport:
        async def read(self, request):
            assert request.options.min_context_slot == 40
            return cases[4]["response"]
    reader = AccountReader("Position", Transport())
    actual = await reader.fetch_many_with_context(cases[4]["request"]["addresses"], ReadOptions("confirmed", 40))
    assert actual.context.slot == 42
    assert actual.value.items[1].status == "missing"
    assert actual.value.items[2].error_code == "ACCOUNT_DECODE_FAILED"

@pytest.mark.asyncio
async def test_full_transactions_keep_execution_and_unavailable_metadata():
    for case in fixture("transactions")["cases"]:
        transport = HttpTransactionTransport("https://gateway.example", auth(lambda request: httpx.Response(200, json=case["response"])))
        actual = await transport.get("fixture-signature", max_supported_transaction_version=1)
        expected = case["response"]["transaction"]
        if expected is None:
            assert actual is None
        else:
            assert actual.transaction == expected["transaction"] and actual.meta == expected["meta"]
            assert actual.metadata_available == expected["metadataAvailable"]
            assert actual.version == expected.get("version")

@pytest.mark.asyncio
async def test_rpc_transactions_match_managed_execution_metadata():
    for case in fixture("transactions")["cases"]:
        requests = []
        def handle(request):
            requests.append(json.loads(request.content))
            return httpx.Response(200, json={"jsonrpc": "2.0", "id": 1, "result": case["upstream"]})
        transport = RpcTransactionTransport("https://rpc.example", http_client=httpx.AsyncClient(transport=httpx.MockTransport(handle)))
        actual = await transport.get("fixture-signature", max_supported_transaction_version=1)
        expected = case["response"]["transaction"]
        assert requests[0]["params"][1]["encoding"] == "jsonParsed"
        if expected is None:
            assert actual is None
        else:
            assert actual.transaction == expected["transaction"] and actual.meta == expected["meta"]
            assert actual.metadata_available == expected["metadataAvailable"]
            assert actual.version == expected.get("version")

@pytest.mark.parametrize("value", [42, "18446744073709551616", "-1", "+42", "1.5"])
def test_invalid_exact_integers_are_rejected(value):
    with pytest.raises(ValueError): decimal_u64(value, "slot")

@pytest.mark.asyncio
async def test_raw_enum_payload_remains_available_to_python_readers():
    expected = fixture("dynamic-tick-account")["expected"]
    class Transport:
        async def read(self, request): return expected
    value = await AccountReader("TickFixture", Transport()).fetch("fixture-address")
    assert int(value["tick"]["Initialized"]["liquidity_gross"]) == 2**100

def test_shared_fixture_bundle_matches_versioned_schemas():
    from jsonschema import Draft202012Validator, FormatChecker
    schema = fixture("schema")
    def validate(name, value):
        definition = {"$ref": f"#/$defs/{name}", "$defs": schema["$defs"]}
        Draft202012Validator(definition, format_checker=FormatChecker()).validate(value)
    for case in fixture("owner-token-accounts")["cases"]:
        if "status" in case:
            validate("error", case["response"])
        else:
            validate("ownerRequest", case["request"])
            validate("ownerPage", case["response"])
    for index, case in enumerate(fixture("contextual-accounts")["cases"]):
        validate("accountRequest" if index < 2 else "accountsRequest", case["request"])
        validate("rawSingleResponse" if index < 2 else "rawBatchResponse" if index < 4 else "typedBatchResponse", case["response"])
    for case in fixture("native-position-query")["cases"]:
        if "status" in case:
            validate("error", case["response"])
        else:
            validate("nativeQuery", case["request"])
            validate("nativePage", case["response"])
    for case in fixture("transactions")["cases"]:
        validate("transactionResponse", case["response"])
