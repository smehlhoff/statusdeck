use std::time::Duration;

use axum::http::HeaderMap;
use base64::Engine;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::ApiError;
use crate::{
    auth::{self, User},
    config::Config,
    domain::{Freshness, stale_threshold},
};

pub(super) async fn authenticated_user(
    state: &super::AppState,
    headers: &HeaderMap,
) -> Result<User, ApiError> {
    auth::user_from_headers(
        &state.database.pool,
        headers,
        state.config.secret_key.expose().as_bytes(),
    )
    .await
}

pub(super) fn require_csrf(headers: &HeaderMap) -> Result<(), ApiError> {
    if auth::csrf_valid(headers) {
        Ok(())
    } else {
        Err(ApiError::forbidden("A valid CSRF token is required."))
    }
}

pub(super) fn freshness(
    last_success: Option<DateTime<Utc>>,
    last_attempt: Option<DateTime<Utc>>,
    next_poll_at: DateTime<Utc>,
    failures: i32,
    poll_interval_seconds: i32,
    config: &Config,
) -> Freshness {
    let Some(success) = last_success else {
        return if last_attempt.is_some() {
            Freshness::Failing
        } else {
            Freshness::NeverChecked
        };
    };
    let now = Utc::now();
    let age = now
        .signed_duration_since(success)
        .to_std()
        .unwrap_or_default();
    let poll_interval = Duration::from_secs(u64::try_from(poll_interval_seconds).unwrap_or(1));
    if failures > 0 {
        Freshness::Failing
    } else if age > stale_threshold(poll_interval, config.stale_multiplier) {
        Freshness::Stale
    } else if now > next_poll_at {
        Freshness::Delayed
    } else {
        Freshness::Fresh
    }
}

pub(super) fn validated_name(value: &str, entity: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "invalid_name",
            format!("{entity} names must contain between 1 and 100 characters."),
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn validated_search(value: Option<&str>) -> Result<Option<&str>, ApiError> {
    let search = value.map(str::trim).filter(|value| !value.is_empty());
    if search.is_some_and(|value| value.chars().count() > 500 || value.contains('\0')) {
        return Err(ApiError::bad_request(
            "invalid_search",
            "Search must contain at most 500 characters and no null characters.",
        ));
    }
    Ok(search)
}

pub(super) fn validate_optional_filter(
    name: &str,
    value: Option<&str>,
    allowed: &[&str],
) -> Result<(), ApiError> {
    if value.is_none_or(|value| allowed.contains(&value)) {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "invalid_filter",
        format!("The {name} filter is invalid."),
    ))
}

pub(super) fn encode_page_cursor(timestamp: DateTime<Utc>, id: Uuid) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        "{}|{}",
        timestamp.to_rfc3339(),
        id
    ))
}

pub(super) fn decode_page_cursor(value: &str) -> Result<(DateTime<Utc>, Uuid), ApiError> {
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|decoded| {
            let (time, id) = decoded.split_once('|')?;
            Some((time.parse().ok()?, id.parse().ok()?))
        });
    decoded.ok_or_else(|| ApiError::bad_request("invalid_cursor", "The cursor is invalid."))
}

pub(super) async fn fanout_active_incidents_for_provider(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    provider_id: Uuid,
    rule_id: Option<Uuid>,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO notification_deliveries (notification_event_id, alert_rule_id, channel_id)
        SELECT event.id, rule.id, rule_channel.channel_id
        FROM incidents incident
        JOIN incident_providers affected
          ON affected.incident_id = incident.id AND affected.provider_id = $1
        JOIN notification_events event
          ON event.entity_id = incident.id
         AND event.lifecycle_generation = incident.lifecycle_generation
         AND event.event_type IN ('incident.detected', 'incident.started', 'incident.reopened')
        JOIN alert_rule_providers scoped ON scoped.provider_id = affected.provider_id
        JOIN monitored_providers monitored ON monitored.provider_id = affected.provider_id AND monitored.enabled
        JOIN alert_rules rule ON rule.id = scoped.alert_rule_id AND rule.enabled AND rule.deleted_at IS NULL
        JOIN alert_rule_channels rule_channel ON rule_channel.alert_rule_id = rule.id
        JOIN notification_channels channel
          ON channel.id = rule_channel.channel_id AND channel.enabled AND channel.deleted_at IS NULL
        WHERE incident.lifecycle IN ('open', 'resolution_pending')
          AND incident.within_provider_scope
          AND ($2::uuid IS NULL OR rule.id = $2)
          AND rule.rule_kind = 'provider'
          AND (
            monitored.monitor_all_components
            OR NOT EXISTS (
              SELECT 1 FROM incident_components incident_component
              WHERE incident_component.incident_id = incident.id
            )
            OR EXISTS (
              SELECT 1 FROM incident_components incident_component
              JOIN monitored_components selected_monitor
                ON selected_monitor.component_id = incident_component.component_id
              WHERE incident_component.incident_id = incident.id
                AND selected_monitor.monitored_provider_id = monitored.id
            )
          )
          AND CASE event.event_type
            WHEN 'incident.detected' THEN rule.notify_detected
            WHEN 'incident.started' THEN rule.notify_started
            WHEN 'incident.reopened' THEN rule.notify_reopened
            ELSE false
          END
          AND (incident.kind <> 'maintenance' OR rule.notify_maintenance)
          AND (CASE incident.severity WHEN 'critical' THEN 4 WHEN 'major' THEN 3 WHEN 'minor' THEN 2 ELSE 1 END)
            >= (CASE rule.min_severity WHEN 'critical' THEN 4 WHEN 'major' THEN 3 WHEN 'minor' THEN 2 ELSE 1 END)
          AND (
            NOT EXISTS (
              SELECT 1 FROM alert_rule_components selected
              WHERE selected.alert_rule_id = rule.id AND selected.provider_id = affected.provider_id
            )
            OR NOT EXISTS (
              SELECT 1 FROM incident_components incident_component
              WHERE incident_component.incident_id = incident.id
            )
            OR EXISTS (
              SELECT 1 FROM incident_components incident_component
              JOIN alert_rule_components selected
                ON selected.component_id = incident_component.component_id
              WHERE incident_component.incident_id = incident.id
                AND selected.alert_rule_id = rule.id
                AND selected.provider_id = affected.provider_id
            )
          )
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(provider_id)
    .bind(rule_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) fn parse_provider_ids(value: &str) -> Result<Vec<Uuid>, ApiError> {
    let ids = value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            Uuid::parse_str(part).map_err(|_| {
                ApiError::bad_request("invalid_provider_ids", "Provider IDs must be valid UUIDs.")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty() || ids.len() > 100 {
        return Err(ApiError::bad_request(
            "invalid_provider_ids",
            "Provide between 1 and 100 provider IDs.",
        ));
    }
    Ok(ids)
}
