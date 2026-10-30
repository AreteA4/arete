//! `a4 program account decode <address>`: one account, decoded by Program
//! Read for the program's installed release.
//!
//! - **Production** (the default): the account is read through the managed
//!   Solana gateway and decoded by hosted Program Read.
//! - **Dev** (`--dev`): the account is read from the project's cloud dev
//!   environment and decoded by the environment's Program Read, so it shows
//!   the fork's values. The read session is minted for the environment with
//!   the account's key, as `a4 dev` does.
//!
//! A program the project depends on decodes at the release arete.lock pins.
//! Without `--program`, the program is the account's owner, looked up in the
//! project and then the registry. Without `--account`, the account type is the IDL account whose
//! discriminator starts the account's data. An environment that serves another
//! release of the program answers for its own account types: `--account` goes
//! to it unchecked, for a type the installed IDL doesn't name.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use clap::Args;
use serde_json::{json, Value};

use crate::api_client::{
    ApiClient, RegistryCapabilityInstallBinding, RegistryProgramInstallResponse,
    RegistryProgramInstallTransport,
};
use crate::commands::dev::env::EnvArgs;
use crate::project::installer::resolve_registry_batch;
use crate::project::lockfile::LockedDependency;
use crate::project::manifest::{DependencyKind, DependencySourceV1, InstallTarget};
use crate::project::resolver::{RegistryDependencyRequest, ResolvedRegistryDependency};
use crate::project::{ProjectLock, ProjectManifest};

#[derive(Args, Debug)]
pub struct DecodeArgs {
    /// The account's address
    pub address: String,

    /// The program: an arete.toml alias or a registry package
    /// [default: the account's owner, when the registry has it]
    #[arg(long, value_name = "ALIAS|PACKAGE")]
    pub program: Option<String>,

    /// The account type, by its IDL name [default: the IDL account whose
    /// discriminator the data starts with]
    #[arg(long, value_name = "TYPE")]
    pub account: Option<String>,

    /// Read the account from the project's cloud dev environment and decode
    /// it there, so it shows the fork's values
    #[arg(long)]
    pub dev: bool,

    #[command(flatten)]
    pub env: EnvArgs,
}

/// Where the read goes. `--dev` selects the environment today; when the
/// SDKs' environment switch lands, this is the one place that reads it, so
/// the command follows the switch like the SDKs do.
enum ReadTarget<'a> {
    Production,
    Dev(&'a EnvArgs),
}

fn read_target(args: &DecodeArgs) -> Result<ReadTarget<'_>> {
    if args.dev {
        return Ok(ReadTarget::Dev(&args.env));
    }
    if args.env.env.is_some() || args.env.project.project.is_some() {
        bail!("--env and --project choose a dev environment; add --dev");
    }
    Ok(ReadTarget::Production)
}

pub fn run(config_path: &str, args: DecodeArgs, json_output: bool) -> Result<()> {
    if !is_pubkey(&args.address) {
        bail!("'{}' is not a Solana address", args.address);
    }
    let client = ApiClient::new()?;
    let decoded = match read_target(&args)? {
        ReadTarget::Production => decode_production(&client, config_path, &args)?,
        ReadTarget::Dev(env) => decode_dev(&client, config_path, &args, env)?,
    };
    print_decoded(&decoded, json_output)
}

/// What a decode read and where.
#[derive(Debug, Clone, PartialEq)]
struct Decoded {
    address: String,
    program: String,
    program_id: String,
    account: String,
    release: String,
    /// `production`, or the dev environment's id.
    environment: String,
    value: Value,
}

fn print_decoded(decoded: &Decoded, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "address": decoded.address,
                "program": decoded.program,
                "programId": decoded.program_id,
                "account": decoded.account,
                "programReleaseHash": decoded.release,
                "environment": decoded.environment,
                "value": decoded.value,
            }))?
        );
        return Ok(());
    }
    println!(
        "{} {} ({} of {}, {})",
        decoded.address, decoded.account, decoded.account, decoded.program, decoded.environment
    );
    println!("{}", serde_json::to_string_pretty(&decoded.value)?);
    Ok(())
}

/// One account's raw state: its owner and data.
#[derive(Debug, Clone, PartialEq)]
struct RawAccount {
    owner: String,
    data: Vec<u8>,
}

