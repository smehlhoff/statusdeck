use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, conditional_request,
    conditional_response_metadata, endpoint, fetch_json, http_client, json_body, parse_time,
    response_content_type, retry_after_seconds,
};
use crate::domain::{
    AffectedScope, IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent,
    ProviderIncident, ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity,
};

#[derive(Default)]
pub struct StatuspageProvider;

#[async_trait]
impl StatusProvider for StatuspageProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        let url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("base_url is required".into()))?;
        let parsed = url::Url::parse(url)
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
        component_scope(config)?;
        configured_strings(config, "excluded_incident_name_prefixes")?;
        configured_path(config, "summary_path")?;
        configured_path(config, "components_path")?;
        configured_path(config, "maintenance_path")?;
        configured_path(config, "incidents_path")?;
        configured_path(config, "native_incidents_path")?;
        if config
            .get("separate_incidents_endpoint")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(ProviderError::Configuration(
                "separate_incidents_endpoint must be a boolean".into(),
            ));
        }
        Ok(())
    }
    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let summary_path = configured_path(&context.public_config, "summary_path")?
            .unwrap_or("/api/v2/summary.json");
        let incidents_path = configured_path(&context.public_config, "incidents_path")?
            .unwrap_or("/api/v2/incidents.json");
        let components_path = configured_path(&context.public_config, "components_path")?;
        let maintenance_path = configured_path(&context.public_config, "maintenance_path")?;
        let request = client.get(
            context
                .base_url
                .join(summary_path)
                .map_err(|error| ProviderError::Configuration(error.to_string()))?,
        );
        let separate_incidents = context
            .public_config
            .get("separate_incidents_endpoint")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let request = if separate_incidents || components_path.is_some() || context.refresh_history
        {
            request
        } else {
            conditional_request(request, context)
        };
        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let metadata = conditional_response_metadata(&response);
        if response.status().as_u16() == 304 {
            return Ok(FetchOutcome::NotModified {
                status: 304,
                etag: metadata.etag,
                last_modified: metadata.last_modified,
            });
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
        let body = json_body(response, 2 * 1024 * 1024).await?;
        let body = if separate_incidents || context.refresh_history {
            merge_event_history(
                &client,
                &context.base_url,
                incidents_path,
                &body,
                (!separate_incidents)
                    .then(|| Utc::now() - Duration::days(super::HISTORY_LOOKBACK_DAYS)),
                IncidentKind::Incident,
            )
            .await?
        } else {
            body
        };
        let body = if let Some(path) = maintenance_path.filter(|_| context.refresh_history) {
            merge_event_history(
                &client,
                &context.base_url,
                path,
                &body,
                Some(Utc::now() - Duration::days(super::HISTORY_LOOKBACK_DAYS)),
                IncidentKind::Maintenance,
            )
            .await?
        } else {
            body
        };
        let body = if let Some(path) = components_path {
            // Some compatible summaries omit supported components that are still
            // published by the dedicated component endpoint.
            let mut components =
                fetch_json(&client, endpoint(&context.base_url, path)?, 2 * 1024 * 1024).await?;
            let values = components
                .get_mut("components")
                .filter(|value| value.is_array())
                .ok_or_else(|| {
                    ProviderError::Parse("component response omitted components".into())
                })?
                .take();
            let mut summary: Value = serde_json::from_slice(&body)
                .map_err(|error| ProviderError::Parse(error.to_string()))?;
            summary
                .as_object_mut()
                .ok_or_else(|| ProviderError::Parse("summary was not an object".into()))?
                .insert("components".into(), values);
            serde_json::to_vec(&summary).map_err(|error| ProviderError::Parse(error.to_string()))?
        } else {
            body
        };
        let page_url = incident_page_url(&body);
        let parsed = parse_summary(
            &body,
            metadata.status,
            metadata.etag,
            metadata.last_modified,
        )?;
        let FetchOutcome::Fetched(mut snapshot) = parsed else {
            return Err(ProviderError::Parse(
                "summary parser returned an unexpected response".into(),
            ));
        };
        if let Some(native_path) = configured_path(&context.public_config, "native_incidents_path")?
        {
            let native = fetch_json(
                &client,
                endpoint(&context.base_url, native_path)?,
                4 * 1024 * 1024,
            )
            .await?;
            enrich_incidentio_incidents(&mut snapshot, &native);
            if let Some(raw_payload) = snapshot.raw_payload.as_mut().and_then(Value::as_object_mut)
            {
                raw_payload.insert("native_incidents".into(), native);
            }
        }
        if separate_incidents {
            // Incident.io-compatible history endpoints are bounded recent history,
            // not a contractual complete active set. Require explicit resolution.
            snapshot.active_incident_set_complete = false;
        }
        let missing = context
            .known_active_incidents
            .iter()
            .filter(|id| {
                !snapshot
                    .incidents
                    .iter()
                    .chain(snapshot.maintenance.iter())
                    .any(|incident| &incident.upstream_id == *id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            snapshot.incidents.extend(
                fetch_missing_events(
                    &client,
                    &context.base_url,
                    incidents_path,
                    page_url.as_ref(),
                    &missing,
                    IncidentKind::Incident,
                )
                .await?,
            );
        }
        if let Some(path) = maintenance_path {
            let missing = missing
                .into_iter()
                .filter(|id| {
                    !snapshot
                        .incidents
                        .iter()
                        .any(|incident| incident.upstream_id == *id)
                })
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                snapshot.maintenance.extend(
                    fetch_missing_events(
                        &client,
                        &context.base_url,
                        path,
                        page_url.as_ref(),
                        &missing,
                        IncidentKind::Maintenance,
                    )
                    .await?,
                );
            }
        }
        apply_component_scope(&mut snapshot, &context.public_config)?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

pub(crate) fn enrich_incidentio_incidents(snapshot: &mut ProviderSnapshot, root: &Value) {
    let Some(native_incidents) = root.get("incidents").and_then(Value::as_array) else {
        return;
    };
    let component_names = snapshot
        .components
        .iter()
        .map(|component| (component.upstream_id.clone(), component.name.clone()))
        .collect::<HashMap<_, _>>();
    for incident in snapshot
        .incidents
        .iter_mut()
        .chain(snapshot.maintenance.iter_mut())
    {
        let Some(native) = native_incidents
            .iter()
            .find(|native| native.get("id").and_then(Value::as_str) == Some(&incident.upstream_id))
        else {
            continue;
        };
        if let Some(title) = native.get("name").and_then(Value::as_str) {
            incident.title = title.to_owned();
        }
        if let Some(phase) = native.get("status").and_then(Value::as_str) {
            incident.original_phase = phase.to_owned();
        }
        incident.created_at = parse_time(native.get("published_at")).or(incident.created_at);
        incident.affected_scopes = incidentio_scopes(
            native.get("affected_components").and_then(Value::as_array),
            &component_names,
        );
        incident.affected_components = incident
            .affected_scopes
            .iter()
            .map(|scope| scope.upstream_id.clone())
            .collect();
        if let Some(updates) = native.get("updates").and_then(Value::as_array) {
            incident.updates = updates
                .iter()
                .filter_map(|update| {
                    let created_at = parse_time(update.get("published_at"));
                    Some(ProviderUpdate {
                        upstream_id: update
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        status: update
                            .get("to_status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        body: update
                            .get("message_string")
                            .or_else(|| {
                                update
                                    .get("message")
                                    .and_then(|message| message.get("markdown"))
                            })
                            .and_then(Value::as_str)?
                            .to_owned(),
                        created_at,
                        updated_at: created_at,
                        display_at: created_at,
                        affected_scopes: incidentio_scopes(
                            update.get("component_statuses").and_then(Value::as_array),
                            &component_names,
                        ),
                    })
                })
                .collect();
            incident.updates.sort_by_key(|update| update.created_at);
            incident.updated_at = incident
                .updates
                .last()
                .and_then(|update| update.updated_at)
                .or(incident.updated_at);
            if incident.lifecycle == IncidentLifecycle::Resolved {
                incident.resolved_at = incident
                    .updates
                    .iter()
                    .rev()
                    .find(|update| matches!(update.status.as_str(), "resolved" | "completed"))
                    .and_then(|update| update.display_at.or(update.created_at))
                    .or(incident.resolved_at);
            }
        }
    }
}

fn incidentio_scopes(
    values: Option<&Vec<Value>>,
    component_names: &HashMap<String, String>,
) -> Vec<AffectedScope> {
    values
        .into_iter()
        .flatten()
        .filter_map(|component| {
            let upstream_id = component.get("component_id")?.as_str()?.to_owned();
            let original_status = component
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(AffectedScope {
                scope_type: "component".into(),
                display_name: component_names
                    .get(upstream_id.as_str())
                    .map_or(upstream_id.as_str(), String::as_str)
                    .to_owned(),
                normalized_status: original_status
                    .as_deref()
                    .map(NormalizedStatus::from_provider),
                original_status,
                upstream_id,
            })
        })
        .collect()
}

#[derive(Default)]
struct ComponentScope {
    ids: Option<HashSet<String>>,
    name_prefixes: Option<HashSet<String>>,
}

impl ComponentScope {
    fn matches(&self, component: &ProviderComponent) -> bool {
        self.ids
            .as_ref()
            .is_some_and(|ids| ids.contains(&component.upstream_id))
            || self.name_prefixes.as_ref().is_some_and(|prefixes| {
                prefixes
                    .iter()
                    .any(|prefix| component.name.starts_with(prefix))
            })
    }
}

fn component_scope(config: &Value) -> Result<Option<ComponentScope>, ProviderError> {
    let scope = ComponentScope {
        ids: configured_strings(config, "component_ids")?,
        name_prefixes: configured_strings(config, "component_name_prefixes")?,
    };
    if scope.ids.is_none() && scope.name_prefixes.is_none() {
        Ok(None)
    } else {
        Ok(Some(scope))
    }
}

pub(super) fn configured_strings(
    config: &Value,
    key: &str,
) -> Result<Option<HashSet<String>>, ProviderError> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| {
        ProviderError::Configuration(format!("{key} must be an array of strings"))
    })?;
    if values.is_empty() {
        return Err(ProviderError::Configuration(format!(
            "{key} must not be empty"
        )));
    }
    let mut configured = HashSet::with_capacity(values.len());
    for value in values {
        let item = value
            .as_str()
            .filter(|item| !item.is_empty())
            .ok_or_else(|| {
                ProviderError::Configuration(format!("{key} must contain non-empty strings"))
            })?;
        if !configured.insert(item.to_owned()) {
            return Err(ProviderError::Configuration(format!(
                "{key} must not contain duplicates"
            )));
        }
    }
    Ok(Some(configured))
}

fn configured_path<'a>(config: &'a Value, key: &str) -> Result<Option<&'a str>, ProviderError> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    let path = value.as_str().ok_or_else(|| {
        ProviderError::Configuration(format!("{key} must be an absolute URL path"))
    })?;
    if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#']) {
        return Err(ProviderError::Configuration(format!(
            "{key} must be an absolute URL path without a query or fragment"
        )));
    }
    Ok(Some(path))
}

