"""Tests for arete.extensions (ported from stack-extensions.test.ts):
namespace deep-merge, factory composition, and connected-client surfacing."""

from __future__ import annotations

import pytest

from arete.extensions import (
    apply_connected_stack_extensions,
    extend_program,
    extend_programs,
    extend_stack,
    merge_namespace,
)
from arete.stack import (
    ConnectedProgram,
    ProgramDef,
    ProgramOperations,
    StackDef,
    flow_operation,
    instruction_operation,
    normalize_program_operations,
)


def make_program(**overrides):
    values = dict(name="ore", program_id="oreProgram", sdk_definition_hash="sdk:hash")
    values.update(overrides)
    return ProgramDef(**values)


class TestMergeNamespace:
    def test_deep_merges_mappings(self):
        merged = merge_namespace(
            {"fees": {"bps": 25, "flat": 1}, "kept": True},
            {"fees": {"bps": 30}, "added": 1},
        )
        assert merged == {"fees": {"bps": 30, "flat": 1}, "kept": True, "added": 1}

    def test_non_mappings_are_replaced(self):
        assert merge_namespace({"a": 1}, 2) == 2
        assert merge_namespace(1, {"a": 1}) == {"a": 1}


class TestExtendProgram:
    def test_attaches_namespaces_and_drops_sdk_hash(self):
        program = make_program(addresses={"treasury": "t1"}, constants={"fee": 1})
        extended = extend_program(
            program,
            addresses={"board": "b1"},
            constants={"fee": 2},
            defaults={"slippage": 5},
            math={"quote": "fn"},
        )
        assert extended.addresses == {"treasury": "t1", "board": "b1"}
        assert extended.constants == {"fee": 2}
        assert extended.defaults == {"slippage": 5}
        assert extended.math == {"quote": "fn"}
        assert extended.sdk_definition_hash is None
        # base is untouched
        assert program.addresses == {"treasury": "t1"}
        assert program.sdk_definition_hash == "sdk:hash"

    def test_composes_operation_factories_base_first(self):
        def base_factory(context):
            return {
                "instructions": {
                    "deploy": "base-deploy",
                    "checkpoint": "base-checkpoint",
                }
            }

        def extension_factory(context):
            return ProgramOperations(
                instructions={"deploy": "ext-deploy"},
                flows={"claim_all": "ext-flow"},
            )

        program = make_program(create_operations=base_factory)
        extended = extend_program(program, create_operations=extension_factory)
        operations = normalize_program_operations(extended.create_operations(None))
        assert operations.instructions == {
            "deploy": "ext-deploy",
            "checkpoint": "base-checkpoint",
        }
        assert operations.flows == {"claim_all": "ext-flow"}

    def test_deep_merges_nested_operation_resources(self):
        def base_factory(context):
            return {"transactions": {"claims": {"ore": "base-ore", "sol": "base-sol"}}}

        def extension_factory(context):
            return {"transactions": {"claims": {"sol": "ext-sol"}}}

        extended = extend_program(
            make_program(create_operations=base_factory),
            create_operations=extension_factory,
        )
        operations = normalize_program_operations(extended.create_operations(None))
        assert operations.transactions == {
            "claims": {"ore": "base-ore", "sol": "ext-sol"}
        }

    def test_extension_only_factory(self):
        extended = extend_program(
            make_program(),
            create_operations=lambda context: {"instructions": {"a": 1}},
        )
        operations = normalize_program_operations(extended.create_operations(None))
        assert operations.instructions == {"a": 1}


class TestExtendPrograms:
    def test_extends_only_targeted_entries(self):
        programs = {"ore": make_program(), "spl": make_program(name="spl")}
        extended = extend_programs(
            programs, {"ore": {"addresses": {"board": "b1"}}}
        )
        assert extended["ore"].addresses == {"board": "b1"}
        assert extended["spl"] is programs["spl"]

    def test_rejects_unknown_program_keys(self):
        with pytest.raises(ValueError, match="unknown program"):
            extend_programs({"ore": make_program()}, {"nope": {}})


class TestExtendStack:
    def test_attaches_namespaces_and_composes_factories(self):
        def base_read(client):
            return {"round": "base-round", "kept": "base-kept"}

        stack = StackDef(
            name="ore-stream",
            addresses={"treasury": "t1"},
            read_arg_counts={"round": 1, "kept": 0},
            create_read=base_read,
        )
        extended = extend_stack(
            stack,
            addresses={"board": "b1"},
            constants={"fee": 1},
            defaults={"slippage": 5},
            math={"quote": "fn"},
            read_arg_counts={"round": 2},
            create_read=lambda client: {"round": "ext-round"},
            create_flows=lambda client: {"claim": "flow"},
        )
        assert extended.addresses == {"treasury": "t1", "board": "b1"}
        assert extended.constants == {"fee": 1}
        assert extended.read_arg_counts == {"round": 2, "kept": 0}
        assert extended.create_read(None) == {
            "round": "ext-round",
            "kept": "base-kept",
        }
        assert extended.create_flows(None) == {"claim": "flow"}
        # base stack untouched
        assert stack.addresses == {"treasury": "t1"}
        assert stack.create_flows is None

    def test_requires_read_arg_counts_with_create_read(self):
        with pytest.raises(ValueError, match="read_arg_counts"):
            extend_stack(StackDef(name="s"), create_read=lambda client: {})


class TestApplyConnectedStackExtensions:
    def test_exposes_namespaces_flows_and_read_on_the_client(self):
        class Client:
            pass

        client = Client()
        flow = flow_operation(lambda **_: None)
        stack = extend_stack(
            StackDef(name="s"),
            addresses={"treasury": "t1"},
            constants={"fee": 1},
            defaults={"slippage": 5},
            math={"quote": "fn"},
            read_arg_counts={"round": 1},
            create_read=lambda c: {"round": (lambda round_id: ("read", c, round_id))},
            create_flows=lambda c: {"claim": flow},
        )
        returned = apply_connected_stack_extensions(client, stack)
        assert returned is client
        assert client.addresses.treasury == "t1"
        assert client.constants.fee == 1
        assert client.defaults.slippage == 5
        assert client.math.quote == "fn"
        assert client.flows.claim is flow
        assert client.read.round(42) == ("read", client, 42)

    def test_does_not_override_existing_client_fields(self):
        class Client:
            addresses = "existing"

        client = Client()
        stack = extend_stack(StackDef(name="s"), addresses={"a": 1})
        apply_connected_stack_extensions(client, stack)
        assert client.addresses == "existing"

    def test_no_extensions_is_a_no_op(self):
        class Client:
            pass

        client = Client()
        apply_connected_stack_extensions(client, StackDef(name="s"))
        assert not hasattr(client, "flows")
        assert not hasattr(client, "read")


class TestConnectedProgramWithExtensions:
    def test_extended_program_operations_surface_on_connected_program(self):
        from arete.instructions import encode_base58

        def base_factory(context):
            return {"instructions": {"base_op": instruction_operation(lambda **_: None)}}

        def extension_factory(context):
            return {"flows": {"ext_flow": flow_operation(lambda **_: None)}}

        program_def = extend_program(
            make_program(
                program_id=encode_base58(bytes([9] * 32)),
                create_operations=base_factory,
            ),
            create_operations=extension_factory,
        )

        class FakeClient:
            wallet = None
            chain = None

        class NullTransport:
            async def read(self, request):
                return None

        connected = ConnectedProgram("ore", program_def, FakeClient(), NullTransport())
        assert connected.instructions.base_op.kind == "instruction"
        assert connected.flows.ext_flow.kind == "flow"
