//! The `protocol-app` template of `a4 create`: the `a4 protocol init`
//! scaffold (a tip jar and its automations) plus a React app wired to the
//! protocol client and an automations server with a Privy signer.
//!
//! The app's dev-only features (the dev panel: dev wallets, funding, steps
//! run as a dev wallet) render only when the client targets a dev
//! environment, and load lazily, so a production build of the published
//! client never runs them. The server uses `@usearete/automations/dev` with
//! a dev client and Privy plus Postgres with the published one.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::init::{InitArgs, InitOutcome};
use crate::project::runtime::lockstep_version;

/// The files the app adds, relative to the project, with `{{name}}` and
/// `{{slug}}` filled in.
const APP_FILES: [(&str, &str); 10] = [
    ("app/index.html", include_str!("app_template/index.html")),
    (
        "app/src/main.tsx",
        include_str!("app_template/src/main.tsx"),
    ),
    ("app/src/App.tsx", include_str!("app_template/src/App.tsx")),
    (
        "app/src/DevPanel.tsx",
        include_str!("app_template/src/DevPanel.tsx"),
    ),
    (
        "app/src/arete.ts",
        include_str!("app_template/src/arete.ts"),
    ),
    (
        "app/src/wallet.ts",
        include_str!("app_template/src/wallet.ts"),
    ),
    (
        "server/index.ts",
        include_str!("app_template/server/index.ts"),
    ),
    (
        "vite.config.ts",
        include_str!("app_template/vite.config.ts"),
    ),
    (
        "tsconfig.app.json",
        include_str!("app_template/tsconfig.app.json"),
    ),
    (".env.example", include_str!("app_template/env.example")),
];

/// `tsconfig.json` for the protocol, its automations and the server (the
/// app has `tsconfig.app.json`).
const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": ["node"],
    "paths": {
      "@programs/*": ["./generated/typescript/programs/*/index.js"]
    }
  },
  "include": ["protocol.ts", "handlers", "generated", "automations", "server"]
}
"#;

const GITIGNORE_LINES: [&str; 3] = ["dist/", ".env", ".env.local"];

/// The template's README section, after the scaffold's.
const README: &str = r#"
## The app

`a4 create --template protocol-app` added a React app (`app/`) on the
protocol client and an automations server (`server/index.ts`):

| File | What it is |
| --- | --- |
| `app/src/arete.ts` | the protocol client (publishable key) and `isDev` |
| `app/src/App.tsx` | the jars, and tipping with a browser wallet (prepare, sign the exact bytes, submit) |
| `app/src/DevPanel.tsx` | dev only: dev wallets, funding, opening a jar as a dev wallet |
| `server/index.ts` | the automations (`automations/index.ts`) with Privy, the webhook route and the consent routes |

```sh
a4 dev up                                         # deploys, generates the client the app imports
a4 auth keys create-publishable --origin http://localhost:5173 --env-file .env.local
npm run app                                       # http://localhost:5173
```

The client `a4 dev up` generates targets the dev environment, so `isDev`
is true and the dev panel shows. Dev-only features are gated on the
client, never on `NODE_ENV` or a hostname, and the dev panel is loaded
lazily inside that branch. (`isDev` reads the environment from the
client's deployment; this is temporary, until the SDK exposes it on the
client.)

### Run the automations in the server

`a4 dev up` runs `automations/index.ts` itself. To run the server instead,
with Privy and the consent routes, forward the events to it:

```toml
# arete.toml
[dev.automations]
forward = "http://localhost:8787/arete/webhook"
```

```sh
cp .env.example .env              # set ARETE_SECRET_KEY, and ARETE_WEBHOOK_SECRET from .arete/dev/automations/webhook-secret
npm run server
curl -X POST localhost:8787/dev/consent -H "authorization: Bearer $APP_API_TOKEN" \
  -d '{"wallet":"alice","instanceId":"<jar>","amount":"1000000"}'
```