pub(super) fn apply_component_scope(
    snapshot: &mut ProviderSnapshot,
    config: &Value,
) -> Result<(), ProviderError> {
    if let Some(prefixes) = configured_strings(config, "excluded_incident_name_prefixes")? {
        snapshot.incidents.retain(|incident| {
            !prefixes
                .iter()
                .any(|prefix| incident.title.starts_with(prefix))
        });
        snapshot.maintenance.retain(|incident| {
            !prefixes
                .iter()
                .any(|prefix| incident.title.starts_with(prefix))
        });
    }
    let Some(scope) = component_scope(config)? else {
        return Ok(());
    };
    snapshot
        .components
        .retain(|component| scope.matches(component));
    let worst = snapshot
        .components
        .iter()
        .max_by_key(|component| component.status.rank())
        .ok_or_else(|| {
            ProviderError::Parse("provider response omitted every configured component".into())
        })?;
    snapshot.overall = worst.status;
    snapshot.original_overall.clone_from(&worst.original_status);
    let component_ids = snapshot
        .components
        .iter()
        .map(|component| component.upstream_id.clone())
        .collect();
    retain_scoped_incidents(&mut snapshot.incidents, &component_ids);
    retain_scoped_incidents(&mut snapshot.maintenance, &component_ids);
    Ok(())
}

