//! Python stack SDK generation.
//!
//! Mirror of [`crate::rust`] emitting a Python package next to the TS/Rust
//! outputs (see `docs/internal/sdk-python-alignment.md` §4). The generated
//! package instantiates the `arete` runtime's binding model
//! (`arete.stack.StackDef` / `ProgramDef`, `arete.views.ViewDef`) as pure
//! data + pure functions:
//!
//! - `__init__.py` — `<STACK_NAME>_STACK: StackDef` binding + re-exports.
//! - `models.py` — entity dataclasses + `*_from_wire` / `*_patch_from_wire`
//!   converters (snake_case pass-through, u64 decimal strings → `int`,
//!   nested structs).
//! - `views.py` — typed view namespace classes + the `VIEWS` map consumed by
//!   `StackDef`.
//! - `programs.py` — per-program `<Ix>Params` TypedDicts, raw builders, raw
//!   `*_handler()` escape hatches, PDA factories, typed account read
//!   definitions, release identity consts + `*_read_descriptor()`, error
//!   metadata tables, and the `PROGRAMS` / `PROGRAM_READS` maps.

use crate::ast::*;
use crate::typescript_instructions::{
    dedupe_errors_by_code, disambiguate_instruction_account_names, normalize_seed_arg_type,
    split_generic,
};
use std::collections::{BTreeMap, HashMap, HashSet};

// ============================================================================
// Public output + config shapes (mirror of rust.rs)
// ============================================================================

#[derive(Debug, Clone)]
pub struct PythonOutput {
    /// Import package name (`ore_stack`) used by [`write_python_package`].
    pub module_name: String,
    pub pyproject_toml: String,
    pub init_py: String,
    pub models_py: String,
    pub views_py: String,
    /// Generated program SDK module (`programs.py`). `None` when the stack
    /// spec declares no instructions.
    pub programs_py: Option<String>,
}

/// Python output for a standalone program SDK. It intentionally omits the
/// live-only `views.py` module and any synthetic `StackDef` binding.
#[derive(Debug, Clone)]
pub struct PythonProgramOutput {
    pub module_name: String,
    pub pyproject_toml: String,
    pub init_py: String,
    pub models_py: String,
    pub programs_py: String,
}

