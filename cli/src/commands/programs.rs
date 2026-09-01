use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use dialoguer::Confirm;

use crate::api_client::{
    ApiClient, CreateUserProgramRequest, UserProgramEventsResponse, UserProgramListResponse,
    UserProgramPromotionResponse, UserProgramResponse, USER_PROGRAM_EVENTS_SCHEMA,
    USER_PROGRAM_LIST_SCHEMA, USER_PROGRAM_PROMOTION_SCHEMA, USER_PROGRAM_SCHEMA,
    USER_PROGRAM_UPLOAD_SCHEMA,
};

const DEFAULT_PAGE_LIMIT: usize = 100;
const WAIT_TIMEOUT: Duration = Duration::from_secs(300);

pub fn push(
    input: &str,
    program_id: Option<&str>,
    alias: Option<String>,
    idempotency_key: Option<String>,
    wait: bool,
    json: bool,
) -> Result<()> {
    let program_spec = load_or_build_program_spec(Path::new(input), program_id)?;
    let expected_program_id = program_spec.payload.program_id.clone();
    let expected_program_spec_hash = program_spec.artifact_hash.to_string();
    let idempotency_key = canonical_idempotency_key(idempotency_key.as_deref())?;
    let request = CreateUserProgramRequest {
        schema: USER_PROGRAM_UPLOAD_SCHEMA.to_string(),
        idempotency_key,
        alias,
        program_spec,
    };
    let client = ApiClient::new()?;
    let mut response = client.create_user_program(&request)?;
    verify_echoed_identity(&response, &expected_program_id, &expected_program_spec_hash)?;
    if wait && !terminal_admission_state(&response.admission_state) {
        response = wait_for_terminal(&client, &response.user_program_id)?;
        verify_echoed_identity(&response, &expected_program_id, &expected_program_spec_hash)?;
    }
    print_program(&response, json)
}

pub fn list(cursor: Option<&str>, json: bool) -> Result<()> {
    if let Some(cursor) = cursor {
        validate_program_cursor(cursor)?;
    }
    let response = ApiClient::new()?.list_user_programs(DEFAULT_PAGE_LIMIT, cursor)?;
    validate_program_list(&response)?;
    print_program_list(&response, json)
}

pub fn status(user_program_id: &str, watch: bool, json: bool) -> Result<()> {
    validate_user_program_id(user_program_id)?;
    let client = ApiClient::new()?;
    let response = if watch {
        wait_for_terminal(&client, user_program_id)?
    } else {
        client.get_user_program(user_program_id)?
    };
    validate_program_response(&response)?;
    print_program(&response, json)
}

pub fn events(user_program_id: &str, after: Option<&str>, json: bool) -> Result<()> {
    validate_user_program_id(user_program_id)?;
    let response =
        ApiClient::new()?.list_user_program_events(user_program_id, after, DEFAULT_PAGE_LIMIT)?;
    validate_events_response(&response)?;
    print_events(&response, json)
}

pub fn archive(user_program_id: &str, yes: bool, json: bool) -> Result<()> {
    validate_user_program_id(user_program_id)?;
    if !yes {
        if !io::stdin().is_terminal() {
            bail!("Archival requires --yes when stdin is not interactive");
        }
        let confirmed = Confirm::new()
            .with_prompt("Archive this program registration? Immutable content is retained")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("Archival cancelled");
        }
    }
    let response = ApiClient::new()?.archive_user_program(user_program_id)?;
    validate_program_response(&response)?;
    if response.user_program_id != user_program_id || response.lifecycle_state != "archived" {
        bail!("Server response did not confirm archival of the requested program");
    }
    print_program(&response, json)
}

