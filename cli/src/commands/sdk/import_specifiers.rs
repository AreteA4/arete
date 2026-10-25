//! Relative import specifiers of generated TypeScript, matched to the
//! project that imports it.
//!
//! Generated modules, and the extension files published with a package,
//! import their siblings as `./name.js`: the `module: nodenext` convention,
//! which `tsc`, `tsx`, Vite and webpack-with-`extensionAlias` all map to
//! `./name.ts`. Turbopack (Next.js 16) does not, unless the project itself
//! resolves like Node (`moduleResolution: nodenext`), and a Next or Vite
//! project resolves like a bundler, where an extensionless `./name` is the
//! form every tool accepts. So when the output's nearest `tsconfig.json`
//! resolves like a bundler ([`runtime::tsconfig_resolves_like_a_bundler`]),
//! the `.js` is dropped from every relative specifier that names a
//! TypeScript file of the output. Nothing else changes, and a project with
//! no `tsconfig.json` or a Node resolution keeps the `.js`.
//!
//! The rewrite runs while the SDK's provenance is written, before its content
//! hash is taken, so `sdk-manifest.json` and `sdkOutputTreeHash` describe
//! the files as they are on disk.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;

use crate::project::runtime;

thread_local! {
    /// Where an output that is generated somewhere else (an install's
    /// staging directory) will finally live.
    static FINAL_OUTPUT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Restores the previous final output location when dropped.
pub(crate) struct FinalOutputGuard(Option<PathBuf>);

impl Drop for FinalOutputGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        FINAL_OUTPUT.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// While the guard lives, outputs written on this thread match the import
/// style of the project at `path`, the directory they are moved to once
/// generated, rather than that of the directory they are staged in.
pub(crate) fn generating_for(path: &Path) -> FinalOutputGuard {
    let previous = FINAL_OUTPUT.with(|cell| cell.replace(Some(path.to_path_buf())));
    FinalOutputGuard(previous)
}

/// Whether TypeScript written to `output_dir` drops `.js` from its relative
/// specifiers.
pub(super) fn extensionless_for(output_dir: &Path) -> bool {
    let probe = FINAL_OUTPUT
        .with(|cell| cell.borrow().clone())
        .unwrap_or_else(|| output_dir.to_path_buf());
    runtime::tsconfig_resolves_like_a_bundler(&probe)
}

fn is_typescript_module(name: &str) -> bool {
    [".ts", ".tsx", ".mts", ".cts"]
        .iter()
        .any(|extension| name.ends_with(extension))
}

/// `from '<spec>'`, `import '<spec>'` and `import('<spec>')`, for a relative
/// `<spec>` ending in `.js`. Capture 2 is the specifier without `.js`.
fn relative_js_specifier() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?m)(?P<lead>(?:^|[^\w$.])(?:from|import)\s*(?:\(\s*)?)(?P<quote>['"])(?P<path>\.\.?/[^'"\n]*?)\.js['"]"#,
        )
        .expect("relative import specifier pattern")
    })
}

/// `source` with `.js` dropped from every relative specifier for which
/// `names_typescript(path_without_extension)` holds; `None` when nothing
/// changed.
pub(super) fn without_js_extensions(
    source: &str,
    names_typescript: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut changed = false;
    let rewritten = relative_js_specifier().replace_all(source, |captures: &regex::Captures| {
        let path = &captures["path"];
        let quote = &captures["quote"];
        if names_typescript(path) {
            changed = true;
            format!("{}{quote}{path}{quote}", &captures["lead"])
        } else {
            captures[0].to_string()
        }
    });
    changed.then(|| rewritten.into_owned())
}

/// Whether `stem` (a path without extension) is a TypeScript module and not
/// a JavaScript one: what an extensionless specifier would resolve to.
fn names_typescript_module(stem: &Path) -> bool {
    let with = |extension: &str| PathBuf::from(format!("{}{extension}", stem.display()));
    !with(".js").is_file()
        && [".ts", ".tsx", ".d.ts"]
            .iter()
            .any(|extension| with(extension).is_file())
}

