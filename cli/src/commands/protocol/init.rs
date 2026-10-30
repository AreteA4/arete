//! `a4 protocol init [dir]`: scaffold a protocol project.
//!
//! Writes a template-only tip jar (`protocol.ts`, no handler code) that pays
//! through the System Program, its automations (`automations/index.ts`, which
//! `a4 dev up` runs through `[dev.automations]`), and everything
//! `arete-protocol check`, `a4 dev up` and `a4 protocol deploy` read:
//!
//! - `arete.toml`, then the System Program from the registry, as `a4 install
//!   program system-program --as system --ts` installs it: `arete.lock`,
//!   `generated/typescript/programs/system/` and `@programs/*` in
//!   `tsconfig.json`;
//! - `package.json` (`@usearete/protocol`, `@usearete/sdk` and `zod`, which
//!   the generated program module imports; `@usearete/automations` and `tsx`
//!   for the automations), `tsconfig.json`, `.gitignore` and `README.md`.
//!
//! The automations are host-agnostic: `arete-automations add <cloudflare|
//! vercel|node>` later writes a host's entry and the production wiring
//! beside them, so nothing here depends on where they will be deployed.
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
//! when any file it writes exists, naming it, with two exceptions.
//! `.gitignore` gains the missing lines. An `arete.toml` holding only what
//! `a4 init` writes (`manifest_version` and `[project]`) gains the
//! protocol's sections; one with dependencies, `[dev]` or any other table
//! still refuses. So `a4 init -y && a4 protocol init .` and
//! `a4 protocol init <dir> && cd <dir> && a4 init -y` build the same project.

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
pub const PROTOCOL_PACKAGE_RANGE: &str = "^0.6.0"; // x-release-please-version

/// The `tsx` range: `a4 dev up` runs the TypeScript automations with it.
const TSX_RANGE: &str = "^4.19.0";

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
  "include": ["*.ts", "handlers", "generated", "automations", "api"]
}
"#;

