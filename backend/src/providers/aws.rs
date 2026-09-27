use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::statuspage::{apply_component_scope, configured_strings, retain_scoped_incidents};
use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, endpoint, fetch_json, http_client,
    json_body, response_content_type, retry_after_seconds,
};
use crate::domain::{
    IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent, ProviderIncident,
    ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

#[derive(Default)]
pub struct AwsProvider;

const SERVICE_CATALOG_URL: &str =
    "https://servicedata-us-west-2-prod.s3.amazonaws.com/services.json";

#[derive(Deserialize)]
struct Service {
    service: String,
    service_name: String,
    region_id: Option<String>,
    region_name: Option<String>,
}

#[async_trait]
impl StatusProvider for AwsProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        if config.get("base_url").and_then(Value::as_str) != Some("https://health.aws.amazon.com") {
            return Err(ProviderError::Configuration(
                "AWS status must use the approved public origin".into(),
            ));
        }
        configured_strings(config, "component_ids")?;
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let url = context
            .base_url
            .join("/public/currentevents")
            .map_err(|error| ProviderError::Configuration(error.to_string()))?;
        let (root, services) = tokio::try_join!(
            fetch_events(&client, url),
            fetch_json(
                &client,
                endpoint(&context.base_url, SERVICE_CATALOG_URL)?,
                4 * 1024 * 1024,
            ),
        )?;
        let mut snapshot = parse_snapshot(root, services, &context.known_active_incidents)?;
        if context.refresh_history || !context.known_active_incidents.is_empty() {
            let history = fetch_events(
                &client,
                endpoint(
                    &context.base_url,
                    "https://history-events-us-west-2-prod.s3.amazonaws.com/historyevents.json",
                )?,
            )
            .await?;
            merge_history(&mut snapshot, history, context)?;
        }
        apply_component_scope(&mut snapshot, &context.public_config)?;
        let component_ids = snapshot
            .components
            .iter()
            .map(|component| component.upstream_id.clone())
            .collect::<HashSet<_>>();
        retain_scoped_incidents(&mut snapshot.incidents, &component_ids);
        if let Some(events) = snapshot.raw_payload.as_ref().and_then(Value::as_array) {
            for incident in &mut snapshot.incidents {
                if let Some(event) = events.iter().find(|event| {
                    event.get("arn").and_then(Value::as_str) == Some(incident.upstream_id.as_str())
                }) {
                    apply_service_recovery(incident, event, &component_ids);
                }
            }
        }
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

async fn fetch_events(client: &reqwest::Client, url: url::Url) -> Result<Value, ProviderError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;
    if response.status().as_u16() == 429 {
        return Err(ProviderError::RateLimited {
            retry_after_seconds: retry_after_seconds(response.headers()),
            content_type: response_content_type(response.headers()),
        });
    }
    if !response.status().is_success() {
        return Err(ProviderError::Http {
            status: response.status().as_u16(),
            content_type: response_content_type(response.headers()),
        });
    }
    let body = json_body(response, 4 * 1024 * 1024).await?;
    let body = decode_utf16(&body)?;
    serde_json::from_str::<Value>(&body).map_err(|error| ProviderError::Parse(error.to_string()))
}

fn merge_history(
    snapshot: &mut ProviderSnapshot,
    history: Value,
    context: &FetchContext,
) -> Result<(), ProviderError> {
    let services = history.as_object().ok_or_else(|| {
        ProviderError::Parse("AWS history did not contain service collections".into())
    })?;
    let mut seen = snapshot
        .incidents
        .iter()
        .map(|incident| incident.upstream_id.clone())
        .collect::<HashSet<_>>();
    for (service, events) in services {
        let events = events.as_array().ok_or_else(|| {
            ProviderError::Parse("AWS history service collection was not an array".into())
        })?;
        for event in events {
            let mut event = event.clone();
            let object = event.as_object_mut().ok_or_else(|| {
                ProviderError::Parse("AWS history event was not an object".into())
            })?;
            object
                .entry("service")
                .or_insert_with(|| Value::String(service.clone()));
            let incident = parse_incident(&event)
                .ok_or_else(|| ProviderError::Parse("AWS history event omitted its ID".into()))?;
            if (!context.refresh_history
                && !context
                    .known_active_incidents
                    .contains(&incident.upstream_id))
                || !seen.insert(incident.upstream_id.clone())
            {
                continue;
            }
            snapshot.incidents.push(incident);
            if let Some(raw) = snapshot.raw_payload.as_mut().and_then(Value::as_array_mut) {
                raw.push(event);
            }
        }
    }
    Ok(())
}

