//! The SDK reference `a4 install` writes into each TypeScript SDK it
//! generates: `sdk-reference.json`, the `README.md` rendered from it, and a
//! `README.md` in each program folder of a stack SDK.
//!
//! The reference is built from what generated the SDK, not from the
//! registry: the stack's entities as the TypeScript generator declares them
//! ([`typescript_entity_references`]), their token-amount scales
//! ([`field_amounts`]), the views block of the generated core module, and
//! the extension sources staged beside it. The files are added to the SDK's
//! payload, so its `sdkOutputTreeHash` covers them like every other
//! generated file, and ownership and drift checks treat them the same way.
//! Rendering lives in [`arete_mcp::sdk_reference`], which `a4 sdk describe`
//! and the `describe_sdk` MCP tool use too.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use arete_interpreter::ast::{SerializableStackSpec, SerializableStreamSpec};
use arete_interpreter::typescript::{
    program_key, typescript_entity_references, TypeScriptEntityReference, TypeScriptFieldReference,
};
use arete_mcp::field_amounts;
use arete_mcp::sdk_reference::{
    render_markdown, render_program_markdown, AmountReference, EntityReference, FieldReference,
    ProgramReference, SdkImport, SdkKind, SdkLanguage, SdkReference, ViewKind, ViewReference,
    README_FILE, REFERENCE_FILE, SCHEMA_VERSION,
};
use regex::Regex;

use super::extension_outline;

/// What the reference says about an install that its generator does not
/// know: the package it came from and where the SDK lands in the project.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReferenceContext {
    /// The registry package and version, for a registry dependency.
    pub package: Option<String>,
    pub version: Option<String>,
    /// The SDK folder relative to the project root
    /// (`generated/typescript/stacks/ore`), when it is inside it.
    pub project_dir: Option<String>,
}

/// The JSDoc notes the TypeScript generator adds to entity fields: each
/// token amount's scale, by entity and wire path.
pub(super) fn typescript_field_notes(
    stack_spec: &SerializableStackSpec,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let Ok(entities) = typescript_entity_references(stack_spec) else {
        return BTreeMap::new();
    };
    stack_spec
        .entities
        .iter()
        .zip(&entities)
        .filter_map(|(spec, entity)| {
            let notes = entity_amounts(spec, &entity.fields)
                .into_iter()
                .map(|(wire, amount)| (wire, sentence(&amount.describe())))
                .collect::<BTreeMap<_, _>>();
            (!notes.is_empty()).then(|| (entity.entity.clone(), notes))
        })
        .collect()
}

/// `text` as a sentence: capitalised, ending in a full stop.
fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    let mut sentence = characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect::<String>())
        .unwrap_or_default();
    if !sentence.ends_with('.') {
        sentence.push('.');
    }
    sentence
}

/// The token-amount scale of each of `fields` a computation of `spec` sets,
/// by wire path, with the paths it refers to in TypeScript naming.
fn entity_amounts(
    spec: &SerializableStreamSpec,
    fields: &[TypeScriptFieldReference],
) -> BTreeMap<String, AmountReference> {
    let Ok(entity) = serde_json::to_value(spec) else {
        return BTreeMap::new();
    };
    let visible = fields
        .iter()
        .map(|field| field.wire.clone())
        .collect::<BTreeSet<_>>();
    let path_of = |wire: &str| typescript_path(fields, wire);
    field_amounts::entity_field_amounts(&entity, &visible)
        .into_iter()
        .filter(|(wire, _)| visible.contains(wire))
        .map(|(wire, amount)| {
            let reference = AmountReference {
                scale: amount.scale,
                decimals: amount.decimals,
                decimals_from: amount.decimals_from.as_deref().map(path_of),
                counterpart: amount.counterpart.as_deref().map(path_of),
            };
            (wire, reference)
        })
        .collect()
}

