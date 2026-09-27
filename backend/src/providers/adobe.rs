use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, affects_current_status, endpoint,
    fetch_json, http_client, incident_status,
};
use crate::domain::{
    IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent, ProviderIncident,
    ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

#[derive(Default)]
pub struct AdobeProvider;

const MARKETO_PRODUCT_ID: &str = "503491";
const MARKETO_US_COMPONENT_ID: &str = "marketo-us";
#[async_trait]
impl StatusProvider for AdobeProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        let url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("base_url is required".into()))?;
        let parsed = url::Url::parse(url)
            .map_err(|_| ProviderError::Configuration("base_url must be a URL".into()))?;
        if parsed.as_str() != "https://data.status.adobe.com/" {
            return Err(ProviderError::Configuration(
                "Adobe status must use the approved API origin".into(),
            ));
        }
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let (events, registry) = tokio::try_join!(
            fetch_json(
                &client,
                endpoint(&context.base_url, "/adobestatus/StatusEvents")?,
                12 * 1024 * 1024
            ),
            fetch_json(
                &client,
                endpoint(&context.base_url, "/adobestatus/SnowServiceRegistry")?,
                12 * 1024 * 1024
            ),
        )?;
        parse_events(events, registry)
    }
}

fn parse_events(root: Value, registry: Value) -> Result<FetchOutcome, ProviderError> {
    let observed_at = Utc::now();
    let mut services = marketo_services(&registry)?;
    for path in ["/incidentEvent/incidents", "/maintenanceEvent/maintenance"] {
        let events = root
            .pointer(path)
            .and_then(Value::as_object)
            .ok_or_else(|| ProviderError::Parse(format!("Adobe response omitted {path}")))?;
        for event in events.values() {
            if let Some(product) = event.pointer(&format!("/products/{MARKETO_PRODUCT_ID}"))
                && product
                    .get("history")
                    .and_then(Value::as_object)
                    .is_none_or(|history| history.is_empty())
            {
                return Err(ProviderError::Parse(
                    "Adobe Marketo event omitted its history".into(),
                ));
            }
        }
    }
    let incident_messages = messages(&root, "incidentEvent");
    let maintenance_messages = messages(&root, "maintenanceEvent");
    let incidents = parse_event_collection(
        root.pointer("/incidentEvent/incidents"),
        IncidentKind::Incident,
        &incident_messages,
        observed_at,
        &services,
    );
    let maintenance = parse_event_collection(
        root.pointer("/maintenanceEvent/maintenance"),
        IncidentKind::Maintenance,
        &maintenance_messages,
        observed_at,
        &services,
    );
    let active_statuses = incidents
        .iter()
        .chain(&maintenance)
        .filter(|incident| affects_current_status(incident))
        .map(incident_status)
        .collect::<Vec<_>>();
    let component_status = rollup(active_statuses);
    let component_status = if component_status == NormalizedStatus::Unknown {
        NormalizedStatus::Operational
    } else {
        component_status
    };
    // Product-level impact without service IDs cannot establish individual service health.
    let active = incidents
        .iter()
        .chain(&maintenance)
        .filter(|incident| affects_current_status(incident))
        .collect::<Vec<_>>();
    let unscoped_impact = active
        .iter()
        .any(|incident| incident.affected_components.len() == 1);
    for service in &mut services {
        service.status = active
            .iter()
            .filter(|incident| incident.affected_components.contains(&service.upstream_id))
            .map(|incident| incident_status(incident))
            .max_by_key(|status| status.rank())
            .unwrap_or(if unscoped_impact {
                NormalizedStatus::Unknown
            } else {
                NormalizedStatus::Operational
            });
        service.original_status = service.status.key().to_owned();
    }
    services.insert(
        0,
        ProviderComponent {
            upstream_id: MARKETO_US_COMPONENT_ID.into(),
            name: "Adobe Marketo Engage (US)".into(),
            group: Some("Americas".into()),
            description: Some("Aggregate coverage for Marketo Americas environments".into()),
            status: component_status,
            original_status: component_status.key().to_owned(),
            position: 0,
        },
    );
    Ok(FetchOutcome::Fetched(ProviderSnapshot {
        observed_at,
        overall: component_status,
        original_overall: component_status.key().to_owned(),
        provider_overall: component_status,
        provider_original_overall: component_status.key().to_owned(),
        components: services,
        incidents,
        maintenance,
        active_incident_set_complete: true,
        response: ResponseMetadata {
            status: 200,
            etag: None,
            last_modified: None,
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(json!({"events": root, "registry": registry})),
    }))
}

fn marketo_services(registry: &Value) -> Result<Vec<ProviderComponent>, ProviderError> {
    let product = registry
        .pointer(&format!("/products/{MARKETO_PRODUCT_ID}"))
        .ok_or_else(|| ProviderError::Parse("Adobe registry omitted Marketo".into()))?;
    let offerings = product
        .get("productOfferings")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("Adobe registry omitted Marketo offerings".into()))?;
    let mut owners = vec![product];
    for id in offerings {
        let offering = id
            .as_str()
            .and_then(|id| registry["offerings"].get(id))
            .ok_or_else(|| ProviderError::Parse("Adobe registry omitted an offering".into()))?;
        owners.push(offering);
    }
    let mut ids = BTreeSet::new();
    for owner in owners {
        let services = owner
            .get("productServices")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::Parse("Adobe registry omitted product services".into())
            })?;
        for id in services {
            ids.insert(id.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
                ProviderError::Parse("Adobe registry contained an invalid service ID".into())
            })?);
        }
    }
    let mut components = Vec::new();
    for id in ids {
        let service = &registry["services"][id];
        let name = service
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ProviderError::Parse("Adobe registry omitted a service name".into()))?;
        let environments = service
            .get("serviceEnvironments")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::Parse("Adobe registry omitted service environments".into())
            })?;
        let mut americas = false;
        for environment in environments {
            let regions = environment
                .as_str()
                .and_then(|id| registry["environments"].get(id))
                .and_then(|env| env.get("regionId"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ProviderError::Parse("Adobe registry omitted environment regions".into())
                })?;
            americas |= regions
                .iter()
                .any(|region| region.as_str() == Some("Americas"));
        }
        if !americas {
            continue;
        }
        components.push(ProviderComponent {
            upstream_id: id.to_owned(),
            name: name.to_owned(),
            group: Some("Americas".into()),
            description: None,
            status: NormalizedStatus::Unknown,
            original_status: "unknown".into(),
            position: i32::try_from(components.len() + 1).unwrap_or(i32::MAX),
        });
    }
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "Adobe registry contained no Marketo Americas services".into(),
        ));
    }
    Ok(components)
}

