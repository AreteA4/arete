"""Tests for arete.stack: the binding model and the connected program
runtime (raw builders, PDA factories, account readers, semantic operations,
attribute namespaces)."""

from __future__ import annotations

import pytest

from arete.instructions import (
    AccountMeta,
    AccountRefSeed,
    ArgRefSeed,
    ArgSchema,
    ErrorMetadata,
    InstructionError,
    InstructionHandler,
    Known,
    LiteralSeed,
    PdaConfig,
    Signer,
    UserProvided,
    encode_base58,
    find_program_address,
)
from arete.operations import create_prepared_instruction
from arete.read import ProgramAccountReadDef, ProgramReadRequest
from arete.stack import (
    AttrNamespace,
    ConnectedProgram,
    Operation,
    ProgramDef,
    ProgramsNamespace,
    StackDef,
    StackEndpoints,
    instruction_operation,
    normalize_program_operations,
    transaction_operation,
    with_programs,
)
from arete.views import ViewDef

PROGRAM_ID = encode_base58(bytes([7] * 32))
SYSTEM_PROGRAM = "11111111111111111111111111111111"
ALICE = encode_base58(bytes([1] * 32))
BOB = encode_base58(bytes([2] * 32))

MINER_PDA_CONFIG = PdaConfig(
    seeds=[LiteralSeed("miner"), AccountRefSeed("authority")]
)

DEPLOY_HANDLER = InstructionHandler(
    program_id=PROGRAM_ID,
    discriminator=bytes([1]),
    accounts=[
        AccountMeta("signer", True, True, Signer()),
        AccountMeta("miner", False, True, UserProvided()),
        AccountMeta("system_program", False, False, Known(SYSTEM_PROGRAM)),
    ],
    args=[ArgSchema("amount", "u64")],
    errors=[ErrorMetadata(code=6000, name="AmountTooSmall", msg="Amount too small")],
)

PROPOSAL_PDA_CONFIG = PdaConfig(seeds=[LiteralSeed("proposal"), ArgRefSeed("id", "u64")])


class FakeClient:
    """Duck-typed connected client for direct ConnectedProgram tests."""

    def __init__(self, wallet=None, chain=None):
        self.wallet = wallet
        self.chain = chain


class FakeWallet:
    public_key = ALICE

    async def sign_and_send(self, instructions, options=None, context=None):
        raise AssertionError("not used")


class FakeReadTransport:
    def __init__(self, responses=None):
        self.requests = []
        self._responses = dict(responses or {})

    async def read(self, request: ProgramReadRequest):
        self.requests.append(request)
        return self._responses.get(request.operation)


def make_program_def(**overrides):
    values = dict(
        name="ore",
        program_id=PROGRAM_ID,
        raw_instructions={"deploy": DEPLOY_HANDLER},
        pdas={"miner": MINER_PDA_CONFIG, "proposal": PROPOSAL_PDA_CONFIG},
        accounts={"miner": ProgramAccountReadDef(account="Miner")},
        errors=(ErrorMetadata(code=6000, name="AmountTooSmall", msg="Amount too small"),),
        program_spec_hash="spec:hash",
    )
    values.update(overrides)
    return ProgramDef(**values)


def connect_program(definition=None, *, wallet=None, transport=None, chain=None):
    client = FakeClient(wallet=wallet, chain=chain)
    return (
        ConnectedProgram(
            "ore",
            definition or make_program_def(),
            client,
            transport or FakeReadTransport(),
        ),
        client,
    )


class TestAttrNamespace:
    def test_attribute_access_and_helpful_errors(self):
        ns = AttrNamespace("programs.ore.raw", {"deploy": 1, "checkpoint": 2})
        assert ns.deploy == 1
        assert "deploy" in ns
        assert sorted(ns) == ["checkpoint", "deploy"]
        with pytest.raises(AttributeError, match="available: checkpoint, deploy"):
            ns.depoly

    def test_nested_mappings_wrap_lazily(self):
        ns = AttrNamespace("math", {"fees": {"bps": 25}})
        assert ns.fees.bps == 25


