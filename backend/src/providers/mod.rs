pub mod adobe;
pub mod aws;
pub mod catalog;
pub mod datadog;
pub mod google_cloud;
pub mod intercom;
pub mod okta;
pub mod pagerduty;
pub mod salesforce;
pub mod slack;
pub mod statusio;
pub mod statuspage;

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::domain::{
    IncidentKind, IncidentLifecycle, NormalizedStatus, ProviderIncident, ProviderSnapshot, Severity,
};
pub(crate) use crate::retry_after::seconds as retry_after_seconds;

const HISTORY_LOOKBACK_DAYS: i64 = 365;

#[derive(Debug, Clone)]
pub struct FetchContext {
    pub base_url: Url,
    pub public_config: Value,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub known_active_incidents: HashSet<String>,
    pub refresh_history: bool,
    pub deadline: Instant,
}

#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Fetched(ProviderSnapshot),
    NotModified {
        status: u16,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

#[derive(Debug)]
struct ConditionalResponseMetadata {
    status: u16,
    etag: Option<String>,
    last_modified: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("rate limited")]
    RateLimited {
        retry_after_seconds: Option<u64>,
        content_type: Option<String>,
    },
    #[error("provider returned HTTP {status}")]
    Http {
        status: u16,
        content_type: Option<String>,
    },
    #[error("provider response was invalid: {0}")]
    Parse(String),
    #[error("provider response exceeded the configured size limit")]
    ResponseTooLarge,
    #[error("provider configuration is invalid: {0}")]
    Configuration(String),
}

impl ProviderError {
    #[must_use]
    pub const fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::RateLimited { .. } => true,
            Self::Http { status, .. } => *status == 408 || *status >= 500,
            Self::Parse(_) | Self::ResponseTooLarge | Self::Configuration(_) => false,
        }
    }

    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        match self {
            Self::RateLimited { .. } => Some(429),
            Self::Http { status, .. } => Some(*status),
            Self::Transport(_)
            | Self::Parse(_)
            | Self::ResponseTooLarge
            | Self::Configuration(_) => None,
        }
    }

    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Transport(_) => "transport",
            Self::RateLimited { .. } => "rate_limit",
            Self::Http { .. } => "http",
            Self::Parse(_) => "parse",
            Self::ResponseTooLarge => "response_too_large",
            Self::Configuration(_) => "configuration",
        }
    }

    #[must_use]
    pub fn response_content_type(&self) -> Option<&str> {
        match self {
            Self::RateLimited { content_type, .. } | Self::Http { content_type, .. } => {
                content_type.as_deref()
            }
            _ => None,
        }
    }
}

#[async_trait]
pub trait StatusProvider: Send + Sync {
    fn validate_config(&self, config: &Value) -> Result<(), ProviderError>;
    async fn fetch_snapshot(&self, context: &FetchContext) -> Result<FetchOutcome, ProviderError>;
}

pub fn registry(key: &str) -> Result<Box<dyn StatusProvider>, ProviderError> {
    match key {
        "adobe" => Ok(Box::new(adobe::AdobeProvider)),
        "aws" => Ok(Box::new(aws::AwsProvider)),
        "datadog" => Ok(Box::new(datadog::DatadogProvider)),
        "google_cloud" => Ok(Box::new(google_cloud::GoogleCloudProvider)),
        "intercom" => Ok(Box::new(intercom::IntercomProvider)),
        "okta" => Ok(Box::new(okta::OktaProvider)),
        "pagerduty" => Ok(Box::new(pagerduty::PagerDutyProvider)),
        "salesforce" => Ok(Box::new(salesforce::SalesforceProvider)),
        "statuspage" => Ok(Box::new(statuspage::StatuspageProvider)),
        "statusio" => Ok(Box::new(statusio::StatusIoProvider)),
        "slack" => Ok(Box::new(slack::SlackProvider)),
        other => Err(ProviderError::Configuration(format!(
            "unsupported adapter {other}"
        ))),
    }
}

#[must_use]
pub fn adapter_version(key: &str) -> &'static str {
    match key {
        "adobe" => "adobe-v2",
        "aws" => "aws-v2",
        "datadog" => "datadog-v1",
        "google_cloud" => "google-cloud-v1",
        "intercom" => "intercom-v1",
        "okta" => "okta-v1",
        "pagerduty" => "pagerduty-v1",
        "salesforce" => "salesforce-v1",
        "statuspage" => "statuspage-v1",
        "statusio" => "statusio-v1",
        "slack" => "slack-v1",
        _ => "unknown",
    }
}

fn endpoint(base_url: &Url, path: &str) -> Result<Url, ProviderError> {
    base_url
        .join(path)
        .map_err(|error| ProviderError::Configuration(error.to_string()))
}