fn event_collection(kind: IncidentKind) -> &'static str {
    match kind {
        IncidentKind::Incident => "incidents",
        IncidentKind::Maintenance => "scheduled_maintenances",
    }
}

async fn merge_event_history(
    client: &reqwest::Client,
    base_url: &url::Url,
    incidents_path: &str,
    summary: &[u8],
    updated_since: Option<chrono::DateTime<Utc>>,
    kind: IncidentKind,
) -> Result<Vec<u8>, ProviderError> {
    let collection = event_collection(kind);
    let incidents =
        fetch_json(client, endpoint(base_url, incidents_path)?, 4 * 1024 * 1024).await?;
    let mut summary = serde_json::from_slice::<Value>(summary)
        .map_err(|error| ProviderError::Parse(error.to_string()))?;
    let history = incidents
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse(format!("history response omitted {collection}")))?;
    let current = match summary.get(collection) {
        Some(Value::Array(values)) => values.as_slice(),
        None if updated_since.is_none() => &[],
        _ => {
            return Err(ProviderError::Parse(format!(
                "summary omitted {collection}"
            )));
        }
    };
    if history.iter().chain(current).any(|value| {
        value
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    }) {
        return Err(ProviderError::Parse(
            "incident response contained an item without an ID".into(),
        ));
    }
    let mut merged = HashMap::<String, Value>::new();
    for incident in history {
        let Some(id) = incident.get("id").and_then(Value::as_str) else {
            continue;
        };
        let active = !matches!(
            incident.get("status").and_then(Value::as_str),
            Some("resolved" | "completed" | "postmortem")
        );
        let recent = updated_since.is_none_or(|since| {
            parse_time(
                incident
                    .get("updated_at")
                    .or_else(|| incident.get("created_at")),
            )
            .is_some_and(|updated| updated >= since)
        });
        if active || recent {
            let mut incident = incident.clone();
            incident["_statusdeck_history"] = Value::Bool(!active);
            merged.insert(id.to_owned(), incident);
        }
    }
    for incident in summary
        .get(collection)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = incident.get("id").and_then(Value::as_str) {
            let mut current = incident.clone();
            current["_statusdeck_history"] = Value::Bool(false);
            merged.insert(id.to_owned(), current);
        }
    }
    let mut incidents = merged.into_values().collect::<Vec<_>>();
    incidents.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .cmp(&right.get("id").and_then(Value::as_str))
    });
    summary
        .as_object_mut()
        .ok_or_else(|| ProviderError::Parse("summary was not an object".into()))?
        .insert(collection.into(), Value::Array(incidents));
    serde_json::to_vec(&summary).map_err(|error| ProviderError::Parse(error.to_string()))
}

