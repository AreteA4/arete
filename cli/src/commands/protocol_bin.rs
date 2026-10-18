//! Locate the `arete-protocol` command from `@usearete/protocol`.
//!
//! The command comes from the project's own dependency. It is never fetched
//! from the npm registry: `npx arete-protocol` would look up an unrelated
//! package of that name, because the command and the package are named
//! differently.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// Overrides the lookup with an explicit `arete-protocol` executable.
pub const ARETE_PROTOCOL_BIN_ENV: &str = "ARETE_PROTOCOL_BIN";

/// `$ARETE_PROTOCOL_BIN`, else `node_modules/.bin/arete-protocol` in the
/// project or the nearest ancestor that has one.
pub fn resolve(project_root: &Path) -> Result<PathBuf> {
    resolve_with(
        project_root,
        std::env::var_os(ARETE_PROTOCOL_BIN_ENV).map(PathBuf::from),
    )
}

fn resolve_with(project_root: &Path, explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(std::path::absolute(path)?);
        }
        bail!("{ARETE_PROTOCOL_BIN_ENV}={} does not exist", path.display());
    }
    let name = if cfg!(windows) {
        "arete-protocol.cmd"
    } else {
        "arete-protocol"
    };
    let project_root = std::path::absolute(project_root)?;
    for dir in project_root.ancestors() {
        let candidate = dir.join("node_modules").join(".bin").join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "`arete-protocol` is not installed for {}. Add @usearete/protocol to the project \
         (`npm install --save-dev @usearete/protocol`), or set {ARETE_PROTOCOL_BIN_ENV} to \
         the `arete-protocol` executable (for example a local checkout's \
         typescript/protocol/dist/cli.js).",
        project_root.display()
    )
}

/// The program and leading arguments that run `bin`: a `.js`/`.mjs` file
/// (such as a checkout's `dist/cli.js`) runs under `node`.
pub fn invocation(bin: &Path) -> (PathBuf, Vec<PathBuf>) {
    match bin.extension().and_then(|ext| ext.to_str()) {
        Some("js" | "mjs" | "cjs") => (PathBuf::from("node"), vec![bin.to_path_buf()]),
        _ => (bin.to_path_buf(), Vec::new()),
    }
}

/// A blocking [`std::process::Command`] for `bin`.
pub fn command(bin: &Path) -> std::process::Command {
    let (program, args) = invocation(bin);
    let mut command = std::process::Command::new(program);
    command.args(args);
    command
}

/// An async [`tokio::process::Command`] for `bin`.
pub fn tokio_command(bin: &Path) -> tokio::process::Command {
    let (program, args) = invocation(bin);
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_project_or_an_ancestor_install() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app/protocol");
        std::fs::create_dir_all(&project).unwrap();
        assert!(resolve_with(&project, None).is_err());
        let bin = root.path().join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) {
            "arete-protocol.cmd"
        } else {
            "arete-protocol"
        };
        std::fs::write(bin.join(name), "").unwrap();
        assert_eq!(resolve_with(&project, None).unwrap(), bin.join(name));
    }

    #[test]
    fn relative_compiler_paths_survive_a_child_directory_change() {
        let cwd = std::env::current_dir().unwrap();
        let root = tempfile::tempdir_in(&cwd).unwrap();
        let project = root.path().join("my-protocol");
        let bin = project.join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = project.join("compiler.js");
        std::fs::write(&script, "console.log('built');\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let entry = bin.join("arete-protocol");
            std::fs::write(&entry, "#!/usr/bin/env node\nconsole.log('built');\n").unwrap();
            std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        std::fs::write(
            bin.join("arete-protocol.cmd"),
            "@echo off\r\nnode \"%~dp0..\\..\\compiler.js\"\r\n",
        )
        .unwrap();
        let relative_project = project.strip_prefix(&cwd).unwrap();
        let relative_script = script.strip_prefix(&cwd).unwrap();
        for explicit in [None, Some(relative_script.to_path_buf())] {
            let executable = resolve_with(relative_project, explicit).unwrap();
            assert!(executable.is_absolute());
            let output = command(&executable)
                .current_dir(relative_project)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "built");
        }
    }

    #[test]
    fn an_explicit_path_must_exist() {
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_with(root.path(), Some(root.path().join("missing"))).is_err());
        let file = root.path().join("cli.js");
        std::fs::write(&file, "").unwrap();
        assert_eq!(resolve_with(root.path(), Some(file.clone())).unwrap(), file);
    }
}