fn apply_service_recovery(incident: &mut ProviderIncident, event: &Value, scope: &HashSet<String>) {
    if incident.lifecycle == IncidentLifecycle::Resolved {
        return;
    }
    let Some(services) = event.get("impacted_services").and_then(Value::as_object) else {
        return;
    };
    let selected = services
        .iter()
        .filter(|(id, _)| scope.contains(id.as_str()))
        .collect::<Vec<_>>();
    if selected.is_empty()
        || selected
            .iter()
            .any(|(_, service)| status_code(service.get("current")) != Some(0))
    {
        return;
    }
    // A regional event can remain open after all configured global services recover.
    // Keep its history, but do not extend scoped impact until regional recovery.
    incident.lifecycle = IncidentLifecycle::Resolved;
    incident.original_phase = "resolved".into();
    incident.metadata["upstream_status"] = event.get("status").cloned().unwrap_or(Value::Null);
    incident.metadata["timing_scope"] = Value::String("configured_services".into());
    let changes = event
        .get("impacted_service_status_changes")
        .and_then(Value::as_array);
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (id, _) in &selected {
        let service_changes = changes
            .into_iter()
            .flatten()
            .filter(|change| change.get("service").and_then(Value::as_str) == Some(id.as_str()));
        let start = service_changes
            .clone()
            .filter(|change| matches!(status_code(change.get("current_status")), Some(1..=3)))
            .filter_map(|change| {
                change
                    .get("timestamp")
                    .and_then(Value::as_i64)
                    .and_then(DateTime::from_timestamp_millis)
            })
            .min();
        let end = service_changes
            .filter(|change| status_code(change.get("current_status")) == Some(0))
            .filter_map(|change| {
                change
                    .get("timestamp")
                    .and_then(Value::as_i64)
                    .and_then(DateTime::from_timestamp_millis)
            })
            .max();
        if let (Some(start), Some(end)) = (start, end) {
            starts.push(start);
            ends.push(end);
        }
    }
    incident.started_at = starts.into_iter().min();
    incident.resolved_at = if ends.len() == selected.len() {
        ends.into_iter().max()
    } else {
        None
    };
    incident.severity = match selected
        .iter()
        .filter_map(|(_, service)| status_code(service.get("max")))
        .max()
    {
        Some(3) => Severity::Critical,
        Some(1 | 2) => Severity::Minor,
        _ => Severity::Info,
    };
}

fn decode_utf16(body: &[u8]) -> Result<String, ProviderError> {
    let (little_endian, body) = match body.get(..2) {
        Some([0xff, 0xfe]) => (true, &body[2..]),
        Some([0xfe, 0xff]) => (false, &body[2..]),
        _ => {
            return String::from_utf8(body.to_vec())
                .map_err(|error| ProviderError::Parse(error.to_string()));
        }
    };
    if body.len() % 2 != 0 {
        return Err(ProviderError::Parse(
            "AWS response contained truncated UTF-16".into(),
        ));
    }
    let units = body.chunks_exact(2).map(|bytes| {
        if little_endian {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        }
    });
    char::decode_utf16(units)
        .map(|value| value.map_err(|error| ProviderError::Parse(error.to_string())))
        .collect()
}

