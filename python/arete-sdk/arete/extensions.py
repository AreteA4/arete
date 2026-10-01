"""Stack and program extensions.

Python port of ``typescript/core/src/stack-extensions.ts``: author-written
code attached to a stack (``read``, ``flows``, ``addresses``, ``constants``,
``defaults``, ``math``) or a program (``read`` and semantic ``operations``),
merged into the generated binding data and surfaced on the connected client.

Extension namespaces deep-merge (later layers win per key, nested mappings
merge recursively); ``create_read`` / ``create_flows`` / ``create_operations``
factories compose (base runs first, extension result merges over it).

Extension bundles (``docs/internal/sdk-core-api.md`` §9) export one mapping
from their entry module: ``PROGRAM_EXTENSIONS`` (keys
:data:`PROGRAM_EXTENSION_KEYS`) or ``STACK_EXTENSIONS`` (keys
:data:`STACK_EXTENSION_KEYS`). Generated packages read it with
:func:`program_extensions_of` / :func:`stack_extensions_of` and apply it with
:func:`extend_program` / :func:`extend_stack`.
"""

from __future__ import annotations

import dataclasses
from types import ModuleType
from typing import Any, Callable, Dict, Mapping, Optional, Tuple

from arete.stack import (
    AttrNamespace,
    OperationNamespace,
    ProgramDef,
    ProgramOperationContext,
    ProgramOperations,
    StackDef,
    normalize_program_operations,
)

#: Version of the extension-authoring surface (canonical §9 "Extension API
#: contract"): the extension helpers in this module, program read attachment,
#: and the instruction helpers generated code imports. Bumped only on a
#: breaking change; recorded as ``[tool.arete] extension-api`` in
#: ``pyproject.toml``. The TypeScript and Rust SDKs export the same value.
EXTENSION_API_VERSION = 1

#: Keys a program bundle's ``PROGRAM_EXTENSIONS`` mapping may carry: the
#: :func:`extend_program` keyword arguments a bundle provides.
PROGRAM_EXTENSION_KEYS: Tuple[str, ...] = (
    "addresses",
    "constants",
    "defaults",
    "math",
    "pdas",
    "create_operations",
    "create_read",
)

#: Keys a stack bundle's ``STACK_EXTENSIONS`` mapping may carry: the
#: :func:`extend_stack` keyword arguments.
STACK_EXTENSION_KEYS: Tuple[str, ...] = (
    "addresses",
    "constants",
    "defaults",
    "math",
    "read_arg_counts",
    "create_read",
    "create_flows",
)

__all__ = [
    "EXTENSION_API_VERSION",
    "PROGRAM_EXTENSION_KEYS",
    "STACK_EXTENSION_KEYS",
    "merge_namespace",
    "merge_program_operations",
    "extend_program",
    "extend_programs",
    "extend_stack",
    "program_extensions_of",
    "stack_extensions_of",
    "with_program_identity",
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
    create_read: Optional[
        Callable[[ProgramOperationContext], Mapping[str, Any]]
    ] = None,
) -> ProgramDef:
    """A copy of ``program`` with extension namespaces merged in and the
    ``create_read`` / ``create_operations`` factories composed (base first,
    extension merged over it; the base result is on the connected program
    before the extension factory runs, as in TS ``extendProgram``).

    The extended definition drops ``sdk_definition_hash`` — it no longer
    byte-matches the generated artifact — and ``package_release_hash``: a
    program extended outside its generated SDK is no longer provably that SDK
    (canonical §9). A generated package stamps identity back with
    :func:`with_program_identity` after applying its own extension.
    """
    updates: Dict[str, Any] = {"sdk_definition_hash": None, "package_release_hash": None}
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
            if base_operations is not None:
                connected = getattr(context, "program", None)
                _merge_connected(connected, "instructions", base_operations.instructions)
                _merge_connected(connected, "transactions", base_operations.transactions)
                _merge_connected(connected, "flows", base_operations.flows)
            extension_operations = (
                normalize_program_operations(extension_factory(context))
                if extension_factory is not None
                else None
            )
            return merge_program_operations(base_operations, extension_operations)

        updates["create_operations"] = composed

    base_read_factory = program.create_read
    extension_read_factory = create_read
    if base_read_factory is not None or extension_read_factory is not None:

        def composed_read(context: ProgramOperationContext) -> Dict[str, Any]:
            base_read = (
                base_read_factory(context) if base_read_factory is not None else None
            )
            if base_read:
                _merge_connected(getattr(context, "program", None), "read", base_read)
            extension_read = (
                extension_read_factory(context)
                if extension_read_factory is not None
                else None
            )
            return merge_namespace(dict(base_read or {}), extension_read or {})

        updates["create_read"] = composed_read

    return dataclasses.replace(program, **updates)


