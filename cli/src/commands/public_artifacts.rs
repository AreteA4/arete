use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use arete_artifacts::{
    load_live_spec, load_live_spec_v2, load_program_spec, load_stack_manifest,
    load_stack_manifest_v2, normalize_legacy_stack_v2, normalize_live_spec_v1,
    normalize_stack_manifest_v1, resolve_stack_composition_v2, selected_views, stack_manifest_v2,
    LiveSpecArtifact, LiveSpecArtifactV2, ProgramSpecArtifact, SelectedViewV2,
    StackManifestArtifactV2, DEFAULT_LIVE_ALIAS, STACK_MANIFEST_SCHEMA_V2,
};

#[derive(Debug, Clone)]
pub(crate) struct LocalArtifactStack {
    pub manifest_path: PathBuf,
    pub manifest_hash: String,
    pub program_specs: Vec<ProgramSpecArtifact>,
    pub live_specs: Vec<(String, LiveSpecArtifactV2)>,
    pub stack_manifest: StackManifestArtifactV2,
}

pub(crate) fn load_local_artifact_stack(manifest_path: &Path) -> Result<LocalArtifactStack> {
    let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    load_local_artifact_stack_with_roots(manifest_path, &[root.to_path_buf()])
}

pub(crate) fn load_local_artifact_stack_with_roots(
    manifest_path: &Path,
    artifact_roots: &[PathBuf],
) -> Result<LocalArtifactStack> {
    reject_parent_traversal(manifest_path, "StackManifest path")?;
    reject_symlink_path(manifest_path, "StackManifest path")?;
    let manifest_path = fs::canonicalize(manifest_path).with_context(|| {
        format!(
            "Failed to resolve StackManifest {}",
            manifest_path.display()
        )
    })?;
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("Failed to read StackManifest {}", manifest_path.display()))?;
    let catalog = ArtifactCatalog::scan(artifact_roots)?;
    let schema = serde_json::from_slice::<serde_json::Value>(&manifest_bytes)?["payload"]["schema"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    match schema.as_str() {
        STACK_MANIFEST_SCHEMA_V2 => load_local_v2_stack(&manifest_path, &manifest_bytes, &catalog),
        arete_artifacts::STACK_MANIFEST_SCHEMA_V1 => {
            load_and_normalize_local_v1_stack(&manifest_path, &manifest_bytes, &catalog)
        }
        _ => bail!("Unsupported StackManifest schema '{schema}'"),
    }
}