fn retain_scoped_incidents(incidents: &mut [ProviderIncident], component_ids: &HashSet<String>) {
    for incident in incidents {
        incident.within_provider_scope = incident.affected_components.is_empty()
            || incident
                .affected_components
                .iter()
                .any(|id| component_ids.contains(id));
    }
}

async fn fetch_missing_events(
    client: &reqwest::Client,
    base_url: &url::Url,
    incidents_path: &str,
    page_url: Option<&url::Url>,
    missing: &[String],
    kind: IncidentKind,
) -> Result<Vec<ProviderIncident>, ProviderError> {
    let collection = event_collection(kind);
    let url = base_url
        .join(incidents_path)
        .map_err(|error| ProviderError::Configuration(error.to_string()))?;
    let root = fetch_json(client, url, 4 * 1024 * 1024).await?;
    let values = root
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse(format!("history response omitted {collection}")))?;
    if values.iter().any(|value| {
        value
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    }) {
        return Err(ProviderError::Parse(
            "incident history contained an item without an ID".into(),
        ));
    }
    Ok(parse_incidents(Some(values), kind, page_url)
        .into_iter()
        .filter(|incident| {
            matches!(incident.lifecycle, IncidentLifecycle::Resolved)
                && missing.iter().any(|id| id == &incident.upstream_id)
        })
        .collect())
}

