use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect_files(
    root: &Path,
    path: &Path,
    package_name: &str,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            collect_files(root, &entry?.path(), package_name, files)?;
        }
    } else if path.is_file() {
        let is_rust_source =
            path.extension().and_then(|extension| extension.to_str()) == Some("rs");
        let is_manifest = path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml");
        if !is_rust_source && !is_manifest {
            return Ok(());
        }
        let relative_path = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        files.push((
            format!("{package_name}/{relative_path}"),
            path.to_path_buf(),
        ));
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("generator input does not exist: {}", path.display()),
        )
        .into());
    }
    Ok(())
}

fn dependency_root(
    metadata: &serde_json::Value,
    package_name: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata did not return a packages array")?;
    let mut matches = packages
        .iter()
        .filter(|package| package["name"].as_str() == Some(package_name));
    let package = matches
        .next()
        .ok_or_else(|| format!("cargo metadata did not resolve {package_name}"))?;
    if matches.next().is_some() {
        return Err(format!("cargo metadata resolved multiple versions of {package_name}").into());
    }
    let manifest_path = package["manifest_path"]
        .as_str()
        .ok_or_else(|| format!("cargo metadata omitted {package_name}'s manifest path"))?;
    Ok(PathBuf::from(manifest_path)
        .parent()
        .ok_or_else(|| format!("{package_name}'s manifest path has no parent"))?
        .to_path_buf())
}

fn update_hash_part(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let metadata_output = Command::new(std::env::var("CARGO")?)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest_dir.join("Cargo.toml"))
        .output()?;
    if !metadata_output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata_output.stderr)
        )
        .into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata_output.stdout)?;

    let mut package_roots = vec![("cli", manifest_dir)];
    for package_name in ["arete-idl", "arete-macros", "arete-interpreter"] {
        package_roots.push((package_name, dependency_root(&metadata, package_name)?));
    }

    let mut files = Vec::new();
    for (package_name, root) in package_roots {
        for input in ["Cargo.toml", "src"] {
            let path = root.join(input);
            println!("cargo:rerun-if-changed={}", path.display());
            collect_files(&root, &path, package_name, &mut files)?;
        }
        if package_name == "cli" {
            let path = root.join("build.rs");
            println!("cargo:rerun-if-changed={}", path.display());
            collect_files(&root, &path, package_name, &mut files)?;
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (label, path) in files {
        update_hash_part(&mut hasher, &label, &fs::read(path)?);
    }

    let hash = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    println!("cargo:rustc-env=ARETE_SDK_GENERATOR_SHA256={hash}");
    Ok(())
}