fn decode_production(client: &ApiClient, config_path: &str, args: &DecodeArgs) -> Result<Decoded> {
    let named = args
        .program
        .as_deref()
        .map(|program| install_for(client, config_path, program))
        .transpose()?;
    // Any program's chain binding names the managed Solana gateway, which
    // reads any account; without --program, the registry's first program
    // provides one.
    let gateway_install = match &named {
        Some(install) => install.clone(),
        None => {
            let first = client
                .list_registry_programs()?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("The registry lists no programs to read through"))?;
            client.get_registry_program_install(&first.install_name, None)?
        }
    };
    let chain = gateway_install.chain_binding.as_ref().ok_or_else(|| {
        anyhow!(
            "{} has no hosted chain read binding to read {} through",
            gateway_install.install_name,
            args.address
        )
    })?;
    let raw = read_chain_account(client, chain, &args.address)?
        .ok_or_else(|| anyhow!("No account exists at {}", args.address))?;
    let install = match named {
        Some(install) => install,
        None => owner_install(client, config_path, &raw.owner, &args.address)?,
    };
    let (binding_id, endpoint, session_endpoint) = program_read_binding(&install)?;
    let account = account_type(&install, &raw, args.account.as_deref())?;
    check_owner(&install, &raw, &args.address)?;
    let release = install.release.program_release_hash.clone();
    let token = client.mint_target_session(
        &session_endpoint,
        &json!({
            "scopes": ["read"],
            "targetKind": "program-read-binding",
            "targetId": binding_id,
            "programReleaseHash": release,
        }),
    )?;
    let value = read_decoded(&endpoint, &release, &account, &args.address, &token)?;
    Ok(Decoded {
        address: args.address.clone(),
        program: install.install_name.clone(),
        program_id: install.definition.program_id.clone(),
        account,
        release,
        environment: "production".to_string(),
        value,
    })
}

/// The environment calls are async; the registry, session and Program Read
/// calls are blocking. Each async step runs to completion on one runtime
/// before any blocking call, so no blocking client runs inside it.
fn decode_dev(
    client: &ApiClient,
    config_path: &str,
    args: &DecodeArgs,
    env_args: &EnvArgs,
) -> Result<Decoded> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?;
    let (rpc, info) = runtime.block_on(async {
        let rpc = env_args.rpc().await?;
        let info = rpc
            .call(
                "getAccountInfo",
                json!([args.address, { "encoding": "base64", "commitment": "processed" }]),
            )
            .await
            .map_err(crate::commands::dev::env::explain)?;
        anyhow::Ok((rpc, info))
    })?;
    let env = rpc.env().to_string();
    let raw = raw_from_rpc(&info)?
        .ok_or_else(|| anyhow!("No account exists at {} in {env}", args.address))?;
    // `--project` names the project, so its arete.toml names the aliases.
    let project_config = env_args
        .project
        .project
        .as_ref()
        .map(|dir| dir.join("arete.toml").display().to_string());
    let config_path = project_config.as_deref().unwrap_or(config_path);
    let install = match args.program.as_deref() {
        Some(program) => install_for(client, config_path, program)?,
        None => owner_install(client, config_path, &raw.owner, &args.address)?,
    };
    check_owner(&install, &raw, &args.address)?;
    let (binding_id, _, session_endpoint) = program_read_binding(&install)?;
    let installed_release = install.release.program_release_hash.clone();
    let resolved = runtime
        .block_on(rpc.api().resolve(
            &env,
            json!({
                "programReads": [{
                    "programId": install.definition.program_id,
                    "programReadBindingId": binding_id,
                    "programReleaseHash": installed_release,
                }],
            }),
        ))
        .map_err(crate::commands::dev::env::explain)?;
    drop(rpc);
    drop(runtime);
    let (endpoint, release) = dev_program_read_target(&resolved)?;
    let account = if release == installed_release {
        account_type(&install, &raw, args.account.as_deref())?
    } else {
        served_account_type(&install, &raw, args.account.as_deref())?
    };
    let token = client.mint_target_session(
        &session_endpoint,
        &json!({
            "scopes": ["read"],
            "targetKind": "program-read-binding",
            "targetId": binding_id,
            "programReleaseHash": installed_release,
            "environment": env,
        }),
    )?;
    let value = read_decoded(&endpoint, &release, &account, &args.address, &token)?;
    Ok(Decoded {
        address: args.address.clone(),
        program: install.install_name.clone(),
        program_id: install.definition.program_id.clone(),
        account,
        release,
        environment: env,
        value,
    })
}