/// The TypeScript path of `wire` in a row with `fields`: the field's own
/// path, else the path of its longest declared parent with the rest in
/// camelCase (`token_metadata.decimals` -> `tokenMetadata.decimals`, as the
/// generated nested types name their members), else `wire` as it is.
fn typescript_path(fields: &[TypeScriptFieldReference], wire: &str) -> String {
    if let Some(field) = fields.iter().find(|field| field.wire == wire) {
        return field.path.clone();
    }
    let parent = fields
        .iter()
        .filter_map(|field| {
            wire.strip_prefix(&field.wire)
                .and_then(|rest| rest.strip_prefix('.'))
                .map(|rest| (field.wire.len(), field.path.clone(), rest))
        })
        .max_by_key(|(length, _, _)| *length)
        .map(|(_, path, rest)| (path, rest));
    // A section is no field of its own, but its fields name it.
    let section = || {
        let (section, rest) = wire.split_once('.')?;
        fields.iter().find_map(|field| {
            let (wire_section, _) = field.wire.split_once('.')?;
            let (path_section, _) = field.path.split_once('.')?;
            (wire_section == section).then(|| (path_section.to_string(), rest))
        })
    };
    match parent.or_else(section) {
        Some((path, rest)) => {
            let rest = rest.split('.').map(camel_case).collect::<Vec<_>>();
            format!("{path}.{}", rest.join("."))
        }
        None => wire.to_string(),
    }
}

/// `snake_case` -> `snakeCase`.
fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for character in name.chars() {
        if character == '_' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(character.to_uppercase());
            upper = false;
        } else {
            out.push(character);
        }
    }
    out
}

/// The reference of the TypeScript stack SDK generated into `output` from
/// `stack_spec`.
pub(super) fn typescript_stack_reference(
    stack_spec: &SerializableStackSpec,
    output: &Path,
    alias: &str,
    context: &ReferenceContext,
) -> Result<SdkReference> {
    let modules = typescript_modules(output)?;
    let (entry, export) = modules
        .iter()
        .find_map(|(name, source)| {
            exported_definition(source, "_STACK").map(|export| (name, export))
        })
        .ok_or_else(|| anyhow::anyhow!("{} exports no stack definition", output.display()))?;
    let core_marker = format!("export const {export}_CORE = {{");
    let (core, core_source) = modules
        .iter()
        .find(|(_, source)| source.contains(&core_marker))
        .map(|(name, source)| (name.clone(), source.as_str()))
        .unwrap_or_else(|| (entry.clone(), ""));
    let views = generated_views(core_source, &core_marker);
    let stack_extension = extension_source(output);
    let entities = typescript_entity_references(stack_spec)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .zip(&stack_spec.entities)
        .map(|(entity, spec)| entity_reference(entity, spec, &views))
        .collect();
    let programs = stack_spec
        .idls
        .iter()
        .enumerate()
        .map(|(index, idl)| {
            let pdas = stack_spec
                .pdas
                .get(&idl.name)
                .or_else(|| stack_spec.pdas.get(&program_key(&idl.name)))
                .map(|pdas| pdas.keys().cloned().collect())
                .or_else(|| {
                    stack_spec
                        .program_specs
                        .get(index)
                        .map(|spec| spec.pdas.keys().cloned().collect())
                })
                .unwrap_or_default();
            program_reference(
                output,
                idl,
                stack_spec.program_ids.get(index).map(String::as_str),
                pdas,
                true,
            )
        })
        .collect();
    Ok(SdkReference {
        schema_version: SCHEMA_VERSION,
        kind: SdkKind::Stack,
        alias: alias.to_string(),
        language: SdkLanguage::Typescript,
        package: context.package.clone(),
        version: context.version.clone(),
        import: SdkImport {
            specifier: specifier(context, entry),
            module: entry.clone(),
            export,
            types_module: (core != *entry).then_some(core),
        },
        entities,
        reads: stack_extension
            .as_deref()
            .map(extension_outline::reads)
            .unwrap_or_default(),
        helpers: stack_extension
            .as_deref()
            .map(extension_outline::helpers)
            .unwrap_or_default(),
        programs,
    })
}

