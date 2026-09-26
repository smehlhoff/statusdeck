use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::{ApiError, AppState, shared::authenticated_user};

#[derive(Serialize, FromRow)]
pub(super) struct Summary {
    event_id: Uuid,
    created_at: DateTime<Utc>,
    payload: Value,
}

pub(super) async fn summary(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let summary = sqlx::query_as::<_, Summary>(
        r#"
        SELECT event.id AS event_id, event.created_at,
               jsonb_set(event.payload, '{entries}', COALESCE((
                   SELECT jsonb_agg(item.entry || jsonb_build_object(
                       'providers', COALESCE(linked.providers, '[]'::jsonb)
                   ) ORDER BY item.ordinality)
                   FROM jsonb_array_elements(event.payload->'entries')
                        WITH ORDINALITY AS item(entry, ordinality)
                   CROSS JOIN LATERAL (
                       SELECT jsonb_agg(jsonb_build_object(
                           'id', provider.id, 'name', provider.name
                       ) ORDER BY provider.name, provider.id) AS providers
                       FROM providers provider
                       WHERE CASE
                           WHEN item.entry->>'event_type' LIKE 'incident.%' THEN EXISTS (
                               SELECT 1 FROM incident_providers incident_provider
                               WHERE incident_provider.incident_id::text = item.entry->>'entity_id'
                                 AND incident_provider.provider_id = provider.id
                           )
                           WHEN item.entry->>'event_type' = 'provider.status_changed'
                               THEN provider.id::text = item.entry->>'entity_id'
                           WHEN item.entry->>'event_type' = 'component.status_changed' THEN EXISTS (
                               SELECT 1 FROM components component
                               WHERE component.id::text = item.entry->>'entity_id'
                                 AND component.provider_id = provider.id
                           )
                           WHEN item.entry->>'event_type' LIKE 'source.%'
                               THEN provider.provider_source_id::text = item.entry->>'entity_id'
                           ELSE false
                       END
                   ) linked
               ), '[]'::jsonb)) AS payload
        FROM notification_events event
        WHERE event.id = $1 AND event.event_type = 'system.quiet_summary'
        "#,
    )
    .bind(id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("notification summary not found"))?;
    Ok(Json(summary))
}