fn load_local_v2_stack(
    manifest_path: &Path,
    manifest_bytes: &[u8],
    catalog: &ArtifactCatalog,
) -> Result<LocalArtifactStack> {
    let stack_manifest = load_stack_manifest_v2(manifest_bytes)
        .with_context(|| format!("Invalid StackManifest {}", manifest_path.display()))?
        .artifact;
    let program_specs = stack_manifest
        .payload
        .programs
        .iter()
        .map(|reference| catalog.unique_program(&reference.artifact_hash.to_string()))
        .collect::<Result<Vec<_>>>()?;
    let live_specs = stack_manifest
        .payload
        .live_specs
        .iter()
        .map(|reference| {
            Ok((
                reference.alias.clone(),
                catalog.unique_v2_live(&reference.artifact_hash.to_string())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    resolve_stack_composition_v2(&stack_manifest, &live_specs, &program_specs)?;
    arete_interpreter::public_artifacts::stack_specs_from_artifacts_v2(
        &program_specs,
        &live_specs,
        &stack_manifest,
    )
    .map_err(anyhow::Error::msg)?;
    Ok(LocalArtifactStack {
        manifest_path: manifest_path.to_path_buf(),
        manifest_hash: stack_manifest.artifact_hash.to_string(),
        program_specs,
        live_specs,
        stack_manifest,
    })
}

fn load_and_normalize_local_v1_stack(
    manifest_path: &Path,
    manifest_bytes: &[u8],
    catalog: &ArtifactCatalog,
) -> Result<LocalArtifactStack> {
    let v1_manifest = load_stack_manifest(manifest_bytes)
        .with_context(|| format!("Invalid V1 StackManifest {}", manifest_path.display()))?
        .artifact;
    if v1_manifest.payload.live_specs.len() != 1 {
        bail!("V1 StackManifest compatibility requires exactly one LiveSpec");
    }
    let program_specs = v1_manifest
        .payload
        .programs
        .iter()
        .map(|reference| catalog.unique_program(&reference.artifact_hash.to_string()))
        .collect::<Result<Vec<_>>>()?;
    let source_reference = &v1_manifest.payload.live_specs[0];
    let v1_live = catalog.unique_v1_live(&source_reference.artifact_hash.to_string())?;
    let live_spec = normalize_live_spec_v1(&v1_live, &program_specs)?;
    let stack_manifest = normalize_stack_manifest_v1(
        &v1_manifest,
        &program_specs,
        &[(
            v1_live.artifact_hash,
            DEFAULT_LIVE_ALIAS.to_string(),
            &live_spec,
        )],
    )?;
    arete_interpreter::public_artifacts::stack_spec_from_artifacts_v2(
        &program_specs,
        &live_spec,
        &stack_manifest,
    )
    .map_err(anyhow::Error::msg)?;
    let live_specs = vec![(DEFAULT_LIVE_ALIAS.to_string(), live_spec.clone())];
    Ok(LocalArtifactStack {
        manifest_path: manifest_path.to_path_buf(),
        manifest_hash: v1_manifest.artifact_hash.to_string(),
        program_specs,
        live_specs,
        stack_manifest,
    })
}

#[derive(Debug, Default)]
struct ArtifactCatalog {
    programs: BTreeMap<String, Vec<(PathBuf, ProgramSpecArtifact)>>,
    v1_lives: BTreeMap<String, Vec<(PathBuf, LiveSpecArtifact)>>,
    v2_lives: BTreeMap<String, Vec<(PathBuf, LiveSpecArtifactV2)>>,
}

impl ArtifactCatalog {
    fn scan(roots: &[PathBuf]) -> Result<Self> {
        let roots = canonical_approved_roots(roots)?;
        let mut files = BTreeSet::new();
        for root in roots {
            collect_artifact_files(&root, &root, &mut files)?;
        }
        let mut catalog = Self::default();
        for path in files {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let bytes = fs::read(&path)
                .with_context(|| format!("Failed to read artifact {}", path.display()))?;
            if name.ends_with(".program-spec.json") {
                let artifact = load_program_spec(&bytes)
                    .with_context(|| format!("Invalid ProgramSpec {}", path.display()))?
                    .artifact;
                catalog
                    .programs
                    .entry(artifact.artifact_hash.to_string())
                    .or_default()
                    .push((path, artifact));
            } else if name.ends_with(".live-spec.json") {
                let schema = serde_json::from_slice::<serde_json::Value>(&bytes)?["payload"]
                    ["schema"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if schema == arete_artifacts::LIVE_SPEC_SCHEMA_V2 {
                    let artifact = load_live_spec_v2(&bytes)
                        .with_context(|| format!("Invalid V2 LiveSpec {}", path.display()))?
                        .artifact;
                    catalog
                        .v2_lives
                        .entry(artifact.artifact_hash.to_string())
                        .or_default()
                        .push((path, artifact));
                } else if schema == arete_artifacts::LIVE_SPEC_SCHEMA_V1 {
                    let artifact = load_live_spec(&bytes)
                        .with_context(|| format!("Invalid V1 LiveSpec {}", path.display()))?
                        .artifact;
                    catalog
                        .v1_lives
                        .entry(artifact.artifact_hash.to_string())
                        .or_default()
                        .push((path, artifact));
                } else {
                    bail!(
                        "Unsupported LiveSpec schema '{schema}' in {}",
                        path.display()
                    );
                }
            }
        }
        Ok(catalog)
    }

    fn unique_program(&self, hash: &str) -> Result<ProgramSpecArtifact> {
        unique_match(&self.programs, hash, "ProgramSpec")
    }

    fn unique_v1_live(&self, hash: &str) -> Result<LiveSpecArtifact> {
        unique_match(&self.v1_lives, hash, "V1 LiveSpec")
    }

    fn unique_v2_live(&self, hash: &str) -> Result<LiveSpecArtifactV2> {
        unique_match(&self.v2_lives, hash, "V2 LiveSpec")
    }
}

fn unique_match<T: Clone>(
    matches: &BTreeMap<String, Vec<(PathBuf, T)>>,
    hash: &str,
    kind: &str,
) -> Result<T> {
    let candidates = matches.get(hash).map(Vec::as_slice).unwrap_or_default();
    match candidates {
        [] => bail!("required {kind} {hash} was not found under approved artifact roots"),
        [(_, artifact)] => Ok(artifact.clone()),
        _ => bail!(
            "required {kind} {hash} is ambiguous across files: {}",
            candidates
                .iter()
                .map(|(path, _)| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn canonical_approved_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut canonical = BTreeSet::new();
    for root in roots {
        reject_parent_traversal(root, "artifact root")?;
        reject_symlink_path(root, "artifact root")?;
        let root = fs::canonicalize(root)
            .with_context(|| format!("Failed to resolve artifact root {}", root.display()))?;
        if !root.is_dir() {
            bail!("artifact root {} is not a directory", root.display());
        }
        if !canonical.insert(root.clone()) {
            bail!("duplicate canonical artifact root {}", root.display());
        }
    }
    Ok(canonical.into_iter().collect())
}

fn collect_artifact_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("Failed to read artifact root {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "symlink is not allowed under artifact roots: {}",
                path.display()
            );
        }
        if metadata.is_dir() {
            collect_artifact_files(root, &path, files)?;
        } else if metadata.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.ends_with(".program-spec.json") || name.ends_with(".live-spec.json")
                })
        {
            let canonical = fs::canonicalize(&path)?;
            if !canonical.starts_with(root) {
                bail!("artifact file escaped approved root: {}", path.display());
            }
            files.insert(canonical);
        }
    }
    Ok(())
}

fn reject_parent_traversal(path: &Path, kind: &str) -> Result<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!(
            "{kind} must not contain parent traversal: {}",
            path.display()
        );
    }
    Ok(())
}

fn reject_symlink_path(path: &Path, kind: &str) -> Result<()> {
    if fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {kind} {}", path.display()))?
        .file_type()
        .is_symlink()
    {
        bail!("{kind} must not be a symlink: {}", path.display());
    }
    Ok(())
}

pub fn build_program(input: &str, output: &str, program_id: Option<&str>) -> Result<()> {
    let input_path = PathBuf::from(input);
    let bytes = fs::read(&input_path)
        .with_context(|| format!("Failed to read IDL {}", input_path.display()))?;
    let payload = arete_hash::build_program_spec_v1_from_bytes(&bytes, program_id)
        .map_err(anyhow::Error::new)
        .with_context(|| format!("Failed to build ProgramSpec from {}", input_path.display()))?;
    let artifact = ProgramSpecArtifact::new(payload)?;
    let output_path = PathBuf::from(output);
    write_json(&output_path, &artifact)?;
    println!("ProgramSpec: {}", output_path.display());
    println!("ProgramSpec hash: {}", artifact.artifact_hash);
    Ok(())
}

pub fn build_live(input: &str, output: Option<String>, program_dir: Option<String>) -> Result<()> {
    let input_path = PathBuf::from(input);
    let bytes = fs::read(&input_path)
        .with_context(|| format!("Failed to read legacy stack {}", input_path.display()))?;
    let artifacts = normalize_legacy_stack_v2(&bytes)
        .with_context(|| format!("Failed to normalize {}", input_path.display()))?;
    let parent = input_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = input_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".stack.json"))
        .unwrap_or("LiveSpec");
    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| parent.join(format!("{stem}.live-spec.json")));
    let program_dir = program_dir.map(PathBuf::from).unwrap_or_else(|| {
        output_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    });
    fs::create_dir_all(&program_dir).with_context(|| {
        format!(
            "Failed to create ProgramSpec directory {}",
            program_dir.display()
        )
    })?;
    write_json(&output_path, &artifacts.live_spec)?;
    for program in &artifacts.legacy.program_specs {
        let name = &program.payload.idl_snapshot.snapshot.name;
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            bail!("ProgramSpec name '{name}' is not safe for an artifact filename");
        }
        write_json(
            &program_dir.join(format!("{name}.program-spec.json")),
            program,
        )?;
    }
    println!("LiveSpec: {}", output_path.display());
    println!("LiveSpec hash: {}", artifacts.live_spec.artifact_hash);
    println!("ProgramSpecs: {}", artifacts.legacy.program_specs.len());
    Ok(())
}

