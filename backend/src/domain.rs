use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedStatus {
    Operational,
    Maintenance,
    Degraded,
    PartialOutage,
    MajorOutage,
    Unknown,
}

impl NormalizedStatus {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Maintenance => "maintenance",
            Self::Degraded => "degraded",
            Self::PartialOutage => "partial_outage",
            Self::MajorOutage => "major_outage",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Operational => 0,
            Self::Maintenance => 1,
            Self::Degraded => 2,
            Self::PartialOutage => 3,
            Self::MajorOutage => 4,
            Self::Unknown => 5,
        }
    }

    #[must_use]
    pub fn from_provider(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "operational" | "none" | "ok" | "resolved" => Self::Operational,
            "under_maintenance" | "undermaintenance" | "maintenance" | "scheduled"
            | "in_progress" => Self::Maintenance,
            "degraded_performance" | "notice" | "degraded" => Self::Degraded,
            "partial_outage" | "partialoutage" | "incident" => Self::PartialOutage,
            "major_outage" | "majoroutage" | "full_outage" | "outage" | "critical" => {
                Self::MajorOutage
            }
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Delayed,
    Stale,
    NeverChecked,
    Failing,
}

pub(crate) fn stale_threshold(poll_interval: Duration, stale_multiplier: u32) -> Duration {
    const MINIMUM_STALE_THRESHOLD: Duration = Duration::from_secs(15 * 60);

    MINIMUM_STALE_THRESHOLD.max(poll_interval.saturating_mul(stale_multiplier))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Critical,
}

impl Severity {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Minor => "minor",
            Self::Major => "major",
            Self::Critical => "critical",
        }
    }

    #[must_use]
    pub fn from_impact(value: Option<&str>) -> Self {
        match value.unwrap_or_default().to_ascii_lowercase().as_str() {
            "critical" | "major_outage" | "outage" => Self::Critical,
            "major" | "partial_outage" | "incident" => Self::Major,
            "minor" | "degraded_performance" | "degraded" | "notice" => Self::Minor,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentKind {
    Incident,
    Maintenance,
}

impl IncidentKind {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Incident => "incident",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentLifecycle {
    Open,
    ResolutionPending,
    Resolved,
}

impl IncidentLifecycle {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ResolutionPending => "resolution_pending",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderComponent {
    pub upstream_id: String,
    pub name: String,
    pub group: Option<String>,
    pub description: Option<String>,
    pub status: NormalizedStatus,
    pub original_status: String,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUpdate {
    pub upstream_id: Option<String>,
    pub status: String,
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub display_at: Option<DateTime<Utc>>,
    pub affected_scopes: Vec<AffectedScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedScope {
    pub scope_type: String,
    pub upstream_id: String,
    pub display_name: String,
    pub normalized_status: Option<NormalizedStatus>,
    pub original_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderIncident {
    pub upstream_id: String,
    pub kind: IncidentKind,
    pub title: String,
    pub url: Option<String>,
    pub lifecycle: IncidentLifecycle,
    pub original_phase: String,
    pub severity: Severity,
    pub original_impact: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub monitoring_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub planned_start_at: Option<DateTime<Utc>>,
    pub planned_end_at: Option<DateTime<Utc>>,
    pub affected_components: Vec<String>,
    pub affected_scopes: Vec<AffectedScope>,
    pub within_provider_scope: bool,
    pub metadata: Value,
    pub updates: Vec<ProviderUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub status: u16,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub observed_at: DateTime<Utc>,
    pub overall: NormalizedStatus,
    pub original_overall: String,
    pub provider_overall: NormalizedStatus,
    pub provider_original_overall: String,
    pub components: Vec<ProviderComponent>,
    pub incidents: Vec<ProviderIncident>,
    pub maintenance: Vec<ProviderIncident>,
    pub active_incident_set_complete: bool,
    pub response: ResponseMetadata,
    pub raw_payload: Option<serde_json::Value>,
}

#[must_use]
pub fn rollup(statuses: impl IntoIterator<Item = NormalizedStatus>) -> NormalizedStatus {
    statuses
        .into_iter()
        .max_by_key(|status| status.rank())
        .unwrap_or(NormalizedStatus::Unknown)
}

pub fn semantic_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value)
        .expect("semantic hash inputs are internal JSON-compatible values");
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardProvider {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub official_url: String,
    pub tags: Vec<String>,
    pub status: NormalizedStatus,
    pub provider_status: NormalizedStatus,
    pub freshness: Freshness,
    pub last_success_at: Option<DateTime<Utc>>,
    pub active_incidents: i64,
    pub open_maintenance: i64,
    pub upcoming_maintenance: i64,
    pub outside_scope_incidents: i64,
    pub affected_components: Vec<String>,
}
