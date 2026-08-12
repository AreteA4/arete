"""Tests for arete.session (ported from session.test.ts): member clients,
program promotion, composition-mode validation, wallet fan-out, and the
execution host."""

from __future__ import annotations

import warnings

import pytest

from arete.client import Arete
from arete.errors import AreteError
from arete.instructions import (
    AccountMeta,
    ArgSchema,
    ErrorMetadata,
    InstructionHandler,
    Signer,
    UserProvided,
    encode_base58,
)
from arete.operations import (
    SignerRegistry,
    create_prepared_instruction,
)
from arete.program_read_transport import (
    HttpAuthMetadata,
    HostedBindingTransportDef,
    LocalHttpTransportDef,
    ProgramReadBinding,
    ProgramReadDescriptor,
    ProgramReleaseReference,
)
from arete.read import ProgramAccountReadDef
from arete.session import SessionError, create_session
from arete.stack import ConnectedProgram, ProgramDef, StackDef, StackEndpoints
from arete.views import ViewDef
from arete.wallet import SendResult

PROGRAM_ID = encode_base58(bytes([7] * 32))
OTHER_PROGRAM_ID = encode_base58(bytes([8] * 32))
ALICE = encode_base58(bytes([1] * 32))
BOB = encode_base58(bytes([2] * 32))
BINDING_ID = "prb_00000000000000000000000000000001"

DEPLOY_HANDLER = InstructionHandler(
    program_id=PROGRAM_ID,
    discriminator=bytes([1]),
    accounts=[
        AccountMeta("signer", True, True, Signer()),
        AccountMeta("miner", False, True, UserProvided()),
    ],
    args=[ArgSchema("amount", "u64")],
)


def make_program(name="ore", program_id=PROGRAM_ID, **overrides):
    values = dict(
        name=name,
        program_id=program_id,
        raw_instructions={"deploy": DEPLOY_HANDLER},
    )
    values.update(overrides)
    return ProgramDef(**values)


def make_stack(name="ore-stream", program_key="ore", **overrides):
    values = dict(
        name=name,
        endpoints=StackEndpoints(ws="", http="https://api.example.test"),
        views={"ore_round": {"latest": ViewDef(mode="list", view="OreRound/latest")}},
        programs={program_key: make_program()},
    )
    values.update(overrides)
    return StackDef(**values)


def local_descriptor():
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash="release:hash", program_spec_hash="spec:hash"
        ),
        transport=LocalHttpTransportDef(),
    )


def hosted_descriptor():
    return ProgramReadDescriptor(
        release=ProgramReleaseReference(
            program_release_hash="release:hash", program_spec_hash="spec:hash"
        ),
        transport=HostedBindingTransportDef(
            binding=ProgramReadBinding(
                endpoint="https://reads.example.test",
                program_read_binding_id=BINDING_ID,
                auth=HttpAuthMetadata(
                    session_endpoint="https://api.example.test/sessions",
                    target_kind="program-read-binding",
                    target_id=BINDING_ID,
                ),
            )
        ),
    )


class FakeWallet:
    def __init__(self, public_key=ALICE):
        self.public_key = public_key
        self.calls = []

    async def sign_and_send(self, instructions, options=None, context=None):
        self.calls.append(
            {"instructions": list(instructions), "options": options, "context": context}
        )
        return SendResult(signature="session-sig", slot=1)