/// The reference of the TypeScript program SDK generated into `output` from
/// `program_spec`.
pub(super) fn typescript_program_reference(
    program_spec: &arete_artifacts::ProgramSpecArtifact,
    output: &Path,
    alias: &str,
    context: &ReferenceContext,
) -> Result<SdkReference> {
    let modules = typescript_modules(output)?;
    let (entry, export) = modules
        .iter()
        .find_map(|(name, source)| {
            exported_definition(source, "_PROGRAM").map(|export| (name, export))
        })
        .ok_or_else(|| anyhow::anyhow!("{} exports no program definition", output.display()))?;
    let payload = &program_spec.payload;
    let program = program_reference(
        output,
        &payload.idl_snapshot.snapshot,
        Some(&payload.program_id),
        payload.pdas.keys().cloned().collect(),
        false,
    );
    Ok(SdkReference {
        schema_version: SCHEMA_VERSION,
        kind: SdkKind::Program,
        alias: alias.to_string(),
        language: SdkLanguage::Typescript,
        package: context.package.clone(),
        version: context.version.clone(),
        import: SdkImport {
            specifier: specifier(context, entry),
            module: entry.clone(),
            export,
            types_module: None,
        },
        entities: Vec::new(),
        reads: Vec::new(),
        helpers: Vec::new(),
        programs: vec![program],
    })
}