pub fn compose_stack(
    name: &str,
    program_paths: &[String],
    live_paths: &[String],
    artifact_dirs: &[String],
    selected_view_values: &[String],
    output: &str,
) -> Result<()> {
    if name.is_empty() {
        bail!("Stack name must not be empty");
    }
    let explicit_programs = program_paths
        .iter()
        .map(|path| load_program(path))
        .collect::<Result<Vec<_>>>()?;
    let live_specs = live_paths
        .iter()
        .map(|binding| {
            let (alias, path) = binding.split_once('=').with_context(|| {
                format!("--live must use alias=path syntax, received '{binding}'")
            })?;
            if alias.is_empty() || path.is_empty() {
                bail!("--live must use a non-empty alias and path");
            }
            Ok((alias.to_string(), load_live_v2(path)?))
        })
        .collect::<Result<Vec<_>>>()?;
    let roots = artifact_dirs.iter().map(PathBuf::from).collect::<Vec<_>>();
    let catalog = ArtifactCatalog::scan(&roots)?;
    let programs = resolve_composition_programs(&explicit_programs, &live_specs, &catalog)?;
    let selected = if selected_view_values.is_empty() {
        live_specs
            .iter()
            .flat_map(|(alias, live)| selected_views(alias, &live.payload))
            .collect()
    } else {
        parse_selected_views(selected_view_values)?
    };
    let manifest = stack_manifest_v2(
        name,
        &programs,
        live_specs
            .iter()
            .map(|(alias, live)| (alias.clone(), live))
            .collect(),
        selected,
    )?;
    let output = PathBuf::from(output);
    write_json(&output, &manifest)?;
    println!("StackManifest: {}", output.display());
    println!("StackManifest hash: {}", manifest.artifact_hash);
    Ok(())
}