pub fn parse_summary(
    body: &[u8],
    status: u16,
    etag: Option<String>,
    last_modified: Option<String>,
) -> Result<FetchOutcome, ProviderError> {
    let root: Value =
        serde_json::from_slice(body).map_err(|error| ProviderError::Parse(error.to_string()))?;
    for key in ["components", "incidents", "scheduled_maintenances"] {
        if key == "scheduled_maintenances" && root.get(key).is_none() {
            continue;
        }
        let values = root
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Parse(format!("summary omitted {key}")))?;
        if values.iter().any(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        }) {
            return Err(ProviderError::Parse(format!(
                "{key} contained an item without an ID"
            )));
        }
    }
    let page = root
        .get("page")
        .ok_or_else(|| ProviderError::Parse("missing page".into()))?;
    let page_url = page
        .get("url")
        .and_then(Value::as_str)
        .and_then(|value| url::Url::parse(value).ok());
    let original_overall = root
        .get("status")
        .and_then(|status| status.get("indicator"))
        .and_then(Value::as_str)
        .or_else(|| page.get("status").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_owned();
    let components = root
        .get("components")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("missing components".into()))?
        .iter()
        .enumerate()
        .filter_map(|(position, value)| {
            let id = value.get("id")?.as_str()?.to_owned();
            let original_status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            Some(ProviderComponent {
                upstream_id: id,
                name: value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed component")
                    .to_owned(),
                group: value
                    .get("group_name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                description: value
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                status: NormalizedStatus::from_provider(&original_status),
                original_status,
                position: i32::try_from(position).unwrap_or(i32::MAX),
            })
        })
        .collect::<Vec<_>>();
    let mut incidents = parse_incidents(
        root.get("incidents").and_then(Value::as_array),
        IncidentKind::Incident,
        page_url.as_ref(),
    );
    for incident in &mut incidents {
        apply_reported_utc_window(incident);
    }
    let maintenance = parse_incidents(
        root.get("scheduled_maintenances").and_then(Value::as_array),
        IncidentKind::Maintenance,
        page_url.as_ref(),
    );
    let overall = match original_overall.as_str() {
        "minor" => NormalizedStatus::Degraded,
        "major" => NormalizedStatus::PartialOutage,
        other => NormalizedStatus::from_provider(other),
    };
    Ok(FetchOutcome::Fetched(ProviderSnapshot {
        observed_at: Utc::now(),
        overall,
        original_overall: original_overall.clone(),
        provider_overall: overall,
        provider_original_overall: original_overall,
        components,
        incidents,
        maintenance,
        // Some compatible feeds omit maintenance entirely; only explicit resolutions
        // are trustworthy when the complete set of both event kinds is unavailable.
        active_incident_set_complete: root.get("scheduled_maintenances").is_some(),
        response: ResponseMetadata {
            status,
            etag,
            last_modified,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(root),
    }))
}

fn incident_page_url(body: &[u8]) -> Option<url::Url> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("page")?
        .get("url")?
        .as_str()
        .and_then(|value| url::Url::parse(value).ok())
}

fn incident_url(value: &Value, page_url: Option<&url::Url>, upstream_id: &str) -> Option<String> {
    if let Some(shortlink) = value.get("shortlink").and_then(Value::as_str) {
        return Some(shortlink.to_owned());
    }
    let mut url = page_url?.clone();
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url.path_segments_mut().ok()?;
    segments.clear().push("incidents").push(upstream_id);
    drop(segments);
    Some(url.into())
}

// Some retrospective incidents publish their actual window in a fixed UTC
// sentence while API timestamps describe backfill creation. Accept only a full,
// explicitly dated pair in a resolved note; approximate/date-only windows stay unknown.
fn apply_reported_utc_window(incident: &mut ProviderIncident) {
    if incident.lifecycle != IncidentLifecycle::Resolved
        || incident
            .started_at
            .or(incident.created_at)
            .zip(incident.resolved_at)
            .is_some_and(|(start, end)| end >= start)
    {
        return;
    }
    for update in incident
        .updates
        .iter()
        .rev()
        .filter(|update| update.status == "resolved")
    {
        let Some(window) = update.body.trim().strip_prefix("Between ") else {
            continue;
        };
        let Some((start, rest)) = window.split_once(" UTC and ") else {
            continue;
        };
        let Some((end, _)) = rest.split_once(" UTC, ") else {
            continue;
        };
        let Some((start, end)) = reported_utc_time(start).zip(reported_utc_time(end)) else {
            continue;
        };
        if end < start || update.created_at.is_none_or(|published| end > published) {
            continue;
        }
        incident.started_at = Some(start);
        incident.resolved_at = Some(end);
        incident.metadata["timing_basis"] = Value::String("reported_utc_window".into());
        incident.metadata["timing_update_id"] = serde_json::json!(update.upstream_id);
        return;
    }
}

fn reported_utc_time(value: &str) -> Option<chrono::DateTime<Utc>> {
    let value = value.replace(", ", " ").replace(" at ", " ");
    ["%B %e %Y %I:%M %p", "%B %e %Y %H:%M"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(&value, format).ok())
        .map(|time| time.and_utc())
        .filter(|time| time.timestamp() > 0)
}