class TestRawInstructions:
    def test_build_with_kwargs(self):
        program, _ = connect_program()
        built = program.raw.deploy.build(amount=5, signer=ALICE, miner=BOB)
        assert built.program_id == PROGRAM_ID
        assert [meta.pubkey for meta in built.accounts] == [ALICE, BOB, SYSTEM_PROGRAM]
        assert built.data == bytes([1]) + (5).to_bytes(8, "little")

    def test_unknown_param_fails_closed(self):
        program, _ = connect_program()
        with pytest.raises(InstructionError, match="Unknown parameter"):
            program.raw.deploy.build(amount=5, signer=ALICE, miner=BOB, typo=1)

    def test_wallet_is_the_reserved_signer_fallback(self):
        program, _ = connect_program()
        built = program.raw.deploy.build(amount=5, miner=BOB, wallet=ALICE)
        assert built.accounts[0].pubkey == ALICE

    def test_payer_account_name_is_not_shadowed_by_the_reserved_option(self):
        # `payer` is an ordinary IDL account name: it must reach account
        # overrides, not the reserved signer-fallback option (which is
        # spelled `wallet`, mirroring TS BuildOptions).
        handler = InstructionHandler(
            program_id=PROGRAM_ID,
            discriminator=bytes([2]),
            accounts=[
                AccountMeta("signer", True, True, Signer()),
                AccountMeta("payer", False, True, UserProvided()),
            ],
            args=[],
        )
        program, _ = connect_program(
            make_program_def(raw_instructions={"fund": handler})
        )
        built = program.raw.fund.build(wallet=ALICE, payer=BOB)
        assert [meta.pubkey for meta in built.accounts] == [ALICE, BOB]

    def test_payer_defaults_to_live_client_wallet(self):
        program, client = connect_program()
        with pytest.raises(InstructionError, match="Missing required accounts"):
            program.raw.deploy.build(amount=5, miner=BOB)
        client.wallet = FakeWallet()  # set after connect: wallet is read live
        built = program.raw.deploy.build(amount=5, miner=BOB)
        assert built.accounts[0].pubkey == ALICE

    def test_handler_escape_hatch(self):
        program, _ = connect_program()
        assert program.raw.deploy.handler is DEPLOY_HANDLER


class TestPdas:
    def test_derive_account_ref_seed(self):
        program, _ = connect_program()
        expected = find_program_address(
            [b"miner", bytes([1] * 32)], PROGRAM_ID
        )
        assert program.pdas.miner.derive(authority=ALICE) == expected

    def test_derive_arg_ref_seed(self):
        program, _ = connect_program()
        expected = find_program_address(
            [b"proposal", (11).to_bytes(8, "little")], PROGRAM_ID
        )
        assert program.pdas.proposal.derive(id=11) == expected

    def test_unknown_seed_kwarg_fails_closed(self):
        program, _ = connect_program()
        with pytest.raises(TypeError, match="unexpected keyword"):
            program.pdas.miner.derive(authority=ALICE, extra=1)

    def test_unknown_pda_name(self):
        program, _ = connect_program()
        with pytest.raises(AttributeError, match="available: miner, proposal"):
            program.pdas.treasury


class TestAccounts:
    @pytest.mark.asyncio
    async def test_account_reader_bound_to_transport(self):
        transport = FakeReadTransport({"fetch": {"balance": "5"}, "exists": {"exists": True}})
        program, _ = connect_program(transport=transport)
        value = await program.accounts.miner.fetch(ALICE)
        assert value == {"balance": "5"}
        assert await program.accounts.miner.exists(ALICE) is True
        assert [request.account for request in transport.requests] == ["Miner", "Miner"]


class TestErrors:
    def test_error_metadata_and_parse(self):
        program, _ = connect_program()
        assert program.errors[0].name == "AmountTooSmall"
        assert program.parse_error(6000).name == "AmountTooSmall"
        assert program.parse_error(9999).name == "CustomError9999"

    def test_program_identity(self):
        program, _ = connect_program()
        assert program.PROGRAM_ID == PROGRAM_ID
        assert program.program_id == PROGRAM_ID
        assert program.program_spec_hash == "spec:hash"


