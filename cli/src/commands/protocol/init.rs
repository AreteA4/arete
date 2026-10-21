//! `a4 protocol init [dir]`: scaffold a protocol project.
//!
//! Writes a template-only tip jar (`protocol.ts`, no handler code) that pays
//! through the System Program, with everything `arete-protocol check`,
//! `a4 dev up` and `a4 protocol deploy` read:
//!
//! - `arete.toml`, then the System Program from the registry, as `a4 install
//!   program system-program --as system --ts` installs it: `arete.lock`,
//!   `generated/typescript/programs/system/` and `@programs/*` in
//!   `tsconfig.json`;
//! - `package.json` (`@usearete/protocol`, `@usearete/sdk` and `zod`, which
//!   the generated program module imports), `tsconfig.json`, `.gitignore`
//!   and `README.md`.
//!
//! The default requires the registry's reviewed System Program package. A
//! registry failure is actionable, not permission to substitute a different
//! program identity. `--local-idl` explicitly opts into offline scaffolding;
//! hosted execution still requires admission of that release.
//!
//! Generated clients are committed, as `a4 install` expects (`a4 install
//! --check` compares them with `arete.lock`), so `.gitignore` leaves
//! `generated/` in and ignores `node_modules/` and `.arete/` (build output
//! and `a4 dev` state). Existing files are never overwritten: init refuses
//! when any file it writes exists, except `.gitignore`, which gains the
//! missing lines.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;
use serde::Serialize;
use serde_json::json;

use crate::project::installer::{add_and_install, AddDependencyOptions};
use crate::project::manifest::InstallTarget;
use crate::project::runtime::{lockstep_version, ZOD_RANGE};

/// The `@usearete/protocol` range a new project depends on.
pub const PROTOCOL_PACKAGE_RANGE: &str = "^0.2.1"; // x-release-please-version

/// The registry package init installs the System Program from.
pub const SYSTEM_PROGRAM_PACKAGE: &str = "system-program";
/// The local alias of the System Program (`@programs/system`).
const SYSTEM_ALIAS: &str = "system";
/// Where the fallback IDL is written, relative to the project.
const SYSTEM_IDL_PATH: &str = "idl/system.idl.json";

/// The System Program's `transfer` instruction, the one the template uses,
/// with the registry program's names, for installs without the registry.
const SYSTEM_IDL: &str = r#"{
  "address": "11111111111111111111111111111111",
  "metadata": {
    "name": "system_program",
    "version": "1.0.0",
    "spec": "0.1.0",
    "description": "The System Program's transfer instruction (u32 LE discriminant 2), the one the tip jar uses, named as the registry's system-program package names it. Add instructions as the protocol needs them, then run a4 install."
  },
  "instructions": [
    {
      "name": "transfer",
      "discriminator": [2, 0, 0, 0],
      "accounts": [
        { "name": "fundingAccount", "writable": true, "signer": true },
        { "name": "recipientAccount", "writable": true }
      ],
      "args": [{ "name": "lamports", "type": "u64" }]
    }
  ],
  "accounts": [],
  "types": [],
  "events": [],
  "errors": []
}
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "paths": {
      "@programs/*": ["./generated/typescript/programs/*/index.js"]
    }
  },
  "include": ["protocol.ts", "handlers", "generated"]
}
"#;

/// Lines `.gitignore` must hold.
const GITIGNORE_LINES: [&str; 2] = ["node_modules/", ".arete/"];

/// Options for `a4 protocol init`.
#[derive(Args, Debug, Clone, Default)]
pub struct InitArgs {
    /// Project directory (created when missing)
    #[arg(default_value = ".")]
    pub dir: PathBuf,

    /// The protocol's name in the spec (`^[a-z][A-Za-z0-9]*$`; default: the
    /// directory name in camelCase)
    #[arg(long)]
    pub name: Option<String>,

    /// Write the files but don't install the System Program (no
    /// `arete.lock` or generated program; run `a4 install` later)
    #[arg(long)]
    pub no_install: bool,

    /// Install the System Program from a local IDL (`idl/system.idl.json`)
    /// instead of the registry, with no API access
    #[arg(long, conflicts_with = "no_install")]
    pub local_idl: bool,
}

