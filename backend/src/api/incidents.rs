use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{
        authenticated_user, decode_page_cursor, encode_page_cursor, parse_provider_ids,
        validate_optional_filter, validated_search,
    },
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IncidentQuery {
    q: Option<String>,
    provider_id: Option<Uuid>,
    provider_ids: Option<String>,
    component_id: Option<Uuid>,
    updated_within: Option<String>,
    affected_status: Option<String>,
    latest_phase: Option<String>,
    severity: Option<String>,
    lifecycle: Option<String>,
    activity: Option<String>,
    kind: Option<String>,
    scope: Option<String>,
    maintenance_window: Option<String>,
    upcoming_within: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    cursor: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
pub(super) struct IncidentRow {
    pub(super) id: Uuid,
    upstream_incident_id: String,
    title: String,
    bookmarked: bool,
    lifecycle: String,
    severity: String,
    kind: String,
    original_phase: String,
    original_impact: Option<String>,
    provider_created_at: Option<DateTime<Utc>>,
    provider_started_at: Option<DateTime<Utc>>,
    provider_updated_at: Option<DateTime<Utc>>,
    provider_monitoring_at: Option<DateTime<Utc>>,
    provider_resolved_at: Option<DateTime<Utc>>,
    provider_metadata: Value,
    duration_seconds: Option<i64>,
    maintenance_uncertain: bool,
    planned_start_at: Option<DateTime<Utc>>,
    planned_end_at: Option<DateTime<Utc>>,
    first_observed_at: DateTime<Utc>,
    pub(super) official_url: Option<String>,
    pub(super) last_observed_at: DateTime<Utc>,
    lifecycle_generation: i32,
    within_provider_scope: bool,
    providers: Value,
}

#[derive(Debug, Serialize, FromRow)]
struct IncidentUpdateResponse {
    id: Uuid,
    status: String,
    body: String,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    display_at: Option<DateTime<Utc>>,
    synthesized: bool,
    components: Value,
    scopes: Value,
}

#[derive(Debug, Serialize, FromRow)]
struct IncidentProviderResponse {
    id: Uuid,
    slug: String,
    name: String,
    official_url: String,
}

#[derive(Debug, Serialize, FromRow)]
struct IncidentComponentResponse {
    id: Uuid,
    provider_id: Uuid,
    name: String,
    normalized_status: Option<String>,
    original_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct IncidentDetailResponse {
    incident: IncidentRow,
    providers: Vec<IncidentProviderResponse>,
    components: Vec<IncidentComponentResponse>,
    scopes: Value,
    updates: Vec<IncidentUpdateResponse>,
}

#[derive(Debug, Serialize)]
pub(super) struct IncidentPage {
    items: Vec<IncidentRow>,
    next_cursor: Option<String>,
}

#[derive(FromRow)]
struct IncidentFeedRow {
    #[sqlx(flatten)]
    incident: IncidentRow,
    activity_at: DateTime<Utc>,
}

pub(super) async fn incidents(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<IncidentQuery>,
) -> Result<Json<IncidentPage>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    validate_optional_filter(
        "severity",
        query.severity.as_deref(),
        &["info", "minor", "major", "critical"],
    )?;
    validate_optional_filter(
        "lifecycle",
        query.lifecycle.as_deref(),
        &["open", "resolution_pending", "resolved"],
    )?;
    validate_optional_filter("activity", query.activity.as_deref(), &["active"])?;
    validate_optional_filter("kind", query.kind.as_deref(), &["incident", "maintenance"])?;
    validate_optional_filter("scope", query.scope.as_deref(), &["monitored", "all"])?;
    validate_optional_filter(
        "maintenance_window",
        query.maintenance_window.as_deref(),
        &["active", "upcoming"],
    )?;
    validate_optional_filter(
        "upcoming_within",
        query.upcoming_within.as_deref(),
        &["7d", "30d"],
    )?;
    validate_optional_filter(
        "updated_within",
        query.updated_within.as_deref(),
        &["24h", "7d", "30d"],
    )?;
    validate_optional_filter(
        "affected_status",
        query.affected_status.as_deref(),
        &[
            "operational",
            "maintenance",
            "degraded",
            "partial_outage",
            "major_outage",
            "unknown",
        ],
    )?;
    validate_optional_filter(
        "latest_phase",
        query.latest_phase.as_deref(),
        &["investigating", "identified", "monitoring", "resolved"],
    )?;
    if query.from.zip(query.to).is_some_and(|(from, to)| from > to) {
        return Err(ApiError::bad_request(
            "invalid_time_range",
            "The from timestamp must not be after the to timestamp.",
        ));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let (cursor_time, cursor_id) = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?
        .map_or((None, None), |(time, id)| (Some(time), Some(id)));
    let search = validated_search(query.q.as_deref())?;
    let provider_ids = query
        .provider_ids
        .as_deref()
        .map(parse_provider_ids)
        .transpose()?;
    let updated_since = match query.updated_within.as_deref() {
        Some("24h") => Some(Utc::now() - Duration::hours(24)),
        Some("7d") => Some(Utc::now() - Duration::days(7)),
        Some("30d") => Some(Utc::now() - Duration::days(30)),
        _ => None,
    };
    let upcoming_until = match query.upcoming_within.as_deref() {
        Some("7d") => Some(Utc::now() + Duration::days(7)),
        Some("30d") => Some(Utc::now() + Duration::days(30)),
        _ => None,
    };
    let scope = query.scope.as_deref().unwrap_or("monitored");
    let maintenance_window = query
        .maintenance_window
        .as_deref()
        .or_else(|| query.upcoming_within.as_ref().map(|_| "upcoming"));
    let rows = sqlx::query_as::<_, IncidentFeedRow>("
        SELECT i.id, i.upstream_incident_id, i.title, EXISTS (SELECT 1 FROM incident_bookmarks b WHERE b.incident_id = i.id AND b.user_id = $20) AS bookmarked, i.lifecycle, i.severity, i.kind, i.original_phase, i.original_impact,
               i.provider_created_at, i.provider_started_at, i.provider_updated_at, i.provider_monitoring_at, i.provider_resolved_at, i.provider_metadata, i.planned_start_at, i.planned_end_at,
               CASE WHEN timing.end_at IS NOT NULL THEN floor(extract(epoch FROM timing.end_at - timing.start_at))::bigint END AS duration_seconds, timing.maintenance_uncertain,
               i.first_observed_at, i.official_url, i.last_observed_at, i.lifecycle_generation, i.within_provider_scope, activity.activity_at,
               COALESCE((SELECT jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name) ORDER BY provider.name, provider.id) FROM incident_providers link JOIN providers provider ON provider.id = link.provider_id WHERE link.incident_id = i.id), '[]'::jsonb) AS providers
        FROM incidents i JOIN incident_timing timing ON timing.id = i.id
        CROSS JOIN LATERAL (
            SELECT GREATEST(i.provider_updated_at, i.provider_created_at,
                (SELECT max(GREATEST(u.provider_updated_at, u.provider_created_at, u.provider_display_at))
                 FROM incident_updates u WHERE u.incident_id = i.id AND NOT u.synthesized)) AS provider_activity_at
        ) published
        CROSS JOIN LATERAL (
            SELECT COALESCE(published.provider_activity_at, i.first_observed_at) AS activity_at
        ) activity
        WHERE EXISTS (SELECT 1 FROM incident_providers subscribed_link JOIN monitored_providers monitored ON monitored.provider_id = subscribed_link.provider_id AND monitored.enabled WHERE subscribed_link.incident_id = i.id)
          AND ($13 = 'all' OR (i.within_provider_scope AND EXISTS (
              SELECT 1 FROM incident_providers scoped_link
              JOIN monitored_providers monitored ON monitored.provider_id = scoped_link.provider_id AND monitored.enabled
              WHERE scoped_link.incident_id = i.id AND (
                  monitored.monitor_all_components
                  OR NOT EXISTS (SELECT 1 FROM incident_components affected WHERE affected.incident_id = i.id)
                  OR EXISTS (SELECT 1 FROM incident_components affected JOIN monitored_components selected ON selected.component_id = affected.component_id WHERE affected.incident_id = i.id AND selected.monitored_provider_id = monitored.id)
              )
          )))
          AND ($1::text IS NULL OR i.title ILIKE '%' || $1 || '%' OR i.upstream_incident_id ILIKE '%' || $1 || '%')
          AND ($2::uuid IS NULL OR EXISTS (SELECT 1 FROM incident_providers link WHERE link.incident_id = i.id AND link.provider_id = $2))
          AND ($3::uuid[] IS NULL OR EXISTS (SELECT 1 FROM incident_providers link WHERE link.incident_id = i.id AND link.provider_id = ANY($3)))
          AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM incident_components link WHERE link.incident_id = i.id AND link.component_id = $4))
          AND ($5::text IS NULL OR i.severity = $5)
          AND ($6::text IS NULL OR i.lifecycle = $6)
          AND ($7::text IS NULL OR i.kind = $7)
          AND ($8::timestamptz IS NULL OR timing.count_at >= $8 OR timing.end_at > $8)
          AND ($9::timestamptz IS NULL OR timing.count_at < $9)
          AND ($10::timestamptz IS NULL OR published.provider_activity_at >= $10)
          AND ($11::text IS NULL OR EXISTS (SELECT 1 FROM incident_components link JOIN component_status_current current ON current.component_id = link.component_id WHERE link.incident_id = i.id AND current.normalized_status = $11))
          AND ($12::text IS NULL OR lower(i.original_phase) = $12)
          AND ($14::text IS NULL OR (i.kind = 'maintenance' AND i.lifecycle IN ('open', 'resolution_pending') AND (
              ($14 = 'active' AND COALESCE(i.planned_start_at, i.first_observed_at) <= now() AND (i.planned_end_at IS NULL OR i.planned_end_at > now()))
              OR ($14 = 'upcoming' AND i.planned_start_at > now() AND ($15::timestamptz IS NULL OR i.planned_start_at <= $15))
          )))
          AND ($16::text IS NULL OR ($16 = 'active' AND i.lifecycle IN ('open', 'resolution_pending')))
          AND ($17::timestamptz IS NULL OR (activity.activity_at, i.id) < ($17, $18))
        ORDER BY activity.activity_at DESC, i.id DESC LIMIT $19
    ")
    .bind(search)
    .bind(query.provider_id)
    .bind(provider_ids)
    .bind(query.component_id)
    .bind(query.severity)
    .bind(query.lifecycle)
    .bind(query.kind)
    .bind(query.from)
    .bind(query.to)
    .bind(updated_since)
    .bind(query.affected_status)
    .bind(query.latest_phase)
    .bind(scope)
    .bind(maintenance_window)
    .bind(upcoming_until)
    .bind(query.activity)
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .bind(user.id)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    let mut items = rows;
    let has_more = usize::try_from(limit).is_ok_and(|limit| items.len() > limit);
    if has_more {
        items.pop();
    }
    let next_cursor = has_more
        .then(|| items.last().map(encode_incident_cursor))
        .flatten();
    Ok(Json(IncidentPage {
        items: items.into_iter().map(|row| row.incident).collect(),
        next_cursor,
    }))
}

