"""Stack and program extensions.

Python port of ``typescript/core/src/stack-extensions.ts``: author-written
code attached to a stack (``read``, ``flows``, ``addresses``, ``constants``,
``defaults``, ``math``) or a program (semantic ``operations``), merged into
the generated binding data and surfaced on the connected client.

Extension namespaces deep-merge (later layers win per key, nested mappings
merge recursively); ``create_read`` / ``create_flows`` / ``create_operations``
factories compose (base runs first, extension result merges over it).
"""

from __future__ import annotations

import dataclasses
from typing import Any, Callable, Dict, Mapping, Optional

from arete.stack import (
    AttrNamespace,
    OperationNamespace,
    ProgramDef,
    ProgramOperationContext,
    ProgramOperations,
    StackDef,
    normalize_program_operations,
)

__all__ = [
    "merge_namespace",
    "merge_program_operations",
    "extend_program",
    "extend_programs",
    "extend_stack",
    "apply_connected_stack_extensions",
]


def merge_namespace(base: Any, extension: Any) -> Any:
    """Deep-merge two namespace values: mappings merge recursively, anything
    else is replaced by the extension value."""
    if isinstance(base, Mapping) and isinstance(extension, Mapping):
        merged: Dict[str, Any] = dict(base)
        for key, value in extension.items():
            merged[key] = merge_namespace(merged[key], value) if key in merged else value
        return merged
    return extension


def merge_program_operations(
    base: Optional[ProgramOperations], extension: Optional[ProgramOperations]
) -> ProgramOperations:
    base = base or ProgramOperations()
    extension = extension or ProgramOperations()
    return ProgramOperations(
        instructions=merge_namespace(base.instructions, extension.instructions),
        transactions=merge_namespace(base.transactions, extension.transactions),
        flows=merge_namespace(base.flows, extension.flows),
    )


_PROGRAM_NAMESPACE_KEYS = (
    "pdas",
    "accounts",
    "queries",
    "addresses",
    "constants",
    "defaults",
    "math",
)


def extend_program(
    program: ProgramDef,
    *,
    raw: Optional[Mapping[str, Any]] = None,
    pdas: Optional[Mapping[str, Any]] = None,
    accounts: Optional[Mapping[str, Any]] = None,
    queries: Optional[Mapping[str, Any]] = None,
    addresses: Optional[Mapping[str, Any]] = None,
    constants: Optional[Mapping[str, Any]] = None,
    defaults: Optional[Mapping[str, Any]] = None,
    math: Optional[Mapping[str, Any]] = None,
    create_operations: Optional[
        Callable[[ProgramOperationContext], Any]
    ] = None,
) -> ProgramDef:
    """A copy of ``program`` with extension namespaces merged in and operation
    factories composed (base first, extension merged over it).

    The extended definition drops ``sdk_definition_hash`` — it no longer
    byte-matches the generated artifact.
    """
    updates: Dict[str, Any] = {"sdk_definition_hash": None}
    provided = {
        "pdas": pdas,
        "accounts": accounts,
        "queries": queries,
        "addresses": addresses,
        "constants": constants,
        "defaults": defaults,
        "math": math,
    }
    for key in _PROGRAM_NAMESPACE_KEYS:
        value = provided[key]
        if value is not None:
            updates[key] = merge_namespace(getattr(program, key), value)
    if raw is not None:
        updates["raw_instructions"] = merge_namespace(program.raw_instructions, raw)

    base_factory = program.create_operations
    extension_factory = create_operations
    if base_factory is not None or extension_factory is not None:

        def composed(context: ProgramOperationContext) -> ProgramOperations:
            base_operations = (
                normalize_program_operations(base_factory(context))
                if base_factory is not None
                else None
            )
            extension_operations = (
                normalize_program_operations(extension_factory(context))
                if extension_factory is not None
                else None
            )
            return merge_program_operations(base_operations, extension_operations)

        updates["create_operations"] = composed

    return dataclasses.replace(program, **updates)


def extend_programs(
    programs: Mapping[str, ProgramDef],
    extensions: Mapping[str, Mapping[str, Any]],
) -> Dict[str, ProgramDef]:
    """Extend only the targeted program entries; extension values are
    :func:`extend_program` keyword mappings."""
    unknown = set(extensions) - set(programs)
    if unknown:
        raise ValueError(
            "extend_programs got extensions for unknown program(s): "
            + ", ".join(sorted(unknown))
        )
    return {
        name: extend_program(program, **extensions[name])
        if name in extensions
        else program
        for name, program in programs.items()
    }


def extend_stack(
    stack: StackDef,
    *,
    addresses: Optional[Mapping[str, Any]] = None,
    constants: Optional[Mapping[str, Any]] = None,
    defaults: Optional[Mapping[str, Any]] = None,
    math: Optional[Mapping[str, Any]] = None,
    read_arg_counts: Optional[Mapping[str, Any]] = None,
    create_read: Optional[Callable[[Any], Mapping[str, Any]]] = None,
    create_flows: Optional[Callable[[Any], Mapping[str, Any]]] = None,
) -> StackDef:
    """A copy of ``stack`` with extension namespaces merged in and connected
    factories (``create_read`` / ``create_flows``) composed.

    ``create_read`` requires ``read_arg_counts`` metadata for every read
    (mirroring the TS type-level requirement at runtime).
    """
    if create_read is not None and read_arg_counts is None:
        raise ValueError(
            "extend_stack requires read_arg_counts when create_read is provided"
        )
    updates: Dict[str, Any] = {}
    provided = {
        "addresses": addresses,
        "constants": constants,
        "defaults": defaults,
        "math": math,
    }
    for key, value in provided.items():
        if value is not None:
            updates[key] = merge_namespace(getattr(stack, key), value)

    if read_arg_counts is not None:
        updates["read_arg_counts"] = (
            merge_namespace(stack.read_arg_counts, read_arg_counts)
            if stack.read_arg_counts
            else dict(read_arg_counts)
        )

    def compose(
        base: Optional[Callable[[Any], Mapping[str, Any]]],
        extension: Optional[Callable[[Any], Mapping[str, Any]]],
    ) -> Optional[Callable[[Any], Mapping[str, Any]]]:
        if base is None:
            return extension
        if extension is None:
            return base

        def composed(client: Any) -> Mapping[str, Any]:
            return merge_namespace(base(client), extension(client))

        return composed

    if create_read is not None:
        updates["create_read"] = compose(stack.create_read, create_read)
    if create_flows is not None:
        updates["create_flows"] = compose(stack.create_flows, create_flows)

    return dataclasses.replace(stack, **updates)


def _define_client_field(client: Any, name: str, value: Any) -> None:
    if value is None:
        return
    existing = getattr(client, name, None)
    if existing is not None:
        return
    setattr(client, name, value)


def apply_connected_stack_extensions(client: Any, stack: StackDef) -> Any:
    """Surface the stack's extension namespaces on the connected client:
    ``addresses`` / ``constants`` / ``defaults`` / ``math`` as attribute
    namespaces, plus ``flows`` (operation namespace) and ``read`` built by the
    connected factories."""
    for key in ("addresses", "constants", "defaults", "math"):
        entries = getattr(stack, key)
        if entries:
            _define_client_field(client, key, AttrNamespace(key, entries))
    if stack.create_flows is not None:
        _define_client_field(
            client, "flows", OperationNamespace("flows", stack.create_flows(client))
        )
    if stack.create_read is not None:
        _define_client_field(
            client, "read", AttrNamespace("read", stack.create_read(client))
        )
    return client