/// Where init installed the System Program from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemProgramSource {
    /// The registry's `system-program` package.
    Registry,
    /// `idl/system.idl.json`.
    LocalIdl,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitOutcome {
    pub dir: String,
    pub name: String,
    pub written: Vec<String>,
    pub installed: bool,
    /// Where the System Program came from; absent with `--no-install`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_program: Option<SystemProgramSource>,
    pub warnings: Vec<String>,
}

/// `my-tip_jar 2` → `myTipJar2`; `None` when nothing usable is left.
pub fn protocol_name_from(dir_name: &str) -> Option<String> {
    let mut name = String::new();
    let mut upper = false;
    for character in dir_name.chars() {
        if !character.is_ascii_alphanumeric() {
            upper = !name.is_empty();
            continue;
        }
        if name.is_empty() {
            if !character.is_ascii_alphabetic() {
                continue;
            }
            name.push(character.to_ascii_lowercase());
        } else if upper {
            name.push(character.to_ascii_uppercase());
        } else {
            name.push(character);
        }
        upper = false;
    }
    name.truncate(64);
    (!name.is_empty()).then_some(name)
}

fn valid_protocol_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(u8::is_ascii_alphanumeric)
}

/// The npm package name for the project directory.
fn package_name(dir_name: &str) -> String {
    let mut name = String::new();
    for character in dir_name.chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character.to_ascii_lowercase());
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_end_matches('-').to_string();
    if name.is_empty() {
        "arete-protocol-project".into()
    } else {
        name
    }
}

pub fn protocol_ts(name: &str) -> String {
    format!(
        r#"/**
 * `{name}`: a tip jar built from instruction templates only (no handler code).
 *
 * - `open` (the create step; the creator signs) sends `amount` lamports to the
 *   recipient with a System Program transfer and names the recipient. The
 *   instance starts `open`.
 * - `close` (the recipient signs) sends `amount` lamports back to the creator
 *   and moves the instance to `closed`, which emits `tipJar.closed`.
 *
 * `@programs/system` is the System Program, installed as
 * `a4 install program system-program --as system --ts` does. See README.md.
 */
import system from "@programs/system";
import {{ defineProtocol, ix }} from "@usearete/protocol";

export default defineProtocol({{
  name: "{name}",
  version: "0.1.0",
  description: "A tip jar: two templated System Program transfers.",
  programs: {{ system }},
  params: {{ amount: "u64" }},
  roles: {{ recipient: {{ description: "Receives the tip and closes the jar." }} }},
  instanceVisibility: "shared",
  machines: {{
    instance: {{
      initial: "open",
      states: {{
        open: "recorded",
        closed: {{ kind: "recorded", terminal: true }},
      }},
    }},
  }},
  steps: {{
    open: {{
      scope: "create",
      title: "Open a tip jar",
      authority: "creator",
      args: {{ recipient: "pubkey", amount: "u64" }},
      guards: {{ amount_zero: "args.amount > 0" }},
      bind: {{ accounts: {{ creator: "wallet" }}, roles: {{ recipient: "args.recipient" }} }},
      build: [
        ix(system, "transfer", {{
          args: {{ lamports: "args.amount" }},
          accounts: {{ fundingAccount: "wallet", recipientAccount: "args.recipient" }},
        }}),
      ],
    }},
    close: {{
      scope: "instance",
      title: "Close the tip jar",
      authority: {{ kind: "role", role: "recipient" }},
      args: {{ amount: "u64" }},
      guards: {{
        not_recipient: "wallet == instance.roles.recipient",
        not_open: 'instance.state == "open"',
      }},
      transitions: [{{ from: ["open"], to: "closed" }}],
      build: [
        ix(system, "transfer", {{
          args: {{ lamports: "args.amount" }},
          accounts: {{ fundingAccount: "wallet", recipientAccount: "instance.accounts.creator" }},
        }}),
      ],
    }},
  }},
  reactions: [
    {{
      id: "announceClosed",
      trigger: {{ kind: "onTransition", machine: "instance", from: [], to: ["closed"] }},
      action: {{ kind: "emit", event: "tipJar.closed" }},
    }},
  ],
  events: {{ "tipJar.closed": "instance" }},
  // The protocol stack (`a4 stream --dev`) tracks both transfers:
  // `fundingAccount` pays, `recipientAccount` receives. The recipient is a
  // wallet, not an account the instance owns, so only transfers sent through
  // Arete count.
  stack: {{
    activity: [
      {{ step: "open", program: "system", instruction: "transfer", instanceAccount: "recipientAccount", walletAccount: "fundingAccount", amountArg: "lamports" }},
      {{ step: "close", program: "system", instruction: "transfer", instanceAccount: "recipientAccount", walletAccount: "fundingAccount", amountArg: "lamports" }},
    ],
  }},
}});
"#
    )
}

