"""Tests for arete.extensions (ported from stack-extensions.test.ts):
namespace deep-merge, factory composition, and connected-client surfacing."""

from __future__ import annotations

import pytest

from arete.extensions import (
    PROGRAM_EXTENSION_KEYS,
    STACK_EXTENSION_KEYS,
    apply_connected_stack_extensions,
    extend_program,
    extend_programs,
    extend_stack,
    merge_namespace,
    program_extensions_of,
    stack_extensions_of,
    with_program_identity,
)
from arete.stack import (
    ConnectedProgram,
    ProgramDef,
    ProgramOperations,
    StackDef,
    flow_operation,
    instruction_operation,
    normalize_program_operations,
    same_program,
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

    def test_drops_package_release_identity_until_stamped_back(self):
        program = make_program(package_release_hash="pkg:ore@1", program_spec_hash="spec")
        extended = extend_program(program, constants={"fee": 2})
        # A program extended outside its generated SDK is no longer provably it.
        assert extended.package_release_hash is None
        assert extended.program_spec_hash == "spec"
        assert not same_program(extended, program)
        assert program.package_release_hash == "pkg:ore@1"
        # Untargeted programs keep theirs.
        programs = extend_programs(
            {"ore": program, "spl": program}, {"ore": {"constants": {"fee": 3}}}
        )
        assert programs["ore"].package_release_hash is None
        assert programs["spl"] is program
        # Generated packages stamp identity back after their own extension.
        stamped = with_program_identity(extended, package_release_hash="pkg:ore@1")
        assert same_program(stamped, program)
        assert stamped.constants == {"fee": 2}
        assert with_program_identity(stamped, package_release_hash=None).package_release_hash is None

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


class TestProgramRead:
    def test_composes_read_factories_base_first(self):
        seen = {}

        def base_read(context):
            return {"board": "base-board", "round": "base-round"}

        def extension_read(context):
            # The base read is on the connected program before the extension
            # factory runs (TS extendProgram).
            seen["base_round"] = context.program.read.round
            return {"round": "ext-round", "miner": "ext-miner"}

        extended = extend_program(
            extend_program(make_program(), create_read=base_read),
            create_read=extension_read,
        )

        class Program:
            key = "ore"

        class Context:
            program = Program()

        assert extended.create_read(Context()) == {
            "board": "base-board",
            "round": "ext-round",
            "miner": "ext-miner",
        }
        assert seen == {"base_round": "base-round"}

    def test_extension_only_read_factory(self):
        extended = extend_program(
            make_program(), create_read=lambda context: {"board": "b"}
        )
        assert extended.create_read(None) == {"board": "b"}
        assert make_program().create_read is None

    def test_read_surfaces_on_connected_program_before_operations(self):
        from arete.instructions import encode_base58

        observed = {}

        async def board_state(address=None):
            return {"address": address}

        def read_factory(context):
            observed["wallet"] = context.wallet
            return {"board_state": board_state}

        def operations_factory(context):
            observed["read"] = context.program.read.board_state
            return {"instructions": {"deploy": instruction_operation(lambda **_: None)}}

        program_def = extend_program(
            make_program(program_id=encode_base58(bytes([9] * 32))),
            create_read=read_factory,
            create_operations=operations_factory,
        )

        class FakeClient:
            wallet = "wallet"
            chain = None

        class NullTransport:
            async def read(self, request):
                return None

        connected = ConnectedProgram("ore", program_def, FakeClient(), NullTransport())
        assert connected.read.board_state is board_state
        assert observed == {"wallet": "wallet", "read": board_state}
        assert connected.instructions.deploy.kind == "instruction"

    def test_program_without_read_has_an_empty_read_namespace(self):
        from arete.instructions import encode_base58

        class FakeClient:
            wallet = None
            chain = None

        class NullTransport:
            async def read(self, request):
                return None

        connected = ConnectedProgram(
            "ore",
            make_program(program_id=encode_base58(bytes([9] * 32))),
            FakeClient(),
            NullTransport(),
        )
        assert len(connected.read) == 0
        with pytest.raises(AttributeError):
            connected.read.board_state


class TestBundleExports:
    def _module(self, name, **attrs):
        import types

        module = types.ModuleType(name)
        for key, value in attrs.items():
            setattr(module, key, value)
        return module

    def test_program_extensions_of_reads_the_mapping(self):
        module = self._module(
            "ore_program.extensions",
            PROGRAM_EXTENSIONS={"constants": {"ore_decimals": 11}},
        )
        assert program_extensions_of(module) == {"constants": {"ore_decimals": 11}}
        extended = extend_program(make_program(), **program_extensions_of(module))
        assert extended.constants == {"ore_decimals": 11}

    def test_missing_export_is_a_clear_import_error(self):
        module = self._module("ore_program.extensions")
        with pytest.raises(ImportError, match="must export PROGRAM_EXTENSIONS"):
            program_extensions_of(module)
        with pytest.raises(ImportError, match="must export STACK_EXTENSIONS"):
            stack_extensions_of(module)

    def test_unknown_keys_and_non_mappings_are_refused(self):
        with pytest.raises(TypeError, match="unknown key"):
            program_extensions_of(
                self._module("m", PROGRAM_EXTENSIONS={"create_flows": None})
            )
        with pytest.raises(TypeError, match="must be a mapping"):
            stack_extensions_of(self._module("m", STACK_EXTENSIONS=[1]))

    def test_keys_match_the_extension_helpers(self):
        import inspect

        assert set(PROGRAM_EXTENSION_KEYS) <= set(
            inspect.signature(extend_program).parameters
        )
        assert set(STACK_EXTENSION_KEYS) <= set(
            inspect.signature(extend_stack).parameters
        )


def test_extension_api_version_is_exported_and_recorded_in_pyproject():
    # Canonical §9 "Extension API contract": the package exports the version
    # and records the same value as `[tool.arete] extension-api`.
    import re
    from pathlib import Path

    import arete

    assert arete.EXTENSION_API_VERSION == 1
    pyproject = (Path(__file__).resolve().parents[1] / "pyproject.toml").read_text()
    section = pyproject.split("[tool.arete]", 1)[1]
    match = re.search(r"^extension-api\s*=\s*(\d+)\s*$", section, re.MULTILINE)
    assert match is not None
    assert int(match.group(1)) == arete.EXTENSION_API_VERSION