fn messages(root: &Value, collection: &str) -> Value {
    root.pointer(&format!("/{collection}/messages/en"))
        .cloned()
        .unwrap_or_else(|| json!({}))
}

fn parse_event_collection(
    collection: Option<&Value>,
    kind: IncidentKind,
    messages: &Value,
    observed_at: DateTime<Utc>,
    services: &[ProviderComponent],
) -> Vec<ProviderIncident> {
    let mut incidents = collection
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|events| events.iter())
        .filter_map(|(event_id, event)| {
            let product = event.pointer(&format!("/products/{MARKETO_PRODUCT_ID}"))?;
            let history = product.get("history")?.as_object()?;
            let mut history_entries = history.iter().collect::<Vec<_>>();
            history_entries.sort_by_key(|(timestamp, _)| timestamp.parse::<i64>().unwrap_or(0));
            let latest = (*history_entries.last()?.1).clone();
            if !affects_us(&latest) {
                return None;
            }
            let original_phase = latest
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_owned();
            let ended_at = product
                .get("endedOn")
                .and_then(Value::as_i64)
                .filter(|value| *value > 0)
                .and_then(DateTime::from_timestamp_secs)
                .filter(|end| {
                    product
                        .get("startedOn")
                        .and_then(Value::as_i64)
                        .is_none_or(|start| end.timestamp() >= start)
                });
            let lifecycle = lifecycle(&original_phase, ended_at, observed_at);
            let created_at = if kind == IncidentKind::Maintenance {
                history_entries
                    .first()
                    .and_then(|(timestamp, _)| unix_time(timestamp))
            } else {
                product
                    .get("startedOn")
                    .and_then(Value::as_i64)
                    .and_then(DateTime::from_timestamp_secs)
            };
            let started_at = if kind == IncidentKind::Maintenance {
                history_entries
                    .iter()
                    .filter(|(_, update)| {
                        matches!(
                            update
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_ascii_lowercase()
                                .as_str(),
                            "started" | "in progress" | "in_progress" | "ongoing"
                        )
                    })
                    .find_map(|(timestamp, _)| unix_time(timestamp))
            } else {
                created_at
            };
            let updates = history_entries
                .into_iter()
                .filter(|(_, update)| affects_us(update))
                .map(|(timestamp, update)| ProviderUpdate {
                    upstream_id: update
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    status: update
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown")
                        .to_owned(),
                    body: message_body(messages, update.get("messageToken")),
                    created_at: unix_time(timestamp),
                    updated_at: unix_time(timestamp),
                    display_at: None,
                    affected_scopes: Vec::new(),
                })
                .collect();
            let original_impact = latest
                .get("severity")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(ProviderIncident {
                upstream_id: event_id.to_owned(),
                kind,
                title: message_text(messages, latest.get("titleToken")),
                url: Some(format!("https://status.adobe.com/#/{event_id}")),
                lifecycle,
                original_phase,
                severity: Severity::from_impact(original_impact.as_deref()),
                original_impact,
                created_at,
                started_at,
                updated_at: latest
                    .get("messageTime")
                    .and_then(Value::as_i64)
                    .and_then(DateTime::from_timestamp_secs),
                monitoring_at: None,
                resolved_at: (lifecycle == IncidentLifecycle::Resolved)
                    .then_some(ended_at)
                    .flatten(),
                planned_start_at: (kind == IncidentKind::Maintenance)
                    .then(|| {
                        event
                            .get("startedOn")
                            .and_then(Value::as_i64)
                            .filter(|timestamp| *timestamp > 0)
                            .and_then(DateTime::from_timestamp_secs)
                    })
                    .flatten(),
                planned_end_at: (kind == IncidentKind::Maintenance)
                    .then(|| {
                        event
                            .get("completedOn")
                            .and_then(Value::as_i64)
                            .filter(|timestamp| *timestamp > 0)
                            .and_then(DateTime::from_timestamp_secs)
                    })
                    .flatten(),
                affected_components: std::iter::once(MARKETO_US_COMPONENT_ID.to_owned())
                    .chain(
                        services
                            .iter()
                            .filter(|service| {
                                latest
                                    .pointer("/serviceImpact/productServices")
                                    .and_then(Value::as_array)
                                    .is_some_and(|ids| {
                                        ids.iter()
                                            .any(|id| id.as_str() == Some(&service.upstream_id))
                                    })
                            })
                            .map(|service| service.upstream_id.clone()),
                    )
                    .collect(),
                affected_scopes: Vec::new(),
                within_provider_scope: true,
                metadata: serde_json::json!({}),
                updates,
            })
        })
        .collect::<Vec<_>>();
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    incidents
}