fn package_json(dir_name: &str) -> Result<String> {
    let value = json!({
        "name": package_name(dir_name),
        "private": true,
        "type": "module",
        "scripts": {
            "check": "arete-protocol check",
            "build": "arete-protocol build"
        },
        "dependencies": {
            "@usearete/protocol": PROTOCOL_PACKAGE_RANGE,
            "@usearete/sdk": lockstep_version(),
            "zod": ZOD_RANGE
        },
        "devDependencies": {
            "typescript": "^5.6.0"
        }
    });
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

fn readme(dir_name: &str, name: &str, local_idl: bool) -> String {
    let system_source = if local_idl {
        "its `transfer` instruction, from `idl/system.idl.json`"
    } else {
        "the registry's `system-program` package"
    };
    format!(
        r#"# {dir_name}

An Arete protocol, `{name}`, created by `a4 protocol init`: a tip jar whose
two steps are System Program transfers built from templates.

| File | What it is |
| --- | --- |
| `protocol.ts` | the protocol definition (`defineProtocol`) |
| `generated/typescript/programs/system/` | the System Program ({system_source}), imported as `@programs/system` |
| `arete.toml`, `arete.lock` | the project's dependencies and their pinned versions |

`generated/` and `arete.lock` are committed; `a4 install --check` checks them
in CI. `.arete/` (build output and local session state) is not.

## Develop

```sh
npm install
npx arete-protocol check          # type-check and validate the definition
a4 dev up                         # local fork + runtime; redeploys on every change
a4 dev step open --args '{{"recipient":"<pubkey>","amount":"1000000"}}'
a4 stream --dev                   # the protocol's instances and activity
```

Add a program: `a4 install program <registry package> --ts`, or put its IDL
in `idl/` and run `a4 install program ./idl/<name>.json --as <alias> --ts`,
then import it from `@programs/<alias>`.

Arete includes any applicable fee in the prepared transaction. The app should
display that fee before signing. `close` accepts an amount chosen by its caller;
returning funds does not automatically make it an exempt refund.

## Deploy

```sh
a4 protocol deploy
```

A program installed from a local IDL is pushed to your account as a private
program first (`a4 program push`), and `arete.lock` records its program
release.

Docs: https://docs.arete.run/cli/local-development/ and
https://docs.arete.run/cli/commands/.
"#
    )
}

/// `a4 protocol init`.
pub fn init(args: &InitArgs, json: bool) -> Result<()> {
    let outcome = scaffold(args)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    for warning in &outcome.warnings {
        crate::ui::print_warning(warning);
    }
    println!(
        "{} {} in {}",
        "Created".green().bold(),
        outcome.name.bold(),
        outcome.dir
    );
    for file in &outcome.written {
        println!("  {file}");
    }
    match outcome.system_program {
        Some(SystemProgramSource::Registry) => println!(
            "  System Program: registry package {SYSTEM_PROGRAM_PACKAGE} (@programs/{SYSTEM_ALIAS})"
        ),
        Some(SystemProgramSource::LocalIdl) => println!(
            "  System Program: {SYSTEM_IDL_PATH} (@programs/{SYSTEM_ALIAS}; `a4 protocol deploy` pushes it as a private program)"
        ),
        None => {}
    }
    let cd = if Path::new(&outcome.dir) == Path::new(".") {
        String::new()
    } else {
        format!("cd {} && ", outcome.dir)
    };
    println!();
    println!("Next: {cd}npm install && npx arete-protocol check && a4 dev up");
    if !outcome.installed {
        println!(
            "Run `a4 install program {SYSTEM_PROGRAM_PACKAGE} --as {SYSTEM_ALIAS} --ts` first to generate @programs/system."
        );
    }
    Ok(())
}

/// Write the project.
pub fn scaffold(args: &InitArgs) -> Result<InitOutcome> {
    scaffold_with(args, install_from_registry)
}

/// [`scaffold`], with the registry install as a seam for tests.
fn scaffold_with(
    args: &InitArgs,
    registry_install: impl FnOnce(&Path) -> Result<()>,
) -> Result<InitOutcome> {
    let dir = &args.dir;
    std::fs::create_dir_all(dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    let absolute = std::fs::canonicalize(dir)?;
    let dir_name = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("protocol")
        .to_string();
    let name = match &args.name {
        Some(name) if valid_protocol_name(name) => name.clone(),
        Some(name) => bail!("--name '{name}' must match ^[a-z][A-Za-z0-9]{{0,63}}$ (e.g. tipJar)"),
        None => protocol_name_from(&dir_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Can't derive a protocol name from '{dir_name}'; pass --name (e.g. --name tipJar)"
            )
        })?,
    };

    let files: Vec<(&str, String)> = vec![
        (
            "arete.toml",
            crate::commands::config::new_manifest_contents(&absolute, Some(dir_name.clone()))?,
        ),
        ("package.json", package_json(&dir_name)?),
        ("tsconfig.json", TSCONFIG.to_string()),
        ("protocol.ts", protocol_ts(&name)),
    ];
    // The fallback IDL is checked up front too, so a fallback never meets
    // an existing file after the rest are written.
    let checked: Vec<&str> = files
        .iter()
        .map(|(path, _)| *path)
        .chain(["README.md", SYSTEM_IDL_PATH])
        .collect();
    let existing: Vec<&str> = checked
        .into_iter()
        .filter(|path| dir.join(path).exists())
        .collect();
    if !existing.is_empty() {
        bail!(
            "{} already has {}; a4 protocol init only writes new files (use an empty directory)",
            dir.display(),
            existing.join(", ")
        );
    }
    let mut written = Vec::new();
    for (path, content) in &files {
        write_new(dir, path, content)?;
        written.push(path.to_string());
    }
    if update_gitignore(&dir.join(".gitignore"))? {
        written.push(".gitignore".into());
    }

    let warnings = Vec::new();
    let system_program = if args.no_install {
        None
    } else if args.local_idl {
        Some(SystemProgramSource::LocalIdl)
    } else {
        registry_install(&absolute).with_context(|| format!(
            "Could not install the reviewed {SYSTEM_PROGRAM_PACKAGE} registry package. Fix the registry/authentication error and run `a4 install program {SYSTEM_PROGRAM_PACKAGE} --as {SYSTEM_ALIAS} --ts` in {}. No local IDL was substituted. For intentional offline scaffolding in a new directory, use `a4 protocol init --local-idl`; hosted execution still requires admission",
            absolute.display()
        ))?;
        Some(SystemProgramSource::Registry)
    };
    if system_program == Some(SystemProgramSource::LocalIdl) {
        write_new(dir, SYSTEM_IDL_PATH, SYSTEM_IDL)?;
        written.push(SYSTEM_IDL_PATH.into());
        install_local_system(&absolute)?;
    }
    let local_idl = system_program == Some(SystemProgramSource::LocalIdl);
    write_new(dir, "README.md", &readme(&dir_name, &name, local_idl))?;
    written.push("README.md".into());
    if system_program.is_some() {
        written.push("arete.lock".into());
        written.push("generated/typescript/programs/system/".into());
    }
    Ok(InitOutcome {
        dir: dir.display().to_string(),
        name,
        written,
        installed: system_program.is_some(),
        system_program,
        warnings,
    })
}

