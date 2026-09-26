use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, endpoint, fetch_json, http_client,
    statuspage,
};
use crate::domain::rollup;

#[derive(Default)]
pub struct IntercomProvider;

#[async_trait]
impl StatusProvider for IntercomProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        statuspage::StatuspageProvider.validate_config(config)
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        // The compatible API ignores subpage_slug; only the native API scopes by region.
        let summary = fetch_json(
            &client,
            endpoint(
                &context.base_url,
                "/proxy/www.finstatus.com?subpage_slug=us-hosting",
            )?,
            2 * 1024 * 1024,
        )
        .await?;
        let mut history = fetch_json(
            &client,
            endpoint(
                &context.base_url,
                "/proxy/www.finstatus.com/incidents?subpage_slug=us-hosting",
            )?,
            4 * 1024 * 1024,
        )
        .await?;
        let mut missing = context.known_active_incidents.clone();
        for incident in array(&summary["summary"], "ongoing_incidents")? {
            if let Some(id) = incident["id"].as_str() {
                missing.insert(id.to_owned());
            }
        }
        for incident in array(&history, "incidents")? {
            if let Some(id) = incident["id"].as_str() {
                missing.remove(id);
            }
        }
        for id in missing {
            let mut url = endpoint(&context.base_url, "/proxy/www.finstatus.com/incidents/")?;
            url.path_segments_mut()
                .map_err(|()| {
                    ProviderError::Configuration("Intercom URL cannot contain path segments".into())
                })?
                .pop_if_empty()
                .push(&id);
            url.set_query(Some("subpage_slug=us-hosting"));
            let detail = fetch_json(&client, url, 2 * 1024 * 1024).await?;
            if detail["incident"]["id"].as_str() != Some(&id) {
                return Err(ProviderError::Parse(
                    "Intercom detail omitted the requested incident".into(),
                ));
            }
            if let Some(incidents) = history["incidents"].as_array_mut() {
                incidents.push(detail["incident"].clone());
            }
        }
        parse_snapshot(&summary, &history)
    }
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse(format!("Intercom response omitted {key}")))
}

fn parse_snapshot(summary_root: &Value, history: &Value) -> Result<FetchOutcome, ProviderError> {
    let summary = &summary_root["summary"];
    if summary["public_url"].as_str() != Some("https://www.finstatus.com/us-hosting") {
        return Err(ProviderError::Parse(
            "Intercom response was not the US hosting page".into(),
        ));
    }
    let affected = array(summary, "affected_components")?;
    let mut components = Vec::new();
    for item in array(&summary["structure"], "items")? {
        let (values, group) = if let Some(group) = item.get("group") {
            (array(group, "components")?.as_slice(), group.get("name"))
        } else if let Some(component) = item.get("component") {
            (std::slice::from_ref(component), None)
        } else {
            return Err(ProviderError::Parse(
                "Intercom structure omitted component or group".into(),
            ));
        };
        for component in values {
            let id = component
                .get("component_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ProviderError::Parse("Intercom component omitted its ID".into()))?;
            let status = affected
                .iter()
                .find(|value| value["component_id"] == id)
                .map_or("operational", |value| {
                    value
                        .get("current_status")
                        .unwrap_or(&value["status"])
                        .as_str()
                        .unwrap_or("unknown")
                });
            components.push(json!({
                "id": id, "name": component["name"], "group_name": group, "status": status,
            }));
        }
    }
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "Intercom US page contained no components".into(),
        ));
    }
    let mut native = array(history, "incidents")?.clone();
    for key in ["ongoing_incidents", "scheduled_maintenances"] {
        for incident in array(summary, key)? {
            if !native.iter().any(|old| old["id"] == incident["id"]) {
                native.push(incident.clone());
            }
        }
    }
    let mut incidents = Vec::new();
    let mut maintenance = Vec::new();
    for incident in &native {
        let id = incident
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProviderError::Parse("Intercom incident omitted its ID".into()))?;
        let value = json!({
            "id": id, "name": incident["name"], "status": incident["status"],
            "created_at": incident["published_at"],
            "started_at": incident["component_impacts"].as_array().into_iter().flatten()
                .filter_map(|impact| super::parse_time(impact.get("start_at"))).min(),
            "shortlink": format!("https://www.finstatus.com/us-hosting/incidents/{id}"),
        });
        if incident["type"] == "maintenance" {
            maintenance.push(value);
        } else {
            incidents.push(value);
        }
    }
    let body = serde_json::to_vec(&json!({
        "page": {"url": summary["public_url"]},
        "components": components, "incidents": incidents, "scheduled_maintenances": maintenance,
    }))
    .map_err(|error| ProviderError::Parse(error.to_string()))?;
    let FetchOutcome::Fetched(mut snapshot) = statuspage::parse_summary(&body, 200, None, None)?
    else {
        return Err(ProviderError::Parse(
            "Intercom parser returned an unexpected response".into(),
        ));
    };
    statuspage::enrich_incidentio_incidents(&mut snapshot, &json!({"incidents": native}));
    snapshot.overall = rollup(snapshot.components.iter().map(|component| component.status));
    snapshot.original_overall = snapshot.overall.key().into();
    snapshot.provider_overall = snapshot.overall;
    snapshot.provider_original_overall = snapshot.original_overall.clone();
    // The native archive is bounded; absence is not evidence of resolution.
    snapshot.active_incident_set_complete = false;
    snapshot.raw_payload = Some(json!({"summary": summary_root, "history": history}));
    Ok(FetchOutcome::Fetched(snapshot))
}