fn parse_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|text| text.parse().ok())
        .filter(|time: &DateTime<Utc>| time.timestamp() > 0)
}

/// Discard invalid duration bounds without inventing a start or recovery time.
pub(crate) fn normalize_incident_timestamps(
    incident: &mut crate::domain::ProviderIncident,
    adapter: &str,
) {
    for time in [
        &mut incident.created_at,
        &mut incident.started_at,
        &mut incident.updated_at,
        &mut incident.monitoring_at,
        &mut incident.resolved_at,
        &mut incident.planned_start_at,
        &mut incident.planned_end_at,
    ] {
        *time = time.filter(|value| value.timestamp() > 0);
    }
    if incident
        .started_at
        .zip(incident.resolved_at)
        .is_some_and(|(start, end)| end < start)
    {
        incident.resolved_at = None;
    }
    if incident.resolved_at.is_none()
        && incident.lifecycle == crate::domain::IncidentLifecycle::Resolved
        && matches!(adapter, "statuspage" | "datadog" | "adobe")
    {
        incident.resolved_at = incident
            .updates
            .iter()
            .filter(|update| {
                matches!(
                    update.status.to_ascii_lowercase().as_str(),
                    "resolved" | "completed" | "closed"
                )
            })
            .filter_map(|update| update.display_at.or(update.created_at))
            .filter(|end| {
                end.timestamp() > 0 && incident.started_at.is_none_or(|start| *end >= start)
            })
            .min();
    }
    if incident
        .planned_start_at
        .zip(incident.planned_end_at)
        .is_some_and(|(start, end)| end < start)
    {
        incident.planned_end_at = None;
    }
    if incident.lifecycle == crate::domain::IncidentLifecycle::Resolved {
        incident.metadata["_statusdeck_history"] = Value::Bool(true);
    }
}

/// Limit new historical imports, retaining open events and known active IDs for reconciliation.
/// Recovery takes precedence so incidents crossing the cutoff retain their full timeline.
/// An undated resolved event cannot establish that it belongs in the import window.
pub(crate) fn limit_history(snapshot: &mut ProviderSnapshot, context: &FetchContext) {
    let cutoff = snapshot.observed_at - chrono::Duration::days(HISTORY_LOOKBACK_DAYS);
    for incidents in [&mut snapshot.incidents, &mut snapshot.maintenance] {
        incidents.retain(|incident| {
            incident.lifecycle != crate::domain::IncidentLifecycle::Resolved
                || context
                    .known_active_incidents
                    .contains(&incident.upstream_id)
                || incident
                    .resolved_at
                    .or(incident.planned_end_at)
                    .or(incident.started_at)
                    .or(incident.planned_start_at)
                    .or(incident.created_at)
                    .is_some_and(|time| time >= cutoff)
        });
    }
}

fn response_content_type(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(128).collect())
}

fn http_client(context: &FetchContext) -> Result<reqwest::Client, ProviderError> {
    let remaining = context.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ProviderError::Transport(
            "provider request deadline elapsed".into(),
        ));
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(remaining.min(Duration::from_secs(5)))
        .timeout(remaining)
        .build()
        .map_err(|error| ProviderError::Transport(error.to_string()))
}

fn conditional_request(
    mut request: reqwest::RequestBuilder,
    context: &FetchContext,
) -> reqwest::RequestBuilder {
    // Active incidents need full observations for absence confirmation, detail
    // refreshes, and time-dependent transitions even when the feed is unchanged.
    if context.refresh_history || !context.known_active_incidents.is_empty() {
        return request;
    }
    if let Some(etag) = &context.etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    if let Some(last_modified) = &context.last_modified {
        request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
    }
    request
}

fn conditional_response_metadata(response: &reqwest::Response) -> ConditionalResponseMetadata {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
    };
    ConditionalResponseMetadata {
        status: response.status().as_u16(),
        etag: header(reqwest::header::ETAG),
        last_modified: header(reqwest::header::LAST_MODIFIED),
    }
}

async fn json_body(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ProviderError> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !(content_type == "application/json" || content_type.ends_with("+json")) {
        return Err(ProviderError::Parse(
            "provider response did not use a JSON content type".into(),
        ));
    }
    response_body(response, maximum_bytes).await
}

async fn response_body(
    mut response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ProviderError> {
    let maximum_bytes_u64 = u64::try_from(maximum_bytes).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes_u64)
    {
        return Err(ProviderError::ResponseTooLarge);
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(maximum_bytes),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(ProviderError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch_json(
    client: &reqwest::Client,
    url: Url,
    maximum_bytes: usize,
) -> Result<Value, ProviderError> {
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
    let body = json_body(response, maximum_bytes).await?;
    serde_json::from_slice(&body).map_err(|error| ProviderError::Parse(error.to_string()))
}

// Adobe and Okta derive component health from incident severity and active maintenance.
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
