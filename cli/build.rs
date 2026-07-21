use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            collect_files(&entry?.path(), files)?;
        }
    } else if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("generator input does not exist: {}", path.display()),
        ));
    }
    Ok(())
}

fn update_hash_part(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let workspace_dir = manifest_dir
        .parent()
        .ok_or("CLI manifest directory has no parent")?;
    let inputs = [
        "Cargo.lock",
        "Cargo.toml",
        "arete-idl/Cargo.toml",
        "arete-idl/src",
        "arete-macros/Cargo.toml",
        "arete-macros/src",
        "cli/Cargo.toml",
        "cli/build.rs",
        "cli/src",
        "interpreter/Cargo.toml",
        "interpreter/src",
    ];

    let mut files = Vec::new();
    for input in inputs {
        let path = workspace_dir.join(input);
        println!("cargo:rerun-if-changed={}", path.display());
        collect_files(&path, &mut files)?;
    }
    files.sort();

    let mut hasher = Sha256::new();
    for path in files {
        let relative_path = path
            .strip_prefix(workspace_dir)?
            .to_string_lossy()
            .replace('\\', "/");
        update_hash_part(&mut hasher, &relative_path, &fs::read(path)?);
    }

    let hash = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    println!("cargo:rustc-env=ARETE_SDK_GENERATOR_SHA256={hash}");
    Ok(())
}
