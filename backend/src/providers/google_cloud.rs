use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
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
pub struct GoogleCloudProvider;

#[async_trait]
impl StatusProvider for GoogleCloudProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        if config.get("base_url").and_then(Value::as_str) != Some("https://status.cloud.google.com")
        {
            return Err(ProviderError::Configuration(
                "Google Cloud status must use the approved origin".into(),
            ));
        }
        configured_strings(config, "component_ids")?;
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let products_url = endpoint(&context.base_url, "/products.json")?;
        let incidents_url = endpoint(&context.base_url, "/incidents.json")?;
        let (products, incidents) = tokio::try_join!(
            fetch_json(&client, products_url, 1024 * 1024),
            fetch_json(&client, incidents_url, 4 * 1024 * 1024),
        )?;
        let mut snapshot = parse_snapshot(products, incidents, &context.known_active_incidents)?;
        apply_component_scope(&mut snapshot, &context.public_config)?;
        Ok(FetchOutcome::Fetched(snapshot))
    }
}

fn parse_snapshot(
    products_root: Value,
    incidents_root: Value,
    known_active: &HashSet<String>,
) -> Result<ProviderSnapshot, ProviderError> {
    let products = products_root
        .get("products")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("Google response omitted products".into()))?;
    let incidents = incidents_root
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Google response did not contain incidents".into()))?;
    let active = incidents
        .iter()
        .filter(|incident| incident.get("end").is_none_or(Value::is_null))
        .collect::<Vec<_>>();
    let components = products
        .iter()
        .enumerate()
        .map(|(position, product)| {
            let upstream_id = product
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ProviderError::Parse("Google product omitted its ID".into()))?
                .to_owned();
            let status = active
                .iter()
                .filter(|incident| affects_product(incident, &upstream_id))
                .map(|incident| google_status(latest_status(incident)))
                .max_by_key(|status| status.rank())
                .unwrap_or(NormalizedStatus::Operational);
            Ok(ProviderComponent {
                upstream_id,
                name: product
                    .get("current_title")
                    .or_else(|| product.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed Google Cloud product")
                    .to_owned(),
                group: None,
                description: None,
                status,
                original_status: status.key().to_owned(),
                position: i32::try_from(position).unwrap_or(i32::MAX),
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    if components.is_empty() {
        return Err(ProviderError::Parse(
            "Google response contained no products".into(),
        ));
    }
    let incidents = incidents
        .iter()
        .filter(|incident| {
            incident.get("end").is_none_or(Value::is_null)
                || parse_time(incident.get("end")).is_some_and(|end| {
                    end >= Utc::now() - chrono::Duration::days(super::HISTORY_LOOKBACK_DAYS)
                })
                || incident
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| known_active.contains(id))
        })
        .map(|value| {
            parse_incident(value)
                .ok_or_else(|| ProviderError::Parse("google_cloud incident omitted its ID".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
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
            content_type: Some("application/json".into()),
        },
        raw_payload: Some(json!({
            "products": products_root,
            "incidents": incidents_root,
        })),
    })
}

fn affects_product(incident: &Value, product_id: &str) -> bool {
    incident
        .get("affected_products")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|product| product.get("id").and_then(Value::as_str) == Some(product_id))
}

fn latest_status(incident: &Value) -> &str {
    incident
        .get("most_recent_update")
        .and_then(|update| update.get("status"))
        .or_else(|| {
            incident
                .get("updates")
                .and_then(Value::as_array)
                .and_then(|updates| updates.first())
                .and_then(|update| update.get("status"))
        })
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
}

fn google_status(value: &str) -> NormalizedStatus {
    match value {
        "AVAILABLE" => NormalizedStatus::Operational,
        "SERVICE_INFORMATION" => NormalizedStatus::Degraded,
        "SERVICE_DISRUPTION" => NormalizedStatus::PartialOutage,
        "SERVICE_OUTAGE" => NormalizedStatus::MajorOutage,
        _ => NormalizedStatus::Unknown,
    }
}

fn parse_incident(value: &Value) -> Option<ProviderIncident> {
    let upstream_id = value.get("id")?.as_str()?.to_owned();
    let lifecycle = if value.get("end").is_none_or(Value::is_null) {
        IncidentLifecycle::Open
    } else {
        IncidentLifecycle::Resolved
    };
    let original_phase = if lifecycle == IncidentLifecycle::Resolved {
        "resolved".to_owned()
    } else {
        latest_status(value).to_owned()
    };
    let updates = value
        .get("updates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|update| ProviderUpdate {
            // Google prepends updates; array positions are not stable identities.
            upstream_id: parse_time(update.get("created").or_else(|| update.get("when")))
                .map(|created| format!("{upstream_id}:{}", created.to_rfc3339())),
            status: update
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN")
                .to_owned(),
            body: update
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at: parse_time(update.get("when").or_else(|| update.get("created"))),
            updated_at: parse_time(update.get("modified")),
            display_at: None,
            affected_scopes: Vec::new(),
        })
        .collect();
    let original_impact = value
        .get("severity")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Some(ProviderIncident {
        upstream_id,
        kind: IncidentKind::Incident,
        title: value
            .get("external_desc")
            .or_else(|| value.get("service_name"))
            .and_then(Value::as_str)
            .unwrap_or("Google Cloud incident")
            .to_owned(),
        url: value.get("uri").and_then(Value::as_str).map(|path| {
            if path.starts_with("https://") {
                path.to_owned()
            } else {
                format!("https://status.cloud.google.com{path}")
            }
        }),
        lifecycle,
        original_phase,
        severity: match original_impact.as_deref() {
            Some("high") => Severity::Critical,
            Some("medium") => Severity::Major,
            Some("low") => Severity::Minor,
            _ => Severity::Info,
        },
        original_impact,
        created_at: parse_time(value.get("created").or_else(|| value.get("begin"))),
        started_at: parse_time(value.get("begin")),
        updated_at: parse_time(value.get("modified")),
        monitoring_at: None,
        resolved_at: parse_time(value.get("end")),
        planned_start_at: None,
        planned_end_at: None,
        affected_components: value
            .get("affected_products")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|product| product.get("id").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .collect(),
        affected_scopes: Vec::new(),
        within_provider_scope: true,
        metadata: serde_json::json!({"_statusdeck_history": lifecycle == IncidentLifecycle::Resolved}),
        updates,
    })
}
