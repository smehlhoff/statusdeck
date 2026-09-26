use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, conditional_request,
    conditional_response_metadata, http_client, json_body, parse_time, response_content_type,
    retry_after_seconds,
};
use crate::domain::{
    AffectedScope, IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent,
    ProviderIncident, ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

use super::statuspage::{apply_component_scope, configured_strings};

#[derive(Default)]
pub struct StatusIoProvider;

#[async_trait]
impl StatusProvider for StatusIoProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        let base_url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("base_url is required".into()))?;
        let base_url = url::Url::parse(base_url)
            .map_err(|_| ProviderError::Configuration("base_url must be a URL".into()))?;
        if base_url.scheme() != "https"
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(ProviderError::Configuration(
                "provider URLs must be credential-free HTTPS origins".into(),
            ));
        }
        let page_id = config
            .get("page_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("page_id is required".into()))?;
        if page_id.is_empty() || !page_id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(ProviderError::Configuration(
                "page_id must contain only ASCII letters and numbers".into(),
            ));
        }
        configured_strings(config, "component_ids")?;
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let page_id = context.public_config["page_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Configuration("page_id is required".into()))?;
        let url = context
            .base_url
            .join(&format!("/1.0/status/{page_id}"))
            .map_err(|error| ProviderError::Configuration(error.to_string()))?;
        let client = http_client(context)?;
        let response = conditional_request(client.get(url), context)
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let metadata = conditional_response_metadata(&response);
        if metadata.status == 304 {
            return Ok(FetchOutcome::NotModified {
                status: metadata.status,
                etag: metadata.etag,
                last_modified: metadata.last_modified,
            });
        }
        if metadata.status == 429 {
            return Err(ProviderError::RateLimited {
                retry_after_seconds: retry_after_seconds(response.headers()),
                content_type: response_content_type(response.headers()),
            });
        }
        if !response.status().is_success() {
            return Err(ProviderError::Http {
                status: metadata.status,
                content_type: response_content_type(response.headers()),
            });
        }
        let body = json_body(response, 4 * 1024 * 1024).await?;
        let mut snapshot = parse_snapshot(
            &body,
            metadata.status,
            metadata.etag,
            metadata.last_modified,
        )?;
        apply_component_scope(&mut snapshot, &context.public_config)?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

fn parse_snapshot(
    body: &[u8],
    status: u16,
    etag: Option<String>,
    last_modified: Option<String>,
) -> Result<ProviderSnapshot, ProviderError> {
    let root = serde_json::from_slice::<Value>(body)
        .map_err(|error| ProviderError::Parse(error.to_string()))?;
    let result = root
        .get("result")
        .ok_or_else(|| ProviderError::Parse("Status.io response omitted result".into()))?;
    for path in ["/incidents", "/maintenance/active", "/maintenance/upcoming"] {
        let values = result
            .pointer(path)
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Parse(format!("Status.io response omitted {path}")))?;
        if values.iter().any(|value| {
            value
                .get("_id")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        }) {
            return Err(ProviderError::Parse(
                "Status.io event omitted its ID".into(),
            ));
        }
    }
    let components = result
        .get("status")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("Status.io response omitted status".into()))?
        .iter()
        .enumerate()
        .map(|(position, component)| {
            let upstream_id = component
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ProviderError::Parse("Status.io component omitted its ID".into()))?
                .to_owned();
            let original_status = component
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_owned();
            Ok(ProviderComponent {
                upstream_id,
                name: component
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed component")
                    .to_owned(),
                group: None,
                description: None,
                status: statusio_status(component.get("status_code"), &original_status),
                original_status,
                position: i32::try_from(position).unwrap_or(i32::MAX),
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "Status.io response contained no components".into(),
        ));
    }
    let incidents = parse_events(
        result.get("incidents").and_then(Value::as_array),
        IncidentKind::Incident,
    );
    let maintenance_root = result.get("maintenance");
    let maintenance = ["active", "upcoming"]
        .into_iter()
        .flat_map(|key| {
            parse_events(
                maintenance_root
                    .and_then(|value| value.get(key))
                    .and_then(Value::as_array),
                IncidentKind::Maintenance,
            )
        })
        .collect::<Vec<_>>();
    let original_overall = result
        .get("status_overall")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_owned();
    let overall = result
        .get("status_overall")
        .and_then(|value| value.get("status_code"))
        .map_or_else(
            || rollup(components.iter().map(|component| component.status)),
            |code| statusio_status(Some(code), &original_overall),
        );
    Ok(ProviderSnapshot {
        observed_at: Utc::now(),
        overall,
        original_overall: original_overall.clone(),
        provider_overall: overall,
        provider_original_overall: original_overall,
        components,
        incidents,
        maintenance,
        active_incident_set_complete: true,
        response: ResponseMetadata {
            status,
            etag,
            last_modified,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(root),
    })
}

