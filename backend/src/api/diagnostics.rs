use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::system_summary::{DELIVERY_GRACE_SECONDS, SummaryWindow};
use super::{
    ApiError, AppState,
    shared::{
        authenticated_user, decode_page_cursor, encode_page_cursor, freshness,
        validate_optional_filter,
    },
};
use crate::domain::Freshness;

#[derive(Debug, Serialize, FromRow)]
pub(super) struct RecordCount {
    record_type: String,
    count: i64,
}

pub(super) async fn data_metrics(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<RecordCount>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let records = sqlx::query_as::<_, RecordCount>(
        "SELECT 'incidents' AS record_type, count(*) FROM incidents WHERE kind = 'incident'
         UNION ALL SELECT 'maintenance', count(*) FROM incidents WHERE kind = 'maintenance'
         UNION ALL SELECT 'incident_updates', count(*) FROM incident_updates
         UNION ALL SELECT 'incident_comments', count(*) FROM incident_comments
         UNION ALL SELECT 'provider_status_changes', count(*) FROM status_changes WHERE provider_id IS NOT NULL
         UNION ALL SELECT 'component_status_changes', count(*) FROM status_changes WHERE component_id IS NOT NULL
         UNION ALL SELECT 'poll_runs', count(*) FROM poll_runs
         UNION ALL SELECT 'poll_payloads', count(*) FROM poll_payloads
         UNION ALL SELECT 'notification_events', count(*) FROM notification_events
         UNION ALL SELECT 'notification_deliveries', count(*) FROM notification_deliveries
         UNION ALL SELECT 'notification_attempts', count(*) FROM notification_attempts
         ORDER BY record_type",
    )
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(records))
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceProviderRow {
    id: Uuid,
    slug: String,
    name: String,
}

#[derive(Debug, FromRow)]
struct SourceDatabaseRow {
    id: Uuid,
    source_key: String,
    adapter: String,
    enabled: bool,
    next_poll_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    consecutive_failures: i32,
    poll_interval_seconds: i32,
    last_error: Option<String>,
    stale_episode_started_at: Option<DateTime<Utc>>,
    affected_providers: sqlx::types::Json<Vec<SourceProviderRow>>,
    last_poll_outcome: Option<String>,
    last_poll_duration_ms: Option<i32>,
    last_poll_http_status: Option<i32>,
    last_poll_adapter_version: Option<String>,
    last_poll_error_class: Option<String>,
    status_checks_24h: i64,
    status_failures_24h: i64,
}

#[derive(Debug, Serialize)]
pub(super) struct SourceRow {
    id: Uuid,
    source_key: String,
    adapter: String,
    enabled: bool,
    next_poll_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    consecutive_failures: i32,
    last_error: Option<String>,
    stale_episode_started_at: Option<DateTime<Utc>>,
    affected_providers: Vec<SourceProviderRow>,
    last_poll_outcome: Option<String>,
    last_poll_duration_ms: Option<i32>,
    last_poll_http_status: Option<i32>,
    last_poll_adapter_version: Option<String>,
    last_poll_error_class: Option<String>,
    status_checks_24h: i64,
    status_failures_24h: i64,
    freshness: Freshness,
}

