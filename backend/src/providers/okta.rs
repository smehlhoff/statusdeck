use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, conditional_request,
    conditional_response_metadata, http_client, response_body, response_content_type,
    retry_after_seconds,
};
use crate::domain::{
    IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderComponent, ProviderIncident,
    ProviderSnapshot, ProviderUpdate, ResponseMetadata, Severity, rollup,
};

#[derive(Default)]
pub struct OktaProvider;

const US_CELLS: &[(&str, &str)] = &[
    ("okta.com:1", "OK1 Cell"),
    ("okta.com:2", "OK2 Cell"),
    ("okta.com:3", "OK3 Cell"),
    ("okta.com:4", "OK4 Cell"),
    ("okta.com:6", "OK6 Cell"),
    ("okta.com:7", "OK7 Cell"),
    ("okta.com:8", "OK8 Cell"),
    ("okta.com:9", "OK9 Cell"),
    ("okta.com:11", "OK11 Cell"),
    ("okta.com:12", "OK12 Cell"),
    ("okta.com:14", "OK14 Cell"),
    ("okta.com:15", "OK15 Cell"),
    ("okta.com:16", "OK16 Cell"),
    ("okta.com:17", "OK17 Cell"),
    ("okta.com:18", "OK18 Cell"),
    ("okta.com:19", "OK19 Cell"),
    ("okta.com:20", "OK20 Cell"),
    ("okta.com:22", "OK22 Cell"),
    ("oktapreview.com:1", "OP1 Preview Cell"),
    ("oktapreview.com:2", "OP2 Preview Cell"),
    ("oktapreview.com:3", "OP3 Preview Cell"),
];
#[async_trait]
impl StatusProvider for OktaProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        let url = config
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Configuration("base_url is required".into()))?;
        let parsed = url::Url::parse(url)
            .map_err(|_| ProviderError::Configuration("base_url must be a URL".into()))?;
        if parsed.as_str() != "https://status.okta.com/" {
            return Err(ProviderError::Configuration(
                "Okta status must use the approved origin".into(),
            ));
        }
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let client = http_client(context)?;
        let request = client.get(context.base_url.clone());
        let response = conditional_request(request, context)
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let metadata = conditional_response_metadata(&response);
        let content_type = response_content_type(response.headers());
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
                content_type,
            });
        }
        if !response.status().is_success() {
            return Err(ProviderError::Http {
                status: metadata.status,
                content_type,
            });
        }
        if !content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("text/html"))
        {
            return Err(ProviderError::Parse(
                "Okta status response was not HTML".into(),
            ));
        }
        let body = response_body(response, 2 * 1024 * 1024).await?;
        let body =
            String::from_utf8(body).map_err(|error| ProviderError::Parse(error.to_string()))?;
        parse_page(
            &body,
            metadata.status,
            metadata.etag,
            metadata.last_modified,
            content_type,
        )
    }
}

fn parse_page(
    body: &str,
    status: u16,
    etag: Option<String>,
    last_modified: Option<String>,
    content_type: Option<String>,
) -> Result<FetchOutcome, ProviderError> {
    let values = embedded_array(body, "incidents")?;
    let planned = embedded_array(body, "planned-outages")?;
    if values.iter().chain(&planned).any(|value| {
        value
            .get("Id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    }) {
        return Err(ProviderError::Parse("Okta event omitted its ID".into()));
    }
    let update_values = embedded_array(body, "updates")?;
    let updates = updates_by_incident(&update_values);
    let mut incidents = parse_incidents(&values, IncidentKind::Incident, &updates);
    let mut maintenance = parse_incidents(&planned, IncidentKind::Maintenance, &updates);
    incidents.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));
    maintenance.sort_by(|left, right| left.upstream_id.cmp(&right.upstream_id));

    let mut component_statuses = HashMap::new();
    for (id, _) in US_CELLS {
        component_statuses.insert(*id, NormalizedStatus::Operational);
    }
    for incident in incidents
        .iter()
        .chain(&maintenance)
        .filter(|incident| affects_current_status(incident))
    {
        let status = incident_status(incident);
        let targets = if incident.affected_components.is_empty() {
            US_CELLS.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        } else {
            incident
                .affected_components
                .iter()
                .map(String::as_str)
                .collect()
        };
        for target in targets {
            if let Some(current) = component_statuses.get_mut(target)
                && status.rank() > current.rank()
            {
                *current = status;
            }
        }
    }
    let components = US_CELLS
        .iter()
        .enumerate()
        .map(|(position, (id, name))| {
            let status = component_statuses[id];
            ProviderComponent {
                upstream_id: (*id).to_owned(),
                name: (*name).to_owned(),
                group: Some("US".into()),
                description: None,
                status,
                original_status: status.key().to_owned(),
                position: i32::try_from(position).unwrap_or(i32::MAX),
            }
        })
        .collect::<Vec<_>>();
    let overall = rollup(components.iter().map(|component| component.status));
    Ok(FetchOutcome::Fetched(ProviderSnapshot {
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
            status,
            etag,
            last_modified,
            content_type,
        },
        raw_payload: Some(
            json!({"incidents": values, "planned_outages": planned, "updates": update_values}),
        ),
    }))
}