fn parse_selected_views(values: &[String]) -> Result<Vec<SelectedViewV2>> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|value| {
            let (alias, view_id) = value.split_once('=').with_context(|| {
                format!("--selected-view must use alias=view_id syntax, received '{value}'")
            })?;
            if alias.is_empty() || view_id.is_empty() || view_id.contains('=') {
                bail!(
                    "--selected-view must use one non-empty alias and view ID, received '{value}'"
                );
            }
            if !seen.insert((alias, view_id)) {
                bail!("--selected-view '{value}' was supplied more than once");
            }
            Ok(SelectedViewV2 {
                live_alias: alias.to_string(),
                view_id: view_id.to_string(),
            })
        })
        .collect()
}

fn load_program(path: &str) -> Result<ProgramSpecArtifact> {
    let path = Path::new(path);
    reject_parent_traversal(path, "ProgramSpec path")?;
    reject_symlink_path(path, "ProgramSpec path")?;
    let bytes =
        fs::read(path).with_context(|| format!("Failed to read ProgramSpec {}", path.display()))?;
    Ok(load_program_spec(&bytes)
        .with_context(|| format!("Invalid ProgramSpec {}", path.display()))?
        .artifact)
}

fn load_live_v2(path: &str) -> Result<LiveSpecArtifactV2> {
    let path = Path::new(path);
    reject_parent_traversal(path, "LiveSpec path")?;
    reject_symlink_path(path, "LiveSpec path")?;
    let bytes =
        fs::read(path).with_context(|| format!("Failed to read LiveSpec {}", path.display()))?;
    Ok(load_live_spec_v2(&bytes)
        .with_context(|| format!("Invalid LiveSpec {}", path.display()))?
        .artifact)
}

