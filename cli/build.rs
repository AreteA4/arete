use arete_hash::{CompilerSourceV1, CompilerV1};
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

fn strip_local_dependency_versions(value: &mut toml::Value) {
    match value {
        toml::Value::Table(table) => {
            if table.contains_key("path") {
                table.remove("version");
            }
            for (_, value) in table.iter_mut() {
                strip_local_dependency_versions(value);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                strip_local_dependency_versions(value);
            }
        }
        _ => {}
    }
}

fn hash_contents(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let contents = fs::read(path)?;
    if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return Ok(contents);
    }

    // Release-only version changes do not alter generator behavior.
    let mut manifest = std::str::from_utf8(&contents)?.parse::<toml::Value>()?;
    if let Some(package) = manifest
        .get_mut("package")
        .and_then(toml::Value::as_table_mut)
    {
        package.remove("version");
    }
    strip_local_dependency_versions(&mut manifest);
    Ok(toml::to_string(&manifest)?.into_bytes())
}

/// The content type of a dev UI file.
fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn dev_ui_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            dev_ui_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Embed the dev UI `a4 dev up` serves: the Vite build of
/// `typescript/dev-ui` (or `A4_DEV_UI_DIST`), each file gzipped into
/// `OUT_DIR`. Without a build, `a4` serves a placeholder page, unless
/// `A4_REQUIRE_DEV_UI` is set (release builds), which fails the build.
fn embed_dev_ui(manifest_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write as _;
    println!("cargo:rerun-if-env-changed=A4_DEV_UI_DIST");
    println!("cargo:rerun-if-env-changed=A4_REQUIRE_DEV_UI");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let dist = match std::env::var_os("A4_DEV_UI_DIST").filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => {
            let package = manifest_dir.join("../typescript/dev-ui");
            let dist = package.join("dist");
            // In a checkout, watch dist/ even before the first UI build
            // (a missing path would rerun this script on every build).
            if package.is_dir() && !dist.exists() {
                let _ = fs::create_dir_all(&dist);
            }
            dist
        }
    };
    if dist.exists() {
        println!("cargo:rerun-if-changed={}", dist.display());
    }
    let required = std::env::var("A4_REQUIRE_DEV_UI")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false);
    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        dev_ui_files(&dist, &mut files)?;
    } else if required {
        return Err(format!(
            "A4_REQUIRE_DEV_UI is set but the dev UI is not built: no index.html in {}. Run `npm ci && npm run build` in typescript/dev-ui (or set A4_DEV_UI_DIST).",
            dist.display()
        )
        .into());
    }
    files.sort();
    let assets_dir = out_dir.join("dev-ui");
    let _ = fs::remove_dir_all(&assets_dir);
    fs::create_dir_all(&assets_dir)?;
    let mut table = String::from("/// Built by build.rs from the dev UI's Vite output.\n");
    table.push_str(&format!("pub const BUILT: bool = {};\n", !files.is_empty()));
    table.push_str("/// (URL path, content type, gzipped bytes).\n");
    table.push_str("pub static ASSETS: &[(&str, &str, &[u8])] = &[\n");
    for (index, file) in files.iter().enumerate() {
        let relative = file
            .strip_prefix(&dist)?
            .to_string_lossy()
            .replace('\\', "/");
        let target = assets_dir.join(format!("{index}.gz"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&fs::read(file)?)?;
        fs::write(&target, encoder.finish()?)?;
        table.push_str(&format!(
            "    ({:?}, {:?}, include_bytes!({:?})),\n",
            format!("/{relative}"),
            mime_of(file),
            target.display().to_string()
        ));
    }
    table.push_str("];\n");
    fs::write(out_dir.join("dev_ui_assets.rs"), table)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    embed_dev_ui(&manifest_dir)?;
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
    for package_name in [
        "arete-hash",
        "arete-idl",
        "arete-macros",
        "arete-interpreter",
    ] {
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

    let mut sources = Vec::with_capacity(files.len());
    for (label, path) in files {
        sources.push(CompilerSourceV1::new(label, hash_contents(&path)?));
    }

    let compiler_hash = CompilerV1::new(sources)?.hash()?;
    println!("cargo:rustc-env=ARETE_SDK_COMPILER_HASH={compiler_hash}");
    Ok(())
}