def _merge_connected(program: Any, key: str, entries: Mapping[str, Any]) -> None:
    """Merge a base factory's result into a connected program's namespace, so
    the extension factory composed over it sees it (TS ``extendProgram``)."""
    if program is None or not entries:
        return
    existing = getattr(program, key, None)
    if isinstance(existing, AttrNamespace):
        label = existing._label
        base = dict(existing._entries)
        namespace_type = type(existing)
    else:
        label = f"programs.{getattr(program, 'key', '?')}.{key}"
        base = {}
        namespace_type = AttrNamespace if key == "read" else OperationNamespace
    try:
        setattr(program, key, namespace_type(label, merge_namespace(base, entries)))
    except AttributeError:
        pass


def _bundle_extensions(
    module: ModuleType, export: str, allowed: Tuple[str, ...], kind: str
) -> Dict[str, Any]:
    name = getattr(module, "__name__", repr(module))
    try:
        mapping = getattr(module, export)
    except AttributeError:
        raise ImportError(
            f"{kind} extension bundle entry '{name}' must export "
            f"{export} = {{...}} with any of the keys {', '.join(allowed)}"
        ) from None
    if not isinstance(mapping, Mapping):
        raise TypeError(
            f"{name}.{export} must be a mapping, got {type(mapping).__name__}"
        )
    unknown = sorted(set(mapping) - set(allowed))
    if unknown:
        raise TypeError(
            f"{name}.{export} has unknown key(s) {', '.join(unknown)} "
            f"(allowed: {', '.join(allowed)})"
        )
    return dict(mapping)


def program_extensions_of(module: ModuleType) -> Dict[str, Any]:
    """The ``PROGRAM_EXTENSIONS`` mapping a program bundle's entry exports,
    as :func:`extend_program` keyword arguments.

    Raises :class:`ImportError` when the entry does not export it and
    :class:`TypeError` when it is not a mapping of
    :data:`PROGRAM_EXTENSION_KEYS`.
    """
    return _bundle_extensions(module, "PROGRAM_EXTENSIONS", PROGRAM_EXTENSION_KEYS, "Program")


def stack_extensions_of(module: ModuleType) -> Dict[str, Any]:
    """The ``STACK_EXTENSIONS`` mapping a stack bundle's entry exports, as
    :func:`extend_stack` keyword arguments.

    Raises :class:`ImportError` when the entry does not export it and
    :class:`TypeError` when it is not a mapping of
    :data:`STACK_EXTENSION_KEYS`.
    """
    return _bundle_extensions(module, "STACK_EXTENSIONS", STACK_EXTENSION_KEYS, "Stack")


def with_program_identity(
    program: ProgramDef, *, package_release_hash: Optional[str]
) -> ProgramDef:
    """A copy of ``program`` carrying the identity of the program SDK it is
    (TS ``withProgramIdentity``).

    Generated program packages call this last, after their own extension, so
    the identity describes exactly the generated SDK; :func:`extend_program`
    drops it again. An empty or ``None`` release removes the identity.
    """
    return dataclasses.replace(
        program, package_release_hash=package_release_hash or None
    )


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
