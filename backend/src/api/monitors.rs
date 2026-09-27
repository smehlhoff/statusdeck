use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, fanout_active_incidents_for_provider, require_csrf},
};

#[derive(Debug, Serialize, FromRow)]
pub(super) struct MonitorRow {
    id: Uuid,
    provider_id: Uuid,
    pub(super) monitor_all_components: bool,
    enabled: bool,
}

#[derive(Debug, FromRow)]
struct StoredMonitor {
    provider_id: Uuid,
    monitor_all_components: bool,
    enabled: bool,
}

pub(super) async fn monitors(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<MonitorRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let monitors = sqlx::query_as::<_, MonitorRow>(
        "SELECT id, provider_id, monitor_all_components, enabled FROM monitored_providers ORDER BY created_at",
    )
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(monitors))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MonitorRequest {
    provider_id: Uuid,
    monitor_all_components: bool,
    component_ids: Vec<Uuid>,
}

fn validate_component_selection(
    monitor_all_components: bool,
    component_ids: &[Uuid],
) -> Result<(), ApiError> {
    if monitor_all_components && !component_ids.is_empty() {
        return Err(ApiError::bad_request(
            "invalid_components",
            "Component selections must be empty when monitoring all components.",
        ));
    }
    if !monitor_all_components && component_ids.is_empty() {
        return Err(ApiError::bad_request(
            "invalid_components",
            "At least one component is required when monitor-all is disabled.",
        ));
    }
    Ok(())
}

async fn schedule_provider_refresh(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    provider_id: Uuid,
) -> Result<(), ApiError> {
    // An active poll already refreshes coverage; never wait for its reconciliation lock.
    sqlx::query(
        r#"
        UPDATE provider_sources SET next_poll_at = now()
        WHERE id IN (
            SELECT source.id FROM provider_sources source
            JOIN providers provider ON provider.provider_source_id = source.id
            WHERE provider.id = $1
            FOR NO KEY UPDATE OF source SKIP LOCKED
        )
        "#,
    )
    .bind(provider_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn monitor_create(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<MonitorRequest>,
) -> Result<(StatusCode, Json<MonitorRow>), ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    validate_component_selection(input.monitor_all_components, &input.component_ids)?;
    let provider_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM providers WHERE id = $1 AND active)",
    )
    .bind(input.provider_id)
    .fetch_one(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    if !provider_exists {
        return Err(ApiError::not_found("provider not found"));
    }
    if !input.monitor_all_components {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM components WHERE provider_id = $1 AND id = ANY($2)",
        )
        .bind(input.provider_id)
        .bind(&input.component_ids)
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
        if usize::try_from(count).ok() != Some(input.component_ids.len()) {
            return Err(ApiError::bad_request(
                "invalid_components",
                "Components must belong to the selected provider.",
            ));
        }
    }
    let mut transaction = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let row = sqlx::query_as::<_, MonitorRow>("INSERT INTO monitored_providers (provider_id, monitor_all_components, enabled) VALUES ($1, $2, true) ON CONFLICT (provider_id) DO UPDATE SET monitor_all_components = EXCLUDED.monitor_all_components, enabled = true, updated_at = now() RETURNING id, provider_id, monitor_all_components, enabled")
        .bind(input.provider_id).bind(input.monitor_all_components).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM monitored_components WHERE monitored_provider_id = $1")
        .bind(row.id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    for component_id in input.component_ids {
        sqlx::query("INSERT INTO monitored_components (monitored_provider_id, provider_id, component_id) VALUES ($1, $2, $3)").bind(row.id).bind(input.provider_id).bind(component_id).execute(&mut *transaction).await.map_err(ApiError::internal)?;
    }
    schedule_provider_refresh(&mut transaction, input.provider_id).await?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata) VALUES ($1, 'monitor.created', 'monitor', $2, $3)").bind(user.id).bind(row.id).bind(json!({"provider_id": input.provider_id})).execute(&mut *transaction).await.map_err(ApiError::internal)?;
    fanout_active_incidents_for_provider(&mut transaction, input.provider_id, None).await?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok((StatusCode::OK, Json(row)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MonitorUpdate {
    monitor_all_components: Option<bool>,
    component_ids: Option<Vec<Uuid>>,
    enabled: Option<bool>,
}

pub(super) async fn monitor_update(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<MonitorUpdate>,
) -> Result<Json<MonitorRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let current = sqlx::query_as::<_, StoredMonitor>(
        "SELECT provider_id, monitor_all_components, enabled FROM monitored_providers WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("monitor not found"))?;
    let monitor_all_components = input
        .monitor_all_components
        .unwrap_or(current.monitor_all_components);
    let enabled = input.enabled.unwrap_or(current.enabled);
    if let Some(component_ids) = &input.component_ids {
        validate_component_selection(monitor_all_components, component_ids)?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM components WHERE provider_id = $1 AND id = ANY($2)",
        )
        .bind(current.provider_id)
        .bind(component_ids)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if !monitor_all_components && usize::try_from(count).ok() != Some(component_ids.len()) {
            return Err(ApiError::bad_request(
                "invalid_components",
                "Components must belong to the selected provider.",
            ));
        }
    } else if !monitor_all_components {
        let selected = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM monitored_components WHERE monitored_provider_id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if selected == 0 {
            return Err(ApiError::bad_request(
                "invalid_components",
                "At least one component is required when monitor-all is disabled.",
            ));
        }
    }
    let row = sqlx::query_as::<_, MonitorRow>("UPDATE monitored_providers SET monitor_all_components = $2, enabled = $3, updated_at = now() WHERE id = $1 RETURNING id, provider_id, monitor_all_components, enabled").bind(id).bind(monitor_all_components).bind(enabled).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    if monitor_all_components {
        sqlx::query("DELETE FROM monitored_components WHERE monitored_provider_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    } else if let Some(component_ids) = input.component_ids {
        sqlx::query("DELETE FROM monitored_components WHERE monitored_provider_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        for component_id in component_ids {
            sqlx::query("INSERT INTO monitored_components (monitored_provider_id, provider_id, component_id) VALUES ($1, $2, $3)").bind(id).bind(current.provider_id).bind(component_id).execute(&mut *tx).await.map_err(ApiError::internal)?;
        }
    }
    if enabled && !current.enabled {
        schedule_provider_refresh(&mut tx, current.provider_id).await?;
    }
    if enabled {
        fanout_active_incidents_for_provider(&mut tx, current.provider_id, None).await?;
    }
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'monitor.updated', 'monitor', $2)")
        .bind(user.id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(row))
}

pub(super) async fn monitor_delete(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let deleted = sqlx::query(
        "UPDATE monitored_providers SET enabled = false, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::not_found("monitor not found"));
    }
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'monitor.disabled', 'monitor', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}