fn encode_incident_cursor(row: &IncidentFeedRow) -> String {
    encode_page_cursor(row.activity_at, row.incident.id)
}

pub(super) async fn incident(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<IncidentDetailResponse>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let mut incident = sqlx::query_as::<_, IncidentRow>("SELECT incident.id, incident.upstream_incident_id, incident.title, EXISTS (SELECT 1 FROM incident_bookmarks b WHERE b.incident_id = incident.id AND b.user_id = $2) AS bookmarked, incident.lifecycle, incident.severity, incident.kind, incident.original_phase, incident.original_impact, incident.provider_created_at, incident.provider_started_at, incident.provider_updated_at, incident.provider_monitoring_at, incident.provider_resolved_at, incident.provider_metadata, CASE WHEN timing.end_at IS NOT NULL THEN floor(extract(epoch FROM timing.end_at - timing.start_at))::bigint END AS duration_seconds, timing.maintenance_uncertain, incident.planned_start_at, incident.planned_end_at, incident.first_observed_at, incident.official_url, incident.last_observed_at, incident.lifecycle_generation, incident.within_provider_scope, COALESCE((SELECT jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name) ORDER BY provider.name, provider.id) FROM incident_providers link JOIN providers provider ON provider.id = link.provider_id WHERE link.incident_id = incident.id), '[]'::jsonb) AS providers FROM incidents incident JOIN incident_timing timing ON timing.id = incident.id WHERE incident.id = $1").bind(id).bind(user.id).fetch_optional(&state.database.pool).await.map_err(ApiError::internal)?.ok_or_else(|| ApiError::not_found("incident not found"))?;
    let updates = sqlx::query_as::<_, IncidentUpdateResponse>("SELECT selected.id, selected.original_status AS status, selected.body, selected.provider_created_at AS created_at, selected.provider_updated_at AS updated_at, selected.provider_display_at AS display_at, selected.synthesized, COALESCE((SELECT jsonb_agg(jsonb_build_object('id', component.id, 'name', component.name) ORDER BY component.position, component.name) FROM incident_update_components snapshot JOIN components component ON component.id = snapshot.component_id WHERE snapshot.incident_update_id = selected.id), '[]'::jsonb) AS components, COALESCE((SELECT jsonb_agg(jsonb_build_object('type', scope.scope_type, 'upstream_id', scope.upstream_id, 'name', scope.display_name, 'normalized_status', scope.normalized_status, 'original_status', scope.original_status) ORDER BY scope.scope_type, scope.display_name) FROM incident_update_affected_scopes scope WHERE scope.incident_update_id = selected.id), '[]'::jsonb) AS scopes FROM (SELECT DISTINCT ON (provider_created_at, body) id, original_status, body, provider_created_at, provider_updated_at, provider_display_at, synthesized FROM incident_updates WHERE incident_id = $1 ORDER BY provider_created_at, body, CASE WHEN original_status = 'active' THEN 1 ELSE 0 END, id DESC) selected ORDER BY COALESCE(selected.provider_display_at, selected.provider_created_at) ASC NULLS LAST, selected.id ASC").bind(id).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let providers = sqlx::query_as::<_, IncidentProviderResponse>("SELECT s.id, s.slug, s.name, s.official_url FROM incident_providers link JOIN providers s ON s.id = link.provider_id WHERE link.incident_id = $1 ORDER BY s.name, s.id")
        .bind(id)
        .fetch_all(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let components = sqlx::query_as::<_, IncidentComponentResponse>("SELECT c.id, c.provider_id, c.name, current.normalized_status, current.original_status FROM incident_components link JOIN components c ON c.id = link.component_id LEFT JOIN component_status_current current ON current.component_id = c.id WHERE link.incident_id = $1 ORDER BY c.position, c.name, c.id")
        .bind(id)
        .fetch_all(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let scopes = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('type', scope.scope_type, 'upstream_id', scope.upstream_id, 'name', scope.display_name, 'normalized_status', scope.normalized_status, 'original_status', scope.original_status) ORDER BY scope.scope_type, scope.display_name), '[]'::jsonb) FROM incident_affected_scopes scope WHERE scope.incident_id = $1")
        .bind(id).fetch_one(&state.database.pool).await.map_err(ApiError::internal)?;
    if incident.official_url.as_deref().is_some_and(|url| {
        !is_valid_incident_link(
            url,
            providers
                .iter()
                .map(|provider| provider.official_url.as_str()),
        )
    }) {
        incident.official_url = None;
    }
    Ok(Json(IncidentDetailResponse {
        incident,
        providers,
        components,
        scopes,
        updates,
    }))
}

fn is_valid_incident_link<'a>(
    candidate: &str,
    provider_urls: impl IntoIterator<Item = &'a str>,
) -> bool {
    if candidate.len() > 1024 {
        return false;
    }
    let Ok(candidate) = url::Url::parse(candidate) else {
        return false;
    };
    if candidate.scheme() != "https"
        || candidate.host_str().is_none()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
    {
        return false;
    }
    candidate.host_str() == Some("stspg.io")
        || provider_urls.into_iter().any(|provider_url| {
            url::Url::parse(provider_url)
                .is_ok_and(|provider_url| provider_url.host_str() == candidate.host_str())
        })
}
