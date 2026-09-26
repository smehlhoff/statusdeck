use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    incidents::IncidentRow,
    monitors::MonitorRow,
    shared::{authenticated_user, freshness, parse_provider_ids},
};
use crate::domain::{Freshness, NormalizedStatus, rollup};

#[derive(Debug, Serialize, FromRow)]
pub(super) struct CatalogProviderRow {
    id: Uuid,
    slug: String,
    name: String,
    description: String,
    tags: Vec<String>,
    official_url: String,
    active: bool,
}

#[derive(Debug, FromRow)]
struct CatalogProviderDatabaseRow {
    id: Uuid,
    slug: String,
    name: String,
    description: String,
    tags: Vec<String>,
    official_url: String,
    active: bool,
    source_adapter: String,
    current_status: Option<String>,
    provider_status: Option<String>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    next_poll_at: DateTime<Utc>,
    consecutive_failures: i32,
    poll_interval_seconds: i32,
    recent_poll_error: Option<String>,
}

pub(super) async fn catalog_providers(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<CatalogProviderRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, CatalogProviderRow>(
        "SELECT id, slug, name, description, tags, official_url, active FROM providers WHERE active ORDER BY name",
    )
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogComponentsQuery {
    provider_ids: String,
}

#[derive(Debug, Serialize, FromRow)]
pub(super) struct CatalogComponentRow {
    id: Uuid,
    provider_id: Uuid,
    name: String,
    group: Option<String>,
    monitored: bool,
}

pub(super) async fn catalog_components(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<CatalogComponentsQuery>,
) -> Result<Json<Vec<CatalogComponentRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let provider_ids = parse_provider_ids(&query.provider_ids)?;
    let rows = sqlx::query_as::<_, CatalogComponentRow>(
        r#"
        SELECT component.id, component.provider_id, component.name,
               component.group_name AS "group",
               COALESCE(monitor.enabled AND (monitor.monitor_all_components OR EXISTS (
                   SELECT 1 FROM monitored_components selected
                   WHERE selected.monitored_provider_id = monitor.id AND selected.component_id = component.id
               )), false) AS monitored
        FROM components component
        JOIN providers provider ON provider.id = component.provider_id
        LEFT JOIN monitored_providers monitor ON monitor.provider_id = provider.id
        WHERE component.active AND provider.active AND component.provider_id = ANY($1)
        ORDER BY provider.name, component.position, component.name, component.id
        "#,
    )
    .bind(&provider_ids)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, FromRow)]