#[derive(Debug, Clone)]
pub struct PythonStackConfig {
    /// Distribution name used in the generated `pyproject.toml`; the import
    /// package name is derived via [`python_module_name`].
    pub package_name: String,
    pub sdk_version: String,
    /// Kept for CLI shape parity with `RustStackConfig`. Python relative
    /// imports are identical in module and package layouts, so this does not
    /// change the generated sources — only which writer the CLI picks.
    pub module_mode: bool,
    /// WebSocket URL for the stack. If None, generates a placeholder comment.
    pub url: Option<String>,
    /// HTTP base URL for the stack (account reads / queries / chain reads).
    pub http_url: Option<String>,
    /// Module stems of hand-authored devex extension files staged next to the
    /// generated output (one `from . import <stem>` each, in order, entry
    /// last). Stems are derived from staged file names via
    /// [`python_module_name`].
    pub extension_modules: Vec<String>,
    /// Module stem of the extension entry file. When set, the generated
    /// `__init__.py` star-imports the entry (`from .<entry> import *`) so
    /// extension helpers come into scope with the stack's own import.
    pub extension_entry: Option<String>,
    /// Published-platform program read overrides keyed by program ID. Mirrors
    /// [`RustStackConfig::program_reads`](crate::rust::RustStackConfig): when
    /// a matching entry exists, its exact `program_spec_hash` /
    /// `program_release_hash` drive the program read layer instead of the
    /// OSS-derived release identity. Transport remains local-http.
    pub program_reads: Vec<PythonProgramReadConfig>,
    /// Managed-hosting transports. Local generation leaves this unset.
    pub gateway: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct PythonProgramReadConfig {
    pub program_id: String,
    pub program_spec_hash: String,
    pub program_release_hash: String,
    /// Exact wire descriptor for a published hosted binding. `None` keeps
    /// the local-HTTP descriptor used by locally generated stack SDKs.
    pub descriptor: Option<serde_json::Value>,
}

impl Default for PythonStackConfig {
    fn default() -> Self {
        Self {
            package_name: "generated-stack".to_string(),
            sdk_version: "0.4".to_string(),
            module_mode: false,
            url: None,
            http_url: None,
            extension_modules: Vec::new(),
            extension_entry: None,
            program_reads: Vec::new(),
            gateway: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PythonCompositionConfig {
    pub stack: PythonStackConfig,
    pub live_urls: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PythonAliasedStackOutput {
    pub alias: String,
    pub module_name: String,
    pub output: PythonOutput,
}

#[derive(Debug, Clone)]
pub struct PythonCompositionOutput {
    pub name: String,
    /// Root import package name for [`write_python_composition_package`].
    pub module_name: String,
    pub pyproject_toml: String,
    pub init_py: String,
    pub live_stacks: Vec<PythonAliasedStackOutput>,
}

// ============================================================================
// Entry points
// ============================================================================

/// Compile a full SerializableStackSpec (multi-entity) into a Python package.
pub fn compile_stack_spec(
    stack_spec: SerializableStackSpec,
    config: Option<PythonStackConfig>,
) -> Result<PythonOutput, String> {
    compile_stack_spec_with_view_selection(stack_spec, config, false)
}

/// Compile a stack model whose `views` have already been projected by a
/// StackManifest selected-view allowlist.
pub fn compile_stack_spec_with_exact_views(
    stack_spec: SerializableStackSpec,
    config: Option<PythonStackConfig>,
) -> Result<PythonOutput, String> {
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Compile only the portable program surface. The result exposes `PROGRAMS`
/// and `PROGRAM_READS` for `arete.create_session(...)` / `with_programs(...)`
/// and contains no view module or synthetic stack definition.
pub fn compile_program_modules(
    stack_spec: SerializableStackSpec,
    config: Option<PythonStackConfig>,
) -> Result<PythonProgramOutput, String> {
    let config = config.unwrap_or_default();
    if stack_spec.idls.is_empty() {
        return Err(format!(
            "Stack '{}' carries no IDLs; a program-only SDK has nothing to emit",
            stack_spec.stack_name
        ));
    }

    let entity_names = stack_spec
        .entities
        .iter()
        .map(|entity| entity.state_name.clone())
        .collect::<Vec<_>>();
    let (models_py, _model_exports, account_structs) =
        generate_stack_models_py(&stack_spec.stack_name, &stack_spec.entities, &entity_names);
    let programs = generate_stack_programs_py(
        &stack_spec.stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &stack_spec.program_specs,
        &account_structs,
        &config.program_reads,
        config.gateway.as_ref(),
        true,
    )
    .ok_or_else(|| {
        format!(
            "Stack '{}' contains no programs to emit",
            stack_spec.stack_name
        )
    })?;
    validate_extension_modules(&config, true)?;

    Ok(PythonProgramOutput {
        module_name: python_module_name(&config.package_name),
        pyproject_toml: generate_stack_pyproject(&config),
        init_py: generate_program_init_py(&stack_spec.stack_name, &config),
        models_py,
        programs_py: programs.code,
    })
}

/// Compile Python output from an explicit StackManifest and its public
/// dependencies.
pub fn compile_public_artifacts(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifact,
    manifest: &arete_artifacts::StackManifestArtifact,
    config: Option<PythonStackConfig>,
) -> Result<PythonOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts(programs, live_spec, manifest)?;
    compile_stack_spec(stack_spec, config)
}

/// Compile typed V2 public artifacts through the current single-live generator.
pub fn compile_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_spec: &arete_artifacts::LiveSpecArtifactV2,
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<PythonStackConfig>,
) -> Result<PythonOutput, String> {
    let stack_spec =
        crate::public_artifacts::stack_spec_from_artifacts_v2(programs, live_spec, manifest)?;
    compile_stack_spec_with_view_selection(stack_spec, config, true)
}

/// Generate one namespaced Python stack package per live alias plus a root
/// `__init__.py` that preserves alias boundaries instead of flattening
/// views/adapters.
pub fn compile_composed_public_artifacts_v2(
    programs: &[arete_artifacts::ProgramSpecArtifact],
    live_specs: &[(String, arete_artifacts::LiveSpecArtifactV2)],
    manifest: &arete_artifacts::StackManifestArtifactV2,
    config: Option<PythonCompositionConfig>,
) -> Result<PythonCompositionOutput, String> {
    let composed =
        crate::public_artifacts::stack_specs_from_artifacts_v2(programs, live_specs, manifest)?;
    if composed.live_specs.is_empty() {
        return Err(
            "Python composition generation requires at least one aliased LiveSpec".to_string(),
        );
    }
    let config = config.unwrap_or_default();
    if !config.stack.extension_modules.is_empty() || config.stack.extension_entry.is_some() {
        return Err(
            "Python composition SDKs do not support stack extensions; extensions attach to a single-live stack module".to_string(),
        );
    }
    let mut live_stacks = Vec::with_capacity(composed.live_specs.len());
    for live in composed.live_specs {
        let module_name = python_module_name(&live.alias);
        let mut live_config = config.stack.clone();
        live_config.module_mode = true;
        live_config.url = config.live_urls.get(&live.alias).cloned();
        let mut output =
            compile_stack_spec_with_view_selection(live.stack_spec, Some(live_config), true)?;
        output.module_name = module_name.clone();
        live_stacks.push(PythonAliasedStackOutput {
            alias: live.alias,
            module_name,
            output,
        });
    }
    let mut init_py = format!(
        "\"\"\"Generated Arete composition binding for `{}`. Do not edit.\"\"\"\n\n",
        composed.name
    );
    for live in &live_stacks {
        init_py.push_str(&format!(
            "from . import {}  # noqa: F401\n",
            live.module_name
        ));
    }
    init_py.push_str(&format!(
        "\n__all__ = [{}]\n",
        live_stacks
            .iter()
            .map(|live| py_string_literal(&live.module_name))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(PythonCompositionOutput {
        name: composed.name,
        module_name: python_module_name(&config.stack.package_name),
        pyproject_toml: generate_stack_pyproject(&config.stack),
        init_py,
        live_stacks,
    })
}

/// Write a plain source package directory (dropped into a user's project):
/// `__init__.py`, `models.py`, `views.py`, and `programs.py` when generated.
pub fn write_python_module(
    output: &PythonOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("__init__.py"), &output.init_py)?;
    std::fs::write(module_dir.join("models.py"), &output.models_py)?;
    std::fs::write(module_dir.join("views.py"), &output.views_py)?;
    if let Some(programs) = &output.programs_py {
        std::fs::write(module_dir.join("programs.py"), programs)?;
    }
    Ok(())
}

/// Write a pip-installable layout: `pyproject.toml` plus the import package
/// directory (mirror of `write_rust_crate`'s packaging wrapper).
pub fn write_python_package(
    output: &PythonOutput,
    package_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(package_dir)?;
    std::fs::write(package_dir.join("pyproject.toml"), &output.pyproject_toml)?;
    write_python_module(output, &package_dir.join(&output.module_name))
}

pub fn write_python_program_module(
    output: &PythonProgramOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("__init__.py"), &output.init_py)?;
    std::fs::write(module_dir.join("models.py"), &output.models_py)?;
    std::fs::write(module_dir.join("programs.py"), &output.programs_py)?;
    Ok(())
}

pub fn write_python_program_package(
    output: &PythonProgramOutput,
    package_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(package_dir)?;
    std::fs::write(package_dir.join("pyproject.toml"), &output.pyproject_toml)?;
    write_python_program_module(output, &package_dir.join(&output.module_name))
}

pub fn write_python_composition_module(
    output: &PythonCompositionOutput,
    module_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(module_dir)?;
    std::fs::write(module_dir.join("__init__.py"), &output.init_py)?;
    for live in &output.live_stacks {
        write_python_module(&live.output, &module_dir.join(&live.module_name))?;
    }
    Ok(())
}

pub fn write_python_composition_package(
    output: &PythonCompositionOutput,
    package_dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(package_dir)?;
    std::fs::write(package_dir.join("pyproject.toml"), &output.pyproject_toml)?;
    write_python_composition_module(output, &package_dir.join(&output.module_name))
}

fn compile_stack_spec_with_view_selection(
    stack_spec: SerializableStackSpec,
    config: Option<PythonStackConfig>,
    exact_views: bool,
) -> Result<PythonOutput, String> {
    let config = config.unwrap_or_default();
    let stack_name = stack_spec.stack_name.clone();
    let stack_kebab = to_kebab_case(&stack_name);

    let mut entity_names: Vec<String> = Vec::new();
    let mut entity_specs: Vec<SerializableStreamSpec> = Vec::new();
    for spec in stack_spec.entities {
        entity_names.push(spec.state_name.clone());
        entity_specs.push(spec);
    }

    let (models_py, model_exports, account_structs) =
        generate_stack_models_py(&stack_name, &entity_specs, &entity_names);

    let programs = generate_stack_programs_py(
        &stack_name,
        &stack_spec.instructions,
        &stack_spec.idls,
        &stack_spec.pdas,
        &stack_spec.program_ids,
        &stack_spec.program_specs,
        &account_structs,
        &config.program_reads,
        config.gateway.as_ref(),
        false,
    );

    let views_py = generate_stack_views_py(&stack_name, &entity_specs, &entity_names, exact_views);

    validate_extension_modules(&config, programs.is_some())?;

    let init_py = generate_stack_init_py(
        &stack_name,
        &stack_kebab,
        &config,
        programs.is_some(),
        &model_exports,
    );
    let pyproject_toml = generate_stack_pyproject(&config);

    Ok(PythonOutput {
        module_name: python_module_name(&config.package_name),
        pyproject_toml,
        init_py,
        models_py,
        views_py,
        programs_py: programs.map(|codegen| codegen.code),
    })
}

// ============================================================================
// pyproject.toml + __init__.py
// ============================================================================

fn generate_stack_pyproject(config: &PythonStackConfig) -> String {
    format!(
        r#"[project]
name = "{name}"
version = "0.1.0"
requires-python = ">=3.9"
dependencies = ["arete-sdk>={sdk}"]

[build-system]
requires = ["setuptools>=61.0"]
build-backend = "setuptools.build_meta"

[tool.setuptools.packages.find]
include = ["{module}*"]
"#,
        name = config.package_name,
        sdk = config.sdk_version,
        module = python_module_name(&config.package_name),
    )
}

/// Validate hand-authored extension module stems against the generated
/// module names. Entry-stem collisions are a hard error because the staged
/// file would shadow (or be shadowed by) a generated file.
fn validate_extension_modules(
    config: &PythonStackConfig,
    has_programs: bool,
) -> Result<(), String> {
    if config.extension_modules.is_empty() && config.extension_entry.is_none() {
        return Ok(());
    }
    if config.extension_entry.is_none() {
        return Err("extension modules were configured without an extension entry".to_string());
    }
    let mut seen = HashSet::new();
    for stem in &config.extension_modules {
        let reserved = matches!(stem.as_str(), "models" | "views" | "__init__")
            || (stem == "programs" && has_programs);
        if reserved {
            return Err(format!(
                "extension file '{stem}.py' collides with the generated '{stem}' module; rename the extension file"
            ));
        }
        if !seen.insert(stem.as_str()) {
            return Err(format!(
                "extension file '{stem}.py' resolves to the same module name as another staged extension file"
            ));
        }
    }
    match &config.extension_entry {
        Some(entry) if config.extension_modules.last() == Some(entry) => Ok(()),
        Some(entry) => Err(format!(
            "extension entry module '{entry}' must be the last configured extension module"
        )),
        None => unreachable!("checked above"),
    }
}

fn generate_stack_init_py(
    stack_name: &str,
    stack_kebab: &str,
    config: &PythonStackConfig,
    has_programs: bool,
    model_exports: &[String],
) -> String {
    let _ = model_exports;
    let stack_const = format!("{}_STACK", to_screaming_snake(stack_name));

    let submodules = if has_programs {
        "models, programs, views"
    } else {
        "models, views"
    };
    let mut star_imports = String::new();
    star_imports.push_str("from .models import *  # noqa: F401,F403\n");
    if has_programs {
        star_imports.push_str("from .programs import *  # noqa: F401,F403\n");
    }
    star_imports.push_str("from .views import *  # noqa: F401,F403\n");

    let ws_line = match config.url.as_deref() {
        Some(url) if !url.is_empty() => format!("        ws={},", py_string_literal(url)),
        _ => "        ws=\"\",  # TODO: Set URL after first deployment in arete.toml".to_string(),
    };
    let http_line = match config.http_url.as_deref() {
        Some(http_url) if !http_url.is_empty() => {
            format!("\n        http={},", py_string_literal(http_url))
        }
        _ => String::new(),
    };

    let programs_kwargs = if has_programs {
        "\n    programs=programs.PROGRAMS,\n    program_reads=programs.PROGRAM_READS,"
    } else {
        ""
    };
    let (gateway_imports, gateway_kwarg) = match config.gateway.as_ref() {
        Some(gateway) => {
            let json = serde_json::to_string(gateway).expect("gateway descriptor must serialize");
            (
                "import json\nfrom arete.gateway import HostedSolanaGatewayBindings\n",
                format!(
                    "\n    gateway=HostedSolanaGatewayBindings.from_dict(json.loads({})),",
                    py_string_literal(&json)
                ),
            )
        }
        None => ("", String::new()),
    };

    let mut all_items = vec![py_string_literal(&stack_const)];
    all_items.push("*models.__all__".to_string());
    if has_programs {
        all_items.push("*programs.__all__".to_string());
    }
    all_items.push("*views.__all__".to_string());

    let mut output = format!(
        r#""""Generated Arete stack binding for `{stack_name}`.

Generated by the Arete interpreter (arete-sdk {sdk}); do not edit.
"""

from __future__ import annotations

{gateway_imports}
from arete.stack import StackDef, StackEndpoints

from . import {submodules}
{star_imports}
{stack_const}: StackDef = StackDef(
    name={kebab},
    endpoints=StackEndpoints(
{ws_line}{http_line}
    ),
    views=views.VIEWS,{programs_kwargs}{gateway_kwarg}
)

__all__ = [
    {all_items},
]
"#,
        stack_name = stack_name,
        sdk = config.sdk_version,
        submodules = submodules,
        star_imports = star_imports,
        stack_const = stack_const,
        kebab = py_string_literal(stack_kebab),
        ws_line = ws_line,
        http_line = http_line,
        programs_kwargs = programs_kwargs,
        gateway_imports = gateway_imports,
        gateway_kwarg = gateway_kwarg,
        all_items = all_items.join(",\n    "),
    );

    append_python_extension_imports(&mut output, config);

    output
}

fn generate_program_init_py(stack_name: &str, config: &PythonStackConfig) -> String {
    let mut output = format!(
        r#""""Generated standalone program SDK for `{stack_name}`.

Generated by the Arete interpreter (arete-sdk {sdk}); do not edit.
`PROGRAMS` and `PROGRAM_READS` compose directly with `arete.create_session`.
"""

from . import models, programs
from .models import *  # noqa: F401,F403
from .programs import *  # noqa: F401,F403

__all__ = [
    *models.__all__,
    *programs.__all__,
]
"#,
        sdk = config.sdk_version,
    );
    append_python_extension_imports(&mut output, config);
    output
}

fn append_python_extension_imports(output: &mut String, config: &PythonStackConfig) {
    if let Some(entry) = &config.extension_entry {
        output.push_str(
            "\n# Hand-authored devex extensions (staged from extensions.json; not generated).\n",
        );
        for stem in &config.extension_modules {
            if stem == entry {
                continue;
            }
            output.push_str(&format!("from . import {stem}  # noqa: F401\n"));
        }
        output.push_str(&format!("from .{entry} import *  # noqa: F401,F403\n"));
    }
}

// ============================================================================
// models.py — entity dataclasses + converters
// ============================================================================

/// How a field value converts from the wire.
#[derive(Debug, Clone)]
enum WireConversion {
    PassThrough,
    Int,
    IntList,
    Nested(String),
    NestedList(String),
    /// `CaptureWrapper` envelope around a nested struct converter.
    CaptureWrapped(String),
    CaptureWrappedList(String),
    /// `EventWrapper` envelope around a nested struct converter.
    EventWrapped(String),
    EventWrappedList(String),
}

impl WireConversion {
    fn render(&self, accessor: &str) -> String {
        match self {
            WireConversion::PassThrough => accessor.to_string(),
            WireConversion::Int => format!("_to_int({accessor})"),
            WireConversion::IntList => format!("_to_int_list({accessor})"),
            WireConversion::Nested(converter) => format!("_convert({accessor}, {converter})"),
            WireConversion::NestedList(converter) => {
                format!("_convert_list({accessor}, {converter})")
            }
            WireConversion::CaptureWrapped(converter) => {
                format!("_convert_capture({accessor}, {converter})")
            }
            WireConversion::CaptureWrappedList(converter) => {
                format!("_convert_capture_list({accessor}, {converter})")
            }
            WireConversion::EventWrapped(converter) => {
                format!("_convert_event({accessor}, {converter})")
            }
            WireConversion::EventWrappedList(converter) => {
                format!("_convert_event_list({accessor}, {converter})")
            }
        }
    }
}

/// Runtime envelope a resolved-struct field arrives in. Mirror of the
/// TypeScript generator's `EventWrapper<T>` / `CaptureWrapper<T>` selection in
/// `field_type_info_to_typescript`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperKind {
    None,
    Capture,
    Event,
}

#[derive(Debug, Clone)]
struct PyField {
    /// Python identifier (snake_case, keyword-escaped).
    name: String,
    /// Wire key (snake_case; `_mapping` normalizes camelCase inputs).
    wire: String,
    annotation: String,
    conversion: WireConversion,
    /// The spec marks this key as required (IDL struct fields that are not
    /// `Option<...>`). Only honoured by strict converters; entity-section
    /// fields are always projected loosely.
    required: bool,
}

fn py_base_annotation(base_type: &BaseType) -> &'static str {
    match base_type {
        BaseType::Integer | BaseType::Timestamp => "int",
        BaseType::Float => "float",
        BaseType::String => "str",
        BaseType::Boolean => "bool",
        BaseType::Binary => "List[int]",
        BaseType::Pubkey => "str",
        BaseType::Array => "List[Any]",
        BaseType::Object | BaseType::Any => "Any",
    }
}

fn wrap_optional(annotation: &str) -> String {
    if annotation == "Any" {
        "Any".to_string()
    } else {
        format!("Optional[{annotation}]")
    }
}

/// Map the element of a `Vec<T>` scalar array to its Python primitive. Mirror
/// of `typescript::typescript_scalar_array_element`; accepts both stored forms
/// of the inner type (`"Vec < f64 >"` and the bare `"f64"`).
fn py_scalar_array_element(inner_type: &str) -> Option<&'static str> {
    let trimmed = inner_type.trim();
    let element = trimmed
        .strip_prefix("Vec <")
        .and_then(|rest| rest.strip_suffix('>'))
        .or_else(|| {
            trimmed
                .strip_prefix("Vec<")
                .and_then(|rest| rest.strip_suffix('>'))
        })
        .map(str::trim)
        .unwrap_or(trimmed);
    match element {
        "f32" | "f64" => Some("float"),
        "bool" => Some("bool"),
        "String" | "&str" | "str" => Some("str"),
        _ => None,
    }
}

/// Annotation + wire conversion for a non-resolved (scalar / scalar-array)
/// field. Shared by the entity-section path and the IDL `ResolvedField` path so
/// the same on-chain array is typed identically whichever way it is reached.
///
/// `Vec<u64>`-shaped fields are stored as `BaseType::Array` with an explicit
/// `integer_kind`, so the integer check has to consult `integer_kind` and not
/// just `base_type` (mirror of `typescript_integer_type`).
fn py_scalar_field_shape(
    base_type: &BaseType,
    integer_kind: Option<IntegerKind>,
    is_array: bool,
    inner_type: Option<&str>,
) -> (String, WireConversion) {
    let integer_array = is_array && matches!(base_type, BaseType::Array) && integer_kind.is_some();
    if integer_array {
        return (wrap_optional("List[int]"), WireConversion::IntList);
    }
    if matches!(base_type, BaseType::Integer | BaseType::Timestamp) {
        return if is_array {
            (wrap_optional("List[int]"), WireConversion::IntList)
        } else {
            (wrap_optional("int"), WireConversion::Int)
        };
    }
    if is_array && matches!(base_type, BaseType::Array) {
        if let Some(element) = inner_type.and_then(py_scalar_array_element) {
            return (
                wrap_optional(&format!("List[{element}]")),
                WireConversion::PassThrough,
            );
        }
    }
    let base = py_base_annotation(base_type);
    let annotation = if is_array && !matches!(base_type, BaseType::Array) {
        wrap_optional(&format!("List[{base}]"))
    } else {
        wrap_optional(base)
    };
    (annotation, WireConversion::PassThrough)
}

/// Which runtime envelope a resolved-struct field arrives in. Mirror of the
/// TypeScript generator: `#[capture]`-fed account fields arrive as
/// `CaptureWrapper<T>` and event/instruction-list fields as `EventWrapper<T>`,
/// never as the bare struct.
fn wrapper_kind_for(
    field: &FieldTypeInfo,
    resolved: &ResolvedStructType,
    capture_fields: &HashSet<String>,
) -> WrapperKind {
    if resolved.is_event || (resolved.is_instruction && field.is_array) {
        return WrapperKind::Event;
    }
    if resolved.is_account
        && (capture_fields.contains(&field.field_name)
            || capture_fields.contains(field.raw_field_name()))
    {
        return WrapperKind::Capture;
    }
    WrapperKind::None
}

/// Build the field render info for one entity-section field.
fn py_field_for(
    field: &FieldTypeInfo,
    resolved_name_map: &HashMap<String, String>,
    capture_fields: &HashSet<String>,
) -> PyField {
    let name = to_snake_case(&field.field_name);
    let wire = name.clone();

    if let Some(resolved) = &field.resolved_type {
        let emitted = resolved_name_map
            .get(&resolved.type_name)
            .cloned()
            .unwrap_or_else(|| to_pascal_case(&resolved.type_name));
        if resolved.is_enum {
            let base = emitted;
            let annotation = if field.is_array {
                wrap_optional(&format!("List[{base}]"))
            } else {
                wrap_optional(&base)
            };
            return PyField {
                name,
                wire,
                annotation,
                conversion: WireConversion::PassThrough,
                required: false,
            };
        }
        let converter = format!("{}_from_wire", to_snake_case(&emitted));
        let wrapper = wrapper_kind_for(field, resolved, capture_fields);
        let element = match wrapper {
            WrapperKind::None => emitted,
            WrapperKind::Capture => format!("CaptureWrapper[{emitted}]"),
            WrapperKind::Event => format!("EventWrapper[{emitted}]"),
        };
        let annotation = if field.is_array {
            wrap_optional(&format!("List[{element}]"))
        } else {
            wrap_optional(&element)
        };
        let conversion = match (wrapper, field.is_array) {
            (WrapperKind::None, false) => WireConversion::Nested(converter),
            (WrapperKind::None, true) => WireConversion::NestedList(converter),
            (WrapperKind::Capture, false) => WireConversion::CaptureWrapped(converter),
            (WrapperKind::Capture, true) => WireConversion::CaptureWrappedList(converter),
            (WrapperKind::Event, false) => WireConversion::EventWrapped(converter),
            (WrapperKind::Event, true) => WireConversion::EventWrappedList(converter),
        };
        return PyField {
            name,
            wire,
            annotation,
            conversion,
            required: false,
        };
    }

    let (annotation, conversion) = py_scalar_field_shape(
        &field.base_type,
        field.effective_integer_kind(),
        field.is_array,
        field
            .inner_type
            .as_deref()
            .or(Some(field.rust_type_name.as_str())),
    );
    PyField {
        name,
        wire,
        annotation,
        conversion,
        required: false,
    }
}

fn py_field_for_resolved(field: &ResolvedField, name: String) -> PyField {
    let wire = resolved_field_wire_name(field);
    let (annotation, conversion) = py_scalar_field_shape(
        &field.base_type,
        field.effective_integer_kind(),
        field.is_array,
        Some(field.field_type.as_str()),
    );
    PyField {
        name,
        wire,
        annotation,
        conversion,
        required: !field.is_optional,
    }
}

/// Produce stable, distinct Python attribute names for resolved IDL fields.
///
/// The general snake-case helper intentionally collapses punctuation, which
/// also makes `padding_0` and `_padding_0` collide. Preserve leading
/// underscores when they distinguish otherwise-identical names, then use a
/// numeric suffix as a total fallback for any remaining normalization clash.
fn canonical_resolved_field_names(fields: &[ResolvedField]) -> Vec<String> {
    let base_names = fields
        .iter()
        .map(|field| to_snake_case(field.raw_field_name()))
        .collect::<Vec<_>>();
    let mut base_counts = BTreeMap::<String, usize>::new();
    for base_name in &base_names {
        *base_counts.entry(base_name.clone()).or_default() += 1;
    }

    let mut used_names = HashSet::new();
    fields
        .iter()
        .zip(base_names)
        .map(|(field, base_name)| {
            let leading_underscores = field
                .raw_field_name()
                .chars()
                .take_while(|character| *character == '_')
                .count();
            let preferred = if base_counts.get(&base_name).copied().unwrap_or_default() > 1
                && leading_underscores > 0
            {
                format!("{}{}", "_".repeat(leading_underscores), base_name)
            } else {
                base_name
            };

            if used_names.insert(preferred.clone()) {
                return preferred;
            }

            let mut suffix = 2;
            loop {
                let candidate = format!("{preferred}_{suffix}");
                if used_names.insert(candidate.clone()) {
                    return candidate;
                }
                suffix += 1;
            }
        })
        .collect()
}

/// Wire names remain snake_case but retain meaningful leading underscores.
fn resolved_field_wire_name(field: &ResolvedField) -> String {
    let raw_name = field.raw_field_name();
    let leading_underscores = raw_name
        .chars()
        .take_while(|character| *character == '_')
        .count();
    format!(
        "{}{}",
        "_".repeat(leading_underscores),
        to_snake_case(raw_name)
    )
}

fn render_dataclass(name: &str, doc: &str, fields: &[PyField]) -> String {
    let mut out = format!("@dataclass\nclass {name}:\n    \"\"\"{doc}\"\"\"\n");
    if !fields.is_empty() {
        out.push('\n');
        for field in fields {
            out.push_str(&format!(
                "    {}: {} = None\n",
                field.name, field.annotation
            ));
        }
    }
    out
}

/// Render a `*_from_wire` converter.
///
/// `strict` converters reject payloads that omit a key the spec marks required
/// (the IDL-derived struct path, mirroring the TypeScript `*Schema` which
/// renders those keys without `.optional()`), so `arete.read`'s
/// `SCHEMA_VALIDATION` guard is reachable instead of silently producing an
/// all-`None` object. Loose converters (entity sections/roots, and every
/// `*_patch_from_wire`) keep `data.get(...)` semantics.
fn render_from_wire(name: &str, fn_name: &str, fields: &[PyField], strict: bool) -> String {
    let doc = if strict {
        format!("Converts a wire payload into :class:`{name}`.\n\n    Raises ``ValueError`` when a required field is absent.")
    } else {
        format!("Converts a wire payload into :class:`{name}`.")
    };
    let mut out = format!("def {fn_name}(value: Any) -> {name}:\n    \"\"\"{doc}\"\"\"\n");
    if fields.is_empty() {
        out.push_str(&format!(
            "    _mapping(value, {})\n    return {name}()\n",
            py_string_literal(name)
        ));
        return out;
    }
    out.push_str(&format!(
        "    data = _mapping(value, {})\n    return {name}(\n",
        py_string_literal(name)
    ));
    for field in fields {
        let accessor = if strict && field.required {
            format!(
                "_require(data, {}, {})",
                py_string_literal(&field.wire),
                py_string_literal(name)
            )
        } else {
            format!("data.get({})", py_string_literal(&field.wire))
        };
        out.push_str(&format!(
            "        {}={},\n",
            field.name,
            field.conversion.render(&accessor)
        ));
    }
    out.push_str("    )\n");
    out
}

fn render_patch_from_wire(name: &str, fn_name: &str, fields: &[PyField]) -> String {
    let mut out = format!(
        "def {fn_name}(value: Any) -> Dict[str, Any]:\n    \"\"\"Converts a partial `{name}` patch; only present keys appear.\"\"\"\n"
    );
    out.push_str(&format!(
        "    data = _mapping(value, {})\n    out: Dict[str, Any] = {{}}\n",
        py_string_literal(&format!("{name} patch"))
    ));
    for field in fields {
        let accessor = format!("data[{}]", py_string_literal(&field.wire));
        out.push_str(&format!(
            "    if {} in data:\n        out[{}] = {}\n",
            py_string_literal(&field.wire),
            py_string_literal(&field.wire),
            field.conversion.render(&accessor)
        ));
    }
    out.push_str("    return out\n");
    out
}

/// Per-entity model naming: mirror of `RustCompiler::build_resolved_type_name_map`.
fn build_resolved_type_name_map(
    spec: &SerializableStreamSpec,
    entity_name: &str,
) -> HashMap<String, String> {
    let mut reserved_names = HashSet::from([
        entity_name.to_string(),
        "EventWrapper".to_string(),
        "CaptureWrapper".to_string(),
    ]);
    for section in &spec.sections {
        if !is_root_section(&section.name) && section.fields.iter().any(|field| field.emit) {
            reserved_names.insert(format!("{}{}", entity_name, to_pascal_case(&section.name)));
        }
    }

    let mut resolved_name_map = HashMap::new();
    for section in &spec.sections {
        for field in &section.fields {
            if !field.emit {
                continue;
            }
            let Some(resolved) = &field.resolved_type else {
                continue;
            };
            if resolved_name_map.contains_key(&resolved.type_name) {
                continue;
            }
            let emitted_name = unique_resolved_type_name(resolved, &mut reserved_names);
            resolved_name_map.insert(resolved.type_name.clone(), emitted_name);
        }
    }
    resolved_name_map
}

fn unique_resolved_type_name(
    resolved: &ResolvedStructType,
    reserved_names: &mut HashSet<String>,
) -> String {
    let base_name = to_pascal_case(&resolved.type_name);
    if reserved_names.insert(base_name.clone()) {
        return base_name;
    }
    let suffix = if resolved.is_account {
        "Account"
    } else if resolved.is_event {
        "Event"
    } else if resolved.is_instruction {
        "Instruction"
    } else {
        "Type"
    };
    let preferred = format!("{}{}", base_name, suffix);
    if reserved_names.insert(preferred.clone()) {
        return preferred;
    }
    let mut index = 2;
    loop {
        let candidate = format!("{}{}{}", base_name, suffix, index);
        if reserved_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn is_root_section(name: &str) -> bool {
    name.eq_ignore_ascii_case("root")
}

/// Target paths fed by an `AsCapture` mapping. Mirror of the TypeScript
/// generator's `is_capture_field`: those fields arrive wrapped in a
/// `CaptureWrapper` envelope rather than as the bare account struct.
fn capture_field_targets(spec: &SerializableStreamSpec) -> HashSet<String> {
    let mut targets = HashSet::new();
    for handler in &spec.handlers {
        for mapping in &handler.mappings {
            if matches!(&mapping.source, MappingSource::AsCapture { .. }) {
                targets.insert(mapping.target_path.clone());
            }
        }
    }
    targets
}

const MODELS_HELPERS: &str = r#"
def _snake_key(key: str) -> str:
    out = []
    for index, ch in enumerate(key):
        if ch.isascii() and ch.isupper():
            if index != 0:
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def _mapping(value: Any, context: str) -> Dict[str, Any]:
    if not isinstance(value, Mapping):
        raise TypeError(
            f"{context} payload must be a mapping, got {type(value).__name__}"
        )
    return {_snake_key(key): item for key, item in value.items()}


def _to_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, (str, float)):
        return int(value)
    raise TypeError(f"Cannot convert {type(value).__name__} to int")


def _to_int_list(value: Any) -> Optional[List[Optional[int]]]:
    if value is None:
        return None
    return [_to_int(item) for item in value]


def _convert(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return converter(value)


def _convert_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [converter(item) for item in value]


def _convert_capture(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return capture_wrapper_from_wire(value, converter)


def _convert_capture_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [capture_wrapper_from_wire(item, converter) for item in value]


def _convert_event(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return event_wrapper_from_wire(value, converter)


def _convert_event_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [event_wrapper_from_wire(item, converter) for item in value]


def _require(data: Mapping[str, Any], key: str, context: str) -> Any:
    if key not in data:
        raise ValueError(f"{context} payload is missing required field '{key}'")
    return data[key]
"#;

/// The runtime envelopes every generated `models.py` carries. `EventWrapper`
/// and `CaptureWrapper` mirror `arete_interpreter::EventWrapper` /
/// `CaptureWrapper` (and the TypeScript `EventWrapper<T>` / `CaptureWrapper<T>`
/// interfaces): capture/event-fed fields arrive wrapped on the wire, so the
/// generated converters parse the envelope and hand `data` to the inner
/// struct converter.
const MODELS_WRAPPERS: &str = r#"@dataclass
class EventWrapper(Generic[_T]):
    """Wrapper for captured events (timestamp + data + provenance)."""

    timestamp: int = 0
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None


def event_wrapper_from_wire(value: Any, converter: Any = None) -> EventWrapper:
    """Converts a wire event wrapper into :class:`EventWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "EventWrapper")
    inner = data.get("data")
    return EventWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
    )


@dataclass
class CaptureWrapper(Generic[_T]):
    """Wrapper for captured accounts (timestamp + address + data + provenance)."""

    timestamp: int = 0
    account_address: Optional[str] = None
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None


def capture_wrapper_from_wire(value: Any, converter: Any = None) -> CaptureWrapper:
    """Converts a wire account-capture wrapper into :class:`CaptureWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "CaptureWrapper")
    inner = data.get("data")
    return CaptureWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        account_address=data.get("account_address"),
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
    )
"#;

/// Generate `models.py` for all entities. Also returns the export list and
/// the map of emitted raw account dataclasses (IDL account type name →
/// emitted Python class name) so the program SDK generator can attach typed
/// account read definitions.
fn generate_stack_models_py(
    stack_name: &str,
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
) -> (String, Vec<String>, BTreeMap<String, String>) {
    let mut exports: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    let mut generated: HashSet<String> = HashSet::new();
    let mut account_structs: BTreeMap<String, String> = BTreeMap::new();

    // Runtime envelopes first: capture/event-fed fields annotate against them.
    blocks.push(MODELS_WRAPPERS.to_string());
    exports.extend([
        "EventWrapper".to_string(),
        "event_wrapper_from_wire".to_string(),
        "CaptureWrapper".to_string(),
        "capture_wrapper_from_wire".to_string(),
    ]);

    for (index, spec) in entity_specs.iter().enumerate() {
        let entity_name = &entity_names[index];
        let resolved_name_map = build_resolved_type_name_map(spec, entity_name);
        let capture_fields = capture_field_targets(spec);

        // -- Resolved types referenced by this entity (emitted before use). --
        for section in &spec.sections {
            for field in &section.fields {
                if !field.emit {
                    continue;
                }
                let Some(resolved) = &field.resolved_type else {
                    continue;
                };
                let emitted_name = resolved_name_map
                    .get(&resolved.type_name)
                    .cloned()
                    .unwrap_or_else(|| to_pascal_case(&resolved.type_name));
                if !generated.insert(emitted_name.clone()) {
                    continue;
                }
                if resolved.is_account && !resolved.is_enum {
                    account_structs
                        .entry(resolved.type_name.clone())
                        .or_insert_with(|| emitted_name.clone());
                }
                if resolved.is_enum {
                    let variants = resolved.enum_variants.join(", ");
                    blocks.push(format!(
                        "# Enum `{}` (variants: {}) passed through as strings.\n{} = str\n",
                        resolved.type_name, variants, emitted_name
                    ));
                    exports.push(emitted_name);
                    continue;
                }
                let fields: Vec<PyField> = resolved
                    .fields
                    .iter()
                    .zip(canonical_resolved_field_names(&resolved.fields))
                    .map(|(field, name)| py_field_for_resolved(field, name))
                    .collect();
                let snake = to_snake_case(&emitted_name);
                let from_wire = format!("{snake}_from_wire");
                let patch = format!("{snake}_patch_from_wire");
                blocks.push(render_dataclass(
                    &emitted_name,
                    &format!("Resolved type `{}`.", resolved.type_name),
                    &fields,
                ));
                blocks.push(render_from_wire(&emitted_name, &from_wire, &fields, true));
                blocks.push(render_patch_from_wire(&emitted_name, &patch, &fields));
                exports.extend([emitted_name.clone(), from_wire, patch]);
            }
        }

        // -- Section dataclasses + converters. --
        let mut section_fields: Vec<(String, String)> = Vec::new(); // (field name, class name)
        for section in &spec.sections {
            if is_root_section(&section.name) || !section.fields.iter().any(|field| field.emit) {
                continue;
            }
            let class_name = format!("{}{}", entity_name, to_pascal_case(&section.name));
            section_fields.push((to_snake_case(&section.name), class_name.clone()));
            if !generated.insert(class_name.clone()) {
                continue;
            }
            let fields: Vec<PyField> = section
                .fields
                .iter()
                .filter(|field| field.emit)
                .map(|field| py_field_for(field, &resolved_name_map, &capture_fields))
                .collect();
            let snake = to_snake_case(&class_name);
            let from_wire = format!("{snake}_from_wire");
            let patch = format!("{snake}_patch_from_wire");
            blocks.push(render_dataclass(
                &class_name,
                &format!("`{}` section of `{}`.", section.name, entity_name),
                &fields,
            ));
            blocks.push(render_from_wire(&class_name, &from_wire, &fields, false));
            blocks.push(render_patch_from_wire(&class_name, &patch, &fields));
            exports.extend([class_name, from_wire, patch]);
        }

        // -- Main entity dataclass + converters. --
        if generated.insert(entity_name.clone()) {
            let root_fields: Vec<PyField> = spec
                .sections
                .iter()
                .filter(|section| is_root_section(&section.name))
                .flat_map(|section| {
                    section
                        .fields
                        .iter()
                        .filter(|field| field.emit)
                        .map(|field| py_field_for(field, &resolved_name_map, &capture_fields))
                        .collect::<Vec<_>>()
                })
                .collect();

            let mut out = format!(
                "@dataclass\nclass {entity_name}:\n    \"\"\"Entity `{entity_name}`.\"\"\"\n"
            );
            if !section_fields.is_empty() || !root_fields.is_empty() {
                out.push('\n');
            }
            for (field_name, class_name) in &section_fields {
                out.push_str(&format!(
                    "    {field_name}: {class_name} = field(default_factory={class_name})\n"
                ));
            }
            for field in &root_fields {
                out.push_str(&format!(
                    "    {}: {} = None\n",
                    field.name, field.annotation
                ));
            }
            blocks.push(out);

            let entity_snake = to_snake_case(entity_name);
            let from_wire_name = format!("{entity_snake}_from_wire");
            let patch_name = format!("{entity_snake}_patch_from_wire");

            let mut from_wire = format!(
                "def {from_wire_name}(value: Any) -> {entity_name}:\n    \"\"\"Converts a merged wire entity into :class:`{entity_name}`.\"\"\"\n"
            );
            if section_fields.is_empty() && root_fields.is_empty() {
                from_wire.push_str(&format!(
                    "    _mapping(value, {})\n    return {entity_name}()\n",
                    py_string_literal(entity_name)
                ));
            } else {
                from_wire.push_str(&format!(
                    "    data = _mapping(value, {})\n    return {entity_name}(\n",
                    py_string_literal(entity_name)
                ));
                for (field_name, class_name) in &section_fields {
                    from_wire.push_str(&format!(
                        "        {field_name}={converter}(data.get({wire}) or {{}}),\n",
                        field_name = field_name,
                        converter = format_args!("{}_from_wire", to_snake_case(class_name)),
                        wire = py_string_literal(field_name),
                    ));
                }
                for field in &root_fields {
                    let accessor = format!("data.get({})", py_string_literal(&field.wire));
                    from_wire.push_str(&format!(
                        "        {}={},\n",
                        field.name,
                        field.conversion.render(&accessor)
                    ));
                }
                from_wire.push_str("    )\n");
            }
            blocks.push(from_wire);

            let mut patch = format!(
                "def {patch_name}(value: Any) -> Dict[str, Any]:\n    \"\"\"Converts a partial `{entity_name}` patch; only present keys appear.\"\"\"\n"
            );
            patch.push_str(&format!(
                "    data = _mapping(value, {})\n    out: Dict[str, Any] = {{}}\n",
                py_string_literal(&format!("{entity_name} patch"))
            ));
            for (field_name, class_name) in &section_fields {
                patch.push_str(&format!(
                    "    if {wire} in data:\n        out[{wire}] = {converter}(data[{wire}] or {{}})\n",
                    wire = py_string_literal(field_name),
                    converter = format_args!(
                        "{}_patch_from_wire",
                        to_snake_case(class_name)
                    ),
                ));
            }
            for field in &root_fields {
                let accessor = format!("data[{}]", py_string_literal(&field.wire));
                patch.push_str(&format!(
                    "    if {wire} in data:\n        out[{wire}] = {expr}\n",
                    wire = py_string_literal(&field.wire),
                    expr = field.conversion.render(&accessor)
                ));
            }
            patch.push_str("    return out\n");
            blocks.push(patch);

            exports.extend([entity_name.clone(), from_wire_name, patch_name]);
        }
    }

    let all_list = exports
        .iter()
        .map(|name| format!("    {},", py_string_literal(name)))
        .collect::<Vec<_>>()
        .join("\n");

    let mut output = format!(
        r#""""Generated entity models for the `{stack_name}` stack. Do not edit.

Wire payloads are snake_case and pass through untransformed; u64/u128 decimal
strings convert to `int`. `*_from_wire` builds full dataclasses (IDL struct
converters reject payloads missing a required key; entity converters leave
missing keys None); `*_patch_from_wire` converts only the keys present in a
patch. Fields fed by `#[capture]` mappings or event handlers arrive inside a
`CaptureWrapper` / `EventWrapper` envelope and are typed as such.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Generic, List, Mapping, Optional, TypeVar

_T = TypeVar("_T")

__all__ = [
{all_list}
]

{helpers}

"#,
        stack_name = stack_name,
        all_list = all_list,
        helpers = MODELS_HELPERS.trim_start_matches('\n'),
    );
    output.push_str(&blocks.join("\n\n"));
    (output, exports, account_structs)
}

// ============================================================================
// views.py — typed view namespaces + the VIEWS map
// ============================================================================

/// Resolve the typed state-view key for one entity. Mirrors the TypeScript
/// generator's `state_view_key_definition`, but degrades (empty `key_fields`,
/// positional scalar key at runtime) instead of failing generation.
fn state_view_key_fields(
    entity_name: &str,
    spec: &SerializableStreamSpec,
) -> Result<String, String> {
    let mut seen = HashSet::new();
    let distinct_keys: Vec<&str> = spec
        .identity
        .primary_keys
        .iter()
        .map(String::as_str)
        .filter(|key| seen.insert(*key))
        .collect();

    if distinct_keys.len() > 1 {
        return Err(format!(
            "composite state key [{}] is not supported; the view takes one positional scalar key",
            distinct_keys.join(", ")
        ));
    }
    let Some(key_path) = distinct_keys.first().copied() else {
        return Err(
            "no primary key recorded; the view takes one positional scalar key".to_string(),
        );
    };
    let key_leaf = key_path.rsplit('.').next().unwrap_or(key_path);
    let field_info = spec.field_mappings.get(key_path).or_else(|| {
        spec.sections.iter().find_map(|section| {
            section.fields.iter().find(|field| {
                field.raw_field_name() == key_path
                    || field.raw_field_name() == key_leaf
                    || field.field_name == key_path
                    || field.field_name == key_leaf
            })
        })
    });
    if let Some(field) = field_info {
        if field.is_array {
            return Err(format!(
                "array state key '{key_path}' is not supported; the view takes one positional scalar key"
            ));
        }
        match field.base_type {
            BaseType::String
            | BaseType::Binary
            | BaseType::Pubkey
            | BaseType::Integer
            | BaseType::Timestamp => {}
            _ => {
                return Err(format!(
                    "state key '{}' with type '{}' is not supported for entity '{}'; the view takes one positional scalar key",
                    key_path, field.rust_type_name, entity_name
                ));
            }
        }
    }
    Ok(to_snake_case(key_leaf))
}

fn generate_stack_views_py(
    stack_name: &str,
    entity_specs: &[SerializableStreamSpec],
    entity_names: &[String],
    exact_views: bool,
) -> String {
    let mut exports: Vec<String> = Vec::new();
    let mut class_blocks: Vec<String> = Vec::new();
    let mut group_entries: Vec<String> = Vec::new();

    for (index, entity_name) in entity_names.iter().enumerate() {
        let spec = &entity_specs[index];
        if exact_views && spec.views.is_empty() {
            continue;
        }
        let parser = format!("models.{}_from_wire", to_snake_case(entity_name));
        let class_name = format!("{entity_name}Views");
        let mut attrs: Vec<(String, String)> = Vec::new();

        let has_state = !exact_views
            || spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/state"));
        if has_state {
            let (key_fields_expr, key_note) = match state_view_key_fields(entity_name, spec) {
                Ok(field_name) => (format!("({},)", py_string_literal(&field_name)), None),
                Err(reason) => ("()".to_string(), Some(reason)),
            };
            let mut code = String::new();
            if let Some(note) = key_note {
                code.push_str(&format!("    # [arete codegen] {note}\n"));
            }
            code.push_str(&format!(
                "    state = ViewDef(\n        mode=\"state\",\n        view={view},\n        key_fields={key_fields},\n        parser={parser},\n    )",
                view = py_string_literal(&format!("{entity_name}/state")),
                key_fields = key_fields_expr,
                parser = parser,
            ));
            attrs.push(("state".to_string(), code));
        }

        let has_list = !exact_views
            || spec
                .views
                .iter()
                .any(|view| view.id == format!("{entity_name}/list"));
        if has_list {
            attrs.push((
                "list".to_string(),
                format!(
                    "    list = ViewDef(mode=\"list\", view={}, parser={})",
                    py_string_literal(&format!("{entity_name}/list")),
                    parser
                ),
            ));
        }

        for view in spec.views.iter().filter(|view| {
            !view.id.ends_with("/state")
                && !view.id.ends_with("/list")
                && view.id.starts_with(entity_name.as_str())
        }) {
            let view_name = view.id.split('/').nth(1).unwrap_or("unknown");
            let attr = to_snake_case(view_name);
            attrs.push((
                attr.clone(),
                format!(
                    "    {} = ViewDef(mode=\"list\", view={}, parser={})",
                    attr,
                    py_string_literal(&view.id),
                    parser
                ),
            ));
        }

        let body = if attrs.is_empty() {
            "    pass".to_string()
        } else {
            attrs
                .iter()
                .map(|(_, code)| code.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        class_blocks.push(format!(
            "class {class_name}:\n    \"\"\"Typed views of the `{entity_name}` entity.\"\"\"\n\n{body}\n",
        ));
        exports.push(class_name.clone());

        let group_key = to_snake_case(entity_name);
        let entries = attrs
            .iter()
            .map(|(attr, _)| {
                format!(
                    "        {}: {}.{},",
                    py_string_literal(attr),
                    class_name,
                    attr
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        group_entries.push(format!(
            "    {}: {{\n{}\n    }},",
            py_string_literal(&group_key),
            entries
        ));
    }

    exports.push("VIEWS".to_string());
    let all_list = exports
        .iter()
        .map(|name| format!("    {},", py_string_literal(name)))
        .collect::<Vec<_>>()
        .join("\n");

    let views_map = if group_entries.is_empty() {
        "VIEWS: Dict[str, Dict[str, ViewDef]] = {}\n".to_string()
    } else {
        format!(
            "VIEWS: Dict[str, Dict[str, ViewDef]] = {{\n{}\n}}\n",
            group_entries.join("\n")
        )
    };

    format!(
        r#""""Generated typed views for the `{stack_name}` stack. Do not edit.

`VIEWS` feeds `StackDef.views`; group keys are snake_case entity names
(`a4.views.<entity>.<view>`). State views declare typed key fields consumed
as keyword arguments (`.use(<key_field>=...)`).
"""

from __future__ import annotations

from typing import Dict

from arete.views import ViewDef

from . import models

__all__ = [
{all_list}
]


{classes}

{views_map}"#,
        stack_name = stack_name,
        all_list = all_list,
        classes = class_blocks.join("\n\n"),
        views_map = views_map,
    )
}

// ============================================================================
// programs.py — program SDK generation
// ============================================================================

/// Result of generating the `programs.py` module for a stack.
#[derive(Debug, Clone)]
pub(crate) struct PythonProgramsCodegen {
    code: String,
}

/// Which runtime names a generated `programs.py` references.
#[derive(Debug, Default)]
struct PythonProgramImports {
    account_meta: bool,
    arg_schema: bool,
    pda: bool,
    pda_factory: bool,
    error_metadata: bool,
    builders: bool,
    reads: bool,
    wire_read_descriptor: bool,
}

fn instruction_snapshot_matches(
    instruction: &InstructionDef,
    snapshot: &IdlInstructionSnapshot,
) -> bool {
    instruction.name == snapshot.name
        && instruction.discriminator == snapshot.discriminator
        && instruction.args.len() == snapshot.args.len()
        && instruction
            .args
            .iter()
            .zip(snapshot.args.iter())
            .all(|(arg, snapshot_arg)| arg.name == snapshot_arg.name)
}

fn find_instruction_snapshot<'a>(
    instruction: &InstructionDef,
    idl: Option<&'a IdlSnapshot>,
) -> Option<&'a IdlInstructionSnapshot> {
    idl?.instructions
        .iter()
        .find(|snapshot| instruction_snapshot_matches(instruction, snapshot))
}

/// A parsed instruction argument type.
#[derive(Debug, Clone)]
struct PyParsedArg {
    /// Python `ArgType` expression for the handler schema.
    schema: String,
    /// Python annotation for the typed params TypedDict.
    param_type: String,
    /// Whether the type is representable by the core serializer.
    supported: bool,
}

fn py_unsupported() -> PyParsedArg {
    PyParsedArg {
        schema: "\"u8\"".to_string(),
        param_type: "Any".to_string(),
        supported: false,
    }
}

fn py_prim(schema: &str, param_type: &str) -> PyParsedArg {
    PyParsedArg {
        schema: py_string_literal(schema),
        param_type: param_type.to_string(),
        supported: true,
    }
}

/// Resolver for IDL-defined types (structs/enums) referenced by instruction
/// args. Resolved types are inlined into arg schemas as `{"struct": ...}` /
/// `{"enum": ...}` dict literals; the typed params annotation for such args
/// is `Any`. Mirrors `RustDefinedTypes`.
struct PythonDefinedTypes<'a> {
    defs: BTreeMap<String, &'a IdlTypeDefSnapshot>,
    lower: BTreeMap<String, String>,
    resolved: BTreeMap<String, Option<PyParsedArg>>,
    visiting: HashSet<String>,
}

impl<'a> PythonDefinedTypes<'a> {
    fn new(idls: &'a [IdlSnapshot]) -> Self {
        let mut defs: BTreeMap<String, &'a IdlTypeDefSnapshot> = BTreeMap::new();
        let mut lower: BTreeMap<String, String> = BTreeMap::new();
        for idl in idls {
            for def in &idl.types {
                if !defs.contains_key(def.name.as_str()) {
                    defs.insert(def.name.clone(), def);
                    lower.insert(def.name.to_lowercase(), def.name.clone());
                }
            }
        }
        PythonDefinedTypes {
            defs,
            lower,
            resolved: BTreeMap::new(),
            visiting: HashSet::new(),
        }
    }

    /// Parse a stringified Rust-ish arg type (what `to_rust_type_string`
    /// produces), resolving bare names against the IDL type definitions.
    fn parse_arg_type(&mut self, raw: &str) -> PyParsedArg {
        let t = raw.trim().trim_start_matches('&').trim();

        if let Some((name, inner)) = split_generic(t) {
            match name {
                "Option" => {
                    let inner = self.parse_arg_type(inner);
                    return PyParsedArg {
                        schema: format!("{{\"option\": {}}}", inner.schema),
                        param_type: wrap_optional(&inner.param_type),
                        supported: inner.supported,
                    };
                }
                "Vec" | "VecU64Len" => {
                    let inner = self.parse_arg_type(inner);
                    let key = if name == "Vec" { "vec" } else { "vecU64Len" };
                    return PyParsedArg {
                        schema: format!("{{\"{}\": {}}}", key, inner.schema),
                        param_type: format!("Sequence[{}]", inner.param_type),
                        supported: inner.supported,
                    };
                }
                _ => return py_unsupported(),
            }
        }

        // Fixed-size array: [T; N].
        if let Some(stripped) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some((ty, n)) = stripped.rsplit_once(';') {
                let inner = self.parse_arg_type(ty.trim());
                let n = n.trim();
                if n.parse::<usize>().is_ok() {
                    return PyParsedArg {
                        schema: format!("{{\"array\": ({}, {})}}", inner.schema, n),
                        param_type: format!("Sequence[{}]", inner.param_type),
                        supported: inner.supported,
                    };
                }
            }
        }

        // Primitive (possibly path-qualified, e.g. solana_pubkey::Pubkey).
        let last = t.rsplit("::").next().unwrap_or(t);
        match last {
            "u8" => py_prim("u8", "int"),
            "u16" => py_prim("u16", "int"),
            "u32" => py_prim("u32", "int"),
            "u64" => py_prim("u64", "int"),
            "u128" => py_prim("u128", "int"),
            "i8" => py_prim("i8", "int"),
            "i16" => py_prim("i16", "int"),
            "i32" => py_prim("i32", "int"),
            "i64" => py_prim("i64", "int"),
            "i128" => py_prim("i128", "int"),
            "f32" => py_prim("f32", "float"),
            "f64" => py_prim("f64", "float"),
            "bool" => py_prim("bool", "bool"),
            "String" | "string" | "str" => py_prim("string", "str"),
            "Pubkey" | "pubkey" | "PublicKey" | "publicKey" => py_prim("pubkey", "str"),
            "bytes" => py_prim("bytes", "bytes"),
            _ => self
                .resolve_defined(t)
                .or_else(|| (last != t).then(|| self.resolve_defined(last)).flatten())
                .unwrap_or_else(py_unsupported),
        }
    }

    fn parse_snapshot_type(&mut self, t: &IdlTypeSnapshot) -> PyParsedArg {
        match t {
            IdlTypeSnapshot::Simple(s) => self.parse_arg_type(s),
            IdlTypeSnapshot::Option(o) => {
                let inner = self.parse_snapshot_type(&o.option);
                PyParsedArg {
                    schema: format!("{{\"option\": {}}}", inner.schema),
                    param_type: wrap_optional(&inner.param_type),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Vec(v) => {
                let inner = self.parse_snapshot_type(&v.vec);
                let key = match v.length_prefix {
                    Some(arete_idl::types::IdlLengthPrefix::U64) => "vecU64Len",
                    _ => "vec",
                };
                PyParsedArg {
                    schema: format!("{{\"{}\": {}}}", key, inner.schema),
                    param_type: format!("Sequence[{}]", inner.param_type),
                    supported: inner.supported,
                }
            }
            IdlTypeSnapshot::Array(arr) => {
                let mut element: Option<PyParsedArg> = None;
                let mut size: Option<u32> = None;
                for part in &arr.array {
                    match part {
                        IdlArrayElementSnapshot::Type(inner) => {
                            element = Some(self.parse_snapshot_type(inner))
                        }
                        IdlArrayElementSnapshot::TypeName(name) => {
                            element = Some(self.parse_arg_type(name))
                        }
                        IdlArrayElementSnapshot::Size(n) => size = Some(*n),
                    }
                }
                match (element, size) {
                    (Some(inner), Some(n)) => PyParsedArg {
                        schema: format!("{{\"array\": ({}, {})}}", inner.schema, n),
                        param_type: format!("Sequence[{}]", inner.param_type),
                        supported: inner.supported,
                    },
                    _ => py_unsupported(),
                }
            }
            IdlTypeSnapshot::HashMap(map) => {
                let key = self.parse_snapshot_type(&map.hash_map.0);
                let value = self.parse_snapshot_type(&map.hash_map.1);
                if !key.supported || key.schema != "\"string\"" || !value.supported {
                    py_unsupported()
                } else {
                    PyParsedArg {
                        schema: format!("{{\"hashMap\": ({}, {})}}", key.schema, value.schema),
                        param_type: "Any".to_string(),
                        supported: true,
                    }
                }
            }
            IdlTypeSnapshot::Tuple(tuple) => {
                let elements = tuple
                    .tuple
                    .iter()
                    .map(|element| self.parse_snapshot_type(element))
                    .collect::<Vec<_>>();
                if elements.iter().any(|element| !element.supported) {
                    py_unsupported()
                } else {
                    PyParsedArg {
                        schema: format!(
                            "{{\"tuple\": [{}]}}",
                            elements
                                .iter()
                                .map(|element| element.schema.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        param_type: "Any".to_string(),
                        supported: true,
                    }
                }
            }
            IdlTypeSnapshot::Defined(d) => {
                let name = match &d.defined {
                    IdlDefinedInnerSnapshot::Named { name } => name.as_str(),
                    IdlDefinedInnerSnapshot::Simple(s) => s.as_str(),
                };
                self.resolve_defined(name).unwrap_or_else(py_unsupported)
            }
        }
    }

    fn resolve_defined(&mut self, name: &str) -> Option<PyParsedArg> {
        if let Some(cached) = self.resolved.get(name) {
            return cached.clone();
        }
        if self.visiting.contains(name) {
            // Recursive types are not supported by instruction codegen.
            return None;
        }
        let key = if self.defs.contains_key(name) {
            name.to_string()
        } else {
            match self.lower.get(&name.to_lowercase()) {
                Some(canonical) => canonical.clone(),
                None => {
                    self.resolved.insert(name.to_string(), None);
                    return None;
                }
            }
        };
        self.visiting.insert(key.clone());
        let def = self.defs[&key];
        let result = match &def.type_def {
            IdlTypeDefKindSnapshot::Struct { fields, .. } => {
                let fields = fields.clone();
                self.resolve_struct(&fields)
            }
            IdlTypeDefKindSnapshot::TupleStruct { .. } => None,
            IdlTypeDefKindSnapshot::Enum { variants, .. } => {
                let variants = variants.clone();
                self.resolve_enum(&variants)
            }
        };
        self.visiting.remove(&key);
        self.resolved.insert(name.to_string(), result.clone());
        if name != key {
            self.resolved.insert(key, result.clone());
        }
        result
    }

    fn resolve_struct(&mut self, fields: &[IdlFieldSnapshot]) -> Option<PyParsedArg> {
        let mut field_exprs: Vec<String> = Vec::new();
        for field in fields {
            let parsed = self.parse_snapshot_type(&field.type_);
            if !parsed.supported {
                return None;
            }
            field_exprs.push(format!(
                "{{\"name\": {}, \"type\": {}}}",
                py_string_literal(&field.name),
                parsed.schema
            ));
        }
        Some(PyParsedArg {
            schema: format!("{{\"struct\": [{}]}}", field_exprs.join(", ")),
            param_type: "Any".to_string(),
            supported: true,
        })
    }

    fn resolve_enum(&mut self, variants: &[IdlEnumVariantSnapshot]) -> Option<PyParsedArg> {
        let mut variant_exprs: Vec<String> = Vec::new();
        for variant in variants {
            let name_literal = py_string_literal(&variant.name);
            if variant.fields.is_empty() {
                variant_exprs.push(name_literal);
                continue;
            }
            let named: Vec<_> = variant
                .fields
                .iter()
                .filter_map(|field| match field {
                    IdlEnumVariantFieldSnapshot::Named(field) => Some(field),
                    IdlEnumVariantFieldSnapshot::Tuple(_) => None,
                })
                .collect();
            if named.len() == variant.fields.len() {
                let mut field_exprs: Vec<String> = Vec::new();
                for field in named {
                    let parsed = self.parse_snapshot_type(&field.type_);
                    if !parsed.supported {
                        return None;
                    }
                    field_exprs.push(format!(
                        "{{\"name\": {}, \"type\": {}}}",
                        py_string_literal(&field.name),
                        parsed.schema
                    ));
                }
                variant_exprs.push(format!(
                    "{{\"name\": {}, \"fields\": [{}]}}",
                    name_literal,
                    field_exprs.join(", ")
                ));
            } else if named.is_empty() {
                let mut element_exprs: Vec<String> = Vec::new();
                for field in &variant.fields {
                    let IdlEnumVariantFieldSnapshot::Tuple(ty) = field else {
                        unreachable!("named.is_empty() guarantees tuple fields");
                    };
                    let parsed = self.parse_snapshot_type(ty);
                    if !parsed.supported {
                        return None;
                    }
                    element_exprs.push(parsed.schema);
                }
                variant_exprs.push(format!(
                    "{{\"name\": {}, \"tuple\": [{}]}}",
                    name_literal,
                    element_exprs.join(", ")
                ));
            } else {
                // Mixed named and tuple fields are not supported.
                return None;
            }
        }
        Some(PyParsedArg {
            schema: format!("{{\"enum\": [{}]}}", variant_exprs.join(", ")),
            param_type: "Any".to_string(),
            supported: true,
        })
    }
}

/// How a mapped account surfaces in the typed params TypedDict.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PyAccountFieldKind {
    /// Signer slot: optional address override (payer fallback applies).
    Signer,
    /// Required user-provided account address.
    Required,
    /// Optional user-provided account address.
    Optional,
}

/// Result of mapping a single instruction account.
struct MappedPyAccount {
    /// `AccountMeta(...)` literal, indented for the handler's accounts list.
    literal: String,
    /// Params field for caller-supplied addresses.
    field: Option<(String, PyAccountFieldKind)>,
    /// Human-readable notes surfaced in the builder's docstring.
    notes: Vec<String>,
    /// Whether the emitted resolution references `Pda` / `PdaConfig`.
    uses_pda: bool,
}

fn py_account_meta_literal(
    acc: &InstructionAccountDef,
    emitted_name: &str,
    resolution: &str,
    comment: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(comment) = comment {
        out.push_str(&format!("            # [arete codegen] {}\n", comment));
    }
    out.push_str(&format!(
        "            AccountMeta(\n                name={name},\n                is_signer={is_signer},\n                is_writable={is_writable},\n                resolution={resolution},\n                is_optional={is_optional},\n            ),",
        name = py_string_literal(emitted_name),
        is_signer = py_bool(acc.is_signer),
        is_writable = py_bool(acc.is_writable),
        resolution = resolution,
        is_optional = py_bool(acc.is_optional),
    ));
    out
}

fn map_py_account(
    acc: &InstructionAccountDef,
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    account_names: &HashSet<&str>,
    arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
) -> MappedPyAccount {
    let emitted_name = account_name_map
        .get(&acc.name)
        .map(String::as_str)
        .unwrap_or(&acc.name);
    let user_field_kind = if acc.is_optional {
        PyAccountFieldKind::Optional
    } else {
        PyAccountFieldKind::Required
    };
    let degraded = |reason: String| -> MappedPyAccount {
        let note = format!(
            "account `{}` degraded to user-provided ({})",
            acc.name, reason
        );
        MappedPyAccount {
            literal: py_account_meta_literal(acc, emitted_name, "UserProvided()", Some(&note)),
            field: Some((emitted_name.to_string(), user_field_kind)),
            notes: vec![note],
            uses_pda: false,
        }
    };

    match &acc.resolution {
        AccountResolution::Signer => MappedPyAccount {
            literal: py_account_meta_literal(acc, emitted_name, "Signer()", None),
            field: Some((emitted_name.to_string(), PyAccountFieldKind::Signer)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::Known { address } => MappedPyAccount {
            literal: py_account_meta_literal(
                acc,
                emitted_name,
                &format!("Known({})", py_string_literal(address)),
                None,
            ),
            field: None,
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::UserProvided => MappedPyAccount {
            literal: py_account_meta_literal(acc, emitted_name, "UserProvided()", None),
            field: Some((emitted_name.to_string(), user_field_kind)),
            notes: Vec::new(),
            uses_pda: false,
        },
        AccountResolution::PdaInline {
            seeds,
            program_id,
            program,
        } => {
            if program.is_some() {
                return degraded(
                    "uses a dynamic PDA program selector not supported by the Python low-level resolver"
                        .to_string(),
                );
            }
            match build_py_pda_config(
                seeds,
                program_id.as_deref(),
                account_names,
                arg_types,
                account_name_map,
            ) {
                Ok((config, notes)) => MappedPyAccount {
                    literal: py_account_meta_literal(
                        acc,
                        emitted_name,
                        &format!("Pda({config})"),
                        None,
                    ),
                    field: None,
                    notes,
                    uses_pda: true,
                },
                Err(reason) => degraded(reason),
            }
        }
        AccountResolution::PdaRef { pda_name } => match pda_lookup.get(pda_name.as_str()) {
            Some(def) => {
                if def.program.is_some() {
                    return degraded(format!(
                        "PDA '{}' uses a dynamic program selector not supported by the Python low-level resolver",
                        pda_name
                    ));
                }
                match build_py_pda_config(
                    &def.seeds,
                    def.program_id.as_deref(),
                    account_names,
                    arg_types,
                    account_name_map,
                ) {
                    Ok((config, notes)) => MappedPyAccount {
                        literal: py_account_meta_literal(
                            acc,
                            emitted_name,
                            &format!("Pda({config})"),
                            None,
                        ),
                        field: None,
                        notes,
                        uses_pda: true,
                    },
                    Err(reason) => degraded(format!("PDA '{}': {}", pda_name, reason)),
                }
            }
            None => degraded(format!("references unknown PDA '{}'", pda_name)),
        },
    }
}

fn py_seed_expr(seed: &PdaSeedDef) -> String {
    match seed {
        PdaSeedDef::Literal { value } => format!("LiteralSeed({})", py_string_literal(value)),
        PdaSeedDef::Bytes { value } => {
            let bytes: Vec<String> = value.iter().map(|b| b.to_string()).collect();
            format!("BytesSeed(bytes([{}]))", bytes.join(", "))
        }
        PdaSeedDef::AccountRef { account_name } => {
            format!("AccountRefSeed({})", py_string_literal(account_name))
        }
        PdaSeedDef::ArgRef { arg_name, arg_type } => {
            match arg_type.as_deref().and_then(normalize_seed_arg_type) {
                Some(canonical) => format!(
                    "ArgRefSeed({}, {})",
                    py_string_literal(arg_name),
                    py_string_literal(&canonical)
                ),
                None => format!("ArgRefSeed({})", py_string_literal(arg_name)),
            }
        }
    }
}

fn py_seed_tuple(seed_exprs: &[String]) -> String {
    if seed_exprs.len() == 1 {
        format!("({},)", seed_exprs[0])
    } else {
        format!("({})", seed_exprs.join(", "))
    }
}

/// Build a `PdaConfig(...)` expression from seed definitions. Returns
/// `Err(reason)` when the PDA cannot be represented by the core resolver, so
/// the caller can degrade to user-provided.
fn build_py_pda_config(
    seeds: &[PdaSeedDef],
    program_id: Option<&str>,
    account_names: &HashSet<&str>,
    arg_types: &BTreeMap<&str, &str>,
    account_name_map: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>), String> {
    let mut seed_exprs: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for seed in seeds {
        match seed {
            PdaSeedDef::Literal { .. } | PdaSeedDef::Bytes { .. } => {
                seed_exprs.push(py_seed_expr(seed));
            }
            PdaSeedDef::AccountRef { account_name } => {
                if account_name.contains('.') {
                    return Err(format!(
                        "seed references account field '{}' which is not supported for auto-resolution",
                        account_name
                    ));
                }
                if !account_names.contains(account_name.as_str()) {
                    return Err(format!(
                        "seed references account '{}' not present in this instruction",
                        account_name
                    ));
                }
                seed_exprs.push(format!(
                    "AccountRefSeed({})",
                    py_string_literal(account_name_map.get(account_name).unwrap_or(account_name))
                ));
            }
            PdaSeedDef::ArgRef { arg_name, arg_type } => {
                let arg_root = arg_name.split('.').next().unwrap_or(arg_name.as_str());
                let present =
                    arg_types.contains_key(arg_name.as_str()) || arg_types.contains_key(arg_root);
                // Prefer the seed's declared type; fall back to the
                // instruction arg's type (Anchor seeds carry no type info).
                let raw_type = arg_type
                    .as_deref()
                    .or_else(|| arg_types.get(arg_name.as_str()).copied())
                    .or_else(|| arg_types.get(arg_root).copied());
                let canonical = raw_type.and_then(normalize_seed_arg_type);
                if !present {
                    if canonical.is_none() {
                        return Err(format!(
                            "seed helper arg '{}' is not present in this instruction and has no primitive type information",
                            arg_name
                        ));
                    }
                    notes.push(format!(
                        "seed input `{}` must be supplied via the `resolve` key when building through the raw handler",
                        arg_name
                    ));
                }
                match canonical {
                    Some(canonical) => seed_exprs.push(format!(
                        "ArgRefSeed({}, {})",
                        py_string_literal(arg_name),
                        py_string_literal(&canonical)
                    )),
                    None => {
                        notes.push(format!(
                            "seed arg `{}` has non-primitive type '{}'; the runtime will use heuristic encoding",
                            arg_name,
                            raw_type.unwrap_or("<unknown>")
                        ));
                        seed_exprs.push(format!("ArgRefSeed({})", py_string_literal(arg_name)));
                    }
                }
            }
        }
    }

    let config = match program_id {
        Some(pid) => format!(
            "PdaConfig(seeds={}, program_id={})",
            py_seed_tuple(&seed_exprs),
            py_string_literal(pid)
        ),
        None => format!("PdaConfig(seeds={})", py_seed_tuple(&seed_exprs)),
    };
    Ok((config, notes))
}

/// Generated code for one instruction.
struct PyInstructionBlock {
    code: String,
    /// snake_case raw-namespace key + handler function name for `ProgramDef`.
    raw_key: String,
    handler_fn: String,
    exports: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn generate_py_instruction_block(
    instr: &InstructionDef,
    instruction_snapshot: Option<&IdlInstructionSnapshot>,
    errors_expr: &str,
    has_program_errors: bool,
    pda_lookup: &BTreeMap<&str, &PdaDefinition>,
    parser: &mut PythonDefinedTypes<'_>,
    needs: &mut PythonProgramImports,
    module_name: &str,
    pascal_prefix: &str,
) -> Result<PyInstructionBlock, String> {
    // --- Parse args; skip the whole instruction on unsupported types. ---
    let mut parsed_args: Vec<(&InstructionArgDef, PyParsedArg)> = Vec::new();
    for (index, arg) in instr.args.iter().enumerate() {
        let parsed = instruction_snapshot
            .and_then(|snapshot| snapshot.args.get(index))
            .map(|snapshot_arg| parser.parse_snapshot_type(&snapshot_arg.type_))
            .unwrap_or_else(|| parser.parse_arg_type(&arg.arg_type));
        if !parsed.supported {
            return Err(format!(
                "arg '{}' has unsupported type '{}'",
                arg.name, arg.arg_type
            ));
        }
        parsed_args.push((arg, parsed));
    }

    // --- Map accounts. ---
    let account_names: HashSet<&str> = instr.accounts.iter().map(|a| a.name.as_str()).collect();
    let arg_types: BTreeMap<&str, &str> = instr
        .args
        .iter()
        .map(|a| (a.name.as_str(), a.arg_type.as_str()))
        .collect();

    let mut account_literals: Vec<String> = Vec::new();
    let mut account_fields: Vec<(String, PyAccountFieldKind)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let account_name_map = disambiguate_instruction_account_names(instr);
    for (source_name, emitted_name) in &account_name_map {
        if source_name != emitted_name {
            notes.push(format!(
                "account `{}` collides with an instruction arg and is exposed as `{}`",
                source_name, emitted_name
            ));
        }
    }
    for acc in &instr.accounts {
        let mapped = map_py_account(
            acc,
            pda_lookup,
            &account_names,
            &arg_types,
            &account_name_map,
        );
        account_literals.push(mapped.literal);
        if let Some(field) = mapped.field {
            account_fields.push(field);
        }
        notes.extend(mapped.notes);
        if mapped.uses_pda {
            needs.pda = true;
        }
    }
    if !instr.accounts.is_empty() {
        needs.account_meta = true;
    }
    if !instr.args.is_empty() {
        needs.arg_schema = true;
    }
    if has_program_errors {
        needs.error_metadata = true;
    }
    needs.builders = true;

    let snake_name = to_snake_case(&instr.name);
    let fn_name = format!("{module_name}_{snake_name}");
    let handler_fn = format!("{fn_name}_handler");
    let params_name = format!("{}{}Params", pascal_prefix, to_pascal_case(&instr.name));

    // --- Typed params TypedDict: args first, then caller-supplied accounts.
    // Keys are runtime parameter names (normally the IDL wire names; account
    // collisions have already been assigned explicit aliases above). The
    // fail-closed `InstructionHandler.build` matches them verbatim. ---
    let mut used_keys: HashSet<String> = HashSet::new();
    let mut param_entries: Vec<String> = Vec::new();
    for (arg, parsed) in &parsed_args {
        used_keys.insert(arg.name.clone());
        param_entries.push(format!(
            "        # arg `{}` (`{}`)\n        {}: {},",
            arg.name,
            arg.arg_type,
            py_string_literal(&arg.name),
            parsed.param_type
        ));
    }
    for (name, kind) in &account_fields {
        if !used_keys.insert(name.clone()) {
            notes.push(format!(
                "account `{}` collides with another params field and has no typed override field",
                name
            ));
            continue;
        }
        let comment = match kind {
            PyAccountFieldKind::Signer => format!(
                "Optional address override for the `{}` signer (defaults to the payer).",
                name
            ),
            PyAccountFieldKind::Required => format!("Address of the `{}` account.", name),
            PyAccountFieldKind::Optional => {
                format!("Optional address of the `{}` account.", name)
            }
        };
        param_entries.push(format!(
            "        # {}\n        {}: str,",
            comment,
            py_string_literal(name)
        ));
    }

    let params_typed_dict = if param_entries.is_empty() {
        format!(
            "# Typed params for `{name}` (no args or caller-supplied accounts).\n{params_name} = TypedDict(\"{params_name}\", {{}}, total=False)",
            name = instr.name,
            params_name = params_name
        )
    } else {
        format!(
            "# Typed params for `{name}`: instruction args plus overridable accounts\n# (wire-name keys; required/optional noted per key).\n{params_name} = TypedDict(\n    \"{params_name}\",\n    {{\n{fields}\n    }},\n    total=False,\n)",
            name = instr.name,
            params_name = params_name,
            fields = param_entries.join("\n")
        )
    };

    // --- Builder docstring. ---
    let mut doc_lines: Vec<String> = instr
        .docs
        .iter()
        .map(|line| line.trim().to_string())
        .collect();
    if doc_lines.is_empty() {
        doc_lines.push(format!("Builds the `{}` instruction.", instr.name));
    }
    doc_lines.push(String::new());
    doc_lines.push(format!(
        "Pure (no network). Params use IDL wire names plus documented account aliases (see `{params_name}`);"
    ));
    doc_lines.push("unknown params fail closed.".to_string());
    doc_lines.push(String::new());
    doc_lines
        .push("Reserved keyword-only options: `wallet` (signer fallback address),".to_string());
    doc_lines.push(
        "`accounts` (unvalidated overrides), `remaining_accounts`. Account names".to_string(),
    );
    doc_lines.push("(including `payer`) stay available as params.".to_string());
    if !notes.is_empty() {
        doc_lines.push(String::new());
        doc_lines.push("Codegen notes:".to_string());
        for note in &notes {
            doc_lines.push(format!("- {}", note));
        }
    }
    let docstring = py_docstring(&doc_lines, "    ");

    let builder = format!(
        // The signer fallback is named `wallet` (TS spelling), not `payer`:
        // `payer` is a real IDL account name, and a reserved kwarg by that
        // name would swallow the advertised `payer` account override.
        "def {fn_name}(\n    *,\n    wallet: Optional[str] = None,\n    accounts: Optional[Mapping[str, str]] = None,\n    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,\n    **params: Any,\n) -> BuiltInstruction:\n{docstring}\n    return {handler_fn}().build(\n        dict(params),\n        payer=wallet,\n        accounts=accounts,\n        remaining_accounts=remaining_accounts,\n    )",
        fn_name = fn_name,
        docstring = docstring,
        handler_fn = handler_fn,
    );

    // --- Handler literal. ---
    let discriminator = instr
        .discriminator
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let accounts_literal = if account_literals.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n{}\n        ]", account_literals.join("\n"))
    };
    let args_literal = if parsed_args.is_empty() {
        "[]".to_string()
    } else {
        let entries: Vec<String> = parsed_args
            .iter()
            .map(|(arg, parsed)| {
                format!(
                    "            ArgSchema(name={}, type={}),",
                    py_string_literal(&arg.name),
                    parsed.schema
                )
            })
            .collect();
        format!("[\n{}\n        ]", entries.join("\n"))
    };

    let handler = format!(
        "def {handler_fn}() -> InstructionHandler:\n    \"\"\"Raw instruction handler for `{name}` (escape hatch).\"\"\"\n    return InstructionHandler(\n        program_id={program_id_const},\n        discriminator=bytes([{discriminator}]),\n        accounts={accounts},\n        args={args},\n        errors={errors},\n    )",
        handler_fn = handler_fn,
        name = instr.name,
        program_id_const = format_args!("{}_PROGRAM_ID", module_name.to_uppercase()),
        discriminator = discriminator,
        accounts = accounts_literal,
        args = args_literal,
        errors = errors_expr,
    );

    Ok(PyInstructionBlock {
        code: format!("{}\n\n\n{}\n\n\n{}", params_typed_dict, builder, handler),
        raw_key: snake_name,
        handler_fn: handler_fn.clone(),
        exports: vec![params_name, fn_name, handler_fn],
    })
}

/// Generate the PDA config map + `PdaFactory` namespace class for one
/// program. Returns `None` when the program declares no PDAs.
fn generate_py_pdas(
    pdas: &BTreeMap<String, PdaDefinition>,
    module_name: &str,
    pascal_prefix: &str,
    needs: &mut PythonProgramImports,
) -> Option<(String, String, String)> {
    let supported_pdas = pdas
        .iter()
        .filter(|(_, definition)| definition.program.is_none())
        .collect::<Vec<_>>();
    if supported_pdas.is_empty() {
        return None;
    }
    needs.pda = true;
    needs.pda_factory = true;

    let const_prefix = module_name.to_uppercase();
    let dict_name = format!("_{const_prefix}_PDAS");
    let class_name = format!("{pascal_prefix}Pdas");
    let program_id_const = format!("{const_prefix}_PROGRAM_ID");

    let mut dict_entries: Vec<String> = Vec::new();
    let mut class_attrs: Vec<String> = Vec::new();
    for (_, def) in supported_pdas {
        let key = to_snake_case(&def.name);
        let seed_exprs: Vec<String> = def.seeds.iter().map(py_seed_expr).collect();
        let config = match &def.program_id {
            Some(pid) => format!(
                "PdaConfig(seeds={}, program_id={})",
                py_seed_tuple(&seed_exprs),
                py_string_literal(pid)
            ),
            None => format!("PdaConfig(seeds={})", py_seed_tuple(&seed_exprs)),
        };
        dict_entries.push(format!("    {}: {},", py_string_literal(&key), config));
        class_attrs.push(format!(
            "    {key} = PdaFactory({name}, {dict_name}[{name}], {program_id_const})",
            key = key,
            name = py_string_literal(&key),
            dict_name = dict_name,
            program_id_const = program_id_const,
        ));
    }

    let code = format!(
        "{dict_name}: Dict[str, PdaConfig] = {{\n{entries}\n}}\n\n\nclass {class_name}:\n    \"\"\"PDA factories for program `{module_name}`: `.derive(**seeds)` returns\n    `(address, bump)`; unknown seed kwargs fail closed.\"\"\"\n\n{attrs}",
        dict_name = dict_name,
        entries = dict_entries.join("\n"),
        class_name = class_name,
        module_name = module_name,
        attrs = class_attrs.join("\n"),
    );
    Some((code, dict_name, class_name))
}

/// Release identity computed at generation time for one program, or the
/// reason the program's read layer is omitted.
type ProgramReadLayer = Result<(String, String, Option<serde_json::Value>), String>;

fn resolve_program_read_layer(
    program_specs: &[arete_hash::ProgramSpecV1],
    program_id: &str,
) -> ProgramReadLayer {
    let Some(spec) = program_specs
        .iter()
        .find(|spec| spec.program_id == program_id)
    else {
        return Err("no program specification was recorded for this program".to_string());
    };
    let spec_hash = spec
        .hash()
        .map_err(|error| format!("failed to compute the program spec hash ({error})"))?;
    let release_hash = spec
        .oss_release_hash()
        .map_err(|error| format!("failed to compute the release hash ({error})"))?;
    Ok((spec_hash.to_string(), release_hash.to_string(), None))
}

/// Generate `programs.py`: one section per program with typed instruction
/// builders, raw handlers, PDA factories, and (when the stack records a
/// program spec for the program) release identity consts plus typed account
/// read definitions. Returns `None` when the stack declares no instructions.
#[allow(clippy::too_many_arguments)]
fn generate_stack_programs_py(
    stack_name: &str,
    instructions: &[InstructionDef],
    idls: &[IdlSnapshot],
    pdas: &BTreeMap<String, BTreeMap<String, PdaDefinition>>,
    program_ids: &[String],
    program_specs: &[arete_hash::ProgramSpecV1],
    account_structs: &BTreeMap<String, String>,
    reads: &[PythonProgramReadConfig],
    gateway: Option<&serde_json::Value>,
    include_idl_only_programs: bool,
) -> Option<PythonProgramsCodegen> {
    if instructions.is_empty() && !include_idl_only_programs {
        return None;
    }

    let default_program_id = program_ids.first().cloned().unwrap_or_default();

    // Group instructions by resolved program id, preserving first-seen order.
    let mut groups: Vec<(String, Vec<&InstructionDef>)> = Vec::new();
    if include_idl_only_programs {
        for (index, idl) in idls.iter().enumerate() {
            let program_id = idl
                .program_id
                .clone()
                .or_else(|| program_ids.get(index).cloned())
                .unwrap_or_default();
            if !groups.iter().any(|(existing, _)| *existing == program_id) {
                groups.push((program_id, Vec::new()));
            }
        }
    }
    for instr in instructions {
        let pid = instr
            .program_id
            .clone()
            .unwrap_or_else(|| default_program_id.clone());
        match groups.iter_mut().find(|(existing, _)| *existing == pid) {
            Some((_, list)) => list.push(instr),
            None => groups.push((pid, vec![instr])),
        }
    }

    let mut parser = PythonDefinedTypes::new(idls);
    let mut needs = PythonProgramImports::default();
    let mut used_module_names: HashSet<String> = HashSet::new();
    let mut sections: Vec<String> = Vec::new();
    let mut exports: Vec<String> = Vec::new();
    let mut program_entries: Vec<String> = Vec::new();
    let mut read_entries: Vec<(String, String)> = Vec::new(); // (key, descriptor fn)
    let mut omitted_reads: Vec<(String, String)> = Vec::new(); // (key, reason)

    for (index, (program_id, group)) in groups.iter().enumerate() {
        let idl = idls
            .iter()
            .find(|idl| idl.program_id.as_deref() == Some(program_id.as_str()));
        let raw_name = match idl {
            Some(idl) => idl.name.clone(),
            None if index == 0 => stack_name.to_string(),
            None => format!("program{}", index),
        };
        let mut module_name = python_module_name(&raw_name);
        if module_name.is_empty() {
            module_name = format!("program{}", index);
        }
        while !used_module_names.insert(module_name.clone()) {
            module_name.push('_');
        }
        let const_prefix = module_name.to_uppercase();
        let pascal_prefix = to_pascal_case(&module_name);

        // PDA registry lookup: this program's group first, then any group.
        let own_pdas = idl.and_then(|idl| pdas.get(idl.name.as_str()));
        let mut pda_lookup: BTreeMap<&str, &PdaDefinition> = BTreeMap::new();
        if let Some(own) = own_pdas {
            for (name, def) in own {
                pda_lookup.insert(name.as_str(), def);
            }
        }
        for group_pdas in pdas.values() {
            for (name, def) in group_pdas {
                pda_lookup.entry(name.as_str()).or_insert(def);
            }
        }

        let program_errors = idl
            .map(|idl| dedupe_errors_by_code(&idl.errors))
            .unwrap_or_default();
        if !program_errors.is_empty() {
            needs.error_metadata = true;
        }
        let errors_const = format!("{const_prefix}_ERRORS");

        let mut blocks: Vec<String> = Vec::new();
        let mut raw_entries: Vec<(String, String)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        for instr in group {
            let errors_expr = if instr.errors.is_empty() {
                if program_errors.is_empty() {
                    "[]".to_string()
                } else {
                    format!("list({errors_const})")
                }
            } else {
                let deduped = dedupe_errors_by_code(&instr.errors);
                needs.error_metadata = true;
                format!(
                    "[\n{}\n        ]",
                    deduped
                        .iter()
                        .map(|error| format!(
                            "            ErrorMetadata(code={}, name={}, msg={}),",
                            error.code,
                            py_string_literal(&error.name),
                            py_string_literal(error.msg.as_deref().unwrap_or(""))
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            match generate_py_instruction_block(
                instr,
                find_instruction_snapshot(instr, idl),
                &errors_expr,
                !program_errors.is_empty(),
                &pda_lookup,
                &mut parser,
                &mut needs,
                &module_name,
                &pascal_prefix,
            ) {
                Ok(block) => {
                    raw_entries.push((block.raw_key.clone(), block.handler_fn.clone()));
                    exports.extend(block.exports.clone());
                    blocks.push(block.code);
                }
                Err(reason) => skipped.push((instr.name.clone(), reason)),
            }
        }

        // --- Program read layer: release identity + typed account read defs. ---
        let read_layer = match reads.iter().find(|r| r.program_id == *program_id) {
            Some(r) => Ok((
                r.program_spec_hash.clone(),
                r.program_release_hash.clone(),
                r.descriptor.clone(),
            )),
            None => resolve_program_read_layer(program_specs, program_id),
        };
        let mut account_read_entries: Vec<String> = Vec::new();
        if read_layer.is_ok() {
            needs.reads = true;
            let accounts = idl.map(|idl| idl.accounts.as_slice()).unwrap_or_default();
            for account in accounts {
                let Some(struct_name) = account_structs.get(&account.name).or_else(|| {
                    account_structs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&account.name))
                        .map(|(_, emitted)| emitted)
                }) else {
                    // No generated dataclass for this account type; no reader.
                    continue;
                };
                account_read_entries.push(format!(
                    "    {}: ProgramAccountReadDef(account={}, parser=models.{}_from_wire),",
                    py_string_literal(&to_snake_case(&account.name)),
                    py_string_literal(&account.name),
                    to_snake_case(struct_name),
                ));
            }
        }

        // --- Assemble the program section. ---
        let mut section = format!(
            "# {rule}\n# Program `{raw_name}` (program ID `{program_id}`)\n# {rule}\n",
            rule = "=".repeat(74),
            raw_name = raw_name,
            program_id = program_id,
        );
        if let Err(reason) = &read_layer {
            section.push_str(&format!("# Program read layer omitted: {}.\n", reason));
        }
        if !skipped.is_empty() {
            section.push_str("# Skipped instructions (unsupported by instruction codegen):\n");
            for (name, reason) in &skipped {
                section.push_str(&format!("# - `{}`: {}\n", name, reason));
            }
        }
        section.push('\n');
        section.push_str(&format!(
            "{const_prefix}_PROGRAM_ID = {}\n",
            py_string_literal(program_id)
        ));
        exports.push(format!("{const_prefix}_PROGRAM_ID"));

        let mut spec_hash_expr = "None".to_string();
        if let Ok((spec_hash, release_hash, descriptor)) = &read_layer {
            let descriptor_expr = match descriptor {
                Some(descriptor) => {
                    needs.wire_read_descriptor = true;
                    let json = serde_json::to_string(descriptor)
                        .expect("program read descriptor must serialize");
                    format!(
                        "program_read_descriptor_from_wire(json.loads({}))",
                        py_string_literal(&json),
                    )
                }
                None => format!(
                    "ProgramReadDescriptor(\n        release=ProgramReleaseReference(\n            program_release_hash={const_prefix}_PROGRAM_RELEASE_HASH,\n            program_spec_hash={const_prefix}_PROGRAM_SPEC_HASH,\n        ),\n        transport=LocalHttpTransportDef(),\n    )"
                ),
            };
            section.push_str(&format!(
                "\n#: Content hash of the exact program specification captured at generation time.\n{const_prefix}_PROGRAM_SPEC_HASH = {spec}\n\n#: Release identity addressing hosted account reads for this program.\n{const_prefix}_PROGRAM_RELEASE_HASH = {release}\n\n\ndef {module_name}_read_descriptor() -> ProgramReadDescriptor:\n    \"\"\"Exact release-addressed read descriptor for program `{module_name}`.\"\"\"\n    return {descriptor_expr}\n",
                spec = py_string_literal(spec_hash),
                release = py_string_literal(release_hash),
            ));
            exports.extend([
                format!("{const_prefix}_PROGRAM_SPEC_HASH"),
                format!("{const_prefix}_PROGRAM_RELEASE_HASH"),
                format!("{module_name}_read_descriptor"),
            ]);
            spec_hash_expr = format!("{const_prefix}_PROGRAM_SPEC_HASH");
        }

        let errors_literal = if program_errors.is_empty() {
            "()".to_string()
        } else {
            format!(
                "(\n{}\n)",
                program_errors
                    .iter()
                    .map(|error| format!(
                        "    ErrorMetadata(code={}, name={}, msg={}),",
                        error.code,
                        py_string_literal(&error.name),
                        py_string_literal(error.msg.as_deref().unwrap_or(""))
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        section.push_str(&format!(
            "\n#: IDL error metadata for program `{module_name}`.\n{errors_const}: Tuple[ErrorMetadata, ...] = {errors_literal}\n",
        ));
        exports.push(errors_const.clone());

        let pdas_generated = generate_py_pdas(
            own_pdas.unwrap_or(&BTreeMap::new()),
            &module_name,
            &pascal_prefix,
            &mut needs,
        );
        let pdas_dict_expr = match &pdas_generated {
            Some((code, dict_name, class_name)) => {
                section.push('\n');
                section.push_str(code);
                section.push('\n');
                exports.push(class_name.clone());
                format!("dict({dict_name})")
            }
            None => "{}".to_string(),
        };

        for block in &blocks {
            section.push('\n');
            section.push_str(block);
            section.push('\n');
        }

        let accounts_expr = if account_read_entries.is_empty() {
            "{}".to_string()
        } else {
            let dict_name = format!("_{const_prefix}_ACCOUNTS");
            section.push_str(&format!(
                "\n{dict_name}: Dict[str, ProgramAccountReadDef] = {{\n{entries}\n}}\n",
                entries = account_read_entries.join("\n"),
            ));
            format!("dict({dict_name})")
        };

        let raw_dict = if raw_entries.is_empty() {
            "{}".to_string()
        } else {
            format!(
                "{{\n{}\n    }}",
                raw_entries
                    .iter()
                    .map(|(key, handler)| format!(
                        "        {}: {}(),",
                        py_string_literal(key),
                        handler
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let program_const = format!("{const_prefix}_PROGRAM");
        let gateway_kwarg = gateway
            .map(|gateway| {
                let json =
                    serde_json::to_string(gateway).expect("gateway descriptor must serialize");
                format!(
                    "\n    gateway=HostedSolanaGatewayBindings.from_dict(json.loads({})),",
                    py_string_literal(&json)
                )
            })
            .unwrap_or_default();
        section.push_str(&format!(
            "\n#: Portable program SDK definition consumed by `arete.stack`.\n{program_const} = ProgramDef(\n    name={name},\n    program_id={const_prefix}_PROGRAM_ID,\n    raw_instructions={raw_dict},\n    pdas={pdas_dict},\n    accounts={accounts},\n    errors={errors_const},\n    program_spec_hash={spec_hash},{gateway_kwarg}\n)\n",
            name = py_string_literal(&raw_name),
            raw_dict = raw_dict,
            pdas_dict = pdas_dict_expr,
            accounts = accounts_expr,
            spec_hash = spec_hash_expr,
            gateway_kwarg = gateway_kwarg,
        ));
        exports.push(program_const.clone());

        program_entries.push(format!(
            "    {}: {},",
            py_string_literal(&module_name),
            program_const
        ));
        match &read_layer {
            Ok(_) => read_entries.push((
                module_name.clone(),
                format!("{module_name}_read_descriptor()"),
            )),
            Err(reason) => omitted_reads.push((module_name.clone(), reason.clone())),
        }

        sections.push(section);
    }

    // --- Imports (only what the generated code references). ---
    let mut instruction_imports: Vec<&str> = Vec::new();
    if needs.account_meta {
        instruction_imports.extend(["AccountMeta", "Known", "Signer", "UserProvided"]);
    }
    if needs.pda {
        instruction_imports.extend([
            "AccountRefSeed",
            "ArgRefSeed",
            "BytesSeed",
            "LiteralSeed",
            "PdaConfig",
        ]);
        if needs.account_meta {
            instruction_imports.push("Pda");
        }
    }
    if needs.arg_schema {
        instruction_imports.push("ArgSchema");
    }
    if needs.builders {
        instruction_imports.extend(["BuiltAccountMeta", "BuiltInstruction", "InstructionHandler"]);
    }
    if needs.error_metadata {
        instruction_imports.push("ErrorMetadata");
    }
    instruction_imports.sort_unstable();
    instruction_imports.dedup();

    let mut import_lines: Vec<String> = Vec::new();
    let mut typing_names: Vec<&str> = vec!["Dict", "Tuple"];
    if needs.builders {
        typing_names.extend(["Any", "Mapping", "Optional", "Sequence", "TypedDict"]);
    }
    typing_names.sort_unstable();
    typing_names.dedup();
    import_lines.push(format!("from typing import {}", typing_names.join(", ")));
    if needs.wire_read_descriptor || gateway.is_some() {
        import_lines.push("import json".to_string());
    }
    import_lines.push(String::new());
    if !instruction_imports.is_empty() {
        import_lines.push(format!(
            "from arete.instructions import (\n{}\n)",
            instruction_imports
                .iter()
                .map(|name| format!("    {},", name))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if needs.reads {
        let wire_import = if needs.wire_read_descriptor {
            "\n    program_read_descriptor_from_wire,"
        } else {
            ""
        };
        import_lines.push(format!(
            "from arete.program_read_transport import (\n    LocalHttpTransportDef,\n    ProgramReadDescriptor,\n    ProgramReleaseReference,{wire_import}\n)"
        ));
        import_lines.push("from arete.read import ProgramAccountReadDef".to_string());
    }
    let mut stack_imports = vec!["ProgramDef"];
    if needs.pda_factory {
        stack_imports.push("PdaFactory");
    }
    stack_imports.sort_unstable();
    import_lines.push(format!(
        "from arete.stack import {}",
        stack_imports.join(", ")
    ));
    if gateway.is_some() {
        import_lines.push("from arete.gateway import HostedSolanaGatewayBindings".to_string());
    }
    if needs.reads {
        import_lines.push(String::new());
        import_lines.push("from . import models".to_string());
    }

    // --- PROGRAMS / PROGRAM_READS maps. ---
    exports.extend(["PROGRAMS".to_string(), "PROGRAM_READS".to_string()]);
    let programs_map = format!(
        "PROGRAMS: Dict[str, ProgramDef] = {{\n{}\n}}\n",
        program_entries.join("\n")
    );
    // `StackDef.program_reads` keys must exactly match `programs` keys when
    // non-empty, so a partial read layer degrades to an empty map (callers
    // can still pass per-program `program_reads` overrides at connect time).
    let reads_map = if omitted_reads.is_empty() && !read_entries.is_empty() {
        format!(
            "PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {{\n{}\n}}\n",
            read_entries
                .iter()
                .map(|(key, expr)| format!("    {}: {},", py_string_literal(key), expr))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        let mut comment = String::new();
        for (key, reason) in &omitted_reads {
            comment.push_str(&format!("# - `{}`: {}\n", key, reason));
        }
        let header = if omitted_reads.is_empty() {
            String::new()
        } else {
            format!(
                "# Program reads omitted (program_reads keys must exactly match programs):\n{comment}"
            )
        };
        let annotation = if needs.reads {
            "Dict[str, ProgramReadDescriptor]"
        } else {
            "Dict[str, object]"
        };
        format!("{header}PROGRAM_READS: {annotation} = {{}}\n")
    };

    let all_list = exports
        .iter()
        .map(|name| format!("    {},", py_string_literal(name)))
        .collect::<Vec<_>>()
        .join("\n");

    let code = format!(
        r#""""Generated program SDKs for the `{stack_name}` stack. Do not edit.

Instruction building is pure (no network access). Each program section
exposes `<PROG>_PROGRAM_ID`, `<Ix>Params` TypedDicts, `<prog>_<ix>(**params)`
builders returning `BuiltInstruction`, raw `<prog>_<ix>_handler()` escape
hatches, and a `<Prog>Pdas` namespace of PDA factories. Programs with a
recorded program spec additionally expose `<PROG>_PROGRAM_SPEC_HASH` /
`<PROG>_PROGRAM_RELEASE_HASH` plus `<prog>_read_descriptor()` for
release-addressed HTTP reads. `PROGRAMS` / `PROGRAM_READS` compose with stack
bindings and standalone session members.
"""

from __future__ import annotations

{imports}

__all__ = [
{all_list}
]


{sections}

{programs_map}
{reads_map}"#,
        stack_name = stack_name,
        imports = import_lines.join("\n"),
        all_list = all_list,
        sections = sections.join("\n\n"),
        programs_map = programs_map,
        reads_map = reads_map,
    );

    Some(PythonProgramsCodegen { code })
}

// ============================================================================
// Naming + literal helpers
// ============================================================================

fn py_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

/// Render a Python double-quoted string literal.
fn py_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render an indented triple-quoted docstring (closing quotes on their own
/// line so content can never terminate the literal early).
fn py_docstring(lines: &[String], indent: &str) -> String {
    let mut out = format!("{indent}\"\"\"");
    for (index, line) in lines.iter().enumerate() {
        let sanitized = line.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
        if index == 0 {
            out.push_str(&sanitized);
        } else if sanitized.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("\n{indent}{sanitized}"));
        }
    }
    out.push_str(&format!("\n{indent}\"\"\""));
    out
}

fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '-', '.', ':'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('_');
            }
            separator = false;
            if ch.is_ascii_uppercase() {
                if !result.is_empty() && !result.ends_with('_') {
                    result.push('_');
                }
                result.push(ch.to_ascii_lowercase());
            } else {
                result.push(ch.to_ascii_lowercase());
            }
        } else {
            separator = true;
        }
    }
    if result
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        result.insert_str(0, "value_");
    }
    if is_python_keyword(&result) {
        result.push('_');
    }
    result
}

fn to_screaming_snake(s: &str) -> String {
    to_snake_case(s).to_uppercase()
}

/// Derive a valid Python module name from an arbitrary alias or file stem
/// (lowercased, non-alphanumerics collapsed to `_`, keywords and leading
/// digits escaped). Shared with the CLI so staged devex extension files wire
/// up under the same stems the composition generator would use. Mirror of
/// [`crate::rust::rust_module_name`].
pub fn python_module_name(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('_');
            }
            separator = false;
            output.push(character.to_ascii_lowercase());
        } else {
            separator = true;
        }
    }
    if output
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        output.insert_str(0, "live_");
    }
    if is_python_keyword(&output) {
        output.push_str("_live");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::process::Command;

    fn identity_spec() -> IdentitySpec {
        IdentitySpec {
            primary_keys: vec!["id.address".to_string()],
            lookup_indexes: vec![],
        }
    }

    const TEST_PROGRAM_ID: &str = "Prog111111111111111111111111111111111111111";

    fn minimal_entity(name: &str) -> SerializableStreamSpec {
        SerializableStreamSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            state_name: name.to_string(),
            program_id: None,
            idl: None,
            identity: identity_spec(),
            handlers: vec![],
            sections: vec![EntitySection {
                name: "id".to_string(),
                fields: vec![FieldTypeInfo::new(
                    "address".to_string(),
                    "String".to_string(),
                )],
                is_nested_struct: false,
                parent_field: None,
            }],
            field_mappings: BTreeMap::new(),
            resolver_hooks: vec![],
            instruction_hooks: vec![],
            resolver_specs: vec![],
            computed_fields: vec![],
            computed_field_specs: vec![],
            content_hash: None,
            views: vec![],
        }
    }

    fn test_idl() -> IdlSnapshot {
        IdlSnapshot {
            name: "demo".to_string(),
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            version: "0.1.0".to_string(),
            accounts: vec![],
            instructions: vec![],
            types: vec![],
            events: vec![],
            errors: vec![IdlErrorSnapshot {
                code: 6000,
                name: "SlippageExceeded".to_string(),
                msg: Some("Slippage exceeded".to_string()),
            }],
            discriminant_size: 8,
        }
    }

    fn instruction_account(name: &str, resolution: AccountResolution) -> InstructionAccountDef {
        InstructionAccountDef {
            name: name.to_string(),
            is_signer: matches!(resolution, AccountResolution::Signer),
            is_writable: true,
            resolution,
            is_optional: false,
            docs: vec![],
        }
    }

    fn instruction_arg(name: &str, arg_type: &str) -> InstructionArgDef {
        InstructionArgDef {
            name: name.to_string(),
            arg_type: arg_type.to_string(),
            docs: vec![],
            amount_hint: None,
        }
    }

    fn programs_stack_spec() -> SerializableStackSpec {
        let mut demo_pdas = BTreeMap::new();
        demo_pdas.insert(
            "counter".to_string(),
            PdaDefinition {
                name: "counter".to_string(),
                seeds: vec![
                    PdaSeedDef::Literal {
                        value: "counter".to_string(),
                    },
                    PdaSeedDef::AccountRef {
                        account_name: "authority".to_string(),
                    },
                ],
                program_id: None,
                program: None,
            },
        );
        let mut pdas = BTreeMap::new();
        pdas.insert("demo".to_string(), demo_pdas);

        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Demo".to_string(),
            program_ids: vec![TEST_PROGRAM_ID.to_string()],
            idls: vec![test_idl()],
            program_specs: vec![],
            entities: vec![minimal_entity("DemoThing")],
            pdas,
            instructions: vec![InstructionDef {
                name: "doThing".to_string(),
                discriminator: vec![12, 34],
                discriminator_size: 2,
                accounts: vec![
                    instruction_account("signer", AccountResolution::Signer),
                    instruction_account("authority", AccountResolution::UserProvided),
                    instruction_account(
                        "counter",
                        AccountResolution::PdaRef {
                            pda_name: "counter".to_string(),
                        },
                    ),
                    instruction_account(
                        "systemProgram",
                        AccountResolution::Known {
                            address: "11111111111111111111111111111111".to_string(),
                        },
                    ),
                ],
                args: vec![
                    instruction_arg("roundId", "u64"),
                    instruction_arg("admin", "solana_pubkey::Pubkey"),
                    instruction_arg("tip", "Option<u64>"),
                ],
                errors: vec![],
                program_id: Some(TEST_PROGRAM_ID.to_string()),
                docs: vec!["Does the thing.".to_string()],
            }],
            content_hash: None,
        }
    }

    #[test]
    fn python_generator_supports_path_qualified_defined_types() {
        let qualified_name =
            "sb_on_demand::actions::pull_feed::pull_feed_submit_response_action::Submission";
        let emitted_name = "SbOnDemandActionsPullFeedPullFeedSubmitResponseActionSubmission";
        assert_eq!(to_pascal_case(qualified_name), emitted_name);

        let mut spec = programs_stack_spec();
        spec.idls[0].types.push(IdlTypeDefSnapshot {
            name: qualified_name.to_string(),
            docs: vec![],
            serialization: None,
            type_def: IdlTypeDefKindSnapshot::Struct {
                kind: "struct".to_string(),
                fields: vec![IdlFieldSnapshot {
                    name: "value".to_string(),
                    type_: IdlTypeSnapshot::Simple("u64".to_string()),
                    amount_hint: None,
                }],
            },
        });
        spec.instructions.push(InstructionDef {
            name: "submit".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![instruction_arg("submission", qualified_name)],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });
        spec.entities[0].sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![snapshot_field("submission", qualified_name, false, false)],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(spec, None).expect("qualified types should generate");
        assert!(output.models_py.contains(&format!("class {emitted_name}:")));
        assert!(!output.models_py.contains("class SbOnDemand::"));
        let programs = output.programs_py.expect("program module");
        assert!(programs.contains("DemoSubmitParams = TypedDict("));
        assert!(programs.contains("\"submission\": Any,"));
        assert!(programs.contains("{\"name\": \"value\", \"type\": \"u64\"}"));
        assert!(!programs.contains("`submit`: arg 'submission' has unsupported type"));
    }

    #[test]
    fn python_generator_supports_inline_tuples_from_idl_snapshots() {
        let mut spec = programs_stack_spec();
        spec.idls[0].types = vec![
            IdlTypeDefSnapshot {
                name: "HookableLifecycleEvent".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Enum {
                    kind: "enum".to_string(),
                    variants: vec![
                        IdlEnumVariantSnapshot {
                            name: "Create".to_string(),
                            fields: vec![],
                        },
                        IdlEnumVariantSnapshot {
                            name: "Transfer".to_string(),
                            fields: vec![],
                        },
                    ],
                },
            },
            IdlTypeDefSnapshot {
                name: "ExternalCheckResult".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![IdlFieldSnapshot {
                        name: "flags".to_string(),
                        type_: IdlTypeSnapshot::Simple("u32".to_string()),
                        amount_hint: None,
                    }],
                },
            },
            IdlTypeDefSnapshot {
                name: "AgentIdentityInitInfo".to_string(),
                docs: vec![],
                serialization: None,
                type_def: IdlTypeDefKindSnapshot::Struct {
                    kind: "struct".to_string(),
                    fields: vec![IdlFieldSnapshot {
                        name: "lifecycleChecks".to_string(),
                        type_: IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                            vec: Box::new(IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
                                tuple: vec![
                                    IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                                        defined: IdlDefinedInnerSnapshot::Named {
                                            name: "HookableLifecycleEvent".to_string(),
                                        },
                                    }),
                                    IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                                        defined: IdlDefinedInnerSnapshot::Named {
                                            name: "ExternalCheckResult".to_string(),
                                        },
                                    }),
                                ],
                            })),
                            length_prefix: None,
                        }),
                        amount_hint: None,
                    }],
                },
            },
        ];
        spec.idls[0].instructions.push(IdlInstructionSnapshot {
            name: "tupleThing".to_string(),
            discriminator: vec![7],
            discriminant: None,
            docs: vec![],
            accounts: vec![],
            args: vec![
                IdlFieldSnapshot {
                    name: "payload".to_string(),
                    type_: IdlTypeSnapshot::Defined(IdlDefinedTypeSnapshot {
                        defined: IdlDefinedInnerSnapshot::Named {
                            name: "AgentIdentityInitInfo".to_string(),
                        },
                    }),
                    amount_hint: None,
                },
                IdlFieldSnapshot {
                    name: "pair".to_string(),
                    type_: IdlTypeSnapshot::Tuple(IdlTupleTypeSnapshot {
                        tuple: vec![
                            IdlTypeSnapshot::Simple("u8".to_string()),
                            IdlTypeSnapshot::Simple("u16".to_string()),
                        ],
                    }),
                    amount_hint: None,
                },
            ],
        });
        spec.instructions.push(InstructionDef {
            name: "tupleThing".to_string(),
            discriminator: vec![7],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![
                instruction_arg("payload", "AgentIdentityInitInfo"),
                instruction_arg("pair", "(u8, u16)"),
            ],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });

        let output = compile_stack_spec(spec, None).expect("inline tuples should generate");
        let programs = output.programs_py.expect("program module");
        assert!(programs.contains("DemoTupleThingParams = TypedDict("));
        assert!(programs.contains("\"pair\": Any,"));
        assert!(programs.contains("{\"tuple\": [\"u8\", \"u16\"]}"));
        assert!(programs.contains(
            "{\"vec\": {\"tuple\": [{\"enum\": [\"Create\", \"Transfer\"]}, {\"struct\":"
        ));
        assert!(!programs.contains("`tupleThing`: arg 'pair' has unsupported type"));
    }

    #[test]
    fn python_generator_aliases_arg_account_collisions_and_pda_refs() {
        let mut spec = programs_stack_spec();
        spec.instructions = vec![InstructionDef {
            name: "decompressV1".to_string(),
            discriminator: vec![8],
            discriminator_size: 1,
            accounts: vec![
                instruction_account("metadata", AccountResolution::UserProvided),
                instruction_account(
                    "record",
                    AccountResolution::PdaInline {
                        seeds: vec![PdaSeedDef::AccountRef {
                            account_name: "metadata".to_string(),
                        }],
                        program_id: None,
                        program: None,
                    },
                ),
            ],
            args: vec![instruction_arg("metadata", "u8")],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        }];

        let output = compile_stack_spec(spec, None).expect("collision should generate");
        let programs = output.programs_py.expect("program module");
        assert!(programs.contains("\"metadata\": int,"));
        assert!(programs.contains("\"metadataAccount\": str,"));
        assert!(programs.contains("name=\"metadataAccount\","));
        assert!(programs.contains("AccountRefSeed(\"metadataAccount\")"));
        assert!(programs.contains(
            "account `metadata` collides with an instruction arg and is exposed as `metadataAccount`"
        ));
        assert!(!programs.contains("has no typed override field"));
    }

    #[test]
    fn python_generator_disambiguates_leading_underscore_fields() {
        let mut metadata = snapshot_field(
            "migrationMetadata",
            "MeteoraDammMigrationMetadata",
            true,
            false,
        );
        metadata.resolved_type.as_mut().unwrap().fields = vec![
            resolved_field("padding_0", "u8", BaseType::Integer),
            resolved_field("_padding_0", "u8", BaseType::Integer),
        ];
        let mut entity = minimal_entity("Migration");
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![metadata],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(stack_of("Migration", entity), None)
            .expect("padding fields should generate");
        let models = output.models_py;
        assert!(models.contains("class MeteoraDammMigrationMetadata:"));
        assert_eq!(
            models
                .matches("    padding_0: Optional[int] = None")
                .count(),
            1
        );
        assert_eq!(
            models
                .matches("    _padding_0: Optional[int] = None")
                .count(),
            1
        );
        assert_eq!(models.matches("        padding_0=").count(), 1);
        assert_eq!(models.matches("        _padding_0=").count(), 1);
        assert!(models.contains("_require(data, \"_padding_0\", \"MeteoraDammMigrationMetadata\")"));

        let path = std::env::temp_dir().join(format!(
            "arete-python-padding-codegen-{}-{:?}.py",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, &models).expect("generated Python model should write");
        let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
        let compiled = Command::new(python)
            .args([
                "-c",
                "import pathlib, sys; compile(pathlib.Path(sys.argv[1]).read_text(), sys.argv[1], 'exec')",
            ])
            .arg(&path)
            .output()
            .expect("Python must be available for generated syntax checks");
        let _ = std::fs::remove_file(&path);
        assert!(
            compiled.status.success(),
            "generated padding model failed Python syntax validation:\n{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
    }

    #[test]
    fn python_generator_renames_account_types_on_collision() {
        let plan_field = FieldTypeInfo {
            field_name: "plan".to_string(),
            raw_name: Some("plan".to_string()),
            canonical_name: Some("plan".to_string()),
            rust_type_name: "Option<serde_json::Value>".to_string(),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: false,
            is_array: false,
            inner_type: Some("Value".to_string()),
            source_path: None,
            resolved_type: Some(ResolvedStructType {
                type_name: "plan".to_string(),
                fields: vec![],
                is_instruction: false,
                is_account: true,
                is_event: false,
                is_enum: false,
                enum_variants: vec![],
            }),
            emit: true,
        };

        let mut spec = minimal_entity("Plan");
        spec.sections.push(EntitySection {
            name: "plan".to_string(),
            fields: vec![plan_field],
            is_nested_struct: false,
            parent_field: None,
        });
        let stack = SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Plan".to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![spec],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        };

        let output = compile_stack_spec(stack, None).expect("python generation should succeed");

        // The resolved account type is renamed off the reserved section name
        // and the section field is typed to the renamed dataclass.
        assert!(output.models_py.contains("class PlanAccount:"));
        assert!(output
            .models_py
            .contains("plan: Optional[PlanAccount] = None"));
        assert!(output
            .models_py
            .contains("plan=_convert(data.get(\"plan\"), plan_account_from_wire),"));
        // The `PlanPlan` section dataclass keeps its own name.
        assert!(output.models_py.contains("class PlanPlan:"));
    }

    #[test]
    fn python_generator_converts_integer_fields() {
        let mut entity = minimal_entity("Plan");
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![
                FieldTypeInfo::new("status".to_string(), "Option<u8>".to_string()),
                FieldTypeInfo::new("supply".to_string(), "u64".to_string()),
            ],
            is_nested_struct: false,
            parent_field: None,
        });
        let stack = SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: "Plan".to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![entity],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        };

        let output = compile_stack_spec(stack, None).expect("python generation should succeed");

        // u64 decimal strings convert to arbitrary-precision int.
        assert!(output.models_py.contains("status: Optional[int] = None"));
        assert!(output.models_py.contains("supply: Optional[int] = None"));
        assert!(output
            .models_py
            .contains("supply=_to_int(data.get(\"supply\")),"));
        assert!(output
            .models_py
            .contains("        out[\"supply\"] = _to_int(data[\"supply\"])"));
        // Strings pass through untransformed.
        assert!(output.models_py.contains("address=data.get(\"address\"),"));
    }

    /// Wrap one entity into a single-entity stack spec.
    fn stack_of(name: &str, entity: SerializableStreamSpec) -> SerializableStackSpec {
        SerializableStackSpec {
            ast_version: CURRENT_AST_VERSION.to_string(),
            stack_name: name.to_string(),
            program_ids: vec![],
            idls: vec![],
            program_specs: vec![],
            entities: vec![entity],
            pdas: BTreeMap::new(),
            instructions: vec![],
            content_hash: None,
        }
    }

    fn resolved_field(name: &str, field_type: &str, base_type: BaseType) -> ResolvedField {
        ResolvedField {
            field_name: name.to_string(),
            raw_name: Some(name.to_string()),
            canonical_name: None,
            field_type: field_type.to_string(),
            base_type,
            integer_kind: IntegerKind::from_rust_type(field_type),
            is_optional: false,
            is_array: field_type.starts_with('['),
        }
    }

    /// A root-section field backed by a resolved struct, plus the handler
    /// mapping that feeds it.
    fn snapshot_field(
        field_name: &str,
        type_name: &str,
        is_account: bool,
        is_event: bool,
    ) -> FieldTypeInfo {
        FieldTypeInfo {
            field_name: field_name.to_string(),
            raw_name: Some(field_name.to_string()),
            canonical_name: None,
            rust_type_name: "Option<serde_json::Value>".to_string(),
            base_type: BaseType::Object,
            integer_kind: None,
            is_optional: true,
            is_array: false,
            inner_type: Some("Value".to_string()),
            source_path: None,
            resolved_type: Some(ResolvedStructType {
                type_name: type_name.to_string(),
                fields: vec![
                    resolved_field("motherlode", "u64", BaseType::Integer),
                    resolved_field("owner", "publicKey", BaseType::Pubkey),
                ],
                is_instruction: false,
                is_account,
                is_event,
                is_enum: false,
                enum_variants: vec![],
            }),
            emit: true,
        }
    }

    fn capture_handler(target_path: &str) -> SerializableHandlerSpec {
        SerializableHandlerSpec {
            source: SourceSpec::Source {
                program_id: None,
                discriminator: None,
                type_name: "Treasury".to_string(),
                serialization: None,
                is_account: true,
            },
            key_resolution: KeyResolutionStrategy::Embedded {
                primary_field: FieldPath::new(&["id", "address"]),
            },
            mappings: vec![SerializableFieldMapping {
                target_path: target_path.to_string(),
                source: MappingSource::AsCapture {
                    field_transforms: BTreeMap::new(),
                },
                transform: None,
                population: PopulationStrategy::LastWrite,
                condition: None,
                when: None,
                stop: None,
                emit: true,
            }],
            conditions: vec![],
            emit: true,
        }
    }

    /// Defect 1: `#[capture]`-fed account fields and event fields arrive
    /// wrapped on the wire (`{timestamp, account_address, data: {...}}`).
    /// Parsing the envelope as the bare struct silently nulls every field.
    #[test]
    fn python_generator_wraps_capture_and_event_fields() {
        let mut entity = minimal_entity("OreTreasury");
        entity.handlers.push(capture_handler("treasury_snapshot"));
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![
                snapshot_field("treasury_snapshot", "Treasury", true, false),
                // Same struct kind, but no AsCapture mapping: stays unwrapped.
                snapshot_field("plain_account", "Vault", true, false),
                snapshot_field("deposit_event", "DepositEvent", false, true),
            ],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(stack_of("OreTreasury", entity), None)
            .expect("python generation should succeed");
        let models = &output.models_py;

        // Both envelopes are emitted and exported.
        assert!(models.contains("class CaptureWrapper(Generic[_T]):"));
        assert!(models.contains("class EventWrapper(Generic[_T]):"));
        assert!(models.contains("def capture_wrapper_from_wire(value: Any, converter: Any = None)"));
        assert!(models.contains("    \"CaptureWrapper\","));
        assert!(models.contains("    \"capture_wrapper_from_wire\","));

        // Capture-fed account field: wrapper annotation + wrapper converter.
        assert!(models.contains("treasury_snapshot: Optional[CaptureWrapper[Treasury]] = None"));
        assert!(models.contains(
            "treasury_snapshot=_convert_capture(data.get(\"treasury_snapshot\"), treasury_from_wire),"
        ));
        assert!(models.contains(
            "out[\"treasury_snapshot\"] = _convert_capture(data[\"treasury_snapshot\"], treasury_from_wire)"
        ));

        // Event field: EventWrapper envelope.
        assert!(models.contains("deposit_event: Optional[EventWrapper[DepositEvent]] = None"));
        assert!(models.contains(
            "deposit_event=_convert_event(data.get(\"deposit_event\"), deposit_event_from_wire),"
        ));

        // Unmapped account field keeps the bare-struct shape.
        assert!(models.contains("plain_account: Optional[Vault] = None"));
        assert!(models
            .contains("plain_account=_convert(data.get(\"plain_account\"), vault_from_wire),"));
        assert!(!models.contains("_convert_capture(data.get(\"plain_account\")"));
    }

    /// Defect 2: `Vec<u64>` entity fields are stored as `BaseType::Array` with
    /// an explicit `integer_kind`, so u64 decimal strings must still convert to
    /// `int` (the IDL path already did; the entity path did not).
    #[test]
    fn python_generator_converts_u64_arrays() {
        let mut entity = minimal_entity("OreRound");
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![
                FieldTypeInfo::new(
                    "deployed_per_square".to_string(),
                    "Option<Vec<u64>>".to_string(),
                ),
                FieldTypeInfo::new(
                    "deployed_per_square_ui".to_string(),
                    "Option<Vec<f64>>".to_string(),
                ),
                FieldTypeInfo::new("labels".to_string(), "Vec<String>".to_string()),
            ],
            is_nested_struct: false,
            parent_field: None,
        });
        // The interpreter records the element kind explicitly for `Vec<u64>`.
        for field in &mut entity.sections[1].fields {
            if field.field_name == "deployed_per_square" {
                field.base_type = BaseType::Array;
                field.integer_kind = Some(IntegerKind::U64);
                field.is_array = true;
                field.inner_type = Some("Vec < u64 >".to_string());
            }
        }

        let output = compile_stack_spec(stack_of("OreRound", entity), None)
            .expect("python generation should succeed");
        let models = &output.models_py;

        assert!(models.contains("deployed_per_square: Optional[List[int]] = None"));
        assert!(
            models.contains("deployed_per_square=_to_int_list(data.get(\"deployed_per_square\")),")
        );
        assert!(models.contains(
            "out[\"deployed_per_square\"] = _to_int_list(data[\"deployed_per_square\"])"
        ));
        assert!(!models.contains("deployed_per_square: Optional[List[Any]] = None"));

        // Non-integer scalar arrays keep their element type and pass through.
        assert!(models.contains("deployed_per_square_ui: Optional[List[float]] = None"));
        assert!(models.contains("deployed_per_square_ui=data.get(\"deployed_per_square_ui\"),"));
        assert!(models.contains("labels: Optional[List[str]] = None"));
    }

    /// Defect 4: IDL struct converters reject payloads that omit a required
    /// key so `arete.read`'s SCHEMA_VALIDATION guard is reachable; entity
    /// sections and every patch converter stay loose.
    #[test]
    fn python_generator_emits_strict_idl_struct_converters() {
        let mut entity = minimal_entity("OreTreasury");
        entity.handlers.push(capture_handler("treasury_snapshot"));
        entity.sections.push(EntitySection {
            name: "root".to_string(),
            fields: vec![snapshot_field("treasury_snapshot", "Treasury", true, false)],
            is_nested_struct: false,
            parent_field: None,
        });

        let output = compile_stack_spec(stack_of("OreTreasury", entity), None)
            .expect("python generation should succeed");
        let models = &output.models_py;

        assert!(models.contains("def _require(data: Mapping[str, Any], key: str, context: str)"));
        assert!(
            models.contains("motherlode=_to_int(_require(data, \"motherlode\", \"Treasury\")),")
        );
        assert!(models.contains("owner=_require(data, \"owner\", \"Treasury\"),"));
        // Patch converters stay loose.
        assert!(models.contains("        out[\"motherlode\"] = _to_int(data[\"motherlode\"])"));
        // Entity sections stay loose.
        assert!(models.contains("address=data.get(\"address\"),"));
    }

    /// Defect 3: `payer` is a real IDL account name, so the reserved
    /// keyword-only fallback is spelled `wallet` (matching the TypeScript
    /// `BuildOptions.wallet`) and `payer=` reaches the account overrides.
    #[test]
    fn python_generator_reserves_wallet_not_payer_on_builders() {
        let mut stack = programs_stack_spec();
        stack.instructions[0]
            .accounts
            .push(instruction_account("payer", AccountResolution::Signer));

        let output = compile_stack_spec(stack, None).expect("python generation should succeed");
        let programs = output
            .programs_py
            .expect("stack with instructions should emit programs.py");

        assert!(programs.contains("    wallet: Optional[str] = None,"));
        assert!(programs.contains("        payer=wallet,"));
        // No reserved kwarg may shadow an IDL account name.
        assert!(!programs.contains("    payer: Optional[str] = None,"));
        // `payer` stays an advertised account override in the params TypedDict.
        assert!(programs.contains("\"payer\": str,"));
        // The remaining reserved options keep their names.
        assert!(programs.contains("    accounts: Optional[Mapping[str, str]] = None,"));
        assert!(programs
            .contains("    remaining_accounts: Optional[Sequence[BuiltAccountMeta]] = None,"));
    }

    #[test]
    fn generated_pyproject_uses_published_arete_sdk_package() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("python generation should succeed");

        assert!(output
            .pyproject_toml
            .contains("dependencies = [\"arete-sdk>=0.4\"]"));
        assert!(output.pyproject_toml.contains("name = \"generated-stack\""));
        assert_eq!(output.module_name, "generated_stack");
    }

    #[test]
    fn python_generator_emits_program_sdk_module() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("python stack generation should succeed");
        let programs = output
            .programs_py
            .expect("programs.py should be generated for stacks with instructions");

        assert!(programs.contains(&format!("DEMO_PROGRAM_ID = \"{}\"", TEST_PROGRAM_ID)));

        // Typed params: wire-name keys, args first, then account overrides.
        assert!(programs.contains("DemoDoThingParams = TypedDict("));
        assert!(programs.contains("        # arg `roundId` (`u64`)\n        \"roundId\": int,"));
        assert!(programs.contains("\"admin\": str,"));
        assert!(programs.contains("\"tip\": Optional[int],"));
        assert!(programs.contains(
            "        # Optional address override for the `signer` signer (defaults to the payer).\n        \"signer\": str,"
        ));
        assert!(programs.contains(
            "        # Address of the `authority` account.\n        \"authority\": str,"
        ));
        assert!(programs.contains("total=False,"));

        // Handler literal fragments.
        assert!(programs.contains("discriminator=bytes([12, 34])"));
        assert!(programs.contains("resolution=Signer(),"));
        assert!(programs.contains("resolution=Known(\"11111111111111111111111111111111\"),"));
        assert!(programs.contains(
            "resolution=Pda(PdaConfig(seeds=(LiteralSeed(\"counter\"), AccountRefSeed(\"authority\")))),"
        ));
        assert!(programs.contains("ArgSchema(name=\"roundId\", type=\"u64\"),"));
        assert!(programs.contains("ArgSchema(name=\"tip\", type={\"option\": \"u64\"}),"));
        assert!(programs.contains("ArgSchema(name=\"admin\", type=\"pubkey\"),"));
        assert!(programs.contains(
            "ErrorMetadata(code=6000, name=\"SlippageExceeded\", msg=\"Slippage exceeded\"),"
        ));

        // Builder + raw handler escape hatch.
        assert!(programs.contains("def demo_do_thing("));
        assert!(programs.contains("def demo_do_thing_handler() -> InstructionHandler:"));
        assert!(programs.contains("return demo_do_thing_handler().build("));

        // PDA factory namespace.
        assert!(programs.contains("class DemoPdas:"));
        assert!(programs.contains(
            "counter = PdaFactory(\"counter\", _DEMO_PDAS[\"counter\"], DEMO_PROGRAM_ID)"
        ));

        // Program definition + stack maps.
        assert!(programs.contains("DEMO_PROGRAM = ProgramDef("));
        assert!(programs.contains(
            "    raw_instructions={\n        \"do_thing\": demo_do_thing_handler(),\n    },"
        ));
        assert!(programs
            .contains("PROGRAMS: Dict[str, ProgramDef] = {\n    \"demo\": DEMO_PROGRAM,\n}"));

        // No program spec recorded: the read layer is omitted with a note.
        assert!(programs.contains(
            "# Program read layer omitted: no program specification was recorded for this program."
        ));
        assert!(!programs.contains("DEMO_PROGRAM_SPEC_HASH ="));
        assert!(!programs.contains("def demo_read_descriptor"));
        assert!(programs.contains("PROGRAM_READS: Dict[str, object] = {}"));

        // Stack wiring.
        assert!(output
            .init_py
            .contains("from . import models, programs, views"));
        assert!(output.init_py.contains("programs=programs.PROGRAMS,"));
        assert!(output
            .init_py
            .contains("program_reads=programs.PROGRAM_READS,"));
        assert!(output.init_py.contains("DEMO_STACK: StackDef = StackDef("));
        assert!(output.init_py.contains("name=\"demo\","));
    }

    #[test]
    fn python_program_compiler_emits_no_view_or_stack_shell() {
        let output = compile_program_modules(
            programs_stack_spec(),
            Some(PythonStackConfig {
                package_name: "demo-program".to_string(),
                program_reads: vec![PythonProgramReadConfig {
                    program_id: TEST_PROGRAM_ID.to_string(),
                    program_spec_hash: "arete:h1:program-spec:sha256:test".to_string(),
                    program_release_hash: "arete:h1:program-release:sha256:test".to_string(),
                    descriptor: Some(serde_json::json!({
                        "release": {
                            "programReleaseHash": "arete:h1:program-release:sha256:test",
                            "programSpecHash": "arete:h1:program-spec:sha256:test"
                        },
                        "transport": {
                            "kind": "hosted-binding",
                            "binding": {
                                "endpoint": "https://reads.example.test",
                                "programReadBindingId": "prb_00000000000000000000000000000001",
                                "auth": {
                                    "sessionEndpoint": "https://auth.example.test/session",
                                    "targetKind": "program-read-binding",
                                    "targetId": "prb_00000000000000000000000000000001"
                                }
                            }
                        }
                    })),
                }],
                ..Default::default()
            }),
        )
        .expect("standalone program generation should succeed");

        assert!(output.init_py.contains("from . import models, programs"));
        assert!(output.init_py.contains("*programs.__all__"));
        assert!(!output.init_py.contains("views"));
        assert!(!output.init_py.contains("StackDef"));
        assert!(output
            .programs_py
            .contains("PROGRAMS: Dict[str, ProgramDef]"));
        assert!(output.programs_py.contains("PROGRAM_READS:"));

        let base = std::env::temp_dir().join(format!(
            "arete-python-program-codegen-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        write_python_program_package(&output, &base).expect("program package should write");
        assert!(base.join("demo_program/programs.py").is_file());
        assert!(base.join("demo_program/models.py").is_file());
        assert!(!base.join("demo_program/views.py").exists());

        // Consumer smoke test: import the written package against the real
        // local Python SDK. `httpx` is stubbed because generated program
        // definitions are pure and the Rust CI job intentionally does not
        // install Python's optional network dependencies.
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("interpreter crate lives in the repo root");
        let stubs = base.join("consumer-stubs");
        std::fs::create_dir_all(&stubs).expect("consumer stub directory");
        std::fs::write(stubs.join("httpx.py"), "# Import-only consumer stub.\n")
            .expect("httpx import stub");
        let mut python_paths = vec![base.clone(), stubs, repo_root.join("python/arete-sdk")];
        if let Some(existing) = std::env::var_os("PYTHONPATH") {
            python_paths.extend(std::env::split_paths(&existing));
        }
        let python_path = std::env::join_paths(python_paths).expect("valid Python import paths");
        let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
        let imported = Command::new(python)
            .args([
                "-c",
                "import demo_program; from demo_program import PROGRAMS, PROGRAM_READS; assert set(PROGRAMS) == {'demo'}; assert set(PROGRAM_READS) == {'demo'}",
            ])
            .env("PYTHONPATH", python_path)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("Python must be available for generated consumer smoke tests");
        assert!(
            imported.status.success(),
            "generated standalone Python package failed to import:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&imported.stdout),
            String::from_utf8_lossy(&imported.stderr),
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_program_compiler_keeps_idl_only_programs() {
        let mut spec = programs_stack_spec();
        spec.instructions.clear();
        let output = compile_program_modules(spec, None)
            .expect("an IDL-only standalone program should still be emitted");
        assert!(output.programs_py.contains("DEMO_PROGRAM = ProgramDef("));
        assert!(output
            .programs_py
            .contains("PROGRAMS: Dict[str, ProgramDef] = {"));
    }

    #[test]
    fn python_generator_without_instructions_omits_programs() {
        let mut spec = programs_stack_spec();
        spec.instructions.clear();

        let output = compile_stack_spec(spec, None).expect("python generation should succeed");

        assert!(output.programs_py.is_none());
        assert!(output.init_py.contains("from . import models, views"));
        assert!(!output.init_py.contains("programs.PROGRAMS"));
        assert!(!output.init_py.contains("program_reads"));
    }

    #[test]
    fn python_generator_notes_skipped_instructions() {
        let mut spec = programs_stack_spec();
        spec.instructions.push(InstructionDef {
            name: "badThing".to_string(),
            discriminator: vec![9],
            discriminator_size: 1,
            accounts: vec![],
            args: vec![instruction_arg("payload", "MysteryType")],
            errors: vec![],
            program_id: Some(TEST_PROGRAM_ID.to_string()),
            docs: vec![],
        });

        let output =
            compile_stack_spec(spec, None).expect("python stack generation should succeed");
        let programs = output.programs_py.expect("programs.py should be generated");

        assert!(programs.contains("# Skipped instructions (unsupported by instruction codegen):"));
        assert!(
            programs.contains("# - `badThing`: arg 'payload' has unsupported type 'MysteryType'")
        );
        assert!(!programs.contains("BadThingParams"));
        assert!(!programs.contains("demo_bad_thing"));
        // The supported instruction is still emitted.
        assert!(programs.contains("DemoDoThingParams = TypedDict("));
    }

    #[test]
    fn python_generator_emits_program_read_layer() {
        // Build a stack spec through the real program-spec pipeline so the
        // recorded ProgramSpecV1 hashes exactly like production specs.
        let idl_json = format!(
            r#"{{
              "address": "{TEST_PROGRAM_ID}",
              "version": "0.1.0",
              "name": "demo",
              "instructions": [
                {{
                  "name": "doThing",
                  "accounts": [{{ "name": "payer", "isMut": true, "isSigner": true }}],
                  "args": [{{ "name": "amount", "type": "u64" }}],
                  "discriminant": {{ "type": "u8", "value": 1 }}
                }}
              ],
              "accounts": [
                {{
                  "name": "Counter",
                  "type": {{
                    "kind": "struct",
                    "fields": [{{ "name": "count", "type": "u64" }}]
                  }}
                }}
              ],
              "types": [],
              "events": [],
              "errors": []
            }}"#
        );
        let mut spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl_json.as_bytes(),
            None,
            "Demo",
        )
        .expect("program-only stack spec should build");
        let expected_spec_hash = spec.program_specs[0].hash().unwrap().to_string();
        let expected_release_hash = spec.program_specs[0]
            .oss_release_hash()
            .unwrap()
            .to_string();

        // One entity whose raw account dataclass (`Counter`) is emitted in
        // models.py.
        let mut entity = minimal_entity("DemoThing");
        entity.sections.push(EntitySection {
            name: "state".to_string(),
            fields: vec![FieldTypeInfo {
                field_name: "counter".to_string(),
                raw_name: Some("counter".to_string()),
                canonical_name: Some("counter".to_string()),
                rust_type_name: "Option<serde_json::Value>".to_string(),
                base_type: BaseType::Object,
                integer_kind: None,
                is_optional: false,
                is_array: false,
                inner_type: Some("Value".to_string()),
                source_path: None,
                resolved_type: Some(ResolvedStructType {
                    type_name: "Counter".to_string(),
                    fields: vec![],
                    is_instruction: false,
                    is_account: true,
                    is_event: false,
                    is_enum: false,
                    enum_variants: vec![],
                }),
                emit: true,
            }],
            is_nested_struct: false,
            parent_field: None,
        });
        spec.entities.push(entity);

        let output =
            compile_stack_spec(spec, None).expect("python stack generation should succeed");
        let programs = output.programs_py.expect("programs.py should be generated");

        // Release identity consts + descriptor.
        assert!(programs.contains(&format!(
            "DEMO_PROGRAM_SPEC_HASH = \"{expected_spec_hash}\""
        )));
        assert!(programs.contains(&format!(
            "DEMO_PROGRAM_RELEASE_HASH = \"{expected_release_hash}\""
        )));
        assert!(programs.contains("def demo_read_descriptor() -> ProgramReadDescriptor:"));
        assert!(programs.contains("transport=LocalHttpTransportDef(),"));
        assert!(!programs.contains("Program read layer omitted"));

        // Typed account read def for the emitted `Counter` dataclass.
        assert!(output.models_py.contains("class Counter:"));
        assert!(programs.contains(
            "\"counter\": ProgramAccountReadDef(account=\"Counter\", parser=models.counter_from_wire),"
        ));
        assert!(programs.contains("accounts=dict(_DEMO_ACCOUNTS),"));
        assert!(programs.contains("program_spec_hash=DEMO_PROGRAM_SPEC_HASH,"));

        // The read map addresses every program, so it is emitted populated.
        assert!(programs.contains(
            "PROGRAM_READS: Dict[str, ProgramReadDescriptor] = {\n    \"demo\": demo_read_descriptor(),\n}"
        ));
    }

    #[test]
    fn python_generator_emits_platform_release_override() {
        let idl_json = format!(
            r#"{{
              "address": "{TEST_PROGRAM_ID}",
              "version": "0.1.0",
              "name": "demo",
              "instructions": [
                {{
                  "name": "doThing",
                  "accounts": [{{ "name": "payer", "isMut": true, "isSigner": true }}],
                  "args": [{{ "name": "amount", "type": "u64" }}],
                  "discriminant": {{ "type": "u8", "value": 1 }}
                }}
              ],
              "accounts": [],
              "types": [],
              "events": [],
              "errors": []
            }}"#
        );
        let spec = crate::program_sdk::build_program_only_stack_spec_from_idl_bytes(
            idl_json.as_bytes(),
            None,
            "Demo",
        )
        .expect("program-only stack spec should build");

        let platform_spec = "arete:h1:program-spec:sha256:platformspec".to_string();
        let platform_release = "arete:h1:program-release:sha256:platformrelease".to_string();
        let config = PythonStackConfig {
            program_reads: vec![PythonProgramReadConfig {
                program_id: TEST_PROGRAM_ID.to_string(),
                program_spec_hash: platform_spec.clone(),
                program_release_hash: platform_release.clone(),
                descriptor: Some(serde_json::json!({
                    "release": {
                        "programReleaseHash": platform_release.clone(),
                        "programSpecHash": platform_spec.clone(),
                    },
                    "transport": {"kind": "hosted-binding", "binding": {
                        "endpoint": "https://reads.example.test",
                        "programReadBindingId": "prb_00000000000000000000000000000001",
                        "auth": {
                            "sessionEndpoint": "https://auth.example.test/session",
                            "targetKind": "program-read-binding",
                            "targetId": "prb_00000000000000000000000000000001"
                        }
                    }}
                })),
            }],
            ..Default::default()
        };

        let output =
            compile_stack_spec(spec, Some(config)).expect("python stack generation should succeed");
        let programs = output.programs_py.expect("programs.py should be generated");

        assert!(programs.contains(&format!("DEMO_PROGRAM_SPEC_HASH = \"{platform_spec}\"")));
        assert!(programs.contains(&format!(
            "DEMO_PROGRAM_RELEASE_HASH = \"{platform_release}\""
        )));
        assert!(programs.contains("def demo_read_descriptor() -> ProgramReadDescriptor:"));
        assert!(programs.contains("program_read_descriptor_from_wire(json.loads("));
        assert!(programs.contains("\\\"kind\\\":\\\"hosted-binding\\\""));
    }

    #[test]
    fn python_generator_emits_stack_urls() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("python stack generation should succeed");
        assert!(output
            .init_py
            .contains("ws=\"\",  # TODO: Set URL after first deployment in arete.toml"));
        assert!(!output.init_py.contains("http="));

        let config = PythonStackConfig {
            url: Some("wss://demo.stack.example".to_string()),
            http_url: Some("https://demo.stack.example".to_string()),
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config))
            .expect("python stack generation should succeed");
        assert!(output.init_py.contains("ws=\"wss://demo.stack.example\","));
        assert!(output
            .init_py
            .contains("http=\"https://demo.stack.example\","));
    }

    #[test]
    fn python_generator_emits_typed_views() {
        let mut spec = programs_stack_spec();
        spec.entities[0].views = vec![
            crate::ast::ViewDef::state("DemoThing", &["id.address"]),
            crate::ast::ViewDef::list("DemoThing"),
            crate::ast::ViewDef {
                id: "DemoThing/latest".to_string(),
                source: ViewSource::Entity {
                    name: "DemoThing".to_string(),
                },
                pipeline: vec![],
                output: ViewOutput::Collection,
            },
        ];

        let output =
            compile_stack_spec(spec, None).expect("python stack generation should succeed");

        assert!(output.views_py.contains("class DemoThingViews:"));
        assert!(output.views_py.contains("view=\"DemoThing/state\","));
        assert!(output.views_py.contains("key_fields=(\"address\",),"));
        assert!(output
            .views_py
            .contains("parser=models.demo_thing_from_wire,"));
        assert!(output.views_py.contains(
            "list = ViewDef(mode=\"list\", view=\"DemoThing/list\", parser=models.demo_thing_from_wire)"
        ));
        assert!(output.views_py.contains(
            "latest = ViewDef(mode=\"list\", view=\"DemoThing/latest\", parser=models.demo_thing_from_wire)"
        ));
        assert!(output
            .views_py
            .contains("VIEWS: Dict[str, Dict[str, ViewDef]] = {"));
        assert!(output.views_py.contains("    \"demo_thing\": {"));
        assert!(output
            .views_py
            .contains("        \"state\": DemoThingViews.state,"));
        assert!(output
            .views_py
            .contains("        \"latest\": DemoThingViews.latest,"));
    }

    #[test]
    fn python_generator_selects_exact_views() {
        let mut spec = programs_stack_spec();
        // A second entity without any selected views must be dropped.
        spec.entities.push(minimal_entity("HiddenThing"));
        spec.entities[0].views = vec![crate::ast::ViewDef {
            id: "DemoThing/latest".to_string(),
            source: ViewSource::Entity {
                name: "DemoThing".to_string(),
            },
            pipeline: vec![],
            output: ViewOutput::Collection,
        }];

        let output = compile_stack_spec_with_exact_views(spec, None)
            .expect("python stack generation should succeed");

        assert!(output.views_py.contains("class DemoThingViews:"));
        assert!(output.views_py.contains("latest = ViewDef("));
        // state/list were not selected; latest is the only view.
        assert!(!output.views_py.contains("view=\"DemoThing/state\""));
        assert!(!output.views_py.contains("view=\"DemoThing/list\""));
        assert!(!output.views_py.contains("HiddenThingViews"));
        assert!(!output.views_py.contains("\"hidden_thing\""));
        // Models are still generated for all entities (readers may use them).
        assert!(output.models_py.contains("class HiddenThing:"));
    }

    #[test]
    fn python_generator_degrades_composite_state_keys() {
        let mut spec = programs_stack_spec();
        spec.entities[0].identity = IdentitySpec {
            primary_keys: vec!["id.address".to_string(), "id.slot".to_string()],
            lookup_indexes: vec![],
        };

        let output =
            compile_stack_spec(spec, None).expect("python stack generation should succeed");

        assert!(output
            .views_py
            .contains("# [arete codegen] composite state key"));
        assert!(output.views_py.contains("key_fields=(),"));
    }

    #[test]
    fn python_generator_wires_extension_modules_after_generated_decls() {
        let config = PythonStackConfig {
            module_mode: true,
            extension_modules: vec!["devex".to_string(), "extensions".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        let output = compile_stack_spec(programs_stack_spec(), Some(config))
            .expect("python stack generation should succeed");
        let init = &output.init_py;

        assert!(init.contains(
            "# Hand-authored devex extensions (staged from extensions.json; not generated)."
        ));
        let stack_def = init.find("DEMO_STACK: StackDef").expect("stack def");
        let devex = init
            .find("from . import devex  # noqa: F401")
            .expect("devex import");
        let entry = init
            .find("from .extensions import *  # noqa: F401,F403")
            .expect("entry star import");
        assert!(stack_def < devex);
        assert!(devex < entry);
        assert!(!init.contains("from .devex import *"));
    }

    #[test]
    fn python_generator_omits_extension_wiring_without_entry() {
        let output = compile_stack_spec(programs_stack_spec(), None)
            .expect("python stack generation should succeed");

        assert!(!output.init_py.contains("Hand-authored devex extensions"));
        assert!(!output.init_py.contains("from .extensions import *"));
    }

    #[test]
    fn python_generator_rejects_extension_module_collisions() {
        for reserved in ["models", "views", "programs"] {
            let config = PythonStackConfig {
                extension_modules: vec![reserved.to_string(), "extensions".to_string()],
                extension_entry: Some("extensions".to_string()),
                ..Default::default()
            };
            let error = compile_stack_spec(programs_stack_spec(), Some(config))
                .expect_err("collision with a generated module must fail");
            assert!(
                error.contains(&format!("'{reserved}.py'")),
                "collision error should name the file: {error}"
            );
        }

        // `programs.py` is only reserved when the stack generates programs.
        let mut no_instructions = programs_stack_spec();
        no_instructions.instructions.clear();
        let config = PythonStackConfig {
            extension_modules: vec!["programs".to_string(), "extensions".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        assert!(compile_stack_spec(no_instructions, Some(config)).is_ok());

        let duplicate = PythonStackConfig {
            extension_modules: vec![
                "devex".to_string(),
                "devex".to_string(),
                "extensions".to_string(),
            ],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        assert!(compile_stack_spec(programs_stack_spec(), Some(duplicate)).is_err());

        let entry_not_last = PythonStackConfig {
            extension_modules: vec!["extensions".to_string(), "devex".to_string()],
            extension_entry: Some("extensions".to_string()),
            ..Default::default()
        };
        assert!(compile_stack_spec(programs_stack_spec(), Some(entry_not_last)).is_err());
    }

    #[test]
    fn python_writer_emits_module_and_package_layouts() {
        let output = compile_stack_spec(
            programs_stack_spec(),
            Some(PythonStackConfig {
                package_name: "demo-stack".to_string(),
                ..Default::default()
            }),
        )
        .expect("python stack generation should succeed");

        let base = std::env::temp_dir().join(format!(
            "arete-python-codegen-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);

        // Module layout: a plain source directory dropped into a project.
        let module_dir = base.join("module");
        write_python_module(&output, &module_dir).expect("module layout should write");
        for file in ["__init__.py", "models.py", "views.py", "programs.py"] {
            assert!(
                module_dir.join(file).is_file(),
                "missing module file {file}"
            );
        }
        assert!(!module_dir.join("pyproject.toml").exists());

        // Package layout: pyproject wrapper + import package directory.
        let package_dir = base.join("package");
        write_python_package(&output, &package_dir).expect("package layout should write");
        assert!(package_dir.join("pyproject.toml").is_file());
        for file in ["__init__.py", "models.py", "views.py", "programs.py"] {
            assert!(
                package_dir.join("demo_stack").join(file).is_file(),
                "missing package file {file}"
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_module_name_escapes_keywords_and_digits() {
        assert_eq!(python_module_name("Ore-Stack"), "ore_stack");
        assert_eq!(python_module_name("1live"), "live_1live");
        assert_eq!(python_module_name("import"), "import_live");
        assert_eq!(to_snake_case("lambda"), "lambda_");
        assert_eq!(to_screaming_snake("OreStream"), "ORE_STREAM");
    }

    /// Regeneration helper for the checked-in ore example. Run with:
    /// `cargo test -p arete-interpreter regenerate_ore_example_python -- --ignored`
    ///
    /// Rewrites `examples/ore-python/ore_stack/{__init__,models,views,programs}.py`
    /// from the checked-in Ore StackManifest artifact closure.
    ///
    /// Extension wiring reuses the `extensions.json` staged in the output
    /// directory (files sorted, entry last, stems via [`python_module_name`])
    /// — a faithful replica of the CLI's output-dir manifest resolution step.
    /// Staged extension files are preserved verbatim, so a second run is a
    /// byte-stable fixed point.
    #[test]
    #[ignore = "writes into examples/ore-python; run explicitly to regenerate"]
    fn regenerate_ore_example_python() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("interpreter crate lives in the repo root")
            .to_path_buf();
        let spec = crate::public_artifacts::ore_stack_spec_from_exact_artifacts();

        let out_dir = repo_root.join("examples/ore-python/ore_stack");
        let (extension_modules, extension_entry) =
            match std::fs::read_to_string(out_dir.join("extensions.json")) {
                Ok(manifest_json) => {
                    let manifest: serde_json::Value = serde_json::from_str(&manifest_json)
                        .expect("staged extensions.json should parse");
                    assert_eq!(
                        manifest["language"].as_str(),
                        Some("python"),
                        "staged ore extensions must be a Python bundle"
                    );
                    let entry_stem = python_module_name(
                        manifest["entry"]
                            .as_str()
                            .and_then(|entry| entry.strip_suffix(".py"))
                            .expect("extensions entry should be a .py file"),
                    );
                    let mut stems: Vec<String> = manifest["files"]
                        .as_array()
                        .expect("extensions files should be an array")
                        .iter()
                        .map(|file| {
                            python_module_name(
                                file.as_str()
                                    .and_then(|file| file.strip_suffix(".py"))
                                    .expect("extension files should be .py files"),
                            )
                        })
                        .filter(|stem| stem != &entry_stem)
                        .collect();
                    stems.sort();
                    stems.dedup();
                    stems.push(entry_stem.clone());
                    (stems, Some(entry_stem))
                }
                Err(_) => (Vec::new(), None),
            };

        let config = PythonStackConfig {
            package_name: "ore-stack".to_string(),
            sdk_version: "0.4".to_string(),
            module_mode: true,
            url: Some("wss://ore.stack.arete.run".to_string()),
            http_url: Some("https://ore.stack.arete.run".to_string()),
            extension_modules,
            extension_entry,
            program_reads: Vec::new(),
            gateway: None,
        };
        let output =
            compile_stack_spec(spec, Some(config)).expect("ore stack should compile to Python");

        write_python_module(&output, &out_dir).expect("ore example should write");
    }

    #[test]
    fn python_generator_selects_u64_length_prefixed_vec_schema() {
        let mut parser = PythonDefinedTypes::new(&[]);
        let vec_of_pubkey = |length_prefix| {
            IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                vec: Box::new(IdlTypeSnapshot::Simple("publicKey".to_string())),
                length_prefix,
            })
        };

        assert_eq!(
            parser.parse_snapshot_type(&vec_of_pubkey(None)).schema,
            "{\"vec\": \"pubkey\"}"
        );
        assert_eq!(
            parser
                .parse_snapshot_type(&vec_of_pubkey(Some(arete_idl::types::IdlLengthPrefix::U32)))
                .schema,
            "{\"vec\": \"pubkey\"}"
        );
        assert_eq!(
            parser
                .parse_snapshot_type(&vec_of_pubkey(Some(arete_idl::types::IdlLengthPrefix::U64)))
                .schema,
            "{\"vecU64Len\": \"pubkey\"}"
        );
    }

    /// Instruction args round-trip through `InstructionArgDef::arg_type` as a string.
    #[test]
    fn python_generator_round_trips_u64_length_prefixed_vec_args() {
        let vec_of_pubkey = |length_prefix| {
            IdlTypeSnapshot::Vec(IdlVecTypeSnapshot {
                vec: Box::new(IdlTypeSnapshot::Simple("publicKey".to_string())),
                length_prefix,
            })
        };
        let borsh = idl_type_snapshot_to_rust_string(&vec_of_pubkey(None));
        let bincode = idl_type_snapshot_to_rust_string(&vec_of_pubkey(Some(
            arete_idl::types::IdlLengthPrefix::U64,
        )));
        assert_eq!(borsh, "Vec<solana_pubkey::Pubkey>");
        assert_eq!(bincode, "VecU64Len<solana_pubkey::Pubkey>");

        let mut parser = PythonDefinedTypes::new(&[]);
        let parsed = parser.parse_arg_type(&bincode);
        assert_eq!(parsed.schema, "{\"vecU64Len\": \"pubkey\"}");
        assert_eq!(parsed.param_type, "Sequence[str]");
        assert_eq!(
            parser.parse_arg_type(&borsh).schema,
            "{\"vec\": \"pubkey\"}"
        );

        let mut spec = programs_stack_spec();
        spec.instructions[0]
            .args
            .push(instruction_arg("newAddresses", &bincode));
        let programs = compile_stack_spec(spec, None)
            .expect("python stack generation should succeed")
            .programs_py
            .expect("programs.py should be generated");
        assert!(
            programs.contains("{\"vecU64Len\": \"pubkey\"}"),
            "u64-prefixed vec arg should reach the generated schema:\n{programs}"
        );
    }
}

fn is_python_keyword(value: &str) -> bool {
    matches!(
        value,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}
