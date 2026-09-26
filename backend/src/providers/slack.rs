use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, fetch_json, http_client, json_body,
    parse_time, response_body, response_content_type, retry_after_seconds,
};
use crate::domain::{
    IncidentKind, IncidentLifecycle, ProviderComponent, ProviderIncident, ProviderSnapshot,
    ProviderUpdate, ResponseMetadata, Severity,
};

#[derive(Default)]
pub struct SlackProvider;

#[async_trait]
impl StatusProvider for SlackProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        let base_url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("base_url is required".into()))?;
        let parsed = url::Url::parse(base_url)
            .map_err(|_| ProviderError::Configuration("base_url must be a URL".into()))?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ProviderError::Configuration(
                "provider URLs must be credential-free HTTPS origins".into(),
            ));
        }
        Ok(())
    }
    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let url = context
            .base_url
            .join("/api/v2.0.0/current")
            .map_err(|error| ProviderError::Configuration(error.to_string()))?;
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
        let body = json_body(response, 2 * 1024 * 1024).await?;
        let FetchOutcome::Fetched(mut snapshot) = parse_current(&body)? else {
            return Err(ProviderError::Parse(
                "current parser returned an unexpected response".into(),
            ));
        };
        let missing = context
            .known_active_incidents
            .iter()
            .filter(|id| {
                !snapshot
                    .incidents
                    .iter()
                    .any(|incident| &incident.upstream_id == *id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if context.refresh_history || !missing.is_empty() {
            let history = fetch_missing_incidents(
                &client,
                &context.base_url,
                &missing,
                context.refresh_history,
            )
            .await?;
            for incident in history {
                if !snapshot
                    .incidents
                    .iter()
                    .any(|current| current.upstream_id == incident.upstream_id)
                {
                    snapshot.incidents.push(incident);
                }
            }
        }
        enrich_resolution_notes(&client, context, &mut snapshot).await?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

// The JSON feed omits note phases. The public detail page identifies the resolved
// note explicitly; its publication timestamp is a bound, not a claimed impact end.
async fn enrich_resolution_notes(
    client: &reqwest::Client,
    context: &FetchContext,
    snapshot: &mut ProviderSnapshot,
) -> Result<(), ProviderError> {
    const DETAIL_CONCURRENCY: usize = 4;
    let mut pending = tokio::task::JoinSet::new();
    let mut requests = Vec::new();
    for (index, incident) in snapshot.incidents.iter().enumerate() {
        if incident.lifecycle != IncidentLifecycle::Resolved || incident.resolved_at.is_some() {
            continue;
        }
        let Some(url) = incident
            .url
            .as_deref()
            .and_then(|url| url::Url::parse(url).ok())
        else {
            continue;
        };
        if url.origin() != context.base_url.origin()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            continue;
        }
        requests.push((index, url));
    }
    let mut requests = requests.into_iter();
    loop {
        while pending.len() < DETAIL_CONCURRENCY {
            let Some((index, url)) = requests.next() else {
                break;
            };
            let client = client.clone();
            pending.spawn(async move {
                Ok::<_, ProviderError>((index, resolution_notes(&client, url).await?))
            });
        }
        let Some(result) = pending.join_next().await else {
            break;
        };
        let (index, times) = result.map_err(|error| {
            ProviderError::Transport(format!("Slack detail task failed: {error}"))
        })??;
        if let Some(incident) = snapshot.incidents.get_mut(index) {
            apply_resolution_notes(incident, &times);
        }
    }
    Ok(())
}

fn apply_resolution_notes(incident: &mut ProviderIncident, times: &[DateTime<Utc>]) {
    for update in &mut incident.updates {
        if update.created_at.is_some_and(|time| times.contains(&time)) {
            update.status = "resolved".into();
        }
    }
    incident.resolved_at = incident
        .updates
        .iter()
        .filter(|update| update.status == "resolved")
        .filter_map(|update| update.created_at)
        .filter(|time| incident.created_at.is_none_or(|start| *time >= start))
        .min();
    if incident.resolved_at.is_some() {
        incident.metadata["resolution_time_basis"] =
            Value::String("resolved_note_publication".into());
    }
}

async fn resolution_notes(
    client: &reqwest::Client,
    url: url::Url,
) -> Result<Vec<DateTime<Utc>>, ProviderError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;
    if response.status().as_u16() == 404 {
        return Ok(Vec::new());
    }
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
    if !response_content_type(response.headers())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return Err(ProviderError::Parse(
            "Slack detail response was not HTML".into(),
        ));
    }
    let body = response_body(response, 1024 * 1024).await?;
    let body = String::from_utf8(body).map_err(|error| ProviderError::Parse(error.to_string()))?;
    Ok(body
        .split("<div class=\"note\">")
        .skip(1)
        .filter(|note| {
            note.split("<p>")
                .next()
                .is_some_and(|header| header.contains("class=\"incident_done_icon\""))
        })
        .filter_map(|note| {
            note.split_once("data-timestamp=\"")?
                .1
                .split_once('"')?
                .0
                .parse::<i64>()
                .ok()
        })
        .filter_map(|seconds| DateTime::from_timestamp(seconds, 0))
        .collect())
}

