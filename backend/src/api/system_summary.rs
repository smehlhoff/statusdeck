use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;

use super::{ApiError, AppState, shared::authenticated_user};

const HEARTBEAT_TOLERANCE_SECONDS: i64 = 120;
// Allow one worker lease for a due delivery before declaring dispatch overdue.
pub(super) const DELIVERY_GRACE_SECONDS: i64 = 120;

#[derive(Debug, Default, Deserialize)]
pub(super) enum SummaryWindow {
    #[serde(rename = "1h")]
    Hour,
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
}

impl SummaryWindow {
    pub(super) fn hours(&self) -> i64 {
        match self {
            Self::Hour => 1,
            Self::Day => 24,
            Self::Week => 168,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SummaryQuery {
    #[serde(default)]
    window: SummaryWindow,
}

#[derive(Serialize, FromRow)]
struct WorkerHeartbeat {
    role: String,
    instance_id: String,
    version: String,
    heartbeat_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    last_completed_at: Option<DateTime<Utc>>,
    fresh: bool,
    in_progress: i64,
    expired_claims: i64,
    oldest_started_at: Option<DateTime<Utc>>,
}

pub(super) async fn system_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Value>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let hours = query.window.hours();
    let now = Utc::now();
    let since = now - chrono::Duration::hours(hours);
    let probe_started = std::time::Instant::now();
    sqlx::query("SELECT 1")
        .execute(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let database_query_ms = probe_started.elapsed().as_secs_f64() * 1_000.0;
    let heartbeats = sqlx::query_as::<_, WorkerHeartbeat>(include_str!("system_workers.sql"))
        .bind(now)
        .bind(HEARTBEAT_TOLERANCE_SECONDS)
        .bind(crate::notifications::dispatcher::DELIVERY_LEASE_SECONDS)
        .fetch_all(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let polling = sqlx::query_scalar::<_, Value>(include_str!("system_polling.sql"))
        .bind(since)
        .bind(now)
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let deliveries = sqlx::query_scalar::<_, Value>(include_str!("system_deliveries.sql"))
        .bind(since)
        .bind(now)
        .bind(DELIVERY_GRACE_SECONDS)
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let database_metrics = sqlx::query_scalar::<_, Value>(include_str!("system_database.sql"))
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let open = state.database.pool.size();
    let idle = state.database.pool.num_idle();
    Ok(Json(json!({
        "observed_at": now,
        "window": {"hours": hours, "since": since, "until": now},
        "application_version": env!("CARGO_PKG_VERSION"),
        "api_uptime_seconds": state.started_at.elapsed().as_secs(),
        "database": "PostgreSQL",
        "database_query_ms": database_query_ms,
        "database_metrics": database_metrics,
        "database_connections": {
            "open": open, "idle": idle, "in_use": (open as usize).saturating_sub(idle),
            "limit": state.database.pool.options().get_max_connections(),
        },
        "migration_version": crate::db::expected_migration_version(),
        "migrations_current": state.database.migrations_current().await,
        "heartbeat_tolerance_seconds": HEARTBEAT_TOLERANCE_SECONDS,
        "delivery_grace_seconds": DELIVERY_GRACE_SECONDS,
        "worker_heartbeats": heartbeats,
        "polling": polling,
        "deliveries": deliveries,
    })))
}
