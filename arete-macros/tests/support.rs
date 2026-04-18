use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub fn escape_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

pub fn macro_manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn workspace_root() -> PathBuf {
    macro_manifest_dir()
        .parent()
        .expect("arete-macros should live in workspace root")
        .to_path_buf()
}

#[allow(dead_code)]
pub fn arete_dir() -> PathBuf {
    workspace_root().join("arete")
}

pub fn cargo_toml(name: &str, dependencies: &[String]) -> String {
    let dependencies = dependencies.join("\n");

    format!(
        r#"[package]
name = "{name}"
version = "0.0.0"
edition = "2021"

[workspace]

[dependencies]
{dependencies}
"#
    )
}

pub struct TempCrate {
    workspace_root: PathBuf,
    temp_dir: TempDir,
}

impl TempCrate {
    pub fn new(
        test_subdir: &str,
        name: &str,
        cargo_toml: String,
        source: &str,
        extra_files: &[(&str, &str)],
    ) -> Self {
        let workspace_root = workspace_root();
        let temp_root = workspace_root.join("target/tests").join(test_subdir);
        fs::create_dir_all(&temp_root).expect("create dynamic test root");

        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("{name}-"))
            .tempdir_in(&temp_root)
            .expect("create temp crate dir");
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("create temp crate src dir");

        fs::write(temp_dir.path().join("Cargo.toml"), cargo_toml).expect("write temp Cargo.toml");
        fs::write(src_dir.join("main.rs"), source).expect("write temp main.rs");

        for (relative_path, contents) in extra_files {
            let file_path = temp_dir.path().join(relative_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).expect("create extra file parent dir");
            }
            fs::write(file_path, contents).expect("write extra test file");
        }

        Self {
            workspace_root,
            temp_dir,
        }
    }

    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    #[allow(dead_code)]
    pub fn cargo_check(&self) -> Output {
        Command::new("cargo")
            .arg("check")
            .arg("--quiet")
            .current_dir(self.path())
            .env("CARGO_TARGET_DIR", self.workspace_root.join("target"))
            .output()
            .expect("run cargo check")
    }

    #[allow(dead_code)]
    pub fn cargo_run(&self) -> Output {
        Command::new("cargo")
            .arg("run")
            .arg("--quiet")
            .current_dir(self.path())
            .env("CARGO_TARGET_DIR", self.workspace_root.join("target"))
            .output()
            .expect("run cargo run")
    }
}