fn parse_snapshot(
    root: Value,
    services: Value,
    known_active: &HashSet<String>,
) -> Result<ProviderSnapshot, ProviderError> {
    let events = root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("AWS response did not contain events".into()))?;
    let active = events
        .iter()
        .filter(|event| status_code(event.get("status")) != Some(0))
        .collect::<Vec<_>>();
    let services: Vec<Service> = serde_json::from_value(services)
        .map_err(|error| ProviderError::Parse(format!("invalid AWS service catalog: {error}")))?;
    let mut seen = HashSet::new();
    let mut components = Vec::new();
    for service in services {
        if service.service.trim().is_empty()
            || service.service_name.trim().is_empty()
            || !seen.insert(service.service.clone())
            || service
                .region_id
                .as_ref()
                .is_some_and(|region| region.is_empty())
            || (service.region_id.is_none() && service.region_name.is_some())
        {
            return Err(ProviderError::Parse(
                "AWS service catalog contains missing, duplicate, or ambiguous identifiers".into(),
            ));
        }
        if service
            .region_id
            .as_ref()
            .is_some_and(|region| !region.starts_with("us-"))
        {
            continue;
        }
        let status = active
            .iter()
            .filter_map(|event| service_status(event, &service.service))
            .max_by_key(|status| status.rank())
            .unwrap_or(NormalizedStatus::Operational);
        let group = service.region_id.map_or_else(
            || "Global".to_owned(),
            |region| {
                service
                    .region_name
                    .map_or_else(|| region.clone(), |name| format!("{name} ({region})"))
            },
        );
        components.push(ProviderComponent {
            upstream_id: service.service,
            name: service.service_name,
            group: Some(group),
            description: None,
            status,
            original_status: status.key().to_owned(),
            position: i32::try_from(components.len()).unwrap_or(i32::MAX),
        });
    }
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "AWS service catalog contains no US/global services".into(),
        ));
    }
    let mut incidents = events
        .iter()
        .filter(|event| {
            status_code(event.get("status")) != Some(0)
                || event
                    .get("arn")
                    .and_then(Value::as_str)
                    .is_some_and(|arn| known_active.contains(arn))
        })
        .map(|value| {
            parse_incident(value)
                .ok_or_else(|| ProviderError::Parse("aws incident omitted its ID".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    let overall = rollup(components.iter().map(|component| component.status));
    Ok(ProviderSnapshot {
        observed_at: Utc::now(),
        overall,
        original_overall: overall.key().to_owned(),
        provider_overall: overall,
        provider_original_overall: overall.key().to_owned(),
        components,
        incidents,
        maintenance: Vec::new(),
        active_incident_set_complete: true,
        response: ResponseMetadata {
            status: 200,
            etag: None,
            last_modified: None,
            content_type: Some("application/json;charset=utf-16".into()),
        },
        raw_payload: Some(root),
    })
}

// Public dashboard codes: 0 resolved, 1 impacted, 2 degraded, 3 disrupted.
fn status_code(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

fn aws_status(code: Option<i64>) -> NormalizedStatus {
    match code {
        Some(0) => NormalizedStatus::Operational,
        Some(1 | 2) => NormalizedStatus::Degraded,
        Some(3) => NormalizedStatus::MajorOutage,
        _ => NormalizedStatus::Unknown,
    }
}

fn service_status(event: &Value, service_id: &str) -> Option<NormalizedStatus> {
    if let Some(service) = event
        .get("impacted_services")
        .and_then(Value::as_object)
        .and_then(|services| services.get(service_id))
    {
        return Some(aws_status(status_code(service.get("current"))));
    }
    (event.get("service").and_then(Value::as_str) == Some(service_id))
        .then(|| aws_status(status_code(event.get("status"))))
}

fn parse_incident(value: &Value) -> Option<ProviderIncident> {
    let upstream_id = value.get("arn")?.as_str()?.to_owned();
    // Archive status records peak impact; the latest structured update records recovery.
    let resolved = status_code(value.get("status")) == Some(0)
        || value
            .get("event_log")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|update| epoch_time(update.get("timestamp")).is_some())
            .max_by_key(|update| epoch_time(update.get("timestamp")))
            .is_some_and(|update| status_code(update.get("status")) == Some(0));
    let lifecycle = if resolved {
        IncidentLifecycle::Resolved
    } else {
        IncidentLifecycle::Open
    };
    let mut updates = value
        .get("event_log")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(position, update)| ProviderUpdate {
            upstream_id: Some(format!("{upstream_id}:{position}")),
            status: update
                .get("status")
                .map_or_else(|| "unknown".into(), Value::to_string),
            body: update
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at: epoch_time(update.get("timestamp")),
            updated_at: epoch_time(update.get("timestamp")),
            display_at: None,
            affected_scopes: Vec::new(),
        })
        .collect::<Vec<_>>();
    updates.sort_by_key(|update| update.created_at);
    let affected_components = value
        .get("impacted_services")
        .and_then(Value::as_object)
        .map_or_else(
            || {
                value
                    .get("service")
                    .and_then(Value::as_str)
                    .map(|service| vec![service.to_owned()])
                    .unwrap_or_default()
            },
            |services| services.keys().cloned().collect(),
        );
    let original_impact = value
        .get("summary")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Some(ProviderIncident {
        upstream_id: upstream_id.clone(),
        kind: IncidentKind::Incident,
        title: format!(
            "{} — {}",
            value
                .get("service_name")
                .and_then(Value::as_str)
                .unwrap_or("AWS service event"),
            value
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("Operational issue")
        ),
        url: Some(format!(
            "https://health.aws.amazon.com/health/status?eventID={}",
            url::form_urlencoded::byte_serialize(upstream_id.as_bytes()).collect::<String>()
        )),
        lifecycle,
        original_phase: if resolved { "resolved" } else { "open" }.into(),
        severity: match value
            .get("event_log")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|update| status_code(update.get("status")))
            .chain(status_code(value.get("status")))
            .max()
        {
            Some(3) => Severity::Critical,
            Some(1 | 2) => Severity::Minor,
            _ => Severity::Info,
        },
        original_impact,
        created_at: epoch_time(value.get("date")),
        started_at: epoch_time(value.get("date")),
        updated_at: updates.last().and_then(|update| update.updated_at),
        monitoring_at: None,
        resolved_at: resolved
            .then(|| updates.last().and_then(|update| update.updated_at))
            .flatten(),
        planned_start_at: None,
        planned_end_at: None,
        affected_components,
        affected_scopes: Vec::new(),
        within_provider_scope: true,
        metadata: serde_json::json!({}),
        updates,
    })
}

fn epoch_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let seconds = value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })?;
    DateTime::from_timestamp(seconds, 0)
}