fn resolve_composition_programs(
    explicit_programs: &[ProgramSpecArtifact],
    live_specs: &[(String, LiveSpecArtifactV2)],
    catalog: &ArtifactCatalog,
) -> Result<Vec<ProgramSpecArtifact>> {
    let mut explicit_by_hash = BTreeMap::new();
    for program in explicit_programs {
        if explicit_by_hash
            .insert(program.artifact_hash.to_string(), program.clone())
            .is_some()
        {
            bail!(
                "ProgramSpec {} was supplied more than once",
                program.artifact_hash
            );
        }
    }
    if live_specs.is_empty() {
        return Ok(explicit_programs.to_vec());
    }

    let mut requirements = Vec::<(String, String)>::new();
    let mut required_by_hash = BTreeMap::<String, String>::new();
    for (alias, live) in live_specs {
        for requirement in &live.payload.programs {
            let hash = requirement.program_spec_hash.to_string();
            if let Some(existing) = required_by_hash.get(&hash) {
                if existing != &requirement.program_id {
                    bail!(
                        "LiveSpec alias '{alias}' requires ProgramSpec {hash} with conflicting program ID '{}'",
                        requirement.program_id
                    );
                }
            } else {
                required_by_hash.insert(hash.clone(), requirement.program_id.clone());
                requirements.push((hash, requirement.program_id.clone()));
            }
        }
    }
    let mut programs = explicit_programs.to_vec();
    for (hash, program_id) in requirements {
        let (program, is_explicit) = match explicit_by_hash.get(&hash) {
            Some(program) => ((*program).clone(), true),
            None => (catalog.unique_program(&hash)?, false),
        };
        if program.payload.program_id != program_id {
            bail!(
                "ProgramSpec {hash} has program ID '{}', not '{}'",
                program.payload.program_id,
                program_id
            );
        }
        if !is_explicit {
            programs.push(program);
        }
    }
    Ok(programs)
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let bytes = arete_hash::canonicalize_jcs(value)?;
    arete_artifacts::atomic_write(path, &bytes)
        .with_context(|| format!("Failed to atomically write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arete_hash::{CanonicalIdlDocument, ProgramSpecV1};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn program() -> ProgramSpecArtifact {
        named_program("system", "11111111111111111111111111111111")
    }

    fn named_program(name: &str, address: &str) -> ProgramSpecArtifact {
        let idl = format!(
            r#"{{"address":"{address}","metadata":{{"name":"{name}","version":"1.0.0","spec":"0.1.0"}},"instructions":[],"accounts":[],"types":[],"events":[],"errors":[]}}"#
        );
        let document = CanonicalIdlDocument::parse(idl.as_bytes(), None).unwrap();
        ProgramSpecArtifact::new(ProgramSpecV1::from_document(&document)).unwrap()
    }

    fn live(programs: &[ProgramSpecArtifact]) -> LiveSpecArtifactV2 {
        LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
            programs
                .iter()
                .map(|program| arete_artifacts::ProgramRequirementV2 {
                    program_id: program.payload.program_id.clone(),
                    program_spec_hash: program.artifact_hash,
                })
                .collect(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap()
    }

    fn entity_live(program: &ProgramSpecArtifact, entity: &str) -> LiveSpecArtifactV2 {
        arete_artifacts::live_spec_v2(
            std::slice::from_ref(program),
            vec![arete_artifacts::PortableEntity::new(entity, "id.address")],
            Vec::new(),
        )
        .unwrap()
    }

    fn test_directory(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "arete-cli-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn composition_rejects_missing_program_requirements() {
        let program = program();
        let live = LiveSpecArtifactV2::new(arete_artifacts::LiveSpecV2::new(
            vec![arete_artifacts::ProgramRequirementV2 {
                program_id: program.payload.program_id.clone(),
                program_spec_hash: program.artifact_hash,
            }],
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        assert!(resolve_composition_programs(
            &[],
            &[("live".to_string(), live)],
            &ArtifactCatalog::default()
        )
        .is_err());
    }

    #[test]
    fn composition_preserves_explicit_independent_programs() {
        let independent = program();
        let required = named_program("required", "Required1111111111111111111111111111111");
        let resolved = resolve_composition_programs(
            &[independent.clone(), required.clone()],
            &[("live".to_string(), live(std::slice::from_ref(&required)))],
            &ArtifactCatalog::default(),
        )
        .unwrap();

        assert_eq!(
            resolved
                .iter()
                .map(|program| program.artifact_hash)
                .collect::<Vec<_>>(),
            vec![independent.artifact_hash, required.artifact_hash]
        );
    }

    #[test]
    fn local_loader_accepts_program_only_v2_manifests() {
        let directory = test_directory("program-only-v2");
        let program = program();
        let artifacts = arete_artifacts::author_stack_v2(arete_artifacts::StackAuthoringV2::new(
            "SystemProgram",
            vec![program.payload.clone()],
            Vec::new(),
        ))
        .unwrap();
        arete_artifacts::write_authored_stack_v2(&directory, "SystemProgram", &artifacts).unwrap();
        let loaded =
            load_local_artifact_stack(&directory.join("SystemProgram.stack-manifest.json"))
                .unwrap();
        assert!(loaded.live_specs.is_empty());
        assert_eq!(loaded.program_specs.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_loader_normalizes_v1_artifacts_to_v2() {
        let directory = test_directory("normalize-v1");
        let program = program();
        let live = arete_artifacts::LiveSpecArtifact::new(arete_artifacts::LiveSpecV1 {
            schema: arete_artifacts::LIVE_SPEC_SCHEMA_V1.to_string(),
            compiler_contract_version: "compiler/v1".to_string(),
            wire_contract_version: "wire/v1".to_string(),
            programs: vec![arete_artifacts::ProgramRequirementV1 {
                program_id: program.payload.program_id.clone(),
                program_spec_hash: program.artifact_hash,
            }],
            entities: Vec::new(),
            legacy_program_extensions: None,
        })
        .unwrap();
        let manifest =
            arete_artifacts::StackManifestArtifact::new(arete_artifacts::StackManifestV1 {
                schema: arete_artifacts::STACK_MANIFEST_SCHEMA_V1.to_string(),
                name: "Legacy".to_string(),
                programs: vec![arete_artifacts::ProgramSpecReferenceV1 {
                    program_id: program.payload.program_id.clone(),
                    artifact_hash: program.artifact_hash,
                }],
                live_specs: vec![arete_artifacts::LiveSpecReferenceV1 {
                    artifact_hash: live.artifact_hash,
                }],
                selected_views: Vec::new(),
                queries: Vec::new(),
                extensions: BTreeMap::new(),
                metadata: BTreeMap::new(),
            })
            .unwrap();
        write_json(&directory.join("system.program-spec.json"), &program).unwrap();
        write_json(&directory.join("Legacy.live-spec.json"), &live).unwrap();
        write_json(&directory.join("Legacy.stack-manifest.json"), &manifest).unwrap();

        let loaded =
            load_local_artifact_stack(&directory.join("Legacy.stack-manifest.json")).unwrap();
        assert_eq!(
            loaded.stack_manifest.payload.schema,
            arete_artifacts::STACK_MANIFEST_SCHEMA_V2
        );
        assert_eq!(
            loaded.live_specs[0].1.payload.schema,
            arete_artifacts::LIVE_SPEC_SCHEMA_V2
        );
        assert_eq!(loaded.live_specs[0].0, DEFAULT_LIVE_ALIAS);
        assert_eq!(loaded.live_specs.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn compose_selected_views_preserve_strict_user_order_and_omission_selects_all() {
        let directory = test_directory("selected-views");
        let program = program();
        let program_path = directory.join("system.program-spec.json");
        let alpha = entity_live(&program, "AlphaState");
        let beta = entity_live(&program, "BetaState");
        let alpha_path = directory.join("alpha.live-spec.json");
        let beta_path = directory.join("beta.live-spec.json");
        write_json(&program_path, &program).unwrap();
        write_json(&alpha_path, &alpha).unwrap();
        write_json(&beta_path, &beta).unwrap();
        let programs = [program_path.display().to_string()];
        let lives = [
            format!("alpha={}", alpha_path.display()),
            format!("beta={}", beta_path.display()),
        ];

        let all_output = directory.join("all.stack-manifest.json");
        compose_stack(
            "All",
            &programs,
            &lives,
            &[],
            &[],
            all_output.to_str().unwrap(),
        )
        .unwrap();
        let all = load_stack_manifest_v2(&fs::read(all_output).unwrap())
            .unwrap()
            .artifact;
        assert_eq!(
            all.payload
                .selected_views
                .iter()
                .map(|selected| { (selected.live_alias.as_str(), selected.view_id.as_str()) })
                .collect::<Vec<_>>(),
            vec![
                ("alpha", "AlphaState/state"),
                ("alpha", "AlphaState/list"),
                ("beta", "BetaState/state"),
                ("beta", "BetaState/list"),
            ]
        );

        let subset_output = directory.join("subset.stack-manifest.json");
        compose_stack(
            "Subset",
            &programs,
            &lives,
            &[],
            &[
                "beta=BetaState/list".to_string(),
                "alpha=AlphaState/state".to_string(),
            ],
            subset_output.to_str().unwrap(),
        )
        .unwrap();
        let subset = load_stack_manifest_v2(&fs::read(subset_output).unwrap())
            .unwrap()
            .artifact;
        assert_eq!(
            subset
                .payload
                .selected_views
                .iter()
                .map(|selected| { (selected.live_alias.as_str(), selected.view_id.as_str()) })
                .collect::<Vec<_>>(),
            vec![("beta", "BetaState/list"), ("alpha", "AlphaState/state")]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn compose_rejects_malformed_duplicate_and_unknown_selected_views() {
        for malformed in ["", "alpha", "=AlphaState/list", "alpha=", "alpha=a=b"] {
            assert!(parse_selected_views(&[malformed.to_string()]).is_err());
        }
        assert!(parse_selected_views(&[
            "alpha=AlphaState/list".to_string(),
            "alpha=AlphaState/list".to_string(),
        ])
        .unwrap_err()
        .to_string()
        .contains("more than once"));

        let directory = test_directory("invalid-selected-views");
        let program = program();
        let program_path = directory.join("system.program-spec.json");
        let live = entity_live(&program, "AlphaState");
        let live_path = directory.join("alpha.live-spec.json");
        write_json(&program_path, &program).unwrap();
        write_json(&live_path, &live).unwrap();
        let programs = [program_path.display().to_string()];
        let lives = [format!("alpha={}", live_path.display())];
        let output = directory.join("invalid.stack-manifest.json");

        assert!(compose_stack(
            "UnknownAlias",
            &programs,
            &lives,
            &[],
            &["missing=AlphaState/list".to_string()],
            output.to_str().unwrap(),
        )
        .unwrap_err()
        .to_string()
        .contains("declared LiveSpec alias"));
        assert!(compose_stack(
            "UnknownView",
            &programs,
            &lives,
            &[],
            &["alpha=AlphaState/missing".to_string()],
            output.to_str().unwrap(),
        )
        .unwrap_err()
        .to_string()
        .contains("does not exist"));
        assert!(compose_stack(
            "NoLives",
            &[],
            &[],
            &[],
            &["alpha=AlphaState/list".to_string()],
            output.to_str().unwrap(),
        )
        .unwrap_err()
        .to_string()
        .contains("declared LiveSpec alias"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn compose_supports_zero_two_and_three_ordered_lives() {
        let directory = test_directory("compose-multi");
        let artifacts = directory.join("artifacts");
        fs::create_dir_all(artifacts.join("nested")).unwrap();
        let program = program();
        write_json(&artifacts.join("nested/system.program-spec.json"), &program).unwrap();
        let shared = live(std::slice::from_ref(&program));
        let first_path = directory.join("first.live-spec.json");
        write_json(&first_path, &shared).unwrap();

        let zero_output = directory.join("zero.stack-manifest.json");
        compose_stack("Zero", &[], &[], &[], &[], zero_output.to_str().unwrap()).unwrap();
        let zero = load_stack_manifest_v2(&fs::read(&zero_output).unwrap())
            .unwrap()
            .artifact;
        assert!(zero.payload.live_specs.is_empty());

        let two_output = directory.join("two.stack-manifest.json");
        compose_stack(
            "Two",
            &[],
            &[
                format!("first={}", first_path.display()),
                format!("second={}", first_path.display()),
            ],
            &[artifacts.display().to_string()],
            &[],
            two_output.to_str().unwrap(),
        )
        .unwrap();
        let two = load_stack_manifest_v2(&fs::read(&two_output).unwrap())
            .unwrap()
            .artifact;
        assert!(!String::from_utf8(fs::read(&two_output).unwrap())
            .unwrap()
            .contains(&directory.display().to_string()));
        assert_eq!(
            two.payload
                .live_specs
                .iter()
                .map(|live| live.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );

        let three_output = directory.join("three.stack-manifest.json");
        compose_stack(
            "Three",
            &[],
            &[
                format!("first={}", first_path.display()),
                format!("second={}", first_path.display()),
                format!("third={}", first_path.display()),
            ],
            &[artifacts.display().to_string()],
            &[],
            three_output.to_str().unwrap(),
        )
        .unwrap();
        let loaded =
            load_local_artifact_stack_with_roots(&three_output, &[artifacts, directory.clone()])
                .unwrap();
        assert_eq!(loaded.live_specs.len(), 3);
        assert_eq!(loaded.live_specs[0].0, "first");
        assert_eq!(loaded.live_specs[1].0, "second");
        assert_eq!(loaded.live_specs[2].0, "third");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn multiple_artifact_roots_are_recursive_and_root_order_stable() {
        let directory = test_directory("root-order");
        let first_root = directory.join("first-root");
        let second_root = directory.join("second-root");
        fs::create_dir_all(first_root.join("deep")).unwrap();
        fs::create_dir_all(second_root.join("deeper")).unwrap();
        let system = program();
        let vote = named_program("vote", "Vote111111111111111111111111111111111111111");
        write_json(&first_root.join("deep/system.program-spec.json"), &system).unwrap();
        write_json(&second_root.join("deeper/vote.program-spec.json"), &vote).unwrap();
        let live = live(&[vote, system]);
        let live_path = directory.join("ordered.live-spec.json");
        write_json(&live_path, &live).unwrap();
        let first_output = directory.join("first.stack-manifest.json");
        let second_output = directory.join("second.stack-manifest.json");
        let bindings = [format!("ordered={}", live_path.display())];
        compose_stack(
            "Stable",
            &[],
            &bindings,
            &[
                first_root.display().to_string(),
                second_root.display().to_string(),
            ],
            &[],
            first_output.to_str().unwrap(),
        )
        .unwrap();
        compose_stack(
            "Stable",
            &[],
            &bindings,
            &[
                second_root.display().to_string(),
                first_root.display().to_string(),
            ],
            &[],
            second_output.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read(first_output).unwrap(),
            fs::read(second_output).unwrap()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn composition_preserves_independent_and_rejects_missing_ambiguous_and_traversing_artifacts() {
        let directory = test_directory("invalid-resolution");
        let root = directory.join("root");
        let duplicate_root = directory.join("duplicate-root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&duplicate_root).unwrap();
        let program = program();
        let unused = named_program("unused", "Unused1111111111111111111111111111111111111");
        let live = live(std::slice::from_ref(&program));
        let live_path = directory.join("live.live-spec.json");
        let program_path = root.join("system.program-spec.json");
        let unused_path = directory.join("unused.program-spec.json");
        write_json(&live_path, &live).unwrap();
        write_json(&program_path, &program).unwrap();
        write_json(&unused_path, &unused).unwrap();
        let binding = [format!("live={}", live_path.display())];
        let output = directory.join("out.stack-manifest.json");

        assert!(
            compose_stack("Missing", &[], &binding, &[], &[], output.to_str().unwrap())
                .unwrap_err()
                .to_string()
                .contains("was not found")
        );
        compose_stack(
            "Unused",
            &[
                program_path.display().to_string(),
                unused_path.display().to_string(),
            ],
            &binding,
            &[],
            &[],
            output.to_str().unwrap(),
        )
        .unwrap();
        let composed = load_stack_manifest_v2(&fs::read(&output).unwrap())
            .unwrap()
            .artifact;
        assert_eq!(composed.payload.programs.len(), 2);

        write_json(
            &duplicate_root.join("duplicate.program-spec.json"),
            &program,
        )
        .unwrap();
        assert!(compose_stack(
            "Ambiguous",
            &[],
            &binding,
            &[
                root.display().to_string(),
                duplicate_root.display().to_string(),
            ],
            &[],
            output.to_str().unwrap()
        )
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));

        let traversal = root.join("..").join("root");
        assert!(ArtifactCatalog::scan(&[traversal])
            .unwrap_err()
            .to_string()
            .contains("parent traversal"));
        assert!(ArtifactCatalog::scan(&[root.clone(), root])
            .unwrap_err()
            .to_string()
            .contains("duplicate canonical"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn approved_roots_reject_symlink_roots_files_and_directories() {
        use std::os::unix::fs::symlink;

        let directory = test_directory("symlinks");
        let root = directory.join("root");
        let outside = directory.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let program = program();
        write_json(&outside.join("system.program-spec.json"), &program).unwrap();

        let root_link = directory.join("root-link");
        symlink(&root, &root_link).unwrap();
        assert!(ArtifactCatalog::scan(&[root_link]).is_err());

        let file_link = root.join("linked.program-spec.json");
        symlink(outside.join("system.program-spec.json"), &file_link).unwrap();
        assert!(ArtifactCatalog::scan(std::slice::from_ref(&root)).is_err());
        fs::remove_file(&file_link).unwrap();

        let directory_link = root.join("linked-directory");
        symlink(&outside, &directory_link).unwrap();
        assert!(ArtifactCatalog::scan(&[root]).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