@pytest.mark.asyncio
class TestValidation:
    async def test_rejects_an_empty_definition(self):
        with pytest.raises(SessionError, match="at least one"):
            await create_session()

    async def test_rejects_unknown_mode(self):
        with pytest.raises(SessionError, match="Unknown session mode"):
            await create_session(stacks={"ore": make_stack()}, mode="federation")

    async def test_composition_requires_explicit_transports(self):
        with pytest.raises(SessionError, match="explicit chain and transaction"):
            await create_session(stacks={"ore": make_stack()}, mode="composition")

    async def test_composition_forbids_fallback_endpoints(self):
        with pytest.raises(SessionError, match="per-member live endpoints"):
            await create_session(
                stacks={"ore": make_stack()},
                mode="composition",
                chain=object(),
                transactions=object(),
                endpoints={"http": "https://shared.example.test"},
            )

    async def test_definition_program_reads_keys_must_match_programs(self):
        with pytest.raises(SessionError, match="exactly match"):
            await create_session(
                programs={"ore": make_program()},
                program_reads={"nope": hosted_descriptor()},
            )

    async def test_unknown_member_option_fails_closed(self):
        with pytest.raises(SessionError, match="Unknown option"):
            await create_session(
                stacks={"ore": make_stack()},
                transport="http",
                stack_options={"ore": {"websocket_url": "wss://x"}},
            )

    async def test_composition_rejects_local_program_reads(self):
        stack = make_stack(
            programs={
                "ore": make_program(
                    accounts={"miner": ProgramAccountReadDef(account="Miner")},
                    program_spec_hash="spec:hash",
                )
            },
            program_reads={"ore": local_descriptor()},
        )
        with pytest.raises(SessionError, match="hosted-binding"):
            await create_session(
                stacks={"ore": stack},
                mode="composition",
                chain=object(),
                transactions=object(),
            )

    async def test_composition_accepts_complete_hosted_override(self):
        stack = make_stack(
            programs={
                "ore": make_program(
                    accounts={"miner": ProgramAccountReadDef(account="Miner")},
                    program_spec_hash="spec:hash",
                )
            },
            program_reads={"ore": local_descriptor()},
        )
        chain = object()
        transactions = object()
        session = await create_session(
            stacks={"ore": stack},
            mode="composition",
            chain=chain,
            transactions=transactions,
            transport="http",
            program_read_overrides={"ore": hosted_descriptor()},
        )
        try:
            assert session.chain is chain
            assert session.transactions is transactions
        finally:
            await session.close()


@pytest.mark.asyncio
class TestMembers:
    async def test_members_get_their_own_clients(self):
        session = await create_session(
            stacks={"ore": make_stack()},
            programs={"spl": make_program(name="spl", program_id=OTHER_PROGRAM_ID)},
            transport="http",
        )
        try:
            assert isinstance(session.stacks.ore, Arete)
            assert isinstance(session.programs.spl, ConnectedProgram)
            assert session.programs.spl.program_id == OTHER_PROGRAM_ID
            # Distinct member clients: the stack client and the synthetic
            # program client are independent objects.
            with pytest.raises(AttributeError, match="no entry 'spl'"):
                session.stacks.spl
        finally:
            await session.close()

    async def test_stack_views_fail_fast_over_http_members(self):
        session = await create_session(
            stacks={"ore": make_stack()}, transport="http"
        )
        try:
            with pytest.raises(AreteError) as excinfo:
                await session.stacks.ore.views.ore_round.latest.get()
            assert excinfo.value.code == "WEBSOCKET_DISABLED"
        finally:
            await session.close()

    async def test_promotes_bundled_programs_by_reference(self):
        session = await create_session(
            stacks={"ore": make_stack()}, transport="http"
        )
        try:
            assert session.programs.ore is session.stacks.ore.programs.ore
        finally:
            await session.close()

    async def test_first_stack_wins_on_bundled_key_collisions_and_warns(self):
        first = make_stack(name="first-stack")
        second = make_stack(name="second-stack")
        with pytest.warns(UserWarning, match="uses 'one' because it was connected first"):
            session = await create_session(
                stacks={"one": first, "two": second}, transport="http"
            )
        try:
            assert session.programs.ore is session.stacks.one.programs.ore
        finally:
            await session.close()

    async def test_explicit_standalone_programs_win_over_promoted_keys(self):
        stack = make_stack()
        with warnings.catch_warnings():
            warnings.simplefilter("error")  # no collision warning expected
            session = await create_session(
                stacks={"stack": stack},
                programs={"ore": make_program(program_id=OTHER_PROGRAM_ID)},
                transport="http",
            )
        try:
            assert session.programs.ore.program_id == OTHER_PROGRAM_ID
            assert session.stacks.stack.programs.ore.program_id == PROGRAM_ID
        finally:
            await session.close()

    async def test_wallet_fans_out_to_every_member(self):
        session = await create_session(
            stacks={"ore": make_stack()},
            programs={"spl": make_program(name="spl", program_id=OTHER_PROGRAM_ID)},
            transport="http",
        )
        try:
            wallet = FakeWallet()
            session.set_wallet(wallet)
            assert session.wallet is wallet
            assert session.stacks.ore.wallet is wallet
            # The standalone program member sees the wallet too: its raw
            # builder now defaults the signer slot to the wallet key.
            built = session.programs.spl.raw.deploy.build(amount=1, miner=BOB)
            assert built.accounts[0].pubkey == ALICE
            session.set_wallet(None)
            assert session.stacks.ore.wallet is None
        finally:
            await session.close()