pub fn promote(user_program_id: &str, make_idl_public: bool, json: bool) -> Result<()> {
    validate_user_program_id(user_program_id)?;
    if !make_idl_public {
        if !io::stdin().is_terminal() {
            bail!(
                "Promotion requires --make-idl-public when stdin is not interactive; the baseline IDL may enter a public OSS repository"
            );
        }
        let confirmed = Confirm::new()
            .with_prompt(
                "Allow this baseline IDL to be reviewed and published in a public OSS repository?",
            )
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("Promotion request cancelled because public-IDL consent was not granted");
        }
    }
    let response = ApiClient::new()?.request_user_program_promotion(user_program_id)?;
    validate_promotion_response(&response, user_program_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("Promotion request: {}", response.promotion_request_id);
        println!("Program: {}", response.user_program_id);
        println!("Status: {}", response.status);
        println!("Public IDL consent: granted");
    }
    Ok(())
}

fn load_or_build_program_spec(
    path: &Path,
    explicit_program_id: Option<&str>,
) -> Result<arete_artifacts::ProgramSpecArtifact> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read program input {}", path.display()))?;
    let looks_like_program_spec = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some(arete_artifacts::PROGRAM_SPEC_KIND);
    if looks_like_program_spec {
        if explicit_program_id.is_some() {
            bail!("--program-id cannot be used when the input is already a ProgramSpec");
        }
        return arete_artifacts::load_program_spec(&bytes)
            .map(|loaded| loaded.artifact)
            .with_context(|| format!("Invalid ProgramSpec {}", path.display()));
    }
    let payload = arete_hash::build_program_spec_v1_from_bytes(&bytes, explicit_program_id)
        .map_err(anyhow::Error::new)
        .with_context(|| format!("Failed to build ProgramSpec from {}", path.display()))?;
    arete_artifacts::ProgramSpecArtifact::new(payload).map_err(Into::into)
}

fn canonical_idempotency_key(value: Option<&str>) -> Result<String> {
    let parsed = match value {
        Some(value) => uuid::Uuid::parse_str(value).context("Invalid --idempotency-key UUID")?,
        None => uuid::Uuid::new_v4(),
    };
    if parsed.is_nil() || value.is_some_and(|value| parsed.to_string() != value) {
        bail!("--idempotency-key must be a non-nil canonical lowercase UUID");
    }
    Ok(parsed.to_string())
}

fn validate_user_program_id(value: &str) -> Result<()> {
    let valid = value.strip_prefix("upr_").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    });
    if !valid {
        bail!("user program ID must match upr_<32 URL-safe characters>");
    }
    Ok(())
}

fn verify_echoed_identity(
    response: &UserProgramResponse,
    expected_program_id: &str,
    expected_program_spec_hash: &str,
) -> Result<()> {
    validate_program_response(response)?;
    if response.program_id != expected_program_id
        || response.program_spec_hash != expected_program_spec_hash
    {
        bail!("Server response did not echo the exact uploaded ProgramSpec identity");
    }
    Ok(())
}

fn wait_for_terminal(client: &ApiClient, user_program_id: &str) -> Result<UserProgramResponse> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut backoff = Duration::from_millis(500);
    loop {
        let response = client.get_user_program(user_program_id)?;
        validate_program_response(&response)?;
        if response.user_program_id != user_program_id {
            bail!("Server response returned a different user program resource");
        }
        if terminal_admission_state(&response.admission_state) {
            return Ok(response);
        }
        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for program admission; the server job is still running. Check with `a4 program status {user_program_id}`"
            );
        }
        thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

fn terminal_admission_state(state: &str) -> bool {
    matches!(state, "ready" | "failed")
}