/// The dev Program Read endpoint and release from a resolve answer, or the
/// refusal with its fix.
fn dev_program_read_target(resolved: &Value) -> Result<(String, String)> {
    let answer = &resolved["programReads"][0];
    match answer["status"].as_str() {
        Some("target") => {
            let endpoint = answer["endpoint"]
                .as_str()
                .filter(|endpoint| !endpoint.is_empty())
                .ok_or_else(|| anyhow!("The environment's Program Read answer has no endpoint"))?;
            let release = answer["programReleaseHash"]
                .as_str()
                .ok_or_else(|| anyhow!("The environment's Program Read answer has no release"))?;
            Ok((endpoint.to_string(), release.to_string()))
        }
        Some("refused") => bail!(
            "{} ({}). {}",
            answer["message"]
                .as_str()
                .unwrap_or("The environment can't read this program"),
            answer["code"].as_str().unwrap_or("refused"),
            answer["fix"].as_str().unwrap_or_default()
        ),
        _ => bail!("The environment returned no Program Read answer"),
    }
}

/// The install for `program`: an arete.toml program alias at the project's
/// pinned release, else `program` as a registry reference.
fn install_for(
    client: &ApiClient,
    config_path: &str,
    program: &str,
) -> Result<RegistryProgramInstallResponse> {
    let pinned = project_install(Path::new(config_path), |alias, _| alias == program)
        .with_context(|| format!("Could not resolve program '{program}'"))?;
    match pinned {
        Some(install) => Ok(install),
        None => client
            .get_registry_program_install(program, None)
            .with_context(|| format!("Could not resolve program '{program}'")),
    }
}

/// The project's registry program dependency `selected` picks, at the release
/// `a4 install` pins it to. No readable arete.toml, or no such dependency, is
/// no answer: the command also runs outside projects.
fn project_install(
    config_path: &Path,
    selected: impl Fn(&str, Option<&LockedDependency>) -> bool,
) -> Result<Option<RegistryProgramInstallResponse>> {
    let Ok(manifest) = ProjectManifest::load(config_path) else {
        return Ok(None);
    };
    let lock = ProjectLock::load_optional(manifest.root.join("arete.lock"))?;
    let Some((targets, request)) = program_request(&manifest, lock.as_ref(), selected) else {
        return Ok(None);
    };
    let resolved = resolve_registry_batch(
        manifest.document.manifest_version,
        &targets,
        std::slice::from_ref(&request),
        None,
    )?;
    match resolved.into_iter().next() {
        Some(ResolvedRegistryDependency::Program { install, .. }) => Ok(Some(*install)),
        _ => bail!(
            "The registry did not resolve program '{}' to a program",
            request.package
        ),
    }
}

/// The resolver request for the selected program dependency: its version
/// requirement, and the locked release while the lock still describes it (as
/// `a4 install` reuses one).
fn program_request(
    manifest: &ProjectManifest,
    lock: Option<&ProjectLock>,
    selected: impl Fn(&str, Option<&LockedDependency>) -> bool,
) -> Option<(Vec<InstallTarget>, RegistryDependencyRequest)> {
    manifest
        .dependencies()
        .find_map(|(kind, alias, dependency)| {
            let DependencySourceV1::Registry(source) = &dependency.source else {
                return None;
            };
            if kind != DependencyKind::Program {
                return None;
            }
            let requirement = dependency.version.clone()?;
            let targets = dependency.selected_targets(&manifest.document.sdk).to_vec();
            let locked = lock
                .into_iter()
                .flat_map(|lock| &lock.dependencies)
                .find(|entry| entry.kind == kind && &entry.alias == alias);
            if !selected(alias, locked) {
                return None;
            }
            let reusable = locked.filter(|entry| {
                entry.source == dependency.source.stable_description()
                    && entry.requirement.as_deref() == Some(requirement.as_str())
                    && entry.targets == targets
            });
            let request_targets = if targets.is_empty() {
                manifest.document.sdk.targets.clone()
            } else {
                targets
            };
            Some((
                request_targets,
                RegistryDependencyRequest {
                    kind,
                    alias: alias.clone(),
                    package: source.registry.clone(),
                    requirement,
                    locked_package_release_hash: reusable
                        .and_then(|entry| entry.package_release_hash.clone()),
                    locked_programs: Vec::new(),
                    locked_stacks: Vec::new(),
                },
            ))
        })
}