pub(super) async fn sources(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<SourceRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, SourceDatabaseRow>(
        r#"SELECT source.id, source.source_key, source.adapter, source.enabled, source.next_poll_at,
                  source.last_attempt_at, source.last_success_at, source.consecutive_failures,
                  source.poll_interval_seconds, source.last_error, source.stale_episode_started_at,
                  COALESCE((SELECT jsonb_agg(jsonb_build_object('id', provider.id, 'slug', provider.slug, 'name', provider.name) ORDER BY provider.name, provider.id)
                            FROM providers provider WHERE provider.provider_source_id = source.id AND provider.active), '[]'::jsonb) AS affected_providers,
                  recent.outcome AS last_poll_outcome, recent.duration_ms AS last_poll_duration_ms,
                  recent.http_status AS last_poll_http_status, recent.adapter_version AS last_poll_adapter_version,
                  recent.error_class AS last_poll_error_class,
                  checks.status_checks_24h, checks.status_failures_24h
           FROM provider_sources source
           LEFT JOIN LATERAL (
               SELECT run.outcome, run.duration_ms, run.http_status, run.adapter_version, run.error_class
               FROM poll_runs run
               WHERE run.provider_source_id = source.id AND run.poll_kind = 'status'
               ORDER BY run.started_at DESC, run.id DESC LIMIT 1
           ) recent ON true
           LEFT JOIN LATERAL (
               SELECT count(*) AS status_checks_24h,
                      count(*) FILTER (WHERE run.outcome = 'failure') AS status_failures_24h
               FROM poll_runs run
               WHERE run.provider_source_id = source.id
                 AND run.poll_kind = 'status'
                 AND run.started_at >= now() - interval '24 hours'
                 AND run.started_at <= now()
                 AND run.finished_at IS NOT NULL
                 AND run.outcome IN ('success', 'not_modified', 'failure')
           ) checks ON true
           ORDER BY source.source_key"#,
    ).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| SourceRow {
                id: row.id,
                source_key: row.source_key,
                adapter: row.adapter,
                enabled: row.enabled,
                next_poll_at: row.next_poll_at,
                last_attempt_at: row.last_attempt_at,
                last_success_at: row.last_success_at,
                consecutive_failures: row.consecutive_failures,
                last_error: row.last_error,
                stale_episode_started_at: row.stale_episode_started_at,
                affected_providers: row.affected_providers.0,
                last_poll_outcome: row.last_poll_outcome,
                last_poll_duration_ms: row.last_poll_duration_ms,
                last_poll_http_status: row.last_poll_http_status,
                last_poll_adapter_version: row.last_poll_adapter_version,
                last_poll_error_class: row.last_poll_error_class,
                status_checks_24h: row.status_checks_24h,
                status_failures_24h: row.status_failures_24h,
                freshness: freshness(
                    row.last_success_at,
                    row.last_attempt_at,
                    row.next_poll_at,
                    row.consecutive_failures,
                    row.poll_interval_seconds,
                    &state.config,
                ),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize, FromRow)]
struct PollRunRow {
    id: Uuid,
    provider_source_id: Uuid,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    outcome: String,
    http_status: Option<i32>,
    duration_ms: Option<i32>,
    component_count: i32,
    incident_count: i32,
    error_message: Option<String>,
    error_class: Option<String>,
    adapter_version: String,
    poll_kind: String,
}