fn validate_program_response(response: &UserProgramResponse) -> Result<()> {
    if response.schema != USER_PROGRAM_SCHEMA {
        bail!("Server returned an unsupported user-program response schema");
    }
    validate_user_program_id(&response.user_program_id)?;
    let program_id = response
        .program_id
        .parse::<arete_sdk::Pubkey>()
        .context("Server returned an invalid program ID")?;
    if program_id.to_string() != response.program_id
        || response
            .program_spec_hash
            .parse::<arete_hash::HashId<arete_hash::ProgramSpec>>()
            .is_err()
        || response.program_release_hash.as_ref().is_some_and(|value| {
            value
                .parse::<arete_hash::HashId<arete_hash::ProgramRelease>>()
                .is_err()
        })
        || response
            .program_read_binding_id
            .as_ref()
            .is_some_and(|value| value.parse::<arete_hash::ProgramReadBindingId>().is_err())
    {
        bail!("Server returned an invalid user-program identity");
    }
    if !matches!(
        response.lifecycle_state.as_str(),
        "active" | "archived" | "disabled"
    ) || !matches!(
        response.admission_state.as_str(),
        "queued" | "leased" | "ready" | "failed"
    ) || !matches!(
        response.visibility.as_str(),
        "private" | "global" | "public"
    ) || !matches!(
        response.operational_status.as_str(),
        "preparing" | "exact" | "unverified" | "no_deployment"
    ) || !matches!(
        response.health.status.as_str(),
        "unverified" | "no_deployment" | "no_activity" | "healthy" | "warning"
    ) || response.health.schema_failure_rate_basis_points > 10_000
        || !valid_event_cursor(&response.event_cursor)
        || response.diagnostic_codes.iter().any(|code| {
            code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
    {
        bail!("Server returned invalid user-program state");
    }
    if response.admission_state == "ready"
        && (response.program_release_hash.is_none() || response.program_read_binding_id.is_none())
    {
        bail!("Ready user-program response omitted its exact installation identity");
    }
    Ok(())
}

fn validate_program_list(response: &UserProgramListResponse) -> Result<()> {
    if response.schema != USER_PROGRAM_LIST_SCHEMA
        || response
            .next_cursor
            .as_ref()
            .is_some_and(|cursor| validate_program_cursor(cursor).is_err())
    {
        bail!("Server returned an invalid user-program list contract");
    }
    for program in &response.items {
        validate_program_response(program)?;
    }
    Ok(())
}

fn validate_program_cursor(value: &str) -> Result<()> {
    let valid = value.strip_prefix("upc_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 128
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    });
    if !valid {
        bail!("program list cursor must be an opaque upc_ value returned by the server");
    }
    Ok(())
}

fn validate_events_response(response: &UserProgramEventsResponse) -> Result<()> {
    if response.schema != USER_PROGRAM_EVENTS_SCHEMA
        || response
            .next_cursor
            .as_ref()
            .is_some_and(|cursor| !valid_event_cursor(cursor))
        || response.items.iter().any(|event| {
            !valid_event_cursor(&event.cursor)
                || event.event_type.is_empty()
                || event.event_type.len() > 64
                || !event
                    .event_type
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                || event.diagnostic_code.as_ref().is_some_and(|code| {
                    code.is_empty()
                        || code.len() > 64
                        || !code.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
        })
    {
        bail!("Server returned an invalid user-program event contract");
    }
    Ok(())
}

fn validate_promotion_response(
    response: &UserProgramPromotionResponse,
    user_program_id: &str,
) -> Result<()> {
    if response.schema != USER_PROGRAM_PROMOTION_SCHEMA
        || response.user_program_id != user_program_id
        || uuid::Uuid::parse_str(&response.promotion_request_id).is_err()
        || !matches!(
            response.status.as_str(),
            "requested" | "reviewing" | "rejected" | "promoted"
        )
    {
        bail!("Server returned an invalid program-promotion response contract");
    }
    Ok(())
}

fn valid_event_cursor(value: &str) -> bool {
    value.strip_prefix("uev_").is_some_and(|suffix| {
        suffix.len() == 11
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

fn print_program(response: &UserProgramResponse, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(response)?);
        return Ok(());
    }
    println!("Program resource: {}", response.user_program_id);
    println!("Program ID: {}", response.program_id);
    println!("ProgramSpec: {}", response.program_spec_hash);
    println!("Admission: {}", response.admission_state);
    println!("Visibility: {}", response.visibility);
    println!("Operational status: {}", response.operational_status);
    println!("Health: {}", response.health.status);
    if let Some(release) = &response.program_release_hash {
        println!("Program Release: {release}");
    }
    if let Some(binding) = &response.program_read_binding_id {
        println!("Program Read binding: {binding}");
    }
    for diagnostic in &response.diagnostic_codes {
        println!("Warning: {diagnostic}");
    }
    Ok(())
}

fn print_program_list(response: &UserProgramListResponse, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(response)?);
    } else if response.items.is_empty() {
        println!("No uploaded programs");
    } else {
        for program in &response.items {
            println!(
                "{}  {}  {}  {}",
                program.user_program_id,
                program.program_id,
                program.admission_state,
                program.visibility
            );
        }
        if let Some(cursor) = &response.next_cursor {
            println!("Next cursor: {cursor}");
        }
    }
    Ok(())
}

fn print_events(response: &UserProgramEventsResponse, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(response)?);
        return Ok(());
    }
    if response.items.is_empty() {
        println!("No program events");
    } else {
        for event in &response.items {
            let detail = event
                .state
                .as_deref()
                .or(event.diagnostic_code.as_deref())
                .unwrap_or("");
            println!("{}  {}  {}", event.occurred_at, event.event_type, detail);
        }
    }
    if let Some(cursor) = &response.next_cursor {
        println!("Next cursor: {cursor}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_hash(kind: &str, digit: char) -> String {
        format!("arete:h1:{kind}:sha256:{}", digit.to_string().repeat(64))
    }

    fn ready_program_response() -> UserProgramResponse {
        UserProgramResponse {
            schema: USER_PROGRAM_SCHEMA.into(),
            user_program_id: "upr_abcdefghijklmnopqrstuvwxyzABCDEF".into(),
            program_id: "11111111111111111111111111111111".into(),
            program_spec_hash: contract_hash("program-spec", '1'),
            alias: Some("ore".into()),
            lifecycle_state: "active".into(),
            admission_state: "ready".into(),
            visibility: "private".into(),
            program_release_hash: Some(contract_hash("program-release", '2')),
            program_read_binding_id: Some("prb_00000000000000000000000000000002".into()),
            operational_status: "exact".into(),
            health: crate::api_client::UserProgramHealth {
                status: "healthy".into(),
                assessed_at: Some("2026-09-01T12:00:00Z".into()),
                schema_relevant_attempts: 100,
                schema_failure_rate_basis_points: 25,
            },
            event_cursor: "uev_00000000001".into(),
            diagnostic_codes: Vec::new(),
            idempotent: false,
        }
    }

    #[test]
    fn validates_opaque_resource_and_idempotency_ids() {
        validate_user_program_id("upr_abcdefghijklmnopqrstuvwxyzABCDEF").unwrap();
        assert!(validate_user_program_id("upr_short").is_err());
        assert!(canonical_idempotency_key(Some("00000000-0000-0000-0000-000000000000")).is_err());
        assert_eq!(
            canonical_idempotency_key(Some("018f8f12-8ac5-7d91-a5df-4b3b65f31a80")).unwrap(),
            "018f8f12-8ac5-7d91-a5df-4b3b65f31a80"
        );
        validate_program_cursor("upc_next-page_123").unwrap();
        assert!(validate_program_cursor("uev_wrong-kind").is_err());
    }

    #[test]
    fn validates_server_program_identity_and_state_before_trusting_it() {
        let response = ready_program_response();
        validate_program_response(&response).unwrap();

        let mut malformed = response.clone();
        malformed.program_id = "not-a-program-id".into();
        assert!(validate_program_response(&malformed).is_err());

        let mut malformed = response.clone();
        malformed.program_release_hash = None;
        assert!(validate_program_response(&malformed).is_err());

        let mut malformed = response;
        malformed.health.schema_failure_rate_basis_points = 10_001;
        assert!(validate_program_response(&malformed).is_err());
    }
}
