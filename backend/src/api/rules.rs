use std::collections::HashMap;

use crate::notifications::quiet_hours::QuietHours;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{
        authenticated_user, fanout_active_incidents_for_provider, require_csrf, validated_name,
    },
};

#[derive(Debug, Serialize, FromRow)]
struct RuleRow {
    id: Uuid,
    name: String,
    rule_kind: String,
    min_severity: String,
    notify_detected: bool,
    notify_started: bool,
    notify_updated: bool,
    notify_resolved: bool,
    notify_reopened: bool,
    notify_maintenance: bool,
    notify_recovered: bool,
    enabled: bool,
    quiet_hours: sqlx::types::Json<QuietHours>,
    quiet_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub(super) struct RuleResponse {
    #[serde(flatten)]
    rule: RuleRow,
    channel_ids: Vec<Uuid>,
    provider_ids: Vec<Uuid>,
    component_ids: Vec<Uuid>,
}

#[derive(Default)]
struct RuleScope {
    channel_ids: Vec<Uuid>,
    provider_ids: Vec<Uuid>,
    component_ids: Vec<Uuid>,
}

pub(super) async fn rules(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<RuleResponse>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, RuleRow>("SELECT id, name, rule_kind, min_severity, notify_detected, notify_started, notify_updated, notify_resolved, notify_reopened, notify_maintenance, notify_recovered, enabled, quiet_hours, quiet_hours_end(quiet_hours, statement_timestamp()) AS quiet_until FROM alert_rules WHERE deleted_at IS NULL ORDER BY name")
        .fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let rule_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let mut scopes = fetch_rule_scopes(&state.database.pool, &rule_ids).await?;
    let mut response = Vec::with_capacity(rows.len());
    for row in rows {
        let scope = scopes.remove(&row.id).unwrap_or_default();
        response.push(RuleResponse {
            rule: row,
            channel_ids: scope.channel_ids,
            provider_ids: scope.provider_ids,
            component_ids: scope.component_ids,
        });
    }
    Ok(Json(response))
}

async fn fetch_rule_scopes(
    pool: &sqlx::PgPool,
    rule_ids: &[Uuid],
) -> Result<HashMap<Uuid, RuleScope>, ApiError> {
    if rule_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let channels = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT alert_rule_id, channel_id FROM alert_rule_channels WHERE alert_rule_id = ANY($1) ORDER BY alert_rule_id, channel_id",
    )
    .bind(rule_ids)
    .fetch_all(pool);
    let providers = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT alert_rule_id, provider_id FROM alert_rule_providers WHERE alert_rule_id = ANY($1) ORDER BY alert_rule_id, provider_id",
    )
    .bind(rule_ids)
    .fetch_all(pool);
    let components = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT alert_rule_id, component_id FROM alert_rule_components WHERE alert_rule_id = ANY($1) ORDER BY alert_rule_id, component_id",
    )
    .bind(rule_ids)
    .fetch_all(pool);
    let (channels, providers, components) =
        tokio::try_join!(channels, providers, components).map_err(ApiError::internal)?;
    let mut scopes = rule_ids
        .iter()
        .map(|id| (*id, RuleScope::default()))
        .collect::<HashMap<_, _>>();
    for (rule_id, channel_id) in channels {
        if let Some(scope) = scopes.get_mut(&rule_id) {
            scope.channel_ids.push(channel_id);
        }
    }
    for (rule_id, provider_id) in providers {
        if let Some(scope) = scopes.get_mut(&rule_id) {
            scope.provider_ids.push(provider_id);
        }
    }
    for (rule_id, component_id) in components {
        if let Some(scope) = scopes.get_mut(&rule_id) {
            scope.component_ids.push(component_id);
        }
    }
    Ok(scopes)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleRequest {
    name: String,
    quiet_hours: QuietHours,
    rule_kind: Option<String>,
    min_severity: Option<String>,
    channel_ids: Vec<Uuid>,
    #[serde(default)]
    provider_ids: Vec<Uuid>,
    #[serde(default)]
    component_ids: Vec<Uuid>,
    notify_detected: Option<bool>,
    notify_started: Option<bool>,
    notify_updated: Option<bool>,
    notify_resolved: Option<bool>,
    notify_reopened: Option<bool>,
    notify_maintenance: Option<bool>,
    notify_recovered: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleUpdateRequest {
    quiet_hours: Option<QuietHours>,
    name: Option<String>,
    rule_kind: Option<String>,
    min_severity: Option<String>,
    channel_ids: Option<Vec<Uuid>>,
    provider_ids: Option<Vec<Uuid>>,
    component_ids: Option<Vec<Uuid>>,
    notify_detected: Option<bool>,
    notify_started: Option<bool>,
    notify_updated: Option<bool>,
    notify_resolved: Option<bool>,
    notify_reopened: Option<bool>,
    notify_maintenance: Option<bool>,
    notify_recovered: Option<bool>,
    enabled: Option<bool>,
}

fn validate_rule_kind(kind: &str) -> Result<(), ApiError> {
    if matches!(kind, "provider" | "system_health") {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "invalid_rule_kind",
            "Rule kind must be provider or system_health.",
        ))
    }
}

