use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, require_csrf},
};

pub(super) async fn resend(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;

    // Serialize resends for a channel and lock it before deliveries, matching channel deletion.
    let channel = sqlx::query_as::<_, (Uuid, bool)>(
        "SELECT channel.id, channel.enabled AND channel.deleted_at IS NULL
         FROM notification_channels channel
         WHERE channel.id = (SELECT channel_id FROM notification_deliveries WHERE id = $1)
         FOR NO KEY UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("delivery not found"))?;
    let (channel_id, channel_enabled) = channel;
    if !channel_enabled {
        return Err(ApiError::conflict(
            "channel_disabled",
            "Enable the original delivery channel before resending.",
        ));
    }

    let (event_id, completed) = sqlx::query_as::<_, (Uuid, bool)>(
        "SELECT notification_event_id,
                status IN ('delivered', 'failed', 'summarized', 'cancelled')
                AND (lease_until IS NULL OR lease_until < now())
         FROM notification_deliveries WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if !completed {
        return Err(ApiError::conflict(
            "delivery_not_completed",
            "This delivery is still queued or sending.",
        ));
    }
    let queued = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
            SELECT 1 FROM notification_deliveries
            WHERE notification_event_id = $1 AND channel_id = $2
              AND status IN ('pending', 'retrying', 'ambiguous', 'held')
        )",
    )
    .bind(event_id)
    .bind(channel_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if queued {
        return Err(ApiError::conflict(
            "delivery_already_queued",
            "This event is already queued for this channel.",
        ));
    }

    // Preserve lifetime attempts, previous success and rule/summary relationships.
    // Only the current send's retry budget and scheduling state are reset.
    sqlx::query(
        "UPDATE notification_deliveries SET status = 'pending', retry_attempt_count = 0,
             resend_requested_at = now(), queued_at = now(), next_attempt_at = now(),
             last_error = NULL, quiet_until = NULL, lease_owner = NULL, lease_until = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata)
         VALUES ($1, 'delivery.resent', 'notification_delivery', $2, $3)",
    )
    .bind(user.id)
    .bind(id)
    .bind(json!({"original_delivery_id": id, "event_id": event_id, "channel_id": channel_id}))
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"delivery_id": id, "event_id": event_id, "status": "queued"})),
    ))
}
