use anyhow::Result;
use colored::Colorize;

use crate::api_client::{ApiClient, EntitySchema, RegistryStackItem, DEFAULT_DOMAIN_SUFFIX};

pub fn list(json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let registry_stacks = client.list_registry()?;
    let user_stacks = client.list_specs().ok();
    let user_deployments = if user_stacks.is_some() {
        client.list_deployments(100).ok()
    } else {
        None
    };

    if json {
        #[derive(serde::Serialize)]
        struct ExploreListOutput {
            registry: Vec<RegistryStackItem>,
            #[serde(skip_serializing_if = "Option::is_none")]
            user_stacks: Option<Vec<UserStackItem>>,
        }

        #[derive(serde::Serialize)]
        struct UserStackItem {
            name: String,
            entity_name: String,
            websocket_url: String,
            status: Option<String>,
        }

        let user_items = user_stacks.map(|specs| {
            let deployment_map: std::collections::HashMap<i32, _> = user_deployments
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.spec_id, d))
                .collect();

            specs
                .into_iter()
                .map(|spec| {
                    let deployment = deployment_map.get(&spec.id);
                    UserStackItem {
                        name: spec.name.clone(),
                        entity_name: spec.entity_name.clone(),
                        websocket_url: spec.websocket_url(DEFAULT_DOMAIN_SUFFIX),
                        status: deployment.map(|d| d.status.to_string()),
                    }
                })
                .collect()
        });

        let output = ExploreListOutput {
            registry: registry_stacks,
            user_stacks: user_items,
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if !registry_stacks.is_empty() {
        println!("\n{}", "Public Registry".bold());
        println!("{}", "-".repeat(60).dimmed());

        for stack in &registry_stacks {
            println!(
                "  {}  {}",
                stack.name.green().bold(),
                stack.websocket_url.cyan()
            );
            if let Some(desc) = &stack.description {
                println!("    {}", desc.dimmed());
            }
            println!("    Entities: {}", stack.entities.join(", "));
            println!();
        }
    }

    if let Some(specs) = user_stacks {
        if !specs.is_empty() {
            let deployment_map: std::collections::HashMap<i32, _> = user_deployments
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.spec_id, d))
                .collect();

            println!("{}", "Your Stacks".bold());
            println!("{}", "-".repeat(60).dimmed());

            for spec in &specs {
                let deployment = deployment_map.get(&spec.id);
                let status = deployment
                    .map(|d| d.status.to_string())
                    .unwrap_or_else(|| "-".to_string());

                println!(
                    "  {}  {}  [{}]",
                    spec.name.green().bold(),
                    spec.websocket_url(DEFAULT_DOMAIN_SUFFIX).cyan(),
                    status,
                );
            }
            println!();
        }
    }

    if registry_stacks.is_empty() {
        println!("{}", "No stacks found in registry.".yellow());
    }

    println!(
        "{}",
        "Tip: Run `a4 explore <name>` for detailed entity info".dimmed()
    );

    Ok(())
}

pub fn show(name: &str, entity: Option<&str>, json: bool) -> Result<()> {
    let client = ApiClient::new()?;
    let schema_response = match client.get_registry_schema(name) {
        Ok(schema) => schema,
        Err(_) => {
            let spec = client.get_spec_by_name(name)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "Stack '{}' not found. Run `a4 explore` to see available stacks.",
                    name
                )
            })?;
            client.get_spec_schema(spec.id)?
        }
    };

    if let Some(entity_name) = entity {
        let entity_schema = schema_response
            .schema
            .entities
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(entity_name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Entity '{}' not found in stack '{}'. Available entities: {}",
                    entity_name,
                    name,
                    schema_response
                        .schema
                        .entities
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;

        if json {
            println!("{}", serde_json::to_string_pretty(&entity_schema)?);
            return Ok(());
        }

        print_entity_detail(entity_schema);
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&schema_response)?);
        return Ok(());
    }

    println!(
        "\n{} {}",
        "Stack:".bold(),
        schema_response.name.green().bold()
    );
    println!("  URL: {}", schema_response.websocket_url.cyan());
    if let Some(desc) = &schema_response.description {
        println!("  {}", desc.dimmed());
    }

    println!("\n{}", "Entities".bold());
    println!("{}", "-".repeat(60).dimmed());

    for entity_schema in &schema_response.schema.entities {
        let view_names: Vec<&str> = entity_schema
            .views
            .iter()
            .map(|v| v.id.split('/').next_back().unwrap_or(v.id.as_str()))
            .collect();

        println!(
            "  {}  {} views  ({})",
            entity_schema.name.green().bold(),
            entity_schema.views.len(),
            view_names.join(", ")
        );
        println!(
            "    Primary key: {}",
            entity_schema.primary_keys.join(", ").cyan()
        );
        println!("    Fields: {}", entity_schema.fields.len());
    }

    println!();
    println!(
        "{}",
        format!("Tip: Run `a4 explore {} <entity>` for field details", name).dimmed()
    );

    Ok(())
}

fn print_entity_detail(entity: &EntitySchema) {
    println!("\n{} {}", "Entity:".bold(), entity.name.green().bold());
    println!("  Primary key: {}", entity.primary_keys.join(", ").cyan());

    println!("\n{}", "Fields".bold());
    println!("{}", "-".repeat(70).dimmed());

    let mut current_section = String::new();
    for field in &entity.fields {
        if field.section != current_section {
            current_section = field.section.clone();
            println!("  {} {}", "-".dimmed(), current_section.bold());
        }

        let nullable_str = if field.nullable { "?" } else { "" };
        println!(
            "    {:<40} {}{}",
            field.path,
            field.rust_type.cyan(),
            nullable_str.dimmed()
        );
    }

    println!("\n{}", "Views".bold());
    println!("{}", "-".repeat(70).dimmed());

    for view in &entity.views {
        let view_short = view.id.split('/').next_back().unwrap_or(view.id.as_str());
        let pipeline_str = if view.pipeline.is_empty() {
            String::new()
        } else {
            let steps: Vec<String> = view
                .pipeline
                .iter()
                .map(|p| {
                    if let Some(sort) = p.get("Sort") {
                        let key = sort
                            .get("key")
                            .and_then(|k| k.get("segments"))
                            .and_then(|s| s.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(".")
                            })
                            .unwrap_or_default();
                        let order = sort.get("order").and_then(|o| o.as_str()).unwrap_or("asc");
                        format!("sort by {} {}", key, order)
                    } else {
                        p.to_string()
                    }
                })
                .collect();
            format!("  ({})", steps.join(", "))
        };

        println!(
            "  {:<20} {:<8}{}",
            view_short.green(),
            view.mode,
            pipeline_str.dimmed()
        );
    }

    println!();
}