/// Write `reference` into `output` and add its files to the SDK's payload.
pub(super) fn write_reference(output: &Path, reference: &SdkReference) -> Result<()> {
    let mut files = vec![
        (REFERENCE_FILE.to_string(), reference.to_json()),
        (README_FILE.to_string(), render_markdown(reference)),
    ];
    if reference.kind == SdkKind::Stack {
        for program in &reference.programs {
            if let (Some(directory), Some(readme)) = (
                &program.directory,
                render_program_markdown(reference, &program.key),
            ) {
                files.push((format!("{directory}/{README_FILE}"), readme));
            }
        }
    }
    for (name, contents) in &files {
        let path = output.join(name);
        fs::write(&path, contents)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    super::add_sdk_payload_files(
        output,
        &files.into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
    )
}

/// The import specifier of `entry` from the project root.
fn specifier(context: &ReferenceContext, entry: &str) -> Option<String> {
    let directory = context.project_dir.as_deref()?;
    let module = entry.strip_suffix(".ts").unwrap_or(entry);
    Some(if directory.is_empty() {
        format!("./{module}.js")
    } else {
        format!("./{}/{module}.js", directory.trim_end_matches('/'))
    })
}

/// The `.ts` modules directly in `output`, by file name, in name order.
fn typescript_modules(output: &Path) -> Result<Vec<(String, String)>> {
    let mut modules = Vec::new();
    for entry in fs::read_dir(output)
        .with_context(|| format!("Failed to read SDK output {}", output.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_file() && name.ends_with(".ts") && !name.ends_with(".d.ts") {
            modules.push((name.to_string(), fs::read_to_string(&path)?));
        }
    }
    modules.sort();
    Ok(modules)
}

/// The `export const X<suffix>` an entry module defines.
fn exported_definition(source: &str, suffix: &str) -> Option<String> {
    let export = Regex::new(&format!(
        r"(?m)^export const ([A-Z_][A-Z0-9_]*{})\b\s*[:=]",
        regex::escape(suffix)
    ))
    .expect("definition regex should compile");
    export
        .captures(source)
        .map(|captures| captures[1].to_string())
}

/// The views the generated core module declares, by entity key: the
/// `views` block of the definition starting at `marker`.
fn generated_views(core: &str, marker: &str) -> BTreeMap<String, Vec<ViewReference>> {
    let entity_line = Regex::new(r"^ {4}(.+): \{$").expect("entity regex should compile");
    let view_line = Regex::new(r"^ {6}(.+?): (stateView|listView)<\w+(?:, (\{.*\}))?>\('([^']+)'")
        .expect("view regex should compile");
    let mut views: BTreeMap<String, Vec<ViewReference>> = BTreeMap::new();
    let Some(at) = core.find(marker) else {
        return views;
    };
    let mut lines = core[at..].lines().skip_while(|line| *line != "  views: {");
    if lines.next().is_none() {
        return views;
    }
    let mut entity: Option<String> = None;
    for line in lines {
        if line.starts_with("  }") {
            break;
        }
        if let Some(captures) = entity_line.captures(line) {
            entity = Some(captures[1].to_string());
            continue;
        }
        let (Some(entity), Some(captures)) = (&entity, view_line.captures(line)) else {
            continue;
        };
        let state = &captures[2] == "stateView";
        views
            .entry(unquoted(entity).to_string())
            .or_default()
            .push(ViewReference {
                id: captures[4].to_string(),
                kind: if state {
                    ViewKind::State
                } else {
                    ViewKind::List
                },
                access: format!("views{}{}", member(entity), member(&captures[1])),
                key: captures.get(3).map(|key| key.as_str().to_string()),
            });
    }
    views
}

fn unquoted(key: &str) -> &str {
    key.strip_prefix('\'')
        .and_then(|key| key.strip_suffix('\''))
        .or_else(|| key.strip_prefix('"').and_then(|key| key.strip_suffix('"')))
        .unwrap_or(key)
}

/// A property access for an object key as the generator writes it.
fn member(key: &str) -> String {
    let bare = unquoted(key);
    if bare.len() == key.len() {
        format!(".{key}")
    } else {
        format!("[{}]", serde_json::Value::String(bare.to_string()))
    }
}

fn entity_reference(
    entity: TypeScriptEntityReference,
    spec: &SerializableStreamSpec,
    views: &BTreeMap<String, Vec<ViewReference>>,
) -> EntityReference {
    let mut amounts = entity_amounts(spec, &entity.fields);
    EntityReference {
        views: views.get(&entity.entity).cloned().unwrap_or_default(),
        fields: entity
            .fields
            .into_iter()
            .map(|field| FieldReference {
                amount: amounts.remove(&field.wire),
                path: field.path,
                wire: field.wire,
                ty: field.ts_type,
                nullable: field.nullable,
            })
            .collect(),
        name: entity.entity,
        type_name: entity.type_name,
    }
}

/// A program: its IDL names, and the reads and operations of the extension
/// in its own folder, when it has one. `nested`: inside a stack SDK, where a
/// program with its own SDK has a `programs/<key>` folder; a program SDK's
/// extension is in `output` itself.
fn program_reference(
    output: &Path,
    idl: &arete_idl::IdlSnapshot,
    program_id: Option<&str>,
    pdas: Vec<String>,
    nested: bool,
) -> ProgramReference {
    let key = program_key(&idl.name);
    let directory = if nested {
        [key.clone(), crate::config::to_kebab_case(&key)]
            .into_iter()
            .map(|name| format!("programs/{name}"))
            .find(|directory| output.join(directory).is_dir())
    } else {
        None
    };
    let extension = match (&directory, nested) {
        (Some(directory), _) => extension_source(&output.join(directory)),
        (None, false) => extension_source(output),
        (None, true) => None,
    };
    let (reads, operations, helpers) = extension
        .map(|source| {
            (
                extension_outline::reads(&source),
                extension_outline::operations(&source),
                extension_outline::helpers(&source),
            )
        })
        .unwrap_or_default();
    ProgramReference {
        key,
        name: idl.name.clone(),
        program_id: program_id
            .map(str::to_string)
            .or_else(|| idl.program_id.clone()),
        directory,
        reads,
        operations,
        accounts: idl
            .accounts
            .iter()
            .map(|account| account.name.clone())
            .collect(),
        instructions: idl
            .instructions
            .iter()
            .map(|instruction| instruction.name.clone())
            .collect(),
        pdas,
        helpers,
    }
}

/// The entry source of the extension staged in `directory`, named by its
/// `extensions.json`.
fn extension_source(directory: &Path) -> Option<String> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("extensions.json")).ok()?).ok()?;
    let entry = manifest.get("entry")?.as_str()?;
    fs::read_to_string(directory.join(entry)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_views_block_gives_each_view_its_kind_access_and_key() {
        let core = r#"
export const ORE_STREAM_STACK_CORE = {
  name: 'ore',
  views: {
    OreRound: {
      state: stateView<OreRound, { roundId: bigint }>('OreRound/state', ['roundId']),
      list: listView<OreRound>('OreRound/list'),
      latest: listView<OreRound>('OreRound/latest'),
    },
    'round-x': {
      'by-id': listView<RoundX>('round-x/by-id'),
    },
  },
} as const;
"#;
        let views = generated_views(core, "export const ORE_STREAM_STACK_CORE = {");
        let round = &views["OreRound"];
        assert_eq!(round.len(), 3);
        assert_eq!(round[0].kind, ViewKind::State);
        assert_eq!(round[0].access, "views.OreRound.state");
        assert_eq!(round[0].key.as_deref(), Some("{ roundId: bigint }"));
        assert_eq!(round[2].id, "OreRound/latest");
        assert_eq!(round[2].kind, ViewKind::List);
        assert_eq!(round[2].key, None);
        assert_eq!(views["round-x"][0].access, r#"views["round-x"]["by-id"]"#);
    }

    #[test]
    fn the_specifier_imports_the_entry_from_the_project_root() {
        let context = ReferenceContext {
            project_dir: Some("generated/typescript/stacks/ore".into()),
            ..ReferenceContext::default()
        };
        assert_eq!(
            specifier(&context, "ore.ts").as_deref(),
            Some("./generated/typescript/stacks/ore/ore.js")
        );
        assert_eq!(specifier(&ReferenceContext::default(), "ore.ts"), None);
    }

    #[test]
    fn amount_paths_inside_renamed_objects_use_typescript_names() {
        let field = |path: &str, wire: &str| TypeScriptFieldReference {
            path: path.into(),
            wire: wire.into(),
            ts_type: "number".into(),
            nullable: true,
        };
        let fields = [
            field("state.totalDeployed", "state.total_deployed"),
            field("tokenMetadata", "token_metadata"),
            field("poolInfo.baseMint", "pool_info.base_mint"),
        ];
        assert_eq!(
            typescript_path(&fields, "state.total_deployed"),
            "state.totalDeployed"
        );
        // A member of a renamed root object.
        assert_eq!(
            typescript_path(&fields, "token_metadata.decimals"),
            "tokenMetadata.decimals"
        );
        assert_eq!(
            typescript_path(&fields, "token_metadata.mint_info.decimals"),
            "tokenMetadata.mintInfo.decimals"
        );
        // A hidden field of a renamed section.
        assert_eq!(
            typescript_path(&fields, "pool_info.quote_decimals"),
            "poolInfo.quoteDecimals"
        );
        assert_eq!(typescript_path(&fields, "other.x_y"), "other.x_y");

        // The JSDoc note is built from the same translation.
        let amount = AmountReference {
            scale: arete_mcp::field_amounts::AmountScale::Ui,
            decimals: None,
            decimals_from: Some(typescript_path(&fields, "token_metadata.decimals")),
            counterpart: None,
        };
        assert_eq!(
            sentence(&amount.describe()),
            "Token amount in whole units (raw / 10^tokenMetadata.decimals)."
        );
    }

    #[test]
    fn notes_are_sentences() {
        assert_eq!(
            sentence("token amount in whole units (raw / 10^9)"),
            "Token amount in whole units (raw / 10^9)."
        );
    }
}