fn parse_incidents(
    values: Option<&Vec<Value>>,
    kind: IncidentKind,
    page_url: Option<&url::Url>,
) -> Vec<ProviderIncident> {
    let mut incidents = values
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let upstream_id = value.get("id")?.as_str()?.to_owned();
            let original_phase = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let lifecycle = match original_phase.as_str() {
                "resolved" | "completed" | "postmortem" => IncidentLifecycle::Resolved,
                _ => IncidentLifecycle::Open,
            };
            let mut updates = value
                .get("incident_updates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|update| ProviderUpdate {
                    upstream_id: update
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    status: update
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    body: update
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    created_at: parse_time(update.get("created_at")),
                    updated_at: parse_time(update.get("updated_at")),
                    display_at: parse_time(update.get("display_at")),
                    affected_scopes: parse_statuspage_scopes(
                        update.get("affected_components").and_then(Value::as_array),
                    ),
                })
                .collect::<Vec<_>>();
            updates.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.upstream_id.cmp(&right.upstream_id))
            });
            let mut affected_scopes =
                parse_statuspage_scopes(value.get("components").and_then(Value::as_array));
            if affected_scopes.is_empty() {
                affected_scopes = component_scopes_from_updates(&updates);
            }
            let mut affected_components = affected_scopes
                .iter()
                .map(|scope| scope.upstream_id.clone())
                .collect::<Vec<_>>();
            affected_components.sort();
            affected_components.dedup();
            let url = incident_url(value, page_url, &upstream_id);
            let started_at = if kind == IncidentKind::Maintenance {
                // Statuspage's top-level started_at can be the announcement's
                // creation time. Only an activity update establishes actual work.
                updates
                    .iter()
                    .filter(|update| update.status == "in_progress")
                    .filter_map(|update| update.display_at.or(update.created_at))
                    .min()
            } else {
                parse_time(value.get("started_at"))
            };
            let resolved_at = parse_time(value.get("resolved_at")).or_else(|| {
                (lifecycle == IncidentLifecycle::Resolved)
                    .then(|| {
                        updates
                            .iter()
                            .filter(|update| {
                                matches!(update.status.as_str(), "resolved" | "completed")
                            })
                            .filter_map(|update| update.display_at.or(update.created_at))
                            .min()
                    })
                    .flatten()
            });
            Some(ProviderIncident {
                upstream_id,
                kind,
                title: value
                    .get("name")
                    .or_else(|| value.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Provider incident")
                    .to_owned(),
                url,
                lifecycle,
                original_phase,
                severity: Severity::from_impact(value.get("impact").and_then(Value::as_str)),
                original_impact: value
                    .get("impact")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                created_at: parse_time(value.get("created_at")),
                started_at,
                updated_at: parse_time(value.get("updated_at")),
                monitoring_at: parse_time(value.get("monitoring_at")),
                resolved_at,
                planned_start_at: (kind == IncidentKind::Maintenance)
                    .then(|| parse_time(value.get("scheduled_for")))
                    .flatten(),
                planned_end_at: (kind == IncidentKind::Maintenance)
                    .then(|| parse_time(value.get("scheduled_until")))
                    .flatten(),
                affected_scopes,
                within_provider_scope: true,
                metadata: value
                    .get("_statusdeck_history")
                    .and_then(Value::as_bool)
                    .filter(|history| *history)
                    .map_or_else(
                        || serde_json::json!({}),
                        |_| serde_json::json!({"_statusdeck_history": true}),
                    ),
                affected_components,
                updates,
            })
        })
        .collect::<Vec<_>>();
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    incidents
}

fn component_scopes_from_updates(updates: &[ProviderUpdate]) -> Vec<AffectedScope> {
    // Updates can retain component links after the incident's list is cleared.
    // Keep each component's latest reported state, including recovery updates.
    let mut seen = HashSet::new();
    let mut scopes = Vec::new();
    for update in updates.iter().rev() {
        for scope in &update.affected_scopes {
            if scope.scope_type == "component" && seen.insert(&scope.upstream_id) {
                scopes.push(scope.clone());
            }
        }
    }
    scopes.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    scopes
}

fn parse_statuspage_scopes(values: Option<&Vec<Value>>) -> Vec<AffectedScope> {
    values
        .into_iter()
        .flatten()
        .filter_map(|component| {
            let upstream_id = component
                .get("code")
                .or_else(|| component.get("id"))?
                .as_str()?
                .to_owned();
            let original_status = component
                .get("new_status")
                .or_else(|| component.get("status"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(AffectedScope {
                scope_type: "component".into(),
                display_name: component
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&upstream_id)
                    .to_owned(),
                normalized_status: original_status
                    .as_deref()
                    .map(NormalizedStatus::from_provider),
                original_status,
                upstream_id,
            })
        })
        .collect()
}