fn write_new(dir: &Path, path: &str, content: &str) -> Result<()> {
    let target = dir.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)
        .with_context(|| format!("Failed to write {}", target.display()))
}

/// Add the missing [`GITIGNORE_LINES`]. Returns whether the file changed.
fn update_gitignore(path: &Path) -> Result<bool> {
    let current = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    let present: Vec<&str> = current.lines().map(str::trim).collect();
    let missing: Vec<&str> = GITIGNORE_LINES
        .iter()
        .copied()
        .filter(|line| {
            let bare = line.trim_end_matches('/');
            !present
                .iter()
                .any(|have| *have == *line || *have == bare || *have == format!("/{line}"))
        })
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if text.is_empty() {
        text.push_str(
            "# Build output and local `a4 dev` state. generated/ and arete.lock are committed.\n",
        );
    }
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(true)
}

/// `a4 install program system-program --as system --ts` in `root`.
fn install_from_registry(root: &Path) -> Result<()> {
    install_system(root, SYSTEM_PROGRAM_PACKAGE)
}

/// `a4 install program ./idl/system.idl.json --as system --ts` in `root`.
fn install_local_system(root: &Path) -> Result<()> {
    install_system(root, &root.join(SYSTEM_IDL_PATH).display().to_string()).with_context(|| {
        format!("Installing the System Program from {SYSTEM_IDL_PATH} failed; fix it and run `a4 install`")
    })
}