@pytest.mark.asyncio
class TestExecution:
    async def make_session(self, **kwargs):
        wallet = FakeWallet()
        session = await create_session(
            stacks={"ore": make_stack()},
            transport="http",
            wallet=wallet,
            **kwargs,
        )
        return session, wallet

    async def test_transaction_executes_on_the_first_connected_member(self):
        session, wallet = await self.make_session()
        try:
            built = session.programs.ore.raw.deploy.build(amount=1, miner=BOB)
            result = await session.transaction([built])
            assert result.signature == "session-sig"
            assert len(wallet.calls) == 1
            # The session transport (the host's default) rides along.
            assert (
                wallet.calls[0]["context"].transaction_transport
                is session.transactions
            )
        finally:
            await session.close()

    async def test_registered_session_signers_are_used(self):
        registry = SignerRegistry([("registered-address", {"kp": 1})])
        session, wallet = await self.make_session(signer_registry=registry)
        try:
            built = session.programs.ore.raw.deploy.build(amount=1, miner=BOB)
            await session.transaction([built], signers=["extra"])
            options = wallet.calls[0]["options"]
            assert options.signers == ({"kp": 1}, "extra")

            prepared = create_prepared_instruction(
                name="needs-registered",
                instruction=built,
                required_signer_addresses=["registered-address"],
            )
            receipt = await session.execute(prepared)
            assert receipt.signatures == ("session-sig",)
        finally:
            await session.close()

    async def test_execution_defaults_apply(self):
        session, wallet = await self.make_session(
            execution={"send": {"confirmation_level": "finalized"}}
        )
        try:
            built = session.programs.ore.raw.deploy.build(amount=1, miner=BOB)
            await session.transaction([built])
            assert wallet.calls[0]["options"].confirmation_level == "finalized"
        finally:
            await session.close()

    async def test_close_disconnects_all_members(self):
        session, _ = await self.make_session(
            programs={"spl": make_program(name="spl", program_id=OTHER_PROGRAM_ID)}
        )
        await session.close()
        assert session.stacks.ore.connection_state == "disconnected"


@pytest.mark.asyncio
class TestSessionChain:
    async def test_shared_fallback_http_endpoint_builds_a_session_chain(self):
        session = await create_session(
            stacks={"ore": make_stack()},
            transport="http",
            endpoints={"http": "https://shared.example.test"},
        )
        try:
            assert session.chain._base_url == "https://shared.example.test"
        finally:
            await session.close()

    async def test_injected_chain_wins(self):
        chain = object()
        session = await create_session(
            stacks={"ore": make_stack()}, transport="http", chain=chain
        )
        try:
            assert session.chain is chain
        finally:
            await session.close()

    async def test_defaults_to_the_execution_hosts_chain(self):
        session = await create_session(stacks={"ore": make_stack()}, transport="http")
        try:
            assert session.chain is session.stacks.ore.chain
        finally:
            await session.close()