With the dev client the server runs on `@usearete/automations/dev`: dev
wallets sign (Privy too, for other wallets, once `PRIVY_*` is set), jobs and
consent are kept in `.arete/dev/app-store.bin`, and transactions simulate on
the fork. Each of those helpers throws for a production client.

### Ship it

```sh
a4 install --locked               # the published client: isDev is false, the dev panel is gone
a4 install --check                # fails while a dev client is still in generated/
npm run build:app                 # dist/app
npm run server -- --migrate       # once, with DATABASE_URL
```

A production build must not contain a dev client, dev wallet keys
(`.arete/` is git-ignored: keep it out of images too), the dev API or an
environment token. In production the server needs `DATABASE_URL`,
`PRIVY_APP_ID`, `PRIVY_APP_SECRET`, `PRIVY_AUTHORIZATION_KEY`,
`PRIVY_WALLETS` and `RPC_URL`.
"#;

fn fill(template: &str, name: &str, slug: &str) -> String {
    template.replace("{{name}}", name).replace("{{slug}}", slug)
}

/// The scaffold's `package.json` with the app's scripts and dependencies.
fn package_json(scaffold: &str) -> Result<String> {
    let mut value: Value = serde_json::from_str(scaffold).context("the scaffold's package.json")?;
    let version = lockstep_version();
    let merge = |value: &mut Value, key: &str, extra: Value| {
        let target = value
            .as_object_mut()
            .expect("package.json is an object")
            .entry(key)
            .or_insert_with(|| json!({}));
        for (name, item) in extra.as_object().expect("an object") {
            target[name] = item.clone();
        }
    };
    merge(
        &mut value,
        "scripts",
        json!({
            "app": "vite",
            "build:app": "vite build",
            "server": "tsx server/index.ts",
            "typecheck": "tsc -p . && tsc -p tsconfig.app.json"
        }),
    );
    merge(
        &mut value,
        "dependencies",
        json!({
            "@privy-io/node": "^0.37.0",
            "@solana/web3.js": "^1.98.0",
            "@usearete/automations": version,
            "@usearete/react": version,
            "pg": "^8.13.0",
            "react": "^19.0.0",
            "react-dom": "^19.0.0"
        }),
    );
    merge(
        &mut value,
        "devDependencies",
        json!({
            "@types/node": "^22.0.0",
            "@types/pg": "^8.11.0",
            "@types/react": "^19.0.0",
            "@types/react-dom": "^19.0.0",
            "@vitejs/plugin-react": "^4.3.0",
            "vite": "^6.0.0"
        }),
    );
    // Runtime dependency now (the server), not only a dev tool.
    if let Some(dev) = value["devDependencies"].as_object_mut() {
        dev.remove("@usearete/automations");
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

/// `a4 create <dir> --template protocol-app`: the protocol scaffold with
/// the app on top of it, both written before the System Program install
/// (which needs the registry), so a failed install leaves the whole project.
pub fn scaffold(args: &InitArgs) -> Result<InitOutcome> {
    let dir = &args.dir;
    let clashing: Vec<&str> = APP_FILES
        .iter()
        .map(|(path, _)| *path)
        .filter(|path| dir.join(path).exists())
        .collect();
    if !clashing.is_empty() {
        bail!(
            "{} already has {}; use an empty directory",
            dir.display(),
            clashing.join(", ")
        );
    }
    super::init::scaffold_extended(args, |dir, name| {
        let mut written = Vec::new();
        add_app(dir, name, &mut written)?;
        Ok(written)
    })
}

/// Write the app over a fresh protocol scaffold in `dir`.
fn add_app(dir: &Path, name: &str, written: &mut Vec<String>) -> Result<()> {
    let slug = crate::commands::protocol::derive_slug(name)?;
    for (path, template) in APP_FILES {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, fill(template, name, &slug))
            .with_context(|| format!("Failed to write {}", target.display()))?;
        written.push(path.to_string());
    }
    let package = dir.join("package.json");
    let merged = package_json(&std::fs::read_to_string(&package)?)?;
    std::fs::write(&package, merged)?;
    std::fs::write(dir.join("tsconfig.json"), TSCONFIG)?;

    let gitignore = dir.join(".gitignore");
    let mut ignore = std::fs::read_to_string(&gitignore).unwrap_or_default();
    for line in GITIGNORE_LINES {
        if !ignore.lines().any(|existing| existing.trim() == line) {
            if !ignore.is_empty() && !ignore.ends_with('\n') {
                ignore.push('\n');
            }
            ignore.push_str(line);
            ignore.push('\n');
        }
    }
    std::fs::write(&gitignore, ignore)?;

    let readme = dir.join("README.md");
    let mut text = std::fs::read_to_string(&readme)?;
    text.push_str(README);
    std::fs::write(&readme, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_layers_onto_the_scaffold() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("my-app");
        let args = InitArgs {
            dir: project.clone(),
            no_install: true,
            ..InitArgs::default()
        };
        let outcome = super::super::init::scaffold(&args).unwrap();
        let mut written = outcome.written.clone();
        add_app(&project, &outcome.name, &mut written).unwrap();

        for (path, _) in APP_FILES {
            assert!(project.join(path).is_file(), "{path}");
            assert!(written.contains(&path.to_string()), "{path}");
        }
        let arete = std::fs::read_to_string(project.join("app/src/arete.ts")).unwrap();
        assert!(arete.contains("generated/typescript/protocols/my-app/my-app.js"));
        assert!(!arete.contains("{{"));
        let server = std::fs::read_to_string(project.join("server/index.ts")).unwrap();
        assert!(server.contains("@usearete/automations/dev"));
        assert!(server.contains("generated/typescript/protocols/my-app/my-app.js"));

        let package: Value =
            serde_json::from_str(&std::fs::read_to_string(project.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(package["scripts"]["check"], "arete-protocol check");
        assert_eq!(package["scripts"]["app"], "vite");
        assert_eq!(
            package["dependencies"]["@usearete/react"],
            lockstep_version()
        );
        assert_eq!(
            package["dependencies"]["@usearete/automations"],
            lockstep_version()
        );
        assert!(package["devDependencies"]["@usearete/automations"].is_null());
        assert_eq!(package["devDependencies"]["tsx"], "^4.19.0");

        let gitignore = std::fs::read_to_string(project.join(".gitignore")).unwrap();
        for line in ["node_modules/", ".arete/", "dist/", ".env", ".env.local"] {
            assert!(gitignore.lines().any(|l| l == line), "{line}");
        }
        let readme = std::fs::read_to_string(project.join("README.md")).unwrap();
        assert!(readme.contains("## The app"));
        assert!(readme.contains("a4 install --locked"));
    }

    #[test]
    fn a_failed_install_leaves_the_app_too() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("my-app");
        let args = InitArgs {
            dir: project.clone(),
            ..InitArgs::default()
        };
        let error = super::super::init::scaffold_inner(
            &args,
            |_| anyhow::bail!("not logged in"),
            |dir, name| {
                let mut written = Vec::new();
                add_app(dir, name, &mut written)?;
                Ok(written)
            },
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("not logged in"));
        assert!(project.join("protocol.ts").is_file());
        assert!(project.join("app/src/App.tsx").is_file());
        assert!(project.join("server/index.ts").is_file());
    }

    #[test]
    fn refuses_a_directory_with_app_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("app/src")).unwrap();
        std::fs::write(dir.path().join("app/src/App.tsx"), "").unwrap();
        let args = InitArgs {
            dir: dir.path().to_path_buf(),
            name: Some("tipJar".into()),
            no_install: true,
            ..InitArgs::default()
        };
        let error = scaffold(&args).unwrap_err().to_string();
        assert!(error.contains("app/src/App.tsx"), "{error}");
        assert!(!dir.path().join("protocol.ts").exists());
    }
}