fn statusio_status(code: Option<&Value>, label: &str) -> NormalizedStatus {
    match code.and_then(Value::as_i64) {
        Some(100) => NormalizedStatus::Operational,
        Some(200) => NormalizedStatus::Maintenance,
        Some(300) => NormalizedStatus::Degraded,
        Some(400) => NormalizedStatus::PartialOutage,
        Some(500 | 600) => NormalizedStatus::MajorOutage,
        _ => NormalizedStatus::from_provider(label),
    }
}

fn parse_events(values: Option<&Vec<Value>>, kind: IncidentKind) -> Vec<ProviderIncident> {
    let mut events = values
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let upstream_id = event
                .get("_id")
                .or_else(|| event.get("id"))?
                .as_str()?
                .to_owned();
            let mut messages = event
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages.sort_by_key(|message| parse_time(message.get("datetime")));
            let latest = messages.last();
            // A final operational update must not erase the incident's reported impact.
            let impact_code = messages
                .iter()
                .filter_map(|message| message.get("status").and_then(Value::as_i64))
                .max();
            let original_phase = latest
                .map(|message| event_phase(message, kind))
                .unwrap_or_else(|| {
                    if kind == IncidentKind::Maintenance {
                        "scheduled"
                    } else {
                        "unknown"
                    }
                })
                .to_owned();
            let lifecycle = match original_phase.to_ascii_lowercase().as_str() {
                "resolved" | "completed" => IncidentLifecycle::Resolved,
                "monitoring" => IncidentLifecycle::ResolutionPending,
                _ => IncidentLifecycle::Open,
            };
            let mut affected_components = event
                .get("components_affected")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(affected_id)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            affected_components.sort();
            affected_components.dedup();
            let updates = messages
                .iter()
                .map(|message| ProviderUpdate {
                    upstream_id: message
                        .get("_id")
                        .or_else(|| message.get("id"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    status: event_phase(message, kind).to_owned(),
                    body: message
                        .get("details")
                        .or_else(|| message.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    created_at: parse_time(message.get("datetime")),
                    updated_at: parse_time(message.get("datetime")),
                    display_at: None,
                    affected_scopes: Vec::new(),
                })
                .collect();
            Some(ProviderIncident {
                upstream_id,
                kind,
                title: event
                    .get("name")
                    .or_else(|| event.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Status.io event")
                    .to_owned(),
                url: None,
                lifecycle,
                original_phase,
                severity: match impact_code {
                    Some(300) => Severity::Minor,
                    Some(400) => Severity::Major,
                    Some(500 | 600) => Severity::Critical,
                    _ => Severity::from_impact(event.get("impact").and_then(Value::as_str)),
                },
                original_impact: impact_code.map(|code| code.to_string()).or_else(|| {
                    event
                        .get("impact")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
                created_at: parse_time(
                    event
                        .get("datetime_open")
                        .or_else(|| event.get("datetime_planned_start")),
                ),
                started_at: match kind {
                    IncidentKind::Incident => parse_time(event.get("datetime_open")),
                    // Opening a maintenance record announces it; an active update confirms work.
                    IncidentKind::Maintenance => messages
                        .iter()
                        .filter(|message| event_phase(message, kind) == "in_progress")
                        .find_map(|message| parse_time(message.get("datetime"))),
                },
                updated_at: latest.and_then(|message| parse_time(message.get("datetime"))),
                monitoring_at: None,
                resolved_at: (lifecycle == IncidentLifecycle::Resolved)
                    .then(|| latest.and_then(|message| parse_time(message.get("datetime"))))
                    .flatten(),
                planned_start_at: (kind == IncidentKind::Maintenance)
                    .then(|| parse_time(event.get("datetime_planned_start")))
                    .flatten(),
                planned_end_at: (kind == IncidentKind::Maintenance)
                    .then(|| parse_time(event.get("datetime_planned_end")))
                    .flatten(),
                affected_components,
                affected_scopes: event
                    .get("containers_affected")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|value| {
                        Some(AffectedScope {
                            scope_type: "container".into(),
                            upstream_id: affected_id(value)?.to_owned(),
                            display_name: value
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or(affected_id(value)?)
                                .to_owned(),
                            normalized_status: None,
                            original_status: None,
                        })
                    })
                    .collect(),
                within_provider_scope: true,
                metadata: serde_json::json!({}),
                updates,
            })
        })
        .collect::<Vec<_>>();
    events.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    events
}

fn affected_id(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .get("_id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
    })
}

fn event_phase(message: &Value, kind: IncidentKind) -> &str {
    let state = message.get("state");
    let code = state
        .and_then(Value::as_i64)
        .or_else(|| state.and_then(Value::as_str)?.parse().ok());
    match (kind, code) {
        (IncidentKind::Incident, Some(100)) => "investigating",
        (IncidentKind::Incident, Some(200)) => "identified",
        (IncidentKind::Incident, Some(300)) => "monitoring",
        (IncidentKind::Incident, Some(400)) => "resolved",
        (IncidentKind::Maintenance, Some(100)) => "scheduled",
        (IncidentKind::Maintenance, Some(200)) => "in_progress",
        (IncidentKind::Maintenance, Some(300)) => "completed",
        _ => state.and_then(Value::as_str).unwrap_or("unknown"),
    }
}