class TestSemanticOperations:
    def test_create_operations_receives_connected_context(self):
        seen = {}

        def create_operations(context):
            seen["wallet"] = context.wallet
            seen["chain"] = context.chain
            seen["program"] = context.program

            def prepare(**input):
                built = context.program.raw.deploy.build(
                    signer=ALICE, miner=BOB, **input
                )
                return create_prepared_instruction(
                    name="ore.deploy", instruction=built, artifacts=input
                )

            return {"instructions": {"deploy": instruction_operation(prepare)}}

        chain = object()
        wallet = FakeWallet()
        program, _ = connect_program(
            make_program_def(create_operations=create_operations),
            wallet=wallet,
            chain=chain,
        )
        assert seen["wallet"] is wallet
        assert seen["chain"] is chain
        assert seen["program"] is program

    @pytest.mark.asyncio
    async def test_prepare_with_kwargs(self):
        def create_operations(context):
            def prepare(**input):
                built = context.program.raw.deploy.build(
                    signer=ALICE, miner=BOB, **input
                )
                return create_prepared_instruction(
                    name="ore.deploy", instruction=built, artifacts=dict(input)
                )

            return {"instructions": {"deploy": instruction_operation(prepare)}}

        program, _ = connect_program(
            make_program_def(create_operations=create_operations)
        )
        prepared = await program.instructions.deploy.prepare(amount=5)
        assert prepared.kind == "instruction"
        assert prepared.artifacts == {"amount": 5}
        assert prepared.transaction.required_signer_addresses == (ALICE,)

    @pytest.mark.asyncio
    async def test_wrong_cardinality_fails_closed(self):
        def create_operations(context):
            def prepare(**input):
                return create_prepared_instruction(
                    name="wrong", instruction=DEPLOY_HANDLER.build(
                        {"amount": 1, "signer": ALICE, "miner": BOB}
                    )
                )

            return {"transactions": {"broken": transaction_operation(prepare)}}

        program, _ = connect_program(
            make_program_def(create_operations=create_operations)
        )
        with pytest.raises(TypeError, match="expected a prepared transaction"):
            await program.transactions.broken.prepare()

    def test_nested_operation_namespaces(self):
        def create_operations(context):
            return {
                "flows": {
                    "claims": {"all": Operation("flow", lambda **_: None)},
                }
            }

        program, _ = connect_program(
            make_program_def(create_operations=create_operations)
        )
        assert program.flows.claims.all.kind == "flow"

    def test_normalize_rejects_unknown_group(self):
        with pytest.raises(TypeError, match="unknown keys"):
            normalize_program_operations({"queries": {}})


class TestStackDef:
    def test_views_shape_matches_views_namespace(self):
        stack = StackDef(
            name="ore-stream",
            endpoints=StackEndpoints(ws="wss://example.com/ws"),
            views={
                "ore_round": {
                    "latest": ViewDef(mode="list", view="OreRound/latest"),
                    "state": ViewDef(
                        mode="state", view="OreRound/state", key_fields=("round_id",)
                    ),
                }
            },
            programs={"ore": make_program_def()},
        )
        assert stack.endpoints.http is None
        assert stack.views["ore_round"]["latest"].mode == "list"

    def test_with_programs_stack_keys_win(self):
        stack = StackDef(name="s", programs={"ore": make_program_def()})
        attached = make_program_def(name="other")
        with pytest.warns(UserWarning, match="already defines that key"):
            merged = with_programs(stack, {"ore": attached, "extra": attached})
        assert merged.programs["ore"].name == "ore"
        assert merged.programs["extra"].name == "other"
        # The original definition is untouched.
        assert "extra" not in stack.programs

    def test_programs_namespace_errors(self):
        program, _ = connect_program()
        namespace = ProgramsNamespace({"ore": program})
        assert namespace.ore is program
        assert "ore" in namespace
        with pytest.raises(AttributeError, match="no program 'spl'"):
            namespace.spl