fn affects_us(update: &Value) -> bool {
    let regions = update.pointer("/locationImpact/serviceRegions");
    regions.and_then(Value::as_array).is_none_or(|regions| {
        regions.is_empty()
            || regions
                .iter()
                .any(|region| region.as_str() == Some("Americas"))
    })
}

fn message_text(messages: &Value, token: Option<&Value>) -> String {
    token
        .and_then(Value::as_str)
        .and_then(|token| messages.get(token))
        .and_then(|message| message.get("textMessage"))
        .and_then(Value::as_str)
        .unwrap_or("Adobe Marketo Engage status event")
        .to_owned()
}

fn message_body(messages: &Value, token: Option<&Value>) -> String {
    // Adobe's plain-text alternative omits embedded link destinations.
    // ProviderContent sanitizes the HTML when displaying an update.
    token
        .and_then(Value::as_str)
        .and_then(|token| messages.get(token))
        .and_then(|message| message.get("htmlMessage"))
        .and_then(Value::as_str)
        .filter(|body| !body.trim().is_empty())
        .map_or_else(|| message_text(messages, token), ToOwned::to_owned)
}

fn unix_time(value: &str) -> Option<DateTime<Utc>> {
    value.parse().ok().and_then(DateTime::from_timestamp_secs)
}

fn lifecycle(
    status: &str,
    ended_at: Option<DateTime<Utc>>,
    observed_at: DateTime<Utc>,
) -> IncidentLifecycle {
    match status.to_ascii_lowercase().as_str() {
        "closed" | "completed" | "dismissed" | "cancelled" => IncidentLifecycle::Resolved,
        _ if ended_at.is_some_and(|ended_at| ended_at <= observed_at) => {
            IncidentLifecycle::Resolved
        }
        _ => IncidentLifecycle::Open,
    }
}