/// The automations definition, relative to the project.
pub const AUTOMATIONS_ENTRY: &str = "automations/index.ts";

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
 *   instance starts `open` and emits `tipJar.opened`.
 * - `tip` (any wallet, as a participant) sends `amount` lamports to the
 *   recipient, as often as it likes while the jar is open, and emits
 *   `tipJar.tipped`. Supporters can automate it: see automations/index.ts.
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
  description: "A tip jar: templated System Program transfers to a recipient, which supporters can automate.",
  programs: {{ system }},
  params: {{ amount: "u64" }},
  roles: {{ recipient: {{ description: "Receives the tips and closes the jar." }} }},
  instanceVisibility: "shared",
  machines: {{
    instance: {{
      initial: "open",
      states: {{
        open: "recorded",
        closed: {{ kind: "recorded", terminal: true }},
      }},
    }},
    // A wallet that tipped. The state is terminal, and `tip` declares the
    // self-loop that lets a supporter tip again.
    participant: {{
      initial: "supporter",
      states: {{ supporter: {{ kind: "recorded", terminal: true }} }},
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
    tip: {{
      scope: "participant",
      title: "Tip the recipient",
      authority: "participant",
      args: {{ amount: "u64" }},
      guards: {{
        amount_zero: "args.amount > 0",
        not_open: 'instance.state == "open"',
      }},
      transitions: [{{ machine: "participant", from: ["supporter"], to: "supporter" }}],
      build: [
        ix(system, "transfer", {{
          args: {{ lamports: "args.amount" }},
          accounts: {{ fundingAccount: "wallet", recipientAccount: "instance.roles.recipient" }},
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
    {{ id: "announceOpened", trigger: {{ kind: "onConfirmed", step: "open" }}, action: {{ kind: "emit", event: "tipJar.opened" }} }},
    {{ id: "announceTip", trigger: {{ kind: "onConfirmed", step: "tip" }}, action: {{ kind: "emit", event: "tipJar.tipped" }} }},
    {{
      id: "announceClosed",
      trigger: {{ kind: "onTransition", machine: "instance", from: [], to: ["closed"] }},
      action: {{ kind: "emit", event: "tipJar.closed" }},
    }},
  ],
  // Named events: automations and webhooks subscribe to them.
  events: {{
    "tipJar.opened": "instance",
    "tipJar.tipped": "participant",
    "tipJar.closed": "instance",
  }},
  // The protocol stack (`a4 stream --dev`) tracks every transfer:
  // `fundingAccount` pays, `recipientAccount` receives. The recipient is a
  // wallet, not an account the instance owns, so only transfers sent through
  // Arete count.
  stack: {{
    activity: [
      {{ step: "open", program: "system", instruction: "transfer", instanceAccount: "recipientAccount", walletAccount: "fundingAccount", amountArg: "lamports" }},
      {{ step: "tip", program: "system", instruction: "transfer", instanceAccount: "recipientAccount", walletAccount: "fundingAccount", amountArg: "lamports" }},
      {{ step: "close", program: "system", instruction: "transfer", instanceAccount: "recipientAccount", walletAccount: "fundingAccount", amountArg: "lamports" }},
    ],
  }},
}});
"#
    )
}

/// `automations/index.ts`: the tip jar's automations, which `a4 dev up` runs
/// against the dev environment and a host runs in production.
pub fn automations_ts(name: &str, slug: &str) -> String {
    format!(
        r#"/**
 * Automations for `{name}`: steps that run for a supporter after they opted in
 * by signing a consent, without them opening the app.
 *
 * - **Recurring tips** (a schedule): once a day, every supporter who
 *   consented to `tip` on a jar tips the amount they chose.
 * - **Tips for new jars** (an event): supporters who follow a recipient tip
 *   each new jar for that recipient as soon as `tipJar.opened` arrives.
 *
 * Handlers only return the steps to run. Each one is checked against the
 * wallet's consent and limits, the policy below and the protocol's guards,
 * then simulated, before it is signed.
 *
 * `a4 dev up` runs this module (`[dev.automations]` in arete.toml) with dev
 * wallets as the supporters; schedules follow the dev environment's clock,
 * so `a4 dev warp +1d` runs the daily tips. For production,
 * `npx @usearete/automations add <cloudflare|vercel|node>` wires the same
 * definition into a host. See README.md.
 */
import {{ defineAutomations }} from "@usearete/automations";

// The protocol's client: the one `a4 dev up` generates for the dev
// environment, or the published one after `a4 install --locked`.
import protocol from "../generated/typescript/protocols/{slug}/{slug}.js";

export default defineAutomations(
  protocol,
  (automations) => {{
    automations.schedule("recurringTips", {{ every: "day" }});
    automations.on("recurringTips", async (_trigger, ctx) => {{
      // Grants for one jar, with the amount as a signed setting:
      //   a4 dev consent alice --steps tip --instance <jar> --param amount=1000000
      const grants = await ctx.consents.list({{ step: "tip" }});
      return grants.flatMap((grant) => {{
        const amount = grant.params.amount;
        if (grant.instanceId === null || typeof amount !== "string") return [];
        // The schedule dispatches once per day, so each jar is tipped once a day.
        return [ctx.step("tip", {{ wallet: grant.wallet, instance: grant.instanceId, args: {{ amount }} }})];
      }});
    }});

    automations.on("tipJar.opened", async (event, ctx) => {{
      const jar = await ctx.client.instances!.get(event.instanceId);
      // Grants for every jar, following one recipient:
      //   a4 dev consent alice --steps tip --param recipient=<pubkey> --param amount=1000000
      const grants = await ctx.consents.list({{ step: "tip" }});
      return grants.flatMap((grant) => {{
        const amount = grant.params.amount;
        if (grant.instanceId !== null || grant.params.recipient !== jar.roles?.recipient || typeof amount !== "string") return [];
        return [ctx.step("tip", {{ wallet: grant.wallet, instance: event.instanceId, args: {{ amount }} }})];
      }});
    }});
  }},
  {{
    // What may be signed: tips are System Program transfers (always allowed),
    // and at most 0.05 SOL may leave a wallet in one transaction. Each
    // supporter's consent sets their own, lower limits.
    policy: {{ programs: [], maxSolPerTransaction: "0.05" }},
  }},
);
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
            "@usearete/automations": lockstep_version(),
            "tsx": TSX_RANGE,
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
steps are System Program transfers built from templates, with automations
that tip on a supporter's behalf once they opt in.

| File | What it is |
| --- | --- |
| `protocol.ts` | the protocol definition (`defineProtocol`) |
| `automations/index.ts` | the automations (`defineAutomations`): recurring tips and tips for new jars |
| `generated/typescript/programs/system/` | the System Program ({system_source}), imported as `@programs/system` |
| `arete.toml`, `arete.lock` | the project's dependencies and their pinned versions; `[dev.automations]` points `a4 dev up` at the automations |

`generated/` and `arete.lock` are committed; `a4 install --check` checks them
in CI. `.arete/` (build output and local session state) is not.

## Develop

```sh
npm install
npx arete-protocol check          # type-check and validate the definition
a4 dev up                         # dev environment; redeploys and restarts the automations on every change
a4 dev wallet alice               # funded dev wallets: alice tips, bob receives
a4 dev wallet bob
a4 dev step open --wallet alice --args '{{"recipient":"<bob>","amount":"1000000"}}'
a4 dev step tip --wallet alice --instance <jar> --args '{{"amount":"1000000"}}'
a4 stream --dev                   # the protocol's instances and activity
```

Add a program: `a4 install program <registry package> --ts`, or put its IDL
in `idl/` and run `a4 install program ./idl/<name>.json --as <alias> --ts`,
then import it from `@programs/<alias>`.

Arete includes any applicable fee in the prepared transaction. The app should
display that fee before signing. `close` accepts an amount chosen by its caller;
returning funds does not automatically make it an exempt refund.

## Automations

`a4 dev up` runs `automations/index.ts` beside the protocol: dev wallets
sign for the supporters, and jobs and consent are kept under
`.arete/dev/automations/`. A supporter opts in by signing a consent; in dev,
`a4 dev consent` signs it with a dev wallet.

Recurring tips, daily, for one jar:

```sh
a4 dev consent alice --steps tip --instance <jar> --param amount=1000000
a4 dev warp +1d                   # schedules follow the dev environment's clock
# automations │ completed: tip for <alice> on <jar>: completed
```

Tips for every new jar a recipient opens:

```sh
a4 dev wallet carol
a4 dev consent carol --steps tip --param recipient=<bob> --param amount=500000
a4 dev step open --wallet alice --args '{{"recipient":"<bob>","amount":"1000000"}}'
# automations │ completed: tip for <carol> on <new jar>: completed
```

## Deploy

Deploy the protocol:

```sh
a4 protocol deploy
```

A program installed from a local IDL is pushed to your account as a private
program first (`a4 program push`), and `arete.lock` records its program
release.

Then deploy the automations to a host of your choice. This writes the host's
entry and config, plus `automations/runtime.ts` (the production client,
signer, Postgres store and policy, read from the environment), and prints
the remaining steps:

```sh
npx @usearete/automations add cloudflare    # or: vercel, node (any container host)
```

Every host follows the same steps: install the published client
(`a4 install --locked`), migrate the store (`npx @usearete/automations migrate`),
set the secrets, deploy with the host's own CLI, then subscribe the webhook:

```sh
a4 protocol webhook add --url https://<host>/arete/webhook --secret-only | npx wrangler secret put ARETE_WEBHOOK_SECRET
```

Docs: https://docs.arete.run/cli/local-development/,
https://docs.arete.run/automations/production/ and
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
    println!(
        "  Automations: {AUTOMATIONS_ENTRY} (`a4 dev up` runs them; `npx @usearete/automations add <cloudflare|vercel|node>` deploys them)"
    );
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

/// [`scaffold`], with `extra` writing more files (returned, relative to the
/// project) once the scaffold's are written and before the System Program
/// is installed, so a failed install leaves them in place too.
pub fn scaffold_extended(
    args: &InitArgs,
    extra: impl FnOnce(&Path, &str) -> Result<Vec<String>>,
) -> Result<InitOutcome> {
    scaffold_inner(args, install_from_registry, extra)
}

/// [`scaffold`], with the registry install as a seam for tests.
fn scaffold_with(
    args: &InitArgs,
    registry_install: impl FnOnce(&Path) -> Result<()>,
) -> Result<InitOutcome> {
    scaffold_inner(args, registry_install, |_, _| Ok(Vec::new()))
}

pub(super) fn scaffold_inner(
    args: &InitArgs,
    registry_install: impl FnOnce(&Path) -> Result<()>,
    extra: impl FnOnce(&Path, &str) -> Result<Vec<String>>,
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

    let slug = crate::commands::protocol::derive_slug(&name)?;
    // An `arete.toml` that `a4 init` wrote is kept as it is and gains the
    // protocol's sections, exactly as a new one would.
    //
    // A protocol project is TypeScript: its spec, handlers, automations and
    // app. Requesting the Rust SDK target too would make every program it
    // installs need a published Rust SDK, which several catalog programs
    // don't have. A mergeable manifest never has `[sdk]`, so appending it is
    // safe.
    let existing_manifest = existing_manifest(&dir.join("arete.toml"))?;
    let base_manifest = match &existing_manifest {
        ExistingManifest::Mergeable(existing) => {
            format!(
                "{}\n\n[sdk]\ntargets = [\"typescript\"]\n",
                existing.trim_end()
            )
        }
        _ => {
            let mut project_manifest =
                crate::commands::config::build_manifest(&absolute, Some(dir_name.clone()))?;
            project_manifest.sdk.targets = vec![InstallTarget::TypeScript];
            project_manifest.to_toml_pretty()?
        }
    };
    let manifest = format!(
        "{}\n[dev.automations]\nentry = \"{AUTOMATIONS_ENTRY}\"\n",
        base_manifest.trim_end()
    );
    let files: Vec<(&str, String)> = vec![
        ("arete.toml", manifest),
        ("package.json", package_json(&dir_name)?),
        ("tsconfig.json", TSCONFIG.to_string()),
        ("protocol.ts", protocol_ts(&name)),
        (AUTOMATIONS_ENTRY, automations_ts(&name, &slug)),
    ];
    // The fallback IDL is checked up front too, so a fallback never meets
    // an existing file after the rest are written.
    let checked: Vec<&str> = files
        .iter()
        .map(|(path, _)| *path)
        .chain(["README.md", SYSTEM_IDL_PATH])
        .collect();
    let existing: Vec<String> = checked
        .into_iter()
        .filter_map(|path| match (path, &existing_manifest) {
            ("arete.toml", ExistingManifest::Mergeable(_)) => None,
            ("arete.toml", ExistingManifest::Refused(why)) => Some(format!("arete.toml ({why})")),
            _ => dir.join(path).exists().then(|| path.to_string()),
        })
        .collect();
    if !existing.is_empty() {
        bail!(
            "{} already has {}; a4 protocol init only writes new files, and only adds to an arete.toml that holds just what `a4 init` writes (manifest_version and [project]). Use an empty directory",
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

    write_new(
        dir,
        "README.md",
        &readme(&dir_name, &name, args.local_idl && !args.no_install),
    )?;
    written.push("README.md".into());
    written.extend(extra(dir, &name)?);

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

/// The tables `a4 init` writes to a new `arete.toml`.
const INIT_MANIFEST_KEYS: [&str; 2] = ["manifest_version", "project"];
/// The `[project]` keys `a4 init` writes.
const INIT_PROJECT_KEYS: [&str; 2] = ["name", "private"];

/// An existing `arete.toml`, as init sees it.
#[derive(Debug, PartialEq, Eq)]
enum ExistingManifest {
    Missing,
    /// Holds only what `a4 init` writes: init adds its sections to it.
    Mergeable(String),
    /// Holds anything else (dependencies, `[dev]`, other tables): init
    /// refuses, saying why.
    Refused(String),
}

fn existing_manifest(path: &Path) -> Result<ExistingManifest> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingManifest::Missing)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()))
        }
    };
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(_) => return Ok(ExistingManifest::Refused("not valid TOML".to_string())),
    };
    let mut extra: Vec<String> = table
        .keys()
        .filter(|key| !INIT_MANIFEST_KEYS.contains(&key.as_str()))
        .map(|key| format!("[{key}]"))
        .collect();
    let mut missing = Vec::new();
    match table.get("project") {
        Some(toml::Value::Table(project)) => extra.extend(
            project
                .keys()
                .filter(|key| !INIT_PROJECT_KEYS.contains(&key.as_str()))
                .map(|key| format!("project.{key}")),
        ),
        Some(_) => extra.push("a `project` that is not a table".to_string()),
        None => missing.push("[project]"),
    }
    if table
        .get("manifest_version")
        .and_then(toml::Value::as_integer)
        != Some(i64::from(crate::project::manifest::MANIFEST_VERSION))
    {
        missing.push("manifest_version = 1");
    }
    let mut problems = Vec::new();
    if !extra.is_empty() {
        problems.push(format!("has {}", extra.join(", ")));
    }
    if !missing.is_empty() {
        problems.push(format!("has no {}", missing.join(" or ")));
    }
    if !problems.is_empty() {
        return Ok(ExistingManifest::Refused(problems.join(" and ")));
    }
    // The keys are the ones `a4 init` writes; their values must also make a
    // manifest that installs (`[project]` with a string `name`, and so on).
    let invalid = match toml::from_str::<crate::project::manifest::ManifestV1>(&text) {
        Ok(document) => document.validate().err().map(|error| error.to_string()),
        Err(error) => Some(error.message().to_string()),
    };
    Ok(match invalid {
        Some(why) => ExistingManifest::Refused(format!("is not a valid manifest: {why}")),
        None => ExistingManifest::Mergeable(text),
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
            .arg("--offline")
            .env_remove("ARETE_API_KEY")
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
            AUTOMATIONS_ENTRY,
            "README.md",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        // `a4 dev up` runs the automations, which import the client it
        // generates for the protocol's slug.
        let config = crate::commands::dev::automations::AutomationsConfig::load(&root)
            .unwrap()
            .unwrap();
        assert_eq!(config.entry.as_deref(), Some(AUTOMATIONS_ENTRY));
        let automations = std::fs::read_to_string(root.join(AUTOMATIONS_ENTRY)).unwrap();
        assert!(
            automations.contains(r#"from "../generated/typescript/protocols/tip-jar/tip-jar.js""#)
        );
        assert!(automations.contains(r#"automations.schedule("recurringTips""#));
        assert!(automations.contains(r#"automations.on("tipJar.opened""#));
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
        assert_eq!(
            package["devDependencies"]["@usearete/automations"],
            package["dependencies"]["@usearete/sdk"],
            "the automations package is versioned with the SDK"
        );
        assert!(package["devDependencies"]["tsx"].is_string());
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

        // A second run refuses rather than overwriting, naming every file
        // and why arete.toml can't take the protocol's sections again.
        let error = scaffold(&args).unwrap_err().to_string();
        assert!(
            error.contains("already has arete.toml (has [dev], [sdk]), package.json"),
            "{error}"
        );
        assert!(scaffold(&InitArgs {
            dir: dir.path().join("other"),
            name: Some("Bad-Name".into()),
            no_install: true,
            ..InitArgs::default()
        })
        .is_err());
    }

    #[test]
    fn adds_to_the_manifest_a4_init_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        std::fs::create_dir_all(&root).unwrap();
        let init_manifest =
            crate::commands::config::new_manifest_contents(&root, Some("my-app".into())).unwrap();
        std::fs::write(root.join("arete.toml"), &init_manifest).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# Mine\n").unwrap();
        let outcome = scaffold(&InitArgs {
            dir: root.clone(),
            no_install: true,
            ..InitArgs::default()
        })
        .unwrap();
        assert!(outcome.written.iter().any(|file| file == "arete.toml"));
        let manifest = std::fs::read_to_string(root.join("arete.toml")).unwrap();
        assert_eq!(
            manifest,
            format!(
                "{}\n\n[sdk]\ntargets = [\"typescript\"]\n[dev.automations]\nentry = \"{AUTOMATIONS_ENTRY}\"\n",
                init_manifest.trim_end()
            ),
            "the project table a4 init wrote is kept as it is"
        );
        assert!(manifest.contains("name = \"my-app\""));
        assert_eq!(
            std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            "# Mine\n"
        );
        let project =
            crate::project::manifest::ProjectManifest::load(root.join("arete.toml")).unwrap();
        assert_eq!(
            project.document.sdk.targets,
            vec![InstallTarget::TypeScript],
            "a merged manifest is TypeScript-only, like a new one"
        );
    }

    #[test]
    fn refuses_a_manifest_with_more_than_a4_init_writes() {
        let base = "manifest_version = 1\n\n[project]\nname = \"app\"\n";
        for (extra, reason) in [
            (
                "\n[dependencies.programs.system]\npackage = \"system-program\"\n",
                "has [dependencies]",
            ),
            ("\n[dev]\nsdk = [\"typescript\"]\n", "has [dev]"),
            ("\n[sdk]\ntargets = [\"rust\"]\n", "has [sdk]"),
            ("extra = 1\n", "has project.extra"),
        ] {
            refuses_existing(&format!("{base}{extra}"), reason);
        }
        for (manifest, reason) in [
            (
                "manifest_version = 1\n\n[project]\nname = 123\n",
                "is not a valid manifest: invalid type: integer `123`, expected a string",
            ),
            (
                "manifest_version = 1\n\n[project]\nprivate = true\n",
                "is not a valid manifest: missing field `name`",
            ),
            (
                "manifest_version = 1\n\n[project]\nname = \"app\"\nprivate = \"yes\"\n",
                "is not a valid manifest: invalid type: string \"yes\", expected a boolean",
            ),
            (
                "manifest_version = 1\n\n[project]\nname = \" \"\n",
                "is not a valid manifest: project.name cannot be empty",
            ),
        ] {
            refuses_existing(manifest, reason);
        }

        fn refuses_existing(original: &str, reason: &str) {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("tip-jar");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("arete.toml"), original).unwrap();
            let error = scaffold(&InitArgs {
                dir: root.clone(),
                no_install: true,
                ..InitArgs::default()
            })
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(&format!("already has arete.toml ({reason})")),
                "{error}"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("arete.toml")).unwrap(),
                original
            );
            assert!(!root.join("package.json").exists(), "nothing written");
        }

        // Other files a4 init doesn't write still refuse, each named.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tip-jar");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("arete.toml"), base).unwrap();
        std::fs::write(root.join("README.md"), "mine").unwrap();
        let error = scaffold(&InitArgs {
            dir: root.clone(),
            no_install: true,
            ..InitArgs::default()
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("already has README.md;"), "{error}");
        assert_eq!(
            std::fs::read_to_string(root.join("arete.toml")).unwrap(),
            base
        );
    }

    /// Every file under `root`, by relative path.
    fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn walk(base: &Path, dir: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(base, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(base).unwrap().to_path_buf(),
                        std::fs::read(&path).unwrap(),
                    );
                }
            }
        }
        let mut files = std::collections::BTreeMap::new();
        walk(root, root, &mut files);
        files
    }

    #[test]
    fn a4_init_and_protocol_init_build_the_same_project_in_either_order() {
        use crate::agents::Env;
        use crate::commands::init::{execute, InitPlan, Selection};
        let a4_init = |root: &Path, home: &Path| {
            let plan = InitPlan {
                dry_run: false,
                force: false,
                name: None,
                global: false,
                skills_ref: None,
                selection: Selection::Detected,
                manifest: true,
                agents_md: true,
                skills: false,
                mcp: true,
            };
            let env = Env::new(root, Some(home.to_path_buf()), &[]);
            let report = execute(&env, &root.join("arete.toml"), &plan).unwrap();
            assert!(!report.has_errors(), "{:?}", report.results);
        };
        let protocol_init = |root: &Path| {
            scaffold_with(
                &InitArgs {
                    dir: root.to_path_buf(),
                    local_idl: true,
                    ..InitArgs::default()
                },
                |_| panic!("--local-idl must not use the registry"),
            )
            .unwrap();
        };

        // `a4 init -y && a4 protocol init .`
        let first = tempfile::tempdir().unwrap();
        let first_root = first.path().join("tip-jar");
        let first_home = first.path().join("home");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&first_home).unwrap();
        a4_init(&first_root, &first_home);
        protocol_init(&first_root);

        // `a4 protocol init tip-jar && cd tip-jar && a4 init -y`
        let second = tempfile::tempdir().unwrap();
        let second_root = second.path().join("tip-jar");
        let second_home = second.path().join("home");
        std::fs::create_dir_all(&second_home).unwrap();
        protocol_init(&second_root);
        a4_init(&second_root, &second_home);

        let first_tree = tree(&first_root);
        let second_tree = tree(&second_root);
        assert_eq!(
            first_tree.keys().collect::<Vec<_>>(),
            second_tree.keys().collect::<Vec<_>>()
        );
        for (path, content) in &first_tree {
            assert_eq!(
                String::from_utf8_lossy(content),
                String::from_utf8_lossy(&second_tree[path]),
                "{}",
                path.display()
            );
        }
        for file in [
            "arete.toml",
            "AGENTS.md",
            "CLAUDE.md",
            "protocol.ts",
            "arete.lock",
        ] {
            assert!(first_tree.contains_key(Path::new(file)), "{file}");
        }
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
        assert!(root.join("README.md").is_file());
        assert!(std::fs::read_to_string(root.join("README.md"))
            .unwrap()
            .contains("a4 dev up"));
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
        // The install rewrites arete.toml and keeps [dev.automations].
        let config = crate::commands::dev::automations::AutomationsConfig::load(&root)
            .unwrap()
            .expect("[dev.automations] survives the install");
        assert_eq!(config.entry.as_deref(), Some(AUTOMATIONS_ENTRY));
        // A protocol project generates TypeScript only, so a program that
        // publishes no Rust SDK installs into it.
        let project =
            crate::project::manifest::ProjectManifest::load(root.join("arete.toml")).unwrap();
        assert_eq!(
            project.document.sdk.targets,
            vec![InstallTarget::TypeScript]
        );
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