fn embedded_array(body: &str, id: &str) -> Result<Vec<Value>, ProviderError> {
    let marker = format!("data-id=\"{id}\">");
    let start = body
        .find(&marker)
        .map(|position| position + marker.len())
        .ok_or_else(|| ProviderError::Parse(format!("Okta page omitted {id}")))?;
    let end = body[start..]
        .find("</span>")
        .map(|position| start + position)
        .ok_or_else(|| ProviderError::Parse(format!("Okta page truncated {id}")))?;
    serde_json::from_str(&body[start..end])
        .map_err(|error| ProviderError::Parse(format!("invalid Okta {id}: {error}")))
}

fn updates_by_incident(values: &[Value]) -> HashMap<String, Vec<ProviderUpdate>> {
    let mut updates = HashMap::<String, Vec<ProviderUpdate>>::new();
    for value in values {
        let Some(incident_id) = value.get("IncidentId__c").and_then(Value::as_str) else {
            continue;
        };
        updates
            .entry(incident_id.to_owned())
            .or_default()
            .push(ProviderUpdate {
                upstream_id: value
                    .get("Id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                status: "update".into(),
                body: value
                    .get("UpdateLog__c")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                created_at: timestamp(value.get("CreatedDate")),
                updated_at: timestamp(value.get("CreatedDate")),
                display_at: None,
                affected_scopes: Vec::new(),
            });
    }
    for values in updates.values_mut() {
        values.sort_by_key(|value| value.created_at);
    }
    updates
}

fn parse_incidents(
    values: &[Value],
    kind: IncidentKind,
    updates: &HashMap<String, Vec<ProviderUpdate>>,
) -> Vec<ProviderIncident> {
    values
        .iter()
        .filter_map(|value| {
            let upstream_id = value.get("Id")?.as_str()?.to_owned();
            let impacted = value.get("Impacted_Cells__c").and_then(Value::as_str);
            let affected_components = impacted
                .into_iter()
                .flat_map(|cells| cells.split(';'))
                .filter(|cell| US_CELLS.iter().any(|(id, _)| id == cell))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if impacted.is_some_and(|cells| !cells.is_empty()) && affected_components.is_empty() {
                return None;
            }
            let original_phase = value
                .get("Status__c")
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_owned();
            let lifecycle = match original_phase.to_ascii_lowercase().as_str() {
                "resolved" | "completed" | "cancelled" => IncidentLifecycle::Resolved,
                "monitoring" => IncidentLifecycle::ResolutionPending,
                _ => IncidentLifecycle::Open,
            };
            let original_impact = value
                .get("Category__c")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let created_at = timestamp(
                value
                    .get("CreatedDate")
                    .or_else(|| value.get("Start_Time__c")),
            );
            let mut incident_updates = updates.get(&upstream_id).cloned().unwrap_or_default();
            if let Some(body) = value
                .get("Log__c")
                .and_then(Value::as_str)
                .filter(|body| !body.trim().is_empty())
                && !incident_updates.iter().any(|update| update.body == body)
            {
                incident_updates.push(ProviderUpdate {
                    upstream_id: Some(format!("initial:{upstream_id}")),
                    status: "update".into(),
                    body: body.to_owned(),
                    created_at,
                    updated_at: timestamp(value.get("LastModifiedDate")),
                    display_at: None,
                    affected_scopes: Vec::new(),
                });
                incident_updates.sort_by_key(|update| update.created_at);
            }
            let started_at = if kind == IncidentKind::Maintenance
                && original_phase.eq_ignore_ascii_case("scheduled")
            {
                None
            } else {
                timestamp(value.get("Start_Time__c"))
            };
            let mut incident = ProviderIncident {
                upstream_id: upstream_id.clone(),
                kind,
                title: value
                    .get("Incident_Title__c")
                    .or_else(|| value.get("Name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Okta status event")
                    .to_owned(),
                url: Some(format!("https://status.okta.com/#incident/{upstream_id}")),
                lifecycle,
                original_phase,
                severity: okta_severity(original_impact.as_deref()),
                original_impact,
                created_at,
                started_at,
                updated_at: timestamp(
                    value
                        .get("Last_Updated__c")
                        .or_else(|| value.get("LastModifiedDate")),
                ),
                monitoring_at: None,
                resolved_at: (lifecycle == IncidentLifecycle::Resolved)
                    // A date without a time cannot establish a recovery instant.
                    .then(|| super::parse_time(value.get("End_Date__c")))
                    .flatten(),
                planned_start_at: (kind == IncidentKind::Maintenance)
                    .then(|| {
                        timestamp(
                            value
                                .get("Start_Time__c")
                                .or_else(|| value.get("CreatedDate")),
                        )
                    })
                    .flatten(),
                planned_end_at: (kind == IncidentKind::Maintenance)
                    .then(|| timestamp(value.get("End_Date__c")))
                    .flatten(),
                affected_components,
                affected_scopes: Vec::new(),
                within_provider_scope: true,
                metadata: serde_json::json!({}),
                updates: incident_updates,
            };
            if kind == IncidentKind::Incident {
                apply_postmortem_timing(&mut incident, value);
            }
            Some(incident)
        })
        .collect()
}

// Only explicitly labeled, fully dated bounds with an unambiguous timezone
// establish impact timing. Date-only fields and duration totals do not.
fn apply_postmortem_timing(incident: &mut ProviderIncident, source: &Value) {
    if incident.lifecycle != IncidentLifecycle::Resolved || incident.resolved_at.is_some() {
        return;
    }
    for update in incident.updates.iter().rev() {
        let mut start = None;
        let mut end = None;
        let mut start_labels = 0;
        let mut end_labels = 0;
        for line in update.body.lines() {
            let Some((label, value)) = line.trim().split_once(':') else {
                continue;
            };
            match label.to_ascii_lowercase().as_str() {
                "incident start" | "start time" => {
                    start_labels += 1;
                    start = postmortem_time_with_date(value, source.get("Start_Date__c"));
                }
                "incident resolved" | "end time" => {
                    end_labels += 1;
                    end = postmortem_time_with_date(value, source.get("End_Date__c"));
                }
                _ => {}
            }
        }
        if start_labels != 1 || end_labels != 1 {
            continue;
        }
        if let Some((start, end)) = start.zip(end).filter(|(start, end)| end >= start) {
            incident.started_at = Some(start);
            incident.resolved_at = Some(end);
            incident.metadata["timing_basis"] = Value::String("postmortem_explicit_bounds".into());
            incident.metadata["timing_update_id"] = json!(update.upstream_id);
            return;
        }
    }
}

fn postmortem_time_with_date(value: &str, date: Option<&Value>) -> Option<DateTime<Utc>> {
    postmortem_time(value).or_else(|| {
        // A source date plus an explicit postmortem time is a complete bound.
        // Never borrow the start date for an end whose source date is missing.
        let date = NaiveDate::parse_from_str(date?.as_str()?, "%Y-%m-%d").ok()?;
        let value = format!("{} {}", date.format("%m/%d/%Y"), value.trim());
        postmortem_time(&value)
    })
}

fn postmortem_time(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim().trim_end_matches('.');
    if let Some(local) = value.strip_suffix(" PT") {
        let local = local.replace(", ", " ").replace(" at ", " ");
        let local = ["%B %e %Y %I:%M %p", "%B %e %Y %I:%M%p", "%m/%d/%Y %I:%M %p"]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(local.trim(), format).ok())?;
        // PT follows Pacific civil time; reject ambiguous and nonexistent DST times.
        return chrono_tz::America::Los_Angeles
            .from_local_datetime(&local)
            .single()
            .map(|time| time.with_timezone(&Utc))
            .filter(|time| time.timestamp() > 0);
    }
    let (local, offset) = if let Some(local) = value.strip_suffix("PDT") {
        (local, "-0700")
    } else {
        (value.strip_suffix("PST")?, "-0800")
    };
    let local = local.replace(", ", " ").replace(" at ", " ");
    let value = format!("{} {offset}", local.trim());
    [
        "%B %e %Y %I:%M %p %z",
        "%B %e %Y %I:%M%p %z",
        "%m/%d/%Y %I:%M %p %z",
    ]
    .iter()
    .find_map(|format| DateTime::parse_from_str(&value, format).ok())
    .map(|time| time.with_timezone(&Utc))
    .filter(|time| time.timestamp() > 0)
}

fn okta_severity(category: Option<&str>) -> Severity {
    match category.unwrap_or_default().to_ascii_lowercase().as_str() {
        value if value.contains("disruption") => Severity::Major,
        value if value.contains("degradation") => Severity::Minor,
        _ => Severity::Info,
    }
}

fn incident_status(incident: &ProviderIncident) -> NormalizedStatus {
    if incident.kind == IncidentKind::Maintenance {
        return NormalizedStatus::Maintenance;
    }
    match incident.severity {
        Severity::Critical | Severity::Major => NormalizedStatus::MajorOutage,
        Severity::Minor => NormalizedStatus::Degraded,
        Severity::Info => NormalizedStatus::PartialOutage,
    }
}

fn affects_current_status(incident: &ProviderIncident) -> bool {
    if incident.lifecycle == IncidentLifecycle::Resolved {
        return false;
    }
    incident.kind == IncidentKind::Incident
        || matches!(
            incident.original_phase.to_ascii_lowercase().as_str(),
            "started" | "in progress" | "in_progress" | "ongoing"
        )
}

fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let value = value.and_then(Value::as_str)?;
    value.parse().ok().or_else(|| {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)
            .map(|value| value.and_utc())
    })
}