fn validate_min_severity(value: &str) -> Result<(), ApiError> {
    if matches!(value, "info" | "minor" | "major" | "critical") {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "invalid_severity",
            "Minimum severity must be info, minor, major, or critical.",
        ))
    }
}

fn rule_write_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|error| error.constraint() == Some("alert_rules_active_name_unique_idx"))
    {
        ApiError::conflict(
            "rule_name_in_use",
            "An alert rule with this name already exists.",
        )
    } else {
        ApiError::internal(error)
    }
}

async fn validate_rule_scope(
    connection: &mut PgConnection,
    kind: &str,
    channel_ids: &[Uuid],
    existing_channel_ids: &[Uuid],
    provider_ids: &[Uuid],
    component_ids: &[Uuid],
) -> Result<(), ApiError> {
    if channel_ids.is_empty() {
        return Err(ApiError::bad_request(
            "missing_channels",
            "At least one channel is required.",
        ));
    }
    if kind == "system_health" && !component_ids.is_empty() {
        return Err(ApiError::bad_request(
            "invalid_scope",
            "System-health rules do not accept component scope.",
        ));
    }
    if kind == "provider" && provider_ids.is_empty() {
        return Err(ApiError::bad_request(
            "missing_providers",
            "Provider rules require at least one monitored provider.",
        ));
    }
    let channels = sqlx::query_scalar::<_, Uuid>("SELECT id FROM notification_channels WHERE deleted_at IS NULL AND id = ANY($1) AND (enabled OR id = ANY($2)) ORDER BY id FOR SHARE")
        .bind(channel_ids).bind(existing_channel_ids).fetch_all(&mut *connection).await.map_err(ApiError::internal)?;
    if channels.len() != channel_ids.len() {
        return Err(ApiError::bad_request(
            "invalid_channels",
            "Every selected channel must exist; new selections must be enabled.",
        ));
    }
    if !provider_ids.is_empty() {
        let provider_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM providers s JOIN monitored_providers monitored ON monitored.provider_id = s.id AND monitored.enabled WHERE s.active AND s.id = ANY($1)")
            .bind(provider_ids).fetch_one(&mut *connection).await.map_err(ApiError::internal)?;
        if usize::try_from(provider_count).ok() != Some(provider_ids.len()) {
            return Err(ApiError::bad_request(
                "invalid_providers",
                "Every selected provider must exist and be active.",
            ));
        }
    }
    if !component_ids.is_empty() {
        let component_count = sqlx::query_scalar::<_, i64>("SELECT count(DISTINCT component.id) FROM components component JOIN monitored_providers monitored ON monitored.provider_id = component.provider_id AND monitored.enabled LEFT JOIN monitored_components selected ON selected.monitored_provider_id = monitored.id AND selected.component_id = component.id WHERE component.active AND component.id = ANY($1) AND (monitored.monitor_all_components OR selected.component_id IS NOT NULL)")
            .bind(component_ids).fetch_one(&mut *connection).await.map_err(ApiError::internal)?;
        if usize::try_from(component_count).ok() != Some(component_ids.len()) {
            return Err(ApiError::bad_request(
                "invalid_components",
                "Every selected component must exist, be active, and be monitored.",
            ));
        }
        let scoped_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM components WHERE id = ANY($1) AND provider_id = ANY($2)",
        )
        .bind(component_ids)
        .bind(provider_ids)
        .fetch_one(&mut *connection)
        .await
        .map_err(ApiError::internal)?;
        if usize::try_from(scoped_count).ok() != Some(component_ids.len()) {
            return Err(ApiError::bad_request(
                "invalid_component_scope",
                "Every selected component must belong to a selected provider.",
            ));
        }
    }
    Ok(())
}

