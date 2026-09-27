use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    FetchContext, FetchOutcome, ProviderError, StatusProvider, statuspage::StatuspageProvider,
};
use crate::domain::{ProviderSnapshot, rollup};

pub(crate) const SITES: [(&str, &str); 5] = [
    ("US1", "https://status.datadoghq.com"),
    ("US3", "https://status.us3.datadoghq.com"),
    ("US5", "https://status.us5.datadoghq.com"),
    ("US1-FED", "https://status.ddog-gov.com"),
    ("US2-FED", "https://status.us2.ddog-gov.com"),
];

#[derive(Default)]
pub struct DatadogProvider;

#[async_trait]
impl StatusProvider for DatadogProvider {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError> {
        if config.get("base_url").and_then(Value::as_str) != Some(SITES[0].1) {
            return Err(ProviderError::Configuration(
                "Datadog requires its approved US origin".into(),
            ));
        }
        Ok(())
    }

    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError> {
        let [us1, us3, us5, fed1, fed2] = SITES;
        // All sites must succeed: a partial inventory must not retire another site's components.
        let (mut combined, us3, us5, fed1, fed2) = tokio::try_join!(
            fetch_site(context, us1),
            fetch_site(context, us3),
            fetch_site(context, us5),
            fetch_site(context, fed1),
            fetch_site(context, fed2),
        )?;
        let mut evidence = json!({us1.0: combined.raw_payload.take()});
        for ((site, _), mut snapshot) in SITES.into_iter().skip(1).zip([us3, us5, fed1, fed2]) {
            evidence[site] = snapshot.raw_payload.take().unwrap_or(Value::Null);
            combined.observed_at = combined.observed_at.min(snapshot.observed_at);
            combined.provider_overall =
                rollup([combined.provider_overall, snapshot.provider_overall]);
            combined.active_incident_set_complete &= snapshot.active_incident_set_complete;
            combined.components.extend(snapshot.components);
            combined.incidents.extend(snapshot.incidents);
            combined.maintenance.extend(snapshot.maintenance);
        }
        for (position, component) in combined.components.iter_mut().enumerate() {
            component.position = i32::try_from(position).unwrap_or(i32::MAX);
        }
        combined.overall = rollup(combined.components.iter().map(|component| component.status));
        combined.original_overall = combined.overall.key().to_owned();
        combined.provider_original_overall = combined.provider_overall.key().to_owned();
        combined.response.etag = None;
        combined.response.last_modified = None;
        combined.raw_payload = Some(evidence);
        Ok(FetchOutcome::Fetched(combined))
    }
}

async fn fetch_site(
    context: &FetchContext,
    (site, origin): (&str, &str),
) -> Result<ProviderSnapshot, ProviderError> {
    let prefix = format!("{site}:");
    let site_context = FetchContext {
        base_url: origin
            .parse()
            .map_err(|error: url::ParseError| ProviderError::Configuration(error.to_string()))?,
        public_config: json!({"base_url": origin, "maintenance_path": "/api/v2/scheduled-maintenances.json"}),
        etag: None,
        last_modified: None,
        known_active_incidents: context
            .known_active_incidents
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix).map(str::to_owned))
            .collect(),
        refresh_history: context.refresh_history,
        deadline: context.deadline,
    };
    let FetchOutcome::Fetched(mut snapshot) =
        StatuspageProvider.fetch_snapshot(&site_context).await?
    else {
        return Err(ProviderError::Parse(format!(
            "Datadog {site} returned no complete snapshot"
        )));
    };
    if snapshot.components.is_empty() {
        return Err(ProviderError::Parse(format!(
            "Datadog {site} returned no components"
        )));
    }
    for component in &mut snapshot.components {
        component.upstream_id.insert_str(0, &prefix);
        component.group = Some(
            component
                .group
                .as_ref()
                .map_or_else(|| site.to_owned(), |group| format!("{site} / {group}")),
        );
    }
    for incident in snapshot
        .incidents
        .iter_mut()
        .chain(&mut snapshot.maintenance)
    {
        incident.upstream_id.insert_str(0, &prefix);
        incident.title = format!("[{site}] {}", incident.title);
        incident.metadata["site"] = json!(site);
        for id in &mut incident.affected_components {
            id.insert_str(0, &prefix);
        }
        for scope in incident.affected_scopes.iter_mut().chain(
            incident
                .updates
                .iter_mut()
                .flat_map(|update| &mut update.affected_scopes),
        ) {
            if scope.scope_type == "component" {
                scope.upstream_id.insert_str(0, &prefix);
            }
        }
    }
    Ok(snapshot)
}