/// The program that owns the account: the project's own dependency on it at
/// its pinned release, else the registry's.
fn owner_install(
    client: &ApiClient,
    config_path: &str,
    owner: &str,
    address: &str,
) -> Result<RegistryProgramInstallResponse> {
    let pinned = project_install(Path::new(config_path), |_, locked| {
        locked.is_some_and(|entry| entry.program_id.as_deref() == Some(owner))
    })?;
    if let Some(install) = pinned {
        return Ok(install);
    }
    let program = client
        .list_registry_programs()?
        .into_iter()
        .find(|program| program.program_id == owner)
        .ok_or_else(|| {
            anyhow!(
                "{address} is owned by {owner}, which the registry doesn't list. Pass --program <package> to decode it with another program"
            )
        })?;
    client.get_registry_program_install(&program.install_name, None)
}

fn check_owner(
    install: &RegistryProgramInstallResponse,
    raw: &RawAccount,
    address: &str,
) -> Result<()> {
    if raw.owner != install.definition.program_id {
        bail!(
            "{address} is owned by {}, not {} ({})",
            raw.owner,
            install.install_name,
            install.definition.program_id
        );
    }
    Ok(())
}

/// The install's Program Read binding: its id, endpoint and session
/// endpoint.
fn program_read_binding(
    install: &RegistryProgramInstallResponse,
) -> Result<(String, String, String)> {
    let RegistryProgramInstallTransport::HostedBinding { binding } = &install.transport;
    let session_endpoint = binding
        .auth
        .get("sessionEndpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "{} has no hosted Program Read binding",
                install.install_name
            )
        })?;
    Ok((
        binding.program_read_binding_id.clone(),
        binding.endpoint.clone(),
        session_endpoint.to_string(),
    ))
}

/// The account type to decode as: `--account`, else the IDL account whose
/// discriminator starts the data, else the IDL's only account.
fn account_type(
    install: &RegistryProgramInstallResponse,
    raw: &RawAccount,
    requested: Option<&str>,
) -> Result<String> {
    let accounts: Vec<&Value> = install.definition.idl_payload["accounts"]
        .as_array()
        .map(|accounts| accounts.iter().collect())
        .unwrap_or_default();
    let name = |account: &Value| account["name"].as_str().map(str::to_string);
    if let Some(requested) = requested {
        return accounts
            .iter()
            .filter_map(|account| name(account))
            .find(|candidate| candidate == requested)
            .ok_or_else(|| {
                anyhow!(
                    "{} has no account type '{requested}'; it has: {}",
                    install.install_name,
                    account_names(&accounts)
                )
            });
    }
    match matching_accounts(&accounts, raw).as_slice() {
        [only] => Ok(only.clone()),
        [] if accounts.len() == 1 => name(accounts[0])
            .ok_or_else(|| anyhow!("{}'s IDL account has no name", install.install_name)),
        [] => bail!(
            "No account type of {} matches the account's data; pass --account <TYPE> (one of: {})",
            install.install_name,
            account_names(&accounts)
        ),
        several => bail!(
            "Several account types of {} match the account's data ({}); pass --account <TYPE>",
            install.install_name,
            several.join(", ")
        ),
    }
}

/// The account type when the environment serves another release than the
/// installed one. Only the installed IDL is at hand, so `--account` is the
/// environment's to check, and the IDL's only account is no evidence: the
/// served release may have others.
fn served_account_type(
    install: &RegistryProgramInstallResponse,
    raw: &RawAccount,
    requested: Option<&str>,
) -> Result<String> {
    if let Some(requested) = requested {
        return Ok(requested.to_string());
    }
    let accounts: Vec<&Value> = install.definition.idl_payload["accounts"]
        .as_array()
        .map(|accounts| accounts.iter().collect())
        .unwrap_or_default();
    if matching_accounts(&accounts, raw).is_empty() {
        bail!(
            "The environment serves another release of {}, and the account's data matches no account type of the installed one; pass --account <TYPE> with the type's name in the release you registered",
            install.install_name
        );
    }
    account_type(install, raw, None)
}