fn install_system(root: &Path, package: &str) -> Result<()> {
    add_and_install(
        root.join("arete.toml"),
        crate::project::manifest::DependencyKind::Program,
        package,
        AddDependencyOptions {
            alias: Some(SYSTEM_ALIAS.into()),
            target: Some(InstallTarget::TypeScript),
            // init's own summary lists the files; the install's npm hints
            // repeat what package.json already declares.
            quiet: true,
            ..AddDependencyOptions::default()
        },
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_directory() {
        assert_eq!(
            protocol_name_from("my-tip_jar 2").as_deref(),
            Some("myTipJar2")
        );
        assert_eq!(protocol_name_from("TipJar").as_deref(), Some("tipJar"));
        assert_eq!(protocol_name_from("2-fast").as_deref(), Some("fast"));
        assert_eq!(protocol_name_from("--"), None);
        assert!(valid_protocol_name("tipJar") && !valid_protocol_name("TipJar"));
        assert!(!valid_protocol_name("tip-jar") && !valid_protocol_name(""));
        assert_eq!(package_name("My Tip_Jar"), "my-tip-jar");
    }

    /// init, then `arete-protocol check` with this checkout's packages
    /// linked in as `npm install` would provide them. Needs `node` and the
    /// built packages (`npm ci && npm run build` in typescript/protocol and
    /// typescript/core); skipped without them.
    #[test]
    fn the_scaffold_passes_arete_protocol_check() {
        let packages = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../typescript");
        let cli = packages.join("protocol/dist/cli.js");
        let zod = packages.join("core/node_modules/zod");
        let node = std::process::Command::new("node").arg("--version").output();
        if !cli.is_file() || !zod.is_dir() || node.is_err() {
            eprintln!("skipped: needs node and the built typescript/protocol and typescript/core");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        // Explicit offline scaffold: the registry is never contacted in tests.
        let outcome = scaffold_with(
            &InitArgs {
                dir: root.clone(),
                local_idl: true,
                ..InitArgs::default()
            },
            |_| panic!("explicit local IDL must not contact the registry"),
        )
        .unwrap();
        assert!(outcome.installed);
        assert_eq!(outcome.system_program, Some(SystemProgramSource::LocalIdl));
        assert!(outcome.warnings.is_empty());
        assert!(root
            .join("generated/typescript/programs/system/index.ts")
            .is_file());
        assert!(root.join("arete.lock").is_file());
        let modules = root.join("node_modules");
        std::fs::create_dir_all(modules.join("@usearete")).unwrap();
        for (link, target) in [
            ("@usearete/protocol", packages.join("protocol")),
            ("@usearete/sdk", packages.join("core")),
            ("zod", zod),
        ] {
            std::os::unix::fs::symlink(std::fs::canonicalize(target).unwrap(), modules.join(link))
                .unwrap();
        }
        let output = std::process::Command::new("node")
            .arg(&cli)
            .arg("check")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "arete-protocol check failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("ok: tipJar@0.1.0"));
    }

    #[test]
    fn writes_the_project_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".gitignore"), "dist\nnode_modules\n").unwrap();
        let args = InitArgs {
            dir: root.clone(),
            no_install: true,
            ..InitArgs::default()
        };
        let outcome = scaffold(&args).unwrap();
        assert_eq!(outcome.name, "tipJar");
        assert!(!outcome.installed);
        assert_eq!(outcome.system_program, None);
        for file in [
            "arete.toml",
            "package.json",
            "tsconfig.json",
            "protocol.ts",
            "README.md",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        // No install, so no fallback IDL.
        assert!(!root.join(SYSTEM_IDL_PATH).exists());
        assert!(std::fs::read_to_string(root.join("protocol.ts"))
            .unwrap()
            .contains(r#"name: "tipJar""#));
        let package: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(package["name"], "tip-jar");
        let protocol_manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../typescript/protocol/package.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            package["dependencies"]["@usearete/protocol"],
            format!("^{}", protocol_manifest["version"].as_str().unwrap()),
            "scaffolds must use the protocol package's released version range",
        );
        for dependency in ["@usearete/protocol", "@usearete/sdk", "zod"] {
            assert!(
                package["dependencies"][dependency].is_string(),
                "{dependency}"
            );
        }
        let tsconfig: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("tsconfig.json")).unwrap())
                .unwrap();
        assert_eq!(
            tsconfig["compilerOptions"]["paths"]["@programs/*"][0],
            "./generated/typescript/programs/*/index.js"
        );
        // `.gitignore` keeps its lines and gains `.arete/`, not generated/.
        let gitignore = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "dist\nnode_modules\n.arete/\n");
        let idl: serde_json::Value = serde_json::from_str(SYSTEM_IDL).unwrap();
        assert_eq!(idl["instructions"][0]["name"], "transfer");
        // The registry program's names, which protocol.ts uses either way.
        let accounts: Vec<&str> = idl["instructions"][0]["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|account| account["name"].as_str().unwrap())
            .collect();
        assert_eq!(accounts, ["fundingAccount", "recipientAccount"]);
        let template = std::fs::read_to_string(root.join("protocol.ts")).unwrap();
        assert!(template.contains(r#"fundingAccount: "wallet""#));

        // A second run refuses rather than overwriting.
        let error = scaffold(&args).unwrap_err().to_string();
        assert!(error.contains("already has arete.toml"), "{error}");
        assert!(scaffold(&InitArgs {
            dir: dir.path().join("other"),
            name: Some("Bad-Name".into()),
            no_install: true,
            ..InitArgs::default()
        })
        .is_err());
    }

    #[test]
    fn installs_from_the_registry_first_and_writes_no_idl() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        let mut asked = None;
        let outcome = scaffold_with(
            &InitArgs {
                dir: root.clone(),
                ..InitArgs::default()
            },
            |project| {
                asked = Some(project.to_path_buf());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(asked, Some(std::fs::canonicalize(&root).unwrap()));
        assert_eq!(outcome.system_program, Some(SystemProgramSource::Registry));
        assert!(outcome.warnings.is_empty());
        assert!(!root.join(SYSTEM_IDL_PATH).exists());
        assert!(!outcome.written.iter().any(|file| file == SYSTEM_IDL_PATH));
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains("registry's `system-program` package"));
    }

    #[test]
    fn registry_failure_never_substitutes_a_local_program_identity() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        let error = scaffold_with(
            &InitArgs {
                dir: root.clone(),
                ..InitArgs::default()
            },
            |_| bail!("registry unavailable"),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("registry unavailable"), "{message}");
        assert!(
            message.contains("a4 install program system-program"),
            "{message}"
        );
        assert!(!root.join(SYSTEM_IDL_PATH).exists());
        assert!(!root.join("arete.lock").exists());
    }

    #[test]
    fn local_idl_skips_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        let outcome = scaffold_with(
            &InitArgs {
                dir: root.clone(),
                local_idl: true,
                ..InitArgs::default()
            },
            |_| panic!("--local-idl must not use the registry"),
        )
        .unwrap();
        assert_eq!(outcome.system_program, Some(SystemProgramSource::LocalIdl));
        assert!(outcome.warnings.is_empty());
        assert!(root.join(SYSTEM_IDL_PATH).is_file());
        let lock = std::fs::read_to_string(root.join("arete.lock")).unwrap();
        assert!(lock.contains("idl/system.idl.json"), "{lock}");
        // An existing fallback IDL refuses init before anything is written.
        let other = dir.path().join("other");
        std::fs::create_dir_all(other.join("idl")).unwrap();
        std::fs::write(other.join(SYSTEM_IDL_PATH), "{}").unwrap();
        let error = scaffold_with(
            &InitArgs {
                dir: other.clone(),
                ..InitArgs::default()
            },
            |_| Ok(()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(SYSTEM_IDL_PATH), "{error}");
        assert!(!other.join("arete.toml").exists());
    }
}
