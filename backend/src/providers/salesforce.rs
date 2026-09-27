use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Map, Value, json};

use super::statuspage::{apply_component_scope, configured_strings};
use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, endpoint, fetch_json, http_client,
    parse_time,
};
use crate::domain::{
    AffectedScope, IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent,
    ProviderIncident, ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

#[derive(Default)]
pub struct SalesforceProvider;

#[async_trait]
impl StatusProvider for SalesforceProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        if config.get("base_url").and_then(Value::as_str)
            != Some("https://api.status.salesforce.com")
        {
            return Err(ProviderError::Configuration(
                "Salesforce status must use the approved API origin".into(),
            ));
        }
        configured_strings(config, "component_ids")?;
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let services_url = endpoint(&context.base_url, "/v1/services")?;
        let incidents_url = endpoint(&context.base_url, "/v1/incidents/active")?;
        let (services, incidents, maintenance) = tokio::try_join!(
            fetch_json(&client, services_url, 2 * 1024 * 1024),
            fetch_json(&client, incidents_url, 2 * 1024 * 1024),
            fetch_events(&client, context, IncidentKind::Maintenance),
        )?;
        let mut snapshot = parse_snapshot(services, incidents, Value::Array(maintenance))?;
        if context.refresh_history {
            let history = fetch_events(&client, context, IncidentKind::Incident).await?;
            for value in &history {
                let mut event = parse_event(value, IncidentKind::Incident).ok_or_else(|| {
                    ProviderError::Parse("Salesforce history incident omitted its ID".into())
                })?;
                if event.lifecycle == IncidentLifecycle::Resolved
                    && !snapshot
                        .incidents
                        .iter()
                        .any(|current| current.upstream_id == event.upstream_id)
                {
                    event.metadata["_statusdeck_history"] = Value::Bool(true);
                    snapshot.incidents.push(event);
                }
            }
            if let Some(raw) = snapshot.raw_payload.as_mut() {
                raw["history"] = Value::Array(history);
            }
        }
        apply_component_scope(&mut snapshot, &context.public_config)?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

async fn fetch_events(
    client: &reqwest::Client,
    context: &FetchContext,
    kind: IncidentKind,
) -> Result<Vec<Value>, ProviderError> {
    const MAX_PAGES: usize = 100;
    // Preserve the Trust API's default maintenance lookback, including future windows.
    const MAINTENANCE_LOOKBACK_DAYS: i64 = 30;
    let (path, page_size, lookback_days) = match kind {
        IncidentKind::Incident => ("/v1/incidents", 100, super::HISTORY_LOOKBACK_DAYS),
        IncidentKind::Maintenance => ("/v1/maintenances", 1000, MAINTENANCE_LOOKBACK_DAYS),
    };
    let since = (Utc::now() - chrono::Duration::days(lookback_days)).to_rfc3339();
    let fetch_page = async |page: usize| {
        let mut url = endpoint(&context.base_url, path)?;
        url.query_pairs_mut()
            .append_pair("startTime", &since)
            .append_pair("limit", &page_size.to_string())
            .append_pair("offset", &(page * page_size).to_string())
            .append_pair("sort", "id")
            .append_pair("order", "ASC");
        fetch_json(client, url, 4 * 1024 * 1024).await
    };
    let mut events = Vec::new();
    let mut seen = HashSet::new();
    // Bound concurrency to fit the polling deadline, and inspect pages in offset order.
    for page in (0..MAX_PAGES).step_by(2) {
        let (first, second) = tokio::join!(fetch_page(page), fetch_page(page + 1));
        for body in [first, second] {
            let Value::Array(values) = body? else {
                return Err(ProviderError::Parse(format!(
                    "Salesforce {path} response must be an array"
                )));
            };
            if values.len() > page_size {
                return Err(ProviderError::Parse(format!(
                    "Salesforce {path} response exceeded its page size"
                )));
            }
            for value in &values {
                let id = value.get("id").and_then(Value::as_i64).ok_or_else(|| {
                    ProviderError::Parse(format!("Salesforce {path} event omitted its numeric ID"))
                })?;
                if !seen.insert(id) {
                    return Err(ProviderError::Parse(format!(
                        "Salesforce {path} pagination repeated an event"
                    )));
                }
            }
            let complete = values.len() < page_size;
            events.extend(values);
            if complete {
                return Ok(events);
            }
        }
    }
    Err(ProviderError::Parse(format!(
        "Salesforce {path} exceeded its pagination limit"
    )))
}

fn parse_snapshot(
    services_root: Value,
    incidents_root: Value,
    maintenance_root: Value,
) -> Result<ProviderSnapshot, ProviderError> {
    let services = services_root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Salesforce response omitted services".into()))?;
    let incident_values = incidents_root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Salesforce response omitted incidents".into()))?;
    let maintenance_values = maintenance_root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Salesforce response omitted maintenance".into()))?;
    let status_by_service = service_statuses(incident_values, maintenance_values);
    let components = services
        .iter()
        .enumerate()
        .map(|(position, service)| {
            let upstream_id = service
                .get("key")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ProviderError::Parse("Salesforce service omitted its key".into()))?
                .to_owned();
            let status = status_by_service
                .get(&upstream_id)
                .copied()
                .unwrap_or(NormalizedStatus::Operational);
            let product_names = service
                .get("Products")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|product| product.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>();
            Ok(ProviderComponent {
                upstream_id: upstream_id.clone(),
                name: service
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .map_or_else(
                        || salesforce_service_name(&upstream_id).to_owned(),
                        ToOwned::to_owned,
                    ),
                group: (!product_names.is_empty()).then(|| product_names.join(", ")),
                description: None,
                status,
                original_status: status.key().to_owned(),
                position: i32::try_from(position).unwrap_or(i32::MAX),
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "Salesforce response contained no services".into(),
        ));
    }
    let mut incidents = incident_values
        .iter()
        .map(|value| {
            parse_event(value, IncidentKind::Incident)
                .ok_or_else(|| ProviderError::Parse("Salesforce incident omitted its ID".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    let now = Utc::now();
    let mut maintenance = maintenance_values
        .iter()
        .map(|value| {
            // An elapsed planned window does not confirm that maintenance ended.
            let mut event = parse_event(value, IncidentKind::Maintenance).ok_or_else(|| {
                ProviderError::Parse("Salesforce maintenance omitted its ID".into())
            })?;
            if event.lifecycle == IncidentLifecycle::Resolved {
                event.metadata["_statusdeck_history"] = Value::Bool(true);
            }
            Ok(event)
        })
        .collect::<Result<Vec<_>, _>>()?;
    maintenance.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    let overall = rollup(components.iter().map(|component| component.status));
    Ok(ProviderSnapshot {
        observed_at: now,
        overall,
        original_overall: overall.key().to_owned(),
        provider_overall: overall,
        provider_original_overall: overall.key().to_owned(),
        components,
        incidents,
        maintenance,
        // Maintenance has a date window; falling outside it does not confirm recovery.
        active_incident_set_complete: false,
        response: ResponseMetadata {
            status: 200,
            etag: None,
            last_modified: None,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(json!({
            "services": services_root,
            "incidents": incidents_root,
            "maintenance": maintenance_root,
        })),
    })
}

fn salesforce_service_name(key: &str) -> &str {
    match key {
        "coreService" => "Core Service",
        "Communities" => "Experience Cloud",
        "liveAgent" => "Live Agent",
        "Agentforce" => "Agentforce",
        "Data" => "Data Cloud",
        "B2BCommerce" => "B2B Commerce",
        "MarketingCloudCoreService" => "Marketing Cloud Core",
        "MarketingCloudRESTAPI" => "Marketing Cloud REST API",
        "AnalyticsCloud" => "Tableau",
        "Dataloader" => "MuleSoft Data Loader",
        "MuleSoftPlatformMCPServer" => "MuleSoft Platform MCP Server",
        other => other,
    }
}

fn service_statuses(
    incidents: &[Value],
    maintenance: &[Value],
) -> HashMap<String, NormalizedStatus> {
    let mut statuses = HashMap::new();
    let now = Utc::now();
    for (incident, kind) in incidents
        .iter()
        .map(|event| (event, IncidentKind::Incident))
        .chain(
            maintenance
                .iter()
                .map(|event| (event, IncidentKind::Maintenance)),
        )
    {
        if event_lifecycle(
            incident
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) == IncidentLifecycle::Resolved
        {
            continue;
        }
        let status = if kind == IncidentKind::Maintenance {
            // Planned windows do not establish that work actually began or ended.
            let active = incident
                .get("MaintenanceImpacts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|impact| {
                    parse_time(impact.get("startTime")).is_some_and(|start| start <= now)
                        && parse_time(impact.get("endTime")).is_none_or(|end| end > now)
                });
            if !active {
                continue;
            }
            NormalizedStatus::Maintenance
        } else {
            salesforce_status(incident_severity(incident).as_deref())
        };
        for service in incident
            .get("serviceKeys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let current = statuses
                .entry(service.to_owned())
                .or_insert(NormalizedStatus::Operational);
            if status.rank() > current.rank() {
                *current = status;
            }
        }
    }
    statuses
}

fn incident_severity(value: &Value) -> Option<String> {
    value
        .get("IncidentImpacts")
        .or_else(|| value.get("MaintenanceImpacts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|impact| impact.get("severity").and_then(Value::as_str))
        .max_by_key(|severity| salesforce_status(Some(severity)).rank())
        .map(ToOwned::to_owned)
}

fn salesforce_status(severity: Option<&str>) -> NormalizedStatus {
    match severity.unwrap_or_default().to_ascii_lowercase().as_str() {
        "critical" | "blocker" => NormalizedStatus::MajorOutage,
        "major" => NormalizedStatus::PartialOutage,
        _ => NormalizedStatus::Degraded,
    }
}

fn event_lifecycle(phase: &str) -> IncidentLifecycle {
    if matches!(
        phase.to_ascii_lowercase().as_str(),
        "completed" | "resolved" | "canceled" | "cancelled"
    ) {
        IncidentLifecycle::Resolved
    } else {
        IncidentLifecycle::Open
    }
}

fn parse_event(value: &Value, kind: IncidentKind) -> Option<ProviderIncident> {
    let id = value.get("id")?;
    let upstream_id = id
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| id.as_i64().map(|id| id.to_string()))?;
    let original_phase = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_owned();
    let lifecycle = event_lifecycle(&original_phase);
    let updates = value
        .get("timeline")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|event| ProviderUpdate {
            upstream_id: event.get("sourceId").map(|id| {
                let id = id
                    .as_str()
                    .map_or_else(|| id.to_string(), ToOwned::to_owned);
                // An impact's start and end share sourceId but are distinct updates.
                format!(
                    "{}:{id}:{}",
                    event
                        .get("sourceType")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    event
                        .get("entryType")
                        .and_then(Value::as_str)
                        .unwrap_or("update")
                )
            }),
            status: event
                .get("entryType")
                .and_then(Value::as_str)
                .unwrap_or("update")
                .to_owned(),
            body: event
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at: parse_time(event.get("startTime").or_else(|| event.get("createdAt"))),
            updated_at: parse_time(event.get("updatedAt")),
            display_at: None,
            affected_scopes: Vec::new(),
        })
        .collect();
    let original_impact = incident_severity(value);
    let title = value
        .get("name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get(if kind == IncidentKind::Maintenance {
                    "MaintenanceImpacts"
                } else {
                    "IncidentImpacts"
                })
                .and_then(Value::as_array)
                .and_then(|impacts| impacts.first())
                .and_then(|impact| impact.get("type"))
                .and_then(Value::as_str)
                .map(humanize_salesforce_type)
        })
        .unwrap_or_else(|| {
            format!(
                "Salesforce {} #{upstream_id}",
                value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("status event")
            )
        });
    Some(ProviderIncident {
        upstream_id: upstream_id.clone(),
        kind,
        title,
        url: Some(format!(
            "https://status.salesforce.com/{}/{upstream_id}",
            match kind {
                IncidentKind::Incident => "incidents",
                IncidentKind::Maintenance => "maintenances",
            }
        )),
        lifecycle,
        original_phase,
        severity: salesforce_severity(original_impact.as_deref()),
        original_impact,
        created_at: parse_time(value.get("createdAt"))
            .or_else(|| impact_time(value, kind, "startTime")),
        started_at: impact_time(value, kind, "startTime"),
        updated_at: parse_time(value.get("updatedAt")),
        monitoring_at: None,
        resolved_at: (lifecycle == IncidentLifecycle::Resolved)
            .then(|| impact_time(value, kind, "endTime"))
            .flatten(),
        planned_start_at: (kind == IncidentKind::Maintenance)
            .then(|| parse_time(value.get("plannedStartTime")))
            .flatten(),
        planned_end_at: (kind == IncidentKind::Maintenance)
            .then(|| parse_time(value.get("plannedEndTime")))
            .flatten(),
        affected_components: value
            .get("serviceKeys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        affected_scopes: salesforce_scopes(value),
        within_provider_scope: true,
        metadata: salesforce_metadata(value),
        updates,
    })
}

fn salesforce_metadata(value: &Value) -> Value {
    let mut metadata = Map::new();
    for (source, target) in [
        ("externalId", "external_id"),
        ("type", "event_type"),
        ("externalMaintenanceType", "maintenance_type"),
        ("releaseType", "release_type"),
        ("substrate", "substrate"),
        ("additionalInformation", "additional_information"),
    ] {
        if let Some(value) = value.get(source).filter(|value| {
            !value.is_null() && value.as_str().is_none_or(|text| !text.trim().is_empty())
        }) {
            metadata.insert(target.into(), value.clone());
        }
    }
    for (source, target) in [("affectsAll", "affects_all"), ("isCore", "is_core")] {
        if let Some(value) = value.get(source).and_then(Value::as_bool) {
            metadata.insert(target.into(), Value::Bool(value));
        }
    }
    if let Some(message) = value.get("message").and_then(Value::as_object) {
        for (source, target) in [
            ("rootCause", "root_cause"),
            ("actionPlan", "action_plan"),
            ("pathToResolution", "path_to_resolution"),
        ] {
            if let Some(text) = message
                .get(source)
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                metadata.insert(target.into(), Value::String(text.to_owned()));
            }
        }
    }
    Value::Object(metadata)
}

fn salesforce_severity(severity: Option<&str>) -> Severity {
    if severity.is_some_and(|value| value.eq_ignore_ascii_case("blocker")) {
        Severity::Critical
    } else {
        Severity::from_impact(severity)
    }
}

fn impact_time(value: &Value, kind: IncidentKind, key: &str) -> Option<chrono::DateTime<Utc>> {
    let times = value
        .get(if kind == IncidentKind::Maintenance {
            "MaintenanceImpacts"
        } else {
            "IncidentImpacts"
        })
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|impact| parse_time(impact.get(key)));
    if key == "endTime" {
        times.max()
    } else {
        times.min()
    }
}

fn salesforce_scopes(value: &Value) -> Vec<AffectedScope> {
    let component_scopes = value
        .get("serviceKeys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| AffectedScope {
            scope_type: "component".into(),
            upstream_id: id.to_owned(),
            display_name: salesforce_service_name(id).to_owned(),
            normalized_status: None,
            original_status: None,
        });
    let instance_scopes = value
        .get("instanceKeys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| AffectedScope {
            scope_type: "instance".into(),
            upstream_id: id.to_owned(),
            display_name: id.to_owned(),
            normalized_status: None,
            original_status: None,
        });
    component_scopes.chain(instance_scopes).collect()
}

fn humanize_salesforce_type(value: &str) -> String {
    let mut title = String::with_capacity(value.len() + 4);
    for (index, character) in value.chars().enumerate() {
        if index > 0 && character.is_ascii_uppercase() {
            title.push(' ');
        }
        if index == 0 {
            title.extend(character.to_uppercase());
        } else {
            title.push(character);
        }
    }
    title
}