/// The names of the IDL accounts whose discriminator starts the data.
fn matching_accounts(accounts: &[&Value], raw: &RawAccount) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| {
            discriminator(account)
                .is_some_and(|prefix| !prefix.is_empty() && raw.data.starts_with(&prefix))
        })
        .filter_map(|account| account["name"].as_str().map(str::to_string))
        .collect()
}

fn discriminator(account: &Value) -> Option<Vec<u8>> {
    account["discriminator"]
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

fn account_names(accounts: &[&Value]) -> String {
    let names: Vec<&str> = accounts
        .iter()
        .filter_map(|account| account["name"].as_str())
        .collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// `POST <chain>/chain/v1/account` with a gateway read session.
fn read_chain_account(
    client: &ApiClient,
    chain: &RegistryCapabilityInstallBinding,
    address: &str,
) -> Result<Option<RawAccount>> {
    let token = client.mint_target_session(
        &chain.auth.session_endpoint,
        &json!({
            "scopes": ["read"],
            "targetKind": chain.auth.target_kind,
            "targetId": chain.auth.target_id,
        }),
    )?;
    let url = format!("{}/chain/v1/account", chain.endpoint.trim_end_matches('/'));
    let body = send(
        http()?
            .post(&url)
            .bearer_auth(token)
            .json(&json!({"address": address, "options": {"commitment": "confirmed"}})),
        "chain read",
    )?;
    raw_from_chain(&body)
}

/// A `/chain/v1/account` answer: `{value: {ownerProgram, data (base64)}}`
/// or `{value: null}`.
fn raw_from_chain(body: &Value) -> Result<Option<RawAccount>> {
    let value = &body["value"];
    if value.is_null() {
        return Ok(None);
    }
    let owner = value["ownerProgram"]
        .as_str()
        .ok_or_else(|| anyhow!("The chain read answered without an owner"))?;
    let data = value["data"]
        .as_str()
        .ok_or_else(|| anyhow!("The chain read answered without data"))?;
    Ok(Some(RawAccount {
        owner: owner.to_string(),
        data: base64::engine::general_purpose::STANDARD
            .decode(data)
            .context("The chain read's data is not base64")?,
    }))
}

/// A `getAccountInfo` (base64) result: `{value: {owner, data: [b64, "base64"]}}`
/// or `{value: null}`.
fn raw_from_rpc(result: &Value) -> Result<Option<RawAccount>> {
    let value = &result["value"];
    if value.is_null() {
        return Ok(None);
    }
    let owner = value["owner"]
        .as_str()
        .ok_or_else(|| anyhow!("getAccountInfo answered without an owner"))?;
    let data = value["data"][0]
        .as_str()
        .ok_or_else(|| anyhow!("getAccountInfo answered without base64 data"))?;
    Ok(Some(RawAccount {
        owner: owner.to_string(),
        data: base64::engine::general_purpose::STANDARD
            .decode(data)
            .context("getAccountInfo's data is not base64")?,
    }))
}

/// `GET <endpoint>/v1/releases/<release>/accounts/<Account>/<address>`.
fn read_decoded(
    endpoint: &str,
    release: &str,
    account: &str,
    address: &str,
    token: &str,
) -> Result<Value> {
    let url = decode_url(endpoint, release, account, address)?;
    let value = send(http()?.get(url).bearer_auth(token), "Program Read")?;
    if value.is_null() {
        bail!("Program Read found no {account} at {address}");
    }
    Ok(value)
}

fn decode_url(endpoint: &str, release: &str, account: &str, address: &str) -> Result<String> {
    let mut url = url::Url::parse(endpoint)
        .with_context(|| format!("Invalid Program Read endpoint {endpoint}"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow!("Invalid Program Read endpoint {endpoint}"))?
        .pop_if_empty()
        .extend(["v1", "releases", release, "accounts", account, address]);
    Ok(url.to_string())
}

fn http() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("a4/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build the HTTP client")
}

/// Send `request`; an error status keeps the server's code and message.
fn send(request: reqwest::blocking::RequestBuilder, what: &str) -> Result<Value> {
    let response = request
        .send()
        .with_context(|| format!("The {what} request failed"))?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let code = body
        .pointer("/error/code")
        .or_else(|| body.get("code"))
        .and_then(Value::as_str);
    let message = body
        .pointer("/error/message")
        .or_else(|| body.get("message"))
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or(text.trim());
    match code {
        Some(code) => bail!("{what} → HTTP {} {code}: {message}", status.as_u16()),
        None => bail!("{what} → HTTP {}: {message}", status.as_u16()),
    }
}

fn is_pubkey(value: &str) -> bool {
    arete_mcp::dev_env::is_pubkey(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(accounts: Value) -> RegistryProgramInstallResponse {
        serde_json::from_value(json!({
            "installName": "klend",
            "displayName": "Kamino Lend",
            "definition": {
                "programId": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                "programSpecHash": "spec",
                "idlContentHash": "idl",
                "normalizedIdlHash": "nidl",
                "idlPayload": {"accounts": accounts},
                "programSpec": {},
            },
            "release": {"programReleaseHash": "arete:h1:program-release:sha256:aa", "programSpecHash": "spec"},
            "transport": {"kind": "hosted-binding", "binding": {
                "endpoint": "https://reads.example.test",
                "programReadBindingId": "prb_00000000000000000000000000000001",
                "auth": {"sessionEndpoint": "https://api.example.test/ws/sessions"},
            }},
        }))
        .expect("install fixture")
    }

    fn raw(data: &[u8]) -> RawAccount {
        RawAccount {
            owner: "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD".into(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn the_account_type_follows_the_discriminator() {
        let install = install(json!([
            {"name": "LendingMarket", "discriminator": [1, 2, 3, 4, 5, 6, 7, 8]},
            {"name": "Reserve", "discriminator": [9, 9, 9, 9, 9, 9, 9, 9]},
        ]));
        let reserve = raw(&[9, 9, 9, 9, 9, 9, 9, 9, 42]);
        assert_eq!(account_type(&install, &reserve, None).unwrap(), "Reserve");
        assert_eq!(
            account_type(&install, &reserve, Some("LendingMarket")).unwrap(),
            "LendingMarket"
        );
        let error = account_type(&install, &reserve, Some("Obligation")).unwrap_err();
        assert!(
            error.to_string().contains("LendingMarket, Reserve"),
            "{error}"
        );
        let unknown = account_type(&install, &raw(&[0; 16]), None).unwrap_err();
        assert!(unknown.to_string().contains("--account"), "{unknown}");
    }

    #[test]
    fn another_served_release_answers_for_its_own_account_types() {
        let install = install(json!([
            {"name": "Reserve", "discriminator": [9, 9, 9, 9, 9, 9, 9, 9]},
        ]));
        // A type only the served release has goes to the environment.
        assert_eq!(
            served_account_type(&install, &raw(&[7; 16]), Some("Vault")).unwrap(),
            "Vault"
        );
        // Data the installed IDL knows still finds its type.
        let reserve = raw(&[9, 9, 9, 9, 9, 9, 9, 9, 42]);
        assert_eq!(
            served_account_type(&install, &reserve, None).unwrap(),
            "Reserve"
        );
        // The IDL's only account isn't assumed for data it doesn't match.
        let error = served_account_type(&install, &raw(&[7; 16]), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--account"), "{error}");
    }

    const KLEND: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";

    fn program_project(version: &str) -> (tempfile::TempDir, ProjectManifest) {
        let dir = tempfile::tempdir().unwrap();
        let mut toml = crate::project::manifest::ManifestV1::new("app".into())
            .to_toml_pretty()
            .unwrap();
        toml.push_str(&format!(
            "\n[dependencies.programs.lending]\nsource = {{ registry = \"klend\" }}\nversion = \"{version}\"\n"
        ));
        std::fs::write(dir.path().join("arete.toml"), toml).unwrap();
        let manifest = ProjectManifest::load(dir.path().join("arete.toml")).unwrap();
        (dir, manifest)
    }

    fn locked_project(requirement: &str, targets: &[InstallTarget]) -> ProjectLock {
        let mut lock = ProjectLock::empty("hash".into());
        lock.dependencies.push(
            toml::from_str(&format!(
                r#"
                kind = "program"
                alias = "lending"
                source = "registry:klend"
                requirement = "{requirement}"
                package_release_hash = "pinned"
                program_id = "{KLEND}"
                generator_contract = "contract"
                "#
            ))
            .unwrap(),
        );
        lock.dependencies[0].targets = targets.to_vec();
        lock
    }

    #[test]
    fn a_project_alias_keeps_its_requirement_and_locked_release() {
        let (_dir, manifest) = program_project("^1.2.0");
        let targets = manifest
            .dependency(DependencyKind::Program, "lending")
            .unwrap()
            .selected_targets(&manifest.document.sdk)
            .to_vec();
        let lock = locked_project("^1.2.0", &targets);

        let (_, request) =
            program_request(&manifest, Some(&lock), |alias, _| alias == "lending").unwrap();
        assert_eq!(request.package, "klend");
        assert_eq!(request.requirement, "^1.2.0");
        assert_eq!(
            request.locked_package_release_hash.as_deref(),
            Some("pinned")
        );

        // The owner's program is found through the lock's program id.
        let by_owner = program_request(&manifest, Some(&lock), |_, locked| {
            locked.is_some_and(|entry| entry.program_id.as_deref() == Some(KLEND))
        });
        assert!(by_owner.is_some());

        // No lock yet: the requirement alone chooses the release.
        let (_, unlocked) =
            program_request(&manifest, None, |alias, _| alias == "lending").unwrap();
        assert_eq!(unlocked.requirement, "^1.2.0");
        assert_eq!(unlocked.locked_package_release_hash, None);

        // A lock for another requirement no longer describes the dependency.
        let stale = locked_project("^1.0.0", &targets);
        let (_, stale) =
            program_request(&manifest, Some(&stale), |alias, _| alias == "lending").unwrap();
        assert_eq!(stale.locked_package_release_hash, None);

        assert!(program_request(&manifest, Some(&lock), |alias, _| alias == "other").is_none());
    }

    #[test]
    fn an_owner_mismatch_names_both_programs() {
        let install = install(json!([]));
        let mut other = raw(&[]);
        other.owner = "11111111111111111111111111111111".into();
        let error = check_owner(&install, &other, "Addr")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("11111111111111111111111111111111"),
            "{error}"
        );
        assert!(error.contains("klend"), "{error}");
    }

    #[test]
    fn raw_accounts_read_from_the_chain_and_the_fork() {
        let chain = json!({"context": {}, "value": {"address": "A", "ownerProgram": "Owner", "lamports": "1", "executable": false, "data": "AQID"}});
        assert_eq!(
            raw_from_chain(&chain).unwrap(),
            Some(RawAccount {
                owner: "Owner".into(),
                data: vec![1, 2, 3]
            })
        );
        assert_eq!(raw_from_chain(&json!({"value": null})).unwrap(), None);
        let rpc = json!({"context": {"slot": 5}, "value": {"owner": "Owner", "data": ["AQID", "base64"], "lamports": 1}});
        assert_eq!(
            raw_from_rpc(&rpc).unwrap(),
            Some(RawAccount {
                owner: "Owner".into(),
                data: vec![1, 2, 3]
            })
        );
        assert_eq!(raw_from_rpc(&json!({"value": null})).unwrap(), None);
    }

    #[test]
    fn dev_reads_use_the_resolved_endpoint_and_release_or_the_refusal_fix() {
        let target = json!({"programReads": [{"status": "target", "endpoint": "https://dev-reads.example.test", "programReleaseHash": "arete:h1:program-release:sha256:bb", "releaseDiffers": true}]});
        assert_eq!(
            dev_program_read_target(&target).unwrap(),
            (
                "https://dev-reads.example.test".to_string(),
                "arete:h1:program-release:sha256:bb".to_string()
            )
        );
        let refused = json!({"programReads": [{"status": "refused", "code": "devenv-surface-unavailable", "message": "Program Read is not available", "fix": "Retry later"}]});
        let error = dev_program_read_target(&refused).unwrap_err().to_string();
        assert!(error.contains("devenv-surface-unavailable"), "{error}");
        assert!(error.contains("Retry later"), "{error}");
    }

    #[test]
    fn decode_urls_address_the_release_route() {
        assert_eq!(
            decode_url(
                "https://reads.example.test/base/",
                "arete:h1:program-release:sha256:aa",
                "Reserve",
                "Addr"
            )
            .unwrap(),
            "https://reads.example.test/base/v1/releases/arete:h1:program-release:sha256:aa/accounts/Reserve/Addr"
        );
    }

    #[test]
    fn env_flags_need_dev() {
        let args = DecodeArgs {
            address: "Addr".into(),
            program: None,
            account: None,
            dev: false,
            env: EnvArgs {
                project: Default::default(),
                env: Some("main".into()),
            },
        };
        assert!(read_target(&args).is_err());
    }
}