/// Drop `.js` from the relative specifiers of the TypeScript `artifacts`
/// (paths relative to `output_dir`) that name TypeScript modules, when the
/// project resolves like a bundler.
pub(super) fn match_project_import_style(output_dir: &Path, artifacts: &[String]) -> Result<()> {
    if !extensionless_for(output_dir) {
        return Ok(());
    }
    for name in artifacts.iter().filter(|name| is_typescript_module(name)) {
        let path = output_dir.join(name);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let directory = path.parent().unwrap_or(output_dir).to_path_buf();
        if let Some(rewritten) = without_js_extensions(&source, |specifier| {
            names_typescript_module(&directory.join(specifier))
        }) {
            fs::write(&path, rewritten)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &str) -> bool {
        true
    }

    #[test]
    fn drops_js_from_relative_imports_exports_and_dynamic_imports() {
        let source = "import { CORE } from './ore-core.js';\n\
            import type * as low from \"../ore-core.js\";\n\
            export * from './ore-core.js';\n\
            export { default } from './programs/ore/__arete-program.js';\n\
            import './side-effect.js';\n\
            type Low = typeof import('./ore-core.js');\n\
            const lazy = () => import( './lazy.js' );\n";
        assert_eq!(
            without_js_extensions(source, always).as_deref(),
            Some(
                "import { CORE } from './ore-core';\n\
                import type * as low from \"../ore-core\";\n\
                export * from './ore-core';\n\
                export { default } from './programs/ore/__arete-program';\n\
                import './side-effect';\n\
                type Low = typeof import('./ore-core');\n\
                const lazy = () => import( './lazy' );\n"
            )
        );
    }

    #[test]
    fn leaves_packages_absolute_urls_strings_and_unresolved_modules_alone() {
        let source = "import { Arete } from '@usearete/sdk';\n\
            import data from 'some-lib/index.js';\n\
            const url = './not-an-import.js';\n\
            const reimport = obj.import('./method.js');\n\
            export { x } from './plain.json';\n\
            import { y } from './shipped-javascript.js';\n";
        assert_eq!(
            without_js_extensions(source, |path| path != "./shipped-javascript"),
            None
        );
    }

    #[test]
    fn only_rewrites_specifiers_that_name_typescript_files_of_the_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"moduleResolution":"bundler"}}"#,
        )
        .unwrap();
        let output = root.join("sdk");
        fs::create_dir_all(output.join("helpers")).unwrap();
        fs::write(output.join("a-core.ts"), "export const CORE = 1;\n").unwrap();
        fs::write(output.join("plain.js"), "export const x = 1;\n").unwrap();
        fs::write(output.join("plain.d.ts"), "export const x: number;\n").unwrap();
        fs::write(
            output.join("helpers/alias.ts"),
            "export * from '../a-core.js';\n",
        )
        .unwrap();
        fs::write(
            output.join("a.ts"),
            "import { CORE } from './a-core.js';\nimport { x } from './plain.js';\nimport { gone } from './missing.js';\n",
        )
        .unwrap();
        let artifacts = ["a.ts", "a-core.ts", "helpers/alias.ts", "plain.js"]
            .map(String::from)
            .to_vec();

        match_project_import_style(&output, &artifacts).unwrap();

        assert_eq!(
            fs::read_to_string(output.join("a.ts")).unwrap(),
            "import { CORE } from './a-core';\nimport { x } from './plain.js';\nimport { gone } from './missing.js';\n"
        );
        assert_eq!(
            fs::read_to_string(output.join("helpers/alias.ts")).unwrap(),
            "export * from '../a-core';\n"
        );

        // Idempotent.
        match_project_import_style(&output, &artifacts).unwrap();
        assert_eq!(
            fs::read_to_string(output.join("helpers/alias.ts")).unwrap(),
            "export * from '../a-core';\n"
        );
    }

    #[test]
    fn a_node_project_and_a_staged_output_bound_for_one_keep_the_js() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let app = root.join("app");
        let staging = root.join(".arete/staging/output");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"moduleResolution":"bundler"}}"#,
        )
        .unwrap();
        fs::write(
            app.join("tsconfig.json"),
            r#"{"compilerOptions":{"module":"nodenext"}}"#,
        )
        .unwrap();
        fs::write(staging.join("b-core.ts"), "export {};\n").unwrap();
        fs::write(staging.join("b.ts"), "export * from './b-core.js';\n").unwrap();
        let artifacts = vec!["b.ts".to_string(), "b-core.ts".to_string()];

        {
            let _guard = generating_for(&app.join("sdk/b"));
            match_project_import_style(&staging, &artifacts).unwrap();
        }
        assert_eq!(
            fs::read_to_string(staging.join("b.ts")).unwrap(),
            "export * from './b-core.js';\n"
        );

        // Without the guard the staging directory's own project decides.
        match_project_import_style(&staging, &artifacts).unwrap();
        assert_eq!(
            fs::read_to_string(staging.join("b.ts")).unwrap(),
            "export * from './b-core';\n"
        );
    }
}
