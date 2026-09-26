use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::statuspage::{apply_component_scope, configured_strings};
use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, endpoint, fetch_json, http_client,
    parse_time,
};
use crate::domain::{
    IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent, ProviderIncident,
    ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

#[derive(Default)]
pub struct PagerDutyProvider;

#[async_trait]
impl StatusProvider for PagerDutyProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        if config.get("base_url").and_then(Value::as_str) != Some("https://status.pagerduty.com") {
            return Err(ProviderError::Configuration(
                "PagerDuty status must use the approved origin".into(),
            ));
        }
        configured_strings(config, "component_ids")?;
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let services_url = endpoint(&context.base_url, "/api/services")?;
        let impacts_url = endpoint(&context.base_url, "/api/impacted_services")?;
        let enums_url = endpoint(&context.base_url, "/api/post_enums")?;
        let posts_url = endpoint(&context.base_url, "/api/posts?is_featured=true&limit=500")?;
        let (services, impacts, enums, posts) = tokio::try_join!(
            fetch_json(&client, services_url, 4 * 1024 * 1024),
            fetch_json(&client, impacts_url, 4 * 1024 * 1024),
            fetch_json(&client, enums_url, 4 * 1024 * 1024),
            fetch_json(&client, posts_url, 4 * 1024 * 1024),
        )?;
        let posts = fetch_post_details(&client, context, posts).await?;
        let mut snapshot = parse_snapshot(services, impacts, enums, posts)?;
        apply_component_scope(&mut snapshot, &context.public_config)?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

async fn fetch_post_details(
    client: &reqwest::Client,
    context: &FetchContext,
    featured: Value,
) -> Result<Value, ProviderError> {
    let mut ids = context.known_active_incidents.clone();
    collect_post_ids(&featured, &mut ids)?;
    if featured
        .get("continuationToken")
        .is_some_and(|value| !value.is_null())
    {
        return Err(ProviderError::Parse(
            "PagerDuty featured posts exceeded one page".into(),
        ));
    }
    if context.refresh_history {
        // The public API rejects year-sized queries; its UI requests short windows.
        let now = Utc::now();
        let cutoff = now - chrono::Duration::days(365);
        let mut until = now;
        while until > cutoff {
            let since = (until - chrono::Duration::days(30)).max(cutoff);
            let mut continuation = None::<String>;
            let mut tokens = HashSet::new();
            for page in 0..10 {
                let mut url = endpoint(&context.base_url, "/api/posts")?;
                url.query_pairs_mut()
                    .append_pair("since", &since.timestamp_millis().to_string())
                    .append_pair("until", &until.timestamp_millis().to_string());
                if let Some(token) = &continuation {
                    url.query_pairs_mut()
                        .append_pair("continuation_token", token);
                }
                let response = fetch_json(client, url, 4 * 1024 * 1024).await?;
                collect_post_ids(&response, &mut ids)?;
                continuation = match response.get("continuationToken") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(token)) if !token.is_empty() => Some(token.clone()),
                    _ => {
                        return Err(ProviderError::Parse(
                            "Invalid PagerDuty continuation token".into(),
                        ));
                    }
                };
                let Some(token) = &continuation else { break };
                if page == 9 || !tokens.insert(token.clone()) {
                    return Err(ProviderError::Parse(
                        "PagerDuty history pagination exceeded its bound or repeated".into(),
                    ));
                }
            }
            until = since;
        }
    }
    if ids.len() > 1000 {
        return Err(ProviderError::Parse(
            "PagerDuty posts exceeded the detail request bound".into(),
        ));
    }
    // List messages are truncated. Retrieve complete posts with bounded concurrency.
    let mut pending = tokio::task::JoinSet::new();
    let mut posts = Vec::with_capacity(ids.len());
    for id in ids {
        let mut url = endpoint(&context.base_url, "/api/posts/")?;
        url.path_segments_mut()
            .map_err(|_| ProviderError::Configuration("Invalid PagerDuty post URL".into()))?
            .pop_if_empty()
            .push(&id);
        let client = client.clone();
        pending.spawn(async move {
            let response = fetch_json(&client, url, 4 * 1024 * 1024).await?;
            let post = response
                .get("post")
                .filter(|post| post.get("id").and_then(Value::as_str) == Some(id.as_str()))
                .ok_or_else(|| {
                    ProviderError::Parse(
                        "PagerDuty detail omitted or mismatched its post ID".into(),
                    )
                })?;
            Ok::<_, ProviderError>(post.clone())
        });
        if pending.len() == 4 {
            posts.push(
                pending
                    .join_next()
                    .await
                    .expect("four pending PagerDuty detail tasks")
                    .map_err(|error| ProviderError::Transport(error.to_string()))??,
            );
        }
    }
    while let Some(result) = pending.join_next().await {
        posts.push(result.map_err(|error| ProviderError::Transport(error.to_string()))??);
    }
    Ok(json!({"posts": posts}))
}