const SERVICES: &[&str] = &[
    "Login/SSO",
    "Messaging",
    "Notifications",
    "Search",
    "Workspace/Org Administration",
    "Canvases",
    "Connectivity",
    "Files",
    "Huddles",
    "Apps/Integrations/APIs",
    "Workflows",
];

async fn fetch_missing_incidents(
    client: &reqwest::Client,
    base_url: &url::Url,
    missing: &[String],
    refresh_history: bool,
) -> Result<Vec<ProviderIncident>, ProviderError> {
    let url = base_url
        .join("/api/v2.0.0/history")
        .map_err(|error| ProviderError::Configuration(error.to_string()))?;
    let root = fetch_json(client, url, 4 * 1024 * 1024).await?;
    Ok(root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Slack history response must be an array".into()))?
        .iter()
        .map(|value| {
            parse_incident(value)
                .ok_or_else(|| ProviderError::Parse("Slack incident omitted its ID".into()))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|incident| {
            matches!(incident.lifecycle, IncidentLifecycle::Resolved)
                && (missing.contains(&incident.upstream_id) || refresh_history)
        })
        .map(|mut incident| {
            incident.metadata["_statusdeck_history"] = Value::Bool(true);
            incident
        })
        .collect())
}

pub fn parse_current(body: &[u8]) -> Result<FetchOutcome, ProviderError> {
    let root: Value =
        serde_json::from_slice(body).map_err(|error| ProviderError::Parse(error.to_string()))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let mut incidents = root
        .get("active_incidents")
        .or_else(|| root.get("incidents"))
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("Slack response omitted incidents".into()))?
        .iter()
        .map(|value| {
            parse_incident(value)
                .ok_or_else(|| ProviderError::Parse("Slack incident omitted its ID".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    let overall = incidents
        .iter()
        .filter(|incident| affects_current_status(incident))
        .map(|incident| status_for_incident(incident.original_impact.as_deref()))
        .max_by_key(|value| value.rank())
        .unwrap_or_else(|| {
            if incidents.is_empty() {
                crate::domain::NormalizedStatus::from_provider(&status)
            } else {
                crate::domain::NormalizedStatus::Operational
            }
        });
    let reported_services = root
        .get("services")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|service| {
            Ok((
                service
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| ProviderError::Parse("Slack service omitted its name".into()))?
                    .to_owned(),
                service
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let services = if reported_services.is_empty() {
        SERVICES
            .iter()
            .map(|name| ((*name).to_owned(), "operational".to_owned()))
            .collect::<Vec<_>>()
    } else {
        reported_services
    };
    let services = services
        .iter()
        .enumerate()
        .map(|(position, (name, reported_status))| {
            let component_status = std::iter::once(crate::domain::NormalizedStatus::from_provider(
                reported_status,
            ))
            .chain(
                incidents
                    .iter()
                    .filter(|incident| affects_current_status(incident))
                    .filter(|incident| {
                        incident
                            .affected_components
                            .iter()
                            .any(|id| id == &stable_service_id(name))
                    })
                    .map(|incident| status_for_incident(incident.original_impact.as_deref())),
            )
            .max_by_key(|value| value.rank())
            .unwrap_or(crate::domain::NormalizedStatus::Operational);
            ProviderComponent {
                upstream_id: stable_service_id(name),
                name: name.clone(),
                group: None,
                description: None,
                status: component_status,
                original_status: component_status.key().to_owned(),
                position: i32::try_from(position).unwrap_or(i32::MAX),
            }
        })
        .collect::<Vec<_>>();
    Ok(FetchOutcome::Fetched(ProviderSnapshot {
        observed_at: Utc::now(),
        overall,
        original_overall: status.clone(),
        provider_overall: overall,
        provider_original_overall: status,
        components: services,
        incidents,
        maintenance: Vec::new(),
        active_incident_set_complete: true,
        response: ResponseMetadata {
            status: 200,
            etag: None,
            last_modified: None,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(root),
    }))
}

fn parse_incident(value: &Value) -> Option<ProviderIncident> {
    let id = value_id(value.get("id").or_else(|| value.get("incident_id")))?;
    let phase = value
        .get("status")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("incident")
        .to_owned();
    let mut updates = value
        .get("notes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|note| {
            let created_at =
                parse_time(note.get("date_created").or_else(|| note.get("created_at")));
            let body = note
                .get("body")
                .or_else(|| note.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            ProviderUpdate {
                upstream_id: value_id(note.get("id"))
                    .or_else(|| created_at.as_ref().map(chrono::DateTime::to_rfc3339)),
                status: note
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("update")
                    .to_owned(),
                body,
                created_at,
                updated_at: parse_time(note.get("date_updated").or_else(|| note.get("updated_at"))),
                display_at: None,
                affected_scopes: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    updates.sort_by_key(|update| update.created_at);
    let mut affected_components = value
        .get("services")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|service| service.as_str().map(stable_service_id))
        .collect::<Vec<_>>();
    affected_components.sort();
    affected_components.dedup();
    let kind = if phase.eq_ignore_ascii_case("scheduled")
        || phase.eq_ignore_ascii_case("completed")
        || phase.eq_ignore_ascii_case("cancelled")
        || value.get("type").and_then(Value::as_str) == Some("maintenance")
    {
        IncidentKind::Maintenance
    } else {
        IncidentKind::Incident
    };
    Some(ProviderIncident {
        upstream_id: id,
        kind,
        title: value
            .get("title")
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("Slack incident")
            .to_owned(),
        url: value
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        lifecycle: if phase.eq_ignore_ascii_case("resolved")
            || phase.eq_ignore_ascii_case("completed")
            || phase.eq_ignore_ascii_case("cancelled")
        {
            IncidentLifecycle::Resolved
        } else {
            IncidentLifecycle::Open
        },
        original_phase: phase,
        severity: Severity::from_impact(
            value
                .get("severity")
                .or_else(|| value.get("type"))
                .and_then(Value::as_str),
        ),
        original_impact: value
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        created_at: parse_time(
            value
                .get("date_created")
                .or_else(|| value.get("created_at")),
        ),
        started_at: None,
        updated_at: parse_time(
            value
                .get("date_updated")
                .or_else(|| value.get("updated_at")),
        ),
        monitoring_at: None,
        resolved_at: parse_time(
            value
                .get("date_resolved")
                .or_else(|| value.get("resolved_at")),
        ),
        planned_start_at: None,
        planned_end_at: None,
        affected_components,
        affected_scopes: Vec::new(),
        within_provider_scope: true,
        metadata: serde_json::json!({}),
        updates,
    })
}

fn value_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn status_for_incident(original: Option<&str>) -> crate::domain::NormalizedStatus {
    match original.unwrap_or_default() {
        "outage" => crate::domain::NormalizedStatus::MajorOutage,
        "incident" => crate::domain::NormalizedStatus::PartialOutage,
        "notice" => crate::domain::NormalizedStatus::Degraded,
        "maintenance" => crate::domain::NormalizedStatus::Maintenance,
        other => crate::domain::NormalizedStatus::from_provider(other),
    }
}

fn affects_current_status(incident: &ProviderIncident) -> bool {
    incident.lifecycle != IncidentLifecycle::Resolved
        && !incident.original_phase.eq_ignore_ascii_case("scheduled")
}

fn stable_service_id(name: &str) -> String {
    format!(
        "slack-{}",
        name.trim()
            .to_ascii_lowercase()
            .replace([' ', '/', '.'], "-")
    )
}