#[derive(Debug, Serialize)]
pub(super) struct PollRunPage {
    items: Vec<PollRunRow>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PollRunQuery {
    source_id: Option<Uuid>,
    outcome: Option<String>,
    started_after: Option<DateTime<Utc>>,
    started_before: Option<DateTime<Utc>>,
    cursor: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn poll_runs(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<PollRunQuery>,
) -> Result<Json<PollRunPage>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    validate_optional_filter(
        "outcome",
        query.outcome.as_deref(),
        &["running", "success", "not_modified", "failure"],
    )?;
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?;
    if query
        .started_after
        .zip(query.started_before)
        .is_some_and(|(after, before)| after > before)
    {
        return Err(ApiError::bad_request(
            "invalid_time_range",
            "The poll-run time range is invalid.",
        ));
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let (cursor_time, cursor_id) = cursor.unzip();
    let mut items = sqlx::query_as::<_, PollRunRow>("SELECT id, provider_source_id, started_at, finished_at, outcome, http_status, duration_ms, component_count, incident_count, error_message, error_class, adapter_version, poll_kind FROM poll_runs WHERE ($1::uuid IS NULL OR provider_source_id = $1) AND ($2::text IS NULL OR outcome = $2) AND ($3::timestamptz IS NULL OR started_at >= $3) AND ($4::timestamptz IS NULL OR started_at <= $4) AND ($5::timestamptz IS NULL OR (started_at, id) < ($5, $6)) ORDER BY started_at DESC, id DESC LIMIT $7")
        .bind(query.source_id).bind(query.outcome).bind(query.started_after).bind(query.started_before)
        .bind(cursor_time).bind(cursor_id).bind(limit + 1).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let has_more = usize::try_from(limit).is_ok_and(|limit| items.len() > limit);
    if has_more {
        items.pop();
    }
    let next_cursor = has_more
        .then(|| {
            items
                .last()
                .map(|row| encode_page_cursor(row.started_at, row.id))
        })
        .flatten();
    Ok(Json(PollRunPage { items, next_cursor }))
}

#[derive(Debug, Serialize, FromRow)]
struct DeliveryRow {
    id: Option<Uuid>,
    #[serde(skip)]
    cursor_id: Uuid,
    can_resend: bool,
    resend_requested_at: Option<DateTime<Utc>>,
    notification_event_id: Uuid,
    event_type: String,
    incident_id: Option<Uuid>,
    #[serde(skip)]
    entity_id: Uuid,
    channel_type: Option<String>,
    channel_name: Option<String>,
    channel_target: Option<String>,
    payload: Value,
    alert_rule_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    status: String,
    suppression_reason: Option<String>,
    created_at: DateTime<Utc>,
    attempt_count: i32,
    next_attempt_at: Option<DateTime<Utc>>,
    quiet_until: Option<DateTime<Utc>>,
    summary_delivery_id: Option<Uuid>,
    last_error: Option<String>,
    delivered_at: Option<DateTime<Utc>>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_response_class: Option<String>,
    last_response_status: Option<i32>,
    last_attempt_ambiguous: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct DeliveryPage {
    items: Vec<DeliveryRow>,
    next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryView {
    #[default]
    All,
    Overdue,
    Issues,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeliveryQuery {
    #[serde(default)]
    view: DeliveryView,
    #[serde(default)]
    window: SummaryWindow,
    status: Option<String>,
    channel_id: Option<Uuid>,
    event_id: Option<Uuid>,
    event_type: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn deliveries(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<DeliveryPage>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    validate_optional_filter(
        "status",
        query.status.as_deref(),
        &[
            "pending",
            "retrying",
            "delivered",
            "ambiguous",
            "failed",
            "held",
            "summarized",
            "cancelled",
            "suppressed",
        ],
    )?;
    if query
        .event_type
        .as_deref()
        .is_some_and(|event_type| event_type.is_empty() || event_type.len() > 128)
    {
        return Err(ApiError::bad_request(
            "invalid_event_type",
            "The event type filter is invalid.",
        ));
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?;
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let (cursor_time, cursor_id) = cursor.unzip();
    let now = Utc::now();
    let since = now - chrono::Duration::hours(query.window.hours());
    let view = match query.view {
        DeliveryView::All => "all",
        DeliveryView::Overdue => "overdue",
        DeliveryView::Issues => "issues",
    };
    let mut items = sqlx::query_as::<_, DeliveryRow>(r#"
        WITH records AS (
          SELECT delivery.id, delivery.id AS cursor_id,
                 (channel.enabled AND channel.deleted_at IS NULL
                  AND delivery.status IN ('delivered', 'failed', 'summarized', 'cancelled')
                  AND (delivery.lease_until IS NULL OR delivery.lease_until < now())
                  AND NOT EXISTS (
                    SELECT 1 FROM notification_deliveries queued
                    WHERE queued.notification_event_id = delivery.notification_event_id
                      AND queued.channel_id = delivery.channel_id
                      AND queued.status IN ('pending', 'retrying', 'ambiguous', 'held')
                  )) AS can_resend,
                 delivery.resend_requested_at, delivery.notification_event_id,
                 event.event_type,
                 CASE WHEN event.event_type LIKE 'incident.%' THEN event.entity_id END AS incident_id,
                 event.entity_id, channel.channel_type, channel.name AS channel_name,
                 channel.display_target AS channel_target, event.payload,
                 delivery.alert_rule_id, delivery.channel_id, delivery.status,
                 NULL::text AS suppression_reason, event.created_at,
                 delivery.attempt_count, delivery.next_attempt_at, delivery.quiet_until,
                 delivery.summary_delivery_id, delivery.last_error, delivery.delivered_at,
                 recent.attempted_at AS last_attempt_at,
                 recent.response_class AS last_response_class,
                 recent.response_status AS last_response_status,
                 recent.ambiguous AS last_attempt_ambiguous,
                 delivery.status IN ('pending', 'retrying', 'ambiguous', 'held')
                   AND delivery.next_attempt_at < $10 - $11 * interval '1 second'
                   AND (delivery.lease_until IS NULL OR delivery.lease_until < $10)
                   AND channel.enabled AND channel.deleted_at IS NULL
                   AND (delivery.resend_requested_at IS NOT NULL OR delivery.alert_rule_id IS NULL
                        OR (rule.enabled AND rule.deleted_at IS NULL)) AS overdue
          FROM notification_deliveries delivery
          JOIN notification_events event ON event.id = delivery.notification_event_id
          JOIN notification_channels channel ON channel.id = delivery.channel_id
          LEFT JOIN alert_rules rule ON rule.id = delivery.alert_rule_id
          LEFT JOIN LATERAL (
            SELECT attempt.attempted_at, attempt.response_class,
                   attempt.response_status, attempt.ambiguous
            FROM notification_attempts attempt
            WHERE attempt.delivery_id = delivery.id
            ORDER BY attempt.attempted_at DESC, attempt.id DESC LIMIT 1
          ) recent ON true
          UNION ALL
          SELECT NULL::uuid, event.id, false, NULL::timestamptz, event.id,
                 event.event_type,
                 CASE WHEN event.event_type LIKE 'incident.%' THEN event.entity_id END,
                 event.entity_id, NULL::text, NULL::text, NULL::text, event.payload,
                 NULL::uuid, NULL::uuid, 'suppressed',
                 'No enabled alert rule and channel matched this event',
                 event.created_at, 0, NULL::timestamptz, NULL::timestamptz,
                 NULL::uuid, NULL::text, NULL::timestamptz, NULL::timestamptz,
                 NULL::text, NULL::integer, NULL::boolean, false
          FROM notification_events event
          WHERE event.covered_by_event_id IS NULL
            AND NOT EXISTS (
            SELECT 1 FROM notification_deliveries delivery
            WHERE delivery.notification_event_id = event.id
          )
        )
        SELECT id, cursor_id, can_resend, resend_requested_at,
               notification_event_id, event_type, incident_id, entity_id,
               channel_type, channel_name, channel_target, payload, alert_rule_id,
               channel_id, status, suppression_reason, created_at, attempt_count,
               next_attempt_at, quiet_until, summary_delivery_id, last_error,
               delivered_at, last_attempt_at, last_response_class,
               last_response_status, last_attempt_ambiguous
        FROM records
        WHERE ($1::text IS NULL OR status = $1)
          AND ($2::uuid IS NULL OR channel_id = $2)
          AND ($3::uuid IS NULL OR notification_event_id = $3)
          AND ($4::text IS NULL OR event_type = $4)
          AND ($5::timestamptz IS NULL OR (created_at, cursor_id) < ($5, $6))
          AND ($8 = 'all'
               OR ($8 = 'issues' AND status IN ('failed', 'ambiguous')
                   AND last_attempt_at >= $9 AND last_attempt_at <= $10)
               OR ($8 = 'overdue' AND overdue))
        ORDER BY created_at DESC, cursor_id DESC LIMIT $7
        "#)
        .bind(query.status).bind(query.channel_id).bind(query.event_id).bind(query.event_type)
        .bind(cursor_time).bind(cursor_id).bind(limit + 1).bind(view).bind(since).bind(now).bind(DELIVERY_GRACE_SECONDS).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let has_more = usize::try_from(limit).is_ok_and(|limit| items.len() > limit);
    if has_more {
        items.pop();
    }
    for item in &mut items {
        if let Some(channel_type) = &item.channel_type {
            item.payload = crate::notifications::dispatcher::render_delivery_payload(
                &state.config,
                item.notification_event_id,
                &item.event_type,
                item.entity_id,
                item.created_at,
                channel_type,
                std::mem::take(&mut item.payload),
            );
        }
    }
    let next_cursor = has_more
        .then(|| {
            items
                .last()
                .map(|row| encode_page_cursor(row.created_at, row.cursor_id))
        })
        .flatten();
    Ok(Json(DeliveryPage { items, next_cursor }))
}

#[derive(Debug, Serialize, FromRow)]
pub(super) struct AttemptRow {
    id: Uuid,
    attempted_at: DateTime<Utc>,
    duration_ms: Option<i32>,
    outcome: String,
    response_class: Option<String>,
    response_status: Option<i32>,
    error_message: Option<String>,
    ambiguous: bool,
}

pub(super) async fn delivery_attempts(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<AttemptRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM notification_deliveries WHERE id = $1)",
    )
    .bind(id)
    .fetch_one(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    if !exists {
        return Err(ApiError::not_found("delivery not found"));
    }
    Ok(Json(sqlx::query_as::<_, AttemptRow>("SELECT id, attempted_at, duration_ms, outcome, response_class, response_status, error_message, ambiguous FROM notification_attempts WHERE delivery_id = $1 ORDER BY attempted_at DESC, id DESC").bind(id).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?))
}