struct ComponentResponse {
    id: Uuid,
    upstream_id: String,
    name: String,
    description: Option<String>,
    group: Option<String>,
    position: i32,
    active: bool,
    status: Option<String>,
    selected: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct CatalogDetail {
    id: Uuid,
    slug: String,
    name: String,
    description: String,
    tags: Vec<String>,
    official_url: String,
    active: bool,
    source_adapter: String,
    current_status: Option<String>,
    provider_status: Option<String>,
    freshness: Freshness,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    recent_poll_error: Option<String>,
    monitor: Option<MonitorRow>,
    components: Vec<ComponentResponse>,
    active_incidents: Vec<IncidentRow>,
}

pub(super) async fn catalog_provider(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<CatalogDetail>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let provider = sqlx::query_as::<_, CatalogProviderDatabaseRow>(
        r#"
        SELECT s.id, s.slug, s.name, s.description, s.tags, s.official_url, s.active,
               ps.adapter AS source_adapter,
               sc.normalized_status AS current_status, sc.provider_normalized_status AS provider_status,
               ps.last_attempt_at, ps.last_success_at, ps.next_poll_at,
               ps.consecutive_failures, ps.poll_interval_seconds,
               ps.last_error AS recent_poll_error
        FROM providers s
        JOIN provider_sources ps ON ps.id = s.provider_source_id
        LEFT JOIN provider_status_current sc ON sc.provider_id = s.id
        WHERE s.id = $1 AND s.active
        "#,
    )
    .bind(id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("provider not found"))?;
    let monitor = sqlx::query_as::<_, MonitorRow>("SELECT id, provider_id, monitor_all_components, enabled FROM monitored_providers WHERE provider_id = $1")
        .bind(id).fetch_optional(&state.database.pool).await.map_err(ApiError::internal)?;
    let components = sqlx::query_as::<_, ComponentResponse>(
        r#"
        SELECT c.id, c.upstream_component_id AS upstream_id, c.name, c.description,
               c.group_name AS "group", c.position, c.active,
               current.normalized_status AS status,
               COALESCE(m.monitor_all_components OR selected.component_id IS NOT NULL, false) AS selected
        FROM components c
        LEFT JOIN component_status_current current ON current.component_id = c.id
        LEFT JOIN monitored_providers m ON m.provider_id = c.provider_id
        LEFT JOIN monitored_components selected
          ON selected.monitored_provider_id = m.id AND selected.component_id = c.id
        WHERE c.provider_id = $1
        ORDER BY c.position, c.name
        "#,
    )
    .bind(id)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    let current_status = {
        let selected_components_only = monitor
            .as_ref()
            .is_some_and(|monitor| !monitor.monitor_all_components);
        let selected_statuses = components
            .iter()
            .filter(|component| {
                component.selected && (component.active || selected_components_only)
            })
            .map(|component| {
                if component.active {
                    component
                        .status
                        .as_deref()
                        .map_or(NormalizedStatus::Unknown, NormalizedStatus::from_provider)
                } else {
                    NormalizedStatus::Unknown
                }
            })
            .collect::<Vec<_>>();
        if selected_statuses.is_empty() && !selected_components_only {
            provider.current_status.clone()
        } else {
            Some(rollup(selected_statuses).key().to_owned())
        }
    };
    let active_incidents = sqlx::query_as::<_, IncidentRow>("SELECT i.id, i.upstream_incident_id, i.title, EXISTS (SELECT 1 FROM incident_bookmarks b WHERE b.incident_id = i.id AND b.user_id = $2) AS bookmarked, i.lifecycle, i.severity, i.kind, i.original_phase, i.original_impact, i.provider_created_at, i.provider_started_at, i.provider_updated_at, i.provider_monitoring_at, i.provider_resolved_at, i.provider_metadata, CASE WHEN timing.end_at IS NOT NULL THEN floor(extract(epoch FROM timing.end_at - timing.start_at))::bigint END AS duration_seconds, timing.maintenance_uncertain, i.planned_start_at, i.planned_end_at, i.first_observed_at, i.official_url, i.last_observed_at, i.lifecycle_generation, i.within_provider_scope, COALESCE((SELECT jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name) ORDER BY provider.name, provider.id) FROM incident_providers incident_link JOIN providers provider ON provider.id = incident_link.provider_id WHERE incident_link.incident_id = i.id), '[]'::jsonb) AS providers FROM incidents i JOIN incident_timing timing ON timing.id = i.id JOIN incident_providers link ON link.incident_id = i.id WHERE link.provider_id = $1 AND i.lifecycle IN ('open', 'resolution_pending') AND i.within_provider_scope ORDER BY i.last_observed_at DESC, i.id DESC")
        .bind(id).bind(user.id).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let provider_freshness = freshness(
        provider.last_success_at,
        provider.last_attempt_at,
        provider.next_poll_at,
        provider.consecutive_failures,
        provider.poll_interval_seconds,
        &state.config,
    );
    Ok(Json(CatalogDetail {
        id: provider.id,
        slug: provider.slug,
        name: provider.name,
        description: provider.description,
        tags: provider.tags,
        official_url: provider.official_url,
        active: provider.active,
        source_adapter: provider.source_adapter,
        current_status,
        provider_status: provider.provider_status,
        freshness: provider_freshness,
        last_attempt_at: provider.last_attempt_at,
        last_success_at: provider.last_success_at,
        recent_poll_error: provider.recent_poll_error,
        monitor,
        components,
        active_incidents,
    }))
}