async fn fetch_rule(pool: &sqlx::PgPool, id: Uuid) -> Result<RuleRow, ApiError> {
    sqlx::query_as::<_, RuleRow>("SELECT id, name, rule_kind, min_severity, notify_detected, notify_started, notify_updated, notify_resolved, notify_reopened, notify_maintenance, notify_recovered, enabled, quiet_hours, quiet_hours_end(quiet_hours, statement_timestamp()) AS quiet_until FROM alert_rules WHERE id = $1 AND deleted_at IS NULL")
        .bind(id).fetch_optional(pool).await.map_err(ApiError::internal)?.ok_or_else(|| ApiError::not_found("alert rule not found"))
}

async fn fetch_rule_for_update(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<RuleRow, ApiError> {
    sqlx::query_as::<_, RuleRow>("SELECT id, name, rule_kind, min_severity, notify_detected, notify_started, notify_updated, notify_resolved, notify_reopened, notify_maintenance, notify_recovered, enabled, quiet_hours, quiet_hours_end(quiet_hours, statement_timestamp()) AS quiet_until FROM alert_rules WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await.map_err(ApiError::internal)?.ok_or_else(|| ApiError::not_found("alert rule not found"))
}

async fn fetch_rule_scope(pool: &sqlx::PgPool, rule_id: Uuid) -> Result<RuleScope, ApiError> {
    let mut connection = pool.acquire().await.map_err(ApiError::internal)?;
    fetch_rule_scope_on_connection(&mut connection, rule_id).await
}

async fn fetch_rule_scope_on_connection(
    connection: &mut PgConnection,
    rule_id: Uuid,
) -> Result<RuleScope, ApiError> {
    let channel_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT channel_id FROM alert_rule_channels WHERE alert_rule_id = $1 ORDER BY channel_id",
    )
    .bind(rule_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(ApiError::internal)?;
    let provider_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT provider_id FROM alert_rule_providers WHERE alert_rule_id = $1 ORDER BY provider_id",
    )
    .bind(rule_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(ApiError::internal)?;
    let component_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT component_id FROM alert_rule_components WHERE alert_rule_id = $1 ORDER BY component_id",
    )
    .bind(rule_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(ApiError::internal)?;

    Ok(RuleScope {
        channel_ids,
        provider_ids,
        component_ids,
    })
}

async fn insert_rule_scope(
    tx: &mut Transaction<'_, Postgres>,
    rule_id: Uuid,
    scope: &RuleScope,
) -> Result<(), ApiError> {
    for channel_id in &scope.channel_ids {
        sqlx::query("INSERT INTO alert_rule_channels (alert_rule_id, channel_id) VALUES ($1, $2)")
            .bind(rule_id)
            .bind(channel_id)
            .execute(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
    }
    for provider_id in &scope.provider_ids {
        sqlx::query(
            "INSERT INTO alert_rule_providers (alert_rule_id, provider_id) VALUES ($1, $2)",
        )
        .bind(rule_id)
        .bind(provider_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    }
    for component_id in &scope.component_ids {
        let provider_id =
            sqlx::query_scalar::<_, Uuid>("SELECT provider_id FROM components WHERE id = $1")
                .bind(component_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(ApiError::internal)?;
        sqlx::query("INSERT INTO alert_rule_components (alert_rule_id, provider_id, component_id) VALUES ($1, $2, $3)")
            .bind(rule_id)
            .bind(provider_id)
            .bind(component_id)
            .execute(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(())
}

async fn replace_rule_scope(
    tx: &mut Transaction<'_, Postgres>,
    rule_id: Uuid,
    scope: &RuleScope,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM alert_rule_channels WHERE alert_rule_id = $1")
        .bind(rule_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM alert_rule_providers WHERE alert_rule_id = $1")
        .bind(rule_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM alert_rule_components WHERE alert_rule_id = $1")
        .bind(rule_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    insert_rule_scope(tx, rule_id, scope).await
}

async fn rule_response(pool: &sqlx::PgPool, rule: RuleRow) -> Result<RuleResponse, ApiError> {
    let scope = fetch_rule_scope(pool, rule.id).await?;
    Ok(RuleResponse {
        rule,
        channel_ids: scope.channel_ids,
        provider_ids: scope.provider_ids,
        component_ids: scope.component_ids,
    })
}

pub(super) async fn rule_create(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<RuleRequest>,
) -> Result<(StatusCode, Json<RuleResponse>), ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let name = validated_name(&input.name, "Rule")?;
    let kind = input.rule_kind.unwrap_or_else(|| "provider".into());
    validate_rule_kind(&kind)?;
    let min_severity = input.min_severity.unwrap_or_else(|| "minor".into());
    validate_min_severity(&min_severity)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    input
        .quiet_hours
        .validate(&mut tx)
        .await
        .map_err(|message| ApiError::bad_request("invalid_quiet_hours", message))?;
    validate_rule_scope(
        &mut tx,
        &kind,
        &input.channel_ids,
        &[],
        &input.provider_ids,
        &input.component_ids,
    )
    .await?;
    let notify_detected = input.notify_detected.unwrap_or(true);
    let notify_started = input.notify_started.unwrap_or(true);
    let notify_updated = input.notify_updated.unwrap_or(true);
    let notify_resolved = input.notify_resolved.unwrap_or(true);
    let notify_reopened = input.notify_reopened.unwrap_or(true);
    let notify_maintenance = input.notify_maintenance.unwrap_or(false);
    let notify_recovered = input.notify_recovered.unwrap_or(true);
    let scope = RuleScope {
        channel_ids: input.channel_ids,
        provider_ids: input.provider_ids,
        component_ids: input.component_ids,
    };
    let rule_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO alert_rules (name, rule_kind, min_severity, notify_detected, notify_started, notify_updated, notify_resolved, notify_reopened, notify_maintenance, notify_recovered, quiet_hours) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id")
        .bind(&name).bind(&kind).bind(&min_severity).bind(notify_detected).bind(notify_started)
        .bind(notify_updated).bind(notify_resolved).bind(notify_reopened).bind(notify_maintenance)
        .bind(notify_recovered).bind(sqlx::types::Json(input.quiet_hours)).fetch_one(&mut *tx).await.map_err(rule_write_error)?;
    insert_rule_scope(&mut tx, rule_id, &scope).await?;
    for provider_id in &scope.provider_ids {
        fanout_active_incidents_for_provider(&mut tx, *provider_id, Some(rule_id)).await?;
    }
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'rule.created', 'alert_rule', $2)").bind(user.id).bind(rule_id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((
        StatusCode::CREATED,
        Json(
            rule_response(
                &state.database.pool,
                fetch_rule(&state.database.pool, rule_id).await?,
            )
            .await?,
        ),
    ))
}

pub(super) async fn rule_update(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<RuleUpdateRequest>,
) -> Result<Json<RuleResponse>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let current = fetch_rule_for_update(&mut tx, id).await?;
    let was_enabled = current.enabled;
    let quiet_hours = input
        .quiet_hours
        .map(sqlx::types::Json)
        .unwrap_or(current.quiet_hours);
    quiet_hours
        .validate(&mut tx)
        .await
        .map_err(|message| ApiError::bad_request("invalid_quiet_hours", message))?;
    let current_scope = fetch_rule_scope_on_connection(&mut tx, id).await?;
    let existing_channel_ids = current_scope.channel_ids.clone();
    let scope_changed = input.rule_kind.is_some()
        || input.channel_ids.is_some()
        || input.provider_ids.is_some()
        || input.component_ids.is_some();
    let kind = input.rule_kind.unwrap_or(current.rule_kind);
    let min_severity = input.min_severity.unwrap_or(current.min_severity);
    let scope = RuleScope {
        channel_ids: input.channel_ids.unwrap_or(current_scope.channel_ids),
        provider_ids: input.provider_ids.unwrap_or(current_scope.provider_ids),
        component_ids: input.component_ids.unwrap_or(current_scope.component_ids),
    };
    let name = validated_name(input.name.as_deref().unwrap_or(&current.name), "Rule")?;
    let enabled = input.enabled.unwrap_or(current.enabled);
    validate_rule_kind(&kind)?;
    validate_min_severity(&min_severity)?;
    if enabled || scope_changed {
        validate_rule_scope(
            &mut tx,
            &kind,
            &scope.channel_ids,
            &existing_channel_ids,
            &scope.provider_ids,
            &scope.component_ids,
        )
        .await?;
    }
    sqlx::query("UPDATE alert_rules SET name = $2, rule_kind = $3, min_severity = $4, notify_detected = $5, notify_started = $6, notify_updated = $7, notify_resolved = $8, notify_reopened = $9, notify_maintenance = $10, notify_recovered = $11, enabled = $12, quiet_hours = $13, updated_at = now() WHERE id = $1")
        .bind(id).bind(name).bind(&kind).bind(&min_severity)
        .bind(input.notify_detected.unwrap_or(current.notify_detected)).bind(input.notify_started.unwrap_or(current.notify_started))
        .bind(input.notify_updated.unwrap_or(current.notify_updated)).bind(input.notify_resolved.unwrap_or(current.notify_resolved))
        .bind(input.notify_reopened.unwrap_or(current.notify_reopened)).bind(input.notify_maintenance.unwrap_or(current.notify_maintenance))
        .bind(input.notify_recovered.unwrap_or(current.notify_recovered)).bind(enabled).bind(quiet_hours).execute(&mut *tx).await.map_err(rule_write_error)?;
    if was_enabled && !enabled {
        sqlx::query("UPDATE notification_deliveries SET status = 'cancelled', last_error = 'delivery cancelled because alert rule was disabled' WHERE alert_rule_id = $1 AND resend_requested_at IS NULL AND status IN ('pending', 'retrying', 'ambiguous', 'held')")
            .bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    }
    replace_rule_scope(&mut tx, id, &scope).await?;
    // Edits affect future events; replaying saved openings here can flood newly
    // enabled channels or event categories with the provider's existing backlog.
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'rule.updated', 'alert_rule', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(
        rule_response(
            &state.database.pool,
            fetch_rule(&state.database.pool, id).await?,
        )
        .await?,
    ))
}

pub(super) async fn rule_delete(
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
    let result = sqlx::query("UPDATE alert_rules SET enabled = false, deleted_at = now(), updated_at = now() WHERE id = $1 AND deleted_at IS NULL")
        .bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("alert rule not found"));
    }
    sqlx::query("UPDATE notification_deliveries SET status = 'failed', lease_owner = NULL, lease_until = NULL, last_error = 'alert rule removed before delivery' WHERE alert_rule_id = $1 AND resend_requested_at IS NULL AND status IN ('pending', 'retrying', 'ambiguous', 'held')")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'rule.deleted', 'alert_rule', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn quiet_hours_preview(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(schedule): Json<QuietHours>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut connection = state
        .database
        .pool
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    schedule
        .validate(&mut connection)
        .await
        .map_err(|message| ApiError::bad_request("invalid_quiet_hours", message))?;
    let until = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT quiet_hours_end($1, statement_timestamp())",
    )
    .bind(sqlx::types::Json(schedule))
    .fetch_one(&mut *connection)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({"quiet_until": until})))
}