fn collect_post_ids(root: &Value, ids: &mut HashSet<String>) -> Result<(), ProviderError> {
    for post in root
        .get("posts")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("PagerDuty response omitted posts".into()))?
    {
        let id = post
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProviderError::Parse("PagerDuty post omitted its ID".into()))?;
        ids.insert(id.to_owned());
        if ids.len() > 1000 {
            return Err(ProviderError::Parse(
                "PagerDuty posts exceeded the detail request bound".into(),
            ));
        }
    }
    Ok(())
}

fn parse_snapshot(
    services_root: Value,
    impacts_root: Value,
    enums_root: Value,
    posts_root: Value,
) -> Result<ProviderSnapshot, ProviderError> {
    let services = services_root
        .get("services")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("PagerDuty response omitted services".into()))?;
    if services.is_empty() {
        return Err(ProviderError::Parse(
            "PagerDuty response contained no services".into(),
        ));
    }
    let enums = enums_root
        .get("post_enums")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("PagerDuty response omitted post_enums".into()))?;
    let enum_names = enums
        .iter()
        .filter_map(|value| {
            Some((
                value.get("id")?.as_str()?.to_owned(),
                value.get("name")?.as_str()?.to_owned(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let impacts = impacts_root
        .get("impacted_services")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("PagerDuty response omitted impacted_services".into()))?
        .iter()
        .map(|value| {
            Ok((
                value
                    .get("service_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        ProviderError::Parse("PagerDuty impact omitted its service ID".into())
                    })?
                    .to_owned(),
                value
                    .get("impact_severity_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        ProviderError::Parse("PagerDuty impact omitted its severity ID".into())
                    })?
                    .to_owned(),
            ))
        })
        .collect::<Result<HashMap<_, _>, ProviderError>>()?;
    let components = services
        .iter()
        .enumerate()
        .filter(|(_, value)| value.get("is_active").and_then(Value::as_bool) != Some(false))
        .map(|(position, value)| {
            let upstream_id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ProviderError::Parse("PagerDuty service omitted its ID".into()))?
                .to_owned();
            let original_status = impacts
                .get(&upstream_id)
                .map_or("operational", |id| {
                    enum_names.get(id).map_or("unknown", String::as_str)
                })
                .to_owned();
            Ok(ProviderComponent {
                upstream_id,
                name: value
                    .get("display_name")
                    .or_else(|| value.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed service")
                    .trim()
                    .to_owned(),
                group: None,
                description: None,
                status: pagerduty_status(&original_status),
                original_status,
                position: i32::try_from(position).unwrap_or(i32::MAX),
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let mut incidents = Vec::new();
    let mut maintenance = Vec::new();
    for post in posts_root
        .get("posts")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("PagerDuty response omitted posts".into()))?
    {
        if !post.get("latest_update").is_some_and(Value::is_object)
            || !post.get("updates").is_some_and(Value::is_array)
        {
            return Err(ProviderError::Parse(
                "PagerDuty post omitted its latest update or update collection".into(),
            ));
        }
        let kind = if post.get("post_type").and_then(Value::as_str) == Some("maintenance") {
            IncidentKind::Maintenance
        } else {
            IncidentKind::Incident
        };
        let event = parse_post(post, kind, &enum_names).ok_or_else(|| {
            ProviderError::Parse("PagerDuty post omitted its ID or latest update".into())
        })?;
        if kind == IncidentKind::Maintenance {
            maintenance.push(event);
        } else {
            incidents.push(event);
        }
    }
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    maintenance.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    let overall = rollup(components.iter().map(|component| component.status));
    Ok(ProviderSnapshot {
        observed_at: Utc::now(),
        overall,
        original_overall: overall.key().to_owned(),
        provider_overall: overall,
        provider_original_overall: overall.key().to_owned(),
        components,
        incidents,
        maintenance,
        active_incident_set_complete: true,
        response: ResponseMetadata {
            status: 200,
            etag: None,
            last_modified: None,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(json!({
            "services": services_root,
            "impacted_services": impacts_root,
            "post_enums": enums_root,
            "posts": posts_root,
        })),
    })
}

fn parse_post(
    value: &Value,
    kind: IncidentKind,
    enum_names: &HashMap<String, String>,
) -> Option<ProviderIncident> {
    let upstream_id = value.get("id")?.as_str()?.to_owned();
    let latest = value.get("latest_update")?;
    let phase = enum_value(latest.get("status_id"), enum_names, "unknown");
    let lifecycle = match phase.as_str() {
        "resolved" | "completed" => IncidentLifecycle::Resolved,
        _ => IncidentLifecycle::Open,
    };
    let impact = enum_value(latest.get("severity_id"), enum_names, "unknown");
    let mut affected_components = value
        .get("updates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(std::iter::once(latest))
        .flat_map(|update| {
            update
                .get("impacts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|impact| impact.get("service_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    affected_components.sort();
    affected_components.dedup();
    let updates = value
        .get("updates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|update| ProviderUpdate {
            upstream_id: update
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            status: enum_value(update.get("status_id"), enum_names, "unknown"),
            body: update
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at: pagerduty_time(update.get("reported_at")),
            updated_at: pagerduty_time(update.get("reported_at")),
            display_at: None,
            affected_scopes: Vec::new(),
        })
        .collect();
    Some(ProviderIncident {
        url: Some(format!(
            "https://status.pagerduty.com/posts/details/{upstream_id}"
        )),
        upstream_id,
        kind,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("PagerDuty status event")
            .to_owned(),
        lifecycle,
        original_phase: phase,
        severity: Severity::from_impact(Some(&impact)),
        original_impact: Some(impact),
        created_at: pagerduty_time(value.get("first_update_at")),
        started_at: match kind {
            IncidentKind::Incident => pagerduty_time(value.get("first_update_at")),
            IncidentKind::Maintenance => value
                .get("updates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|update| {
                    enum_value(update.get("status_id"), enum_names, "unknown") == "in progress"
                })
                .filter_map(|update| pagerduty_time(update.get("reported_at")))
                .min(),
        },
        updated_at: pagerduty_time(latest.get("reported_at")),
        monitoring_at: None,
        resolved_at: (lifecycle == IncidentLifecycle::Resolved)
            .then(|| pagerduty_time(latest.get("reported_at")))
            .flatten(),
        planned_start_at: (kind == IncidentKind::Maintenance)
            .then(|| pagerduty_time(value.get("starts_at")))
            .flatten(),
        planned_end_at: (kind == IncidentKind::Maintenance)
            .then(|| pagerduty_time(value.get("ends_at")))
            .flatten(),
        affected_components,
        affected_scopes: Vec::new(),
        within_provider_scope: true,
        metadata: serde_json::json!({}),
        updates,
    })
}

fn enum_value(value: Option<&Value>, names: &HashMap<String, String>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .and_then(|id| names.get(id))
        .map_or_else(|| fallback.to_owned(), Clone::clone)
}

fn pagerduty_status(value: &str) -> NormalizedStatus {
    match value.to_ascii_lowercase().as_str() {
        "maintenance" => NormalizedStatus::Maintenance,
        "minor" => NormalizedStatus::Degraded,
        "major" | "partial outage" => NormalizedStatus::PartialOutage,
        "outage" => NormalizedStatus::MajorOutage,
        other => NormalizedStatus::from_provider(other),
    }
}

fn pagerduty_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_i64)
        .filter(|millis| *millis > 0)
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| parse_time(value))
}
