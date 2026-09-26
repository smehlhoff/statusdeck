use std::collections::HashMap;

use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, freshness},
};
use crate::domain::{DashboardProvider, Freshness, NormalizedStatus, rollup};

#[derive(Debug, Serialize)]
pub(super) struct DashboardResponse {
    counts: Value,
    providers: Vec<DashboardProvider>,
}

pub(super) async fn dashboard(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<DashboardResponse>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, String, Vec<String>, Option<String>, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, i32, i32, i64, i64, i64, i64)>("
        WITH scoped_incidents AS MATERIALIZED (
            SELECT ins.provider_id, i.kind, i.planned_start_at, i.planned_end_at, i.first_observed_at,
                   i.within_provider_scope AND (
                       m.monitor_all_components
                       OR NOT EXISTS (SELECT 1 FROM incident_components affected WHERE affected.incident_id = i.id)
                       OR EXISTS (
                           SELECT 1 FROM incident_components affected
                           JOIN monitored_components selected ON selected.component_id = affected.component_id
                           WHERE affected.incident_id = i.id AND selected.monitored_provider_id = m.id
                       )
                   ) AS within_scope
            FROM incident_providers ins
            JOIN incidents i ON i.id = ins.incident_id AND i.lifecycle IN ('open', 'resolution_pending')
            JOIN monitored_providers m ON m.provider_id = ins.provider_id AND m.enabled
        ), incident_counts AS (
            SELECT provider_id,
                   count(*) FILTER (WHERE within_scope AND kind = 'incident') AS active_incidents,
                   count(*) FILTER (WHERE within_scope AND kind = 'maintenance' AND COALESCE(planned_start_at, first_observed_at) <= now() AND (planned_end_at IS NULL OR planned_end_at > now())) AS open_maintenance,
                   count(*) FILTER (WHERE within_scope AND kind = 'maintenance' AND planned_start_at > now()) AS upcoming_maintenance,
                   count(*) FILTER (WHERE NOT within_scope) AS outside_scope_incidents
            FROM scoped_incidents GROUP BY provider_id
        )
        SELECT s.id, s.slug, s.name, s.official_url, s.tags, sc.normalized_status, sc.provider_normalized_status, ps.last_success_at, ps.last_attempt_at, ps.next_poll_at,
               ps.consecutive_failures, ps.poll_interval_seconds,
               COALESCE(counts.active_incidents, 0), COALESCE(counts.open_maintenance, 0),
               COALESCE(counts.upcoming_maintenance, 0), COALESCE(counts.outside_scope_incidents, 0)
        FROM monitored_providers m JOIN providers s ON s.id = m.provider_id
        JOIN provider_sources ps ON ps.id = s.provider_source_id
        LEFT JOIN provider_status_current sc ON sc.provider_id = s.id
        LEFT JOIN incident_counts counts ON counts.provider_id = s.id
        WHERE m.enabled AND s.active
        ORDER BY s.name
    ").fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let component_state = sqlx::query_as::<_, (Uuid, Vec<String>, Vec<String>)>("
        SELECT m.provider_id,
               COALESCE(array_agg(CASE WHEN c.active THEN COALESCE(csc.normalized_status, 'unknown') ELSE 'unknown' END ORDER BY c.position) FILTER (WHERE c.id IS NOT NULL), ARRAY[]::text[]),
               COALESCE(array_agg(c.name ORDER BY c.position) FILTER (WHERE c.id IS NOT NULL AND (NOT c.active OR COALESCE(csc.normalized_status, 'unknown') <> 'operational')), ARRAY[]::text[])
        FROM monitored_providers m
        JOIN providers s ON s.id = m.provider_id
        LEFT JOIN components c ON c.provider_id = s.id AND (c.active OR NOT m.monitor_all_components)
        LEFT JOIN monitored_components mc ON mc.monitored_provider_id = m.id AND mc.component_id = c.id
        LEFT JOIN component_status_current csc ON csc.component_id = c.id
        WHERE m.enabled AND s.active AND (m.monitor_all_components OR mc.component_id IS NOT NULL)
        GROUP BY m.provider_id
    ").fetch_all(&state.database.pool).await.map_err(ApiError::internal)?.into_iter().map(|(provider_id, statuses, affected)| (provider_id, (statuses, affected))).collect::<HashMap<_, _>>();
    let mut providers = rows
        .into_iter()
        .map(
            |(
                id,
                slug,
                name,
                official_url,
                tags,
                status,
                provider_status,
                last_success,
                last_attempt,
                next_poll_at,
                failures,
                poll_interval_seconds,
                active_incidents,
                open_maintenance,
                upcoming_maintenance,
                outside_scope_incidents,
            )| {
                let freshness = freshness(
                    last_success,
                    last_attempt,
                    next_poll_at,
                    failures,
                    poll_interval_seconds,
                    &state.config,
                );
                let scoped_provider_status = status
                    .as_deref()
                    .map_or(NormalizedStatus::Unknown, NormalizedStatus::from_provider);
                let (status, affected_components) = component_state
                    .get(&id)
                    .map(|(statuses, affected)| {
                        let status = if statuses.is_empty() {
                            scoped_provider_status
                        } else {
                            rollup(
                                statuses
                                    .iter()
                                    .map(|value| NormalizedStatus::from_provider(value)),
                            )
                        };
                        (status, affected.clone())
                    })
                    .unwrap_or((scoped_provider_status, Vec::new()));
                let provider_status = provider_status
                    .as_deref()
                    .map_or(NormalizedStatus::Unknown, NormalizedStatus::from_provider);
                DashboardProvider {
                    id,
                    slug,
                    name,
                    official_url,
                    tags,
                    status,
                    provider_status,
                    freshness,
                    last_success_at: last_success,
                    active_incidents,
                    open_maintenance,
                    upcoming_maintenance,
                    outside_scope_incidents,
                    affected_components,
                }
            },
        )
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| {
        dashboard_status_rank(right.status)
            .cmp(&dashboard_status_rank(left.status))
            .then_with(|| {
                dashboard_freshness_rank(right.freshness)
                    .cmp(&dashboard_freshness_rank(left.freshness))
            })
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut counts = serde_json::Map::new();
    for status in [
        NormalizedStatus::MajorOutage,
        NormalizedStatus::PartialOutage,
        NormalizedStatus::Degraded,
        NormalizedStatus::Maintenance,
        NormalizedStatus::Operational,
        NormalizedStatus::Unknown,
    ] {
        counts.insert(
            status.key().to_owned(),
            json!(
                providers
                    .iter()
                    .filter(|provider| provider.status == status)
                    .count()
            ),
        );
    }
    counts.insert(
        "stale".into(),
        json!(
            providers
                .iter()
                .filter(|provider| matches!(
                    provider.freshness,
                    Freshness::Stale | Freshness::Failing | Freshness::NeverChecked
                ))
                .count()
        ),
    );
    Ok(Json(DashboardResponse {
        counts: Value::Object(counts),
        providers,
    }))
}

const fn dashboard_status_rank(status: NormalizedStatus) -> u8 {
    match status {
        NormalizedStatus::MajorOutage => 5,
        NormalizedStatus::PartialOutage => 4,
        NormalizedStatus::Degraded => 3,
        NormalizedStatus::Maintenance => 2,
        NormalizedStatus::Unknown => 1,
        NormalizedStatus::Operational => 0,
    }
}

const fn dashboard_freshness_rank(freshness: Freshness) -> u8 {
    match freshness {
        Freshness::Failing => 4,
        Freshness::Stale => 3,
        Freshness::NeverChecked => 2,
        Freshness::Delayed => 1,
        Freshness::Fresh => 0,
    }
}
