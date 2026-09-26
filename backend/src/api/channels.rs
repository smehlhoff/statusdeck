use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::net::IpAddr;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, require_csrf, validated_name},
};
use crate::{
    domain::semantic_hash,
    notifications::{is_valid_channel_target, secrets},
};

#[derive(Debug, Serialize, FromRow)]
pub(super) struct ChannelRow {
    id: Uuid,
    name: String,
    channel_type: String,
    display_target: String,
    enabled: bool,
    last_tested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct StoredChannelConfig {
    name: String,
    channel_type: String,
    encrypted_config: String,
    display_target: String,
    enabled: bool,
}

pub(super) async fn channels(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChannelRow>>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let channels = sqlx::query_as::<_, ChannelRow>("SELECT id, name, channel_type, display_target, enabled, last_tested_at FROM notification_channels WHERE deleted_at IS NULL ORDER BY name")
        .fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    Ok(Json(channels))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChannelRequest {
    name: String,
    channel_type: String,
    target: String,
    signing_secret: Option<String>,
    token: Option<String>,
    bot_email: Option<String>,
    stream: Option<String>,
    topic: Option<String>,
}

pub(super) async fn channel_create(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<ChannelRequest>,
) -> Result<(StatusCode, Json<ChannelRow>), ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    if !matches!(
        input.channel_type.as_str(),
        "webhook" | "discord" | "slack" | "mattermost" | "gotify" | "ntfy" | "zulip"
    ) {
        return Err(ApiError::bad_request(
            "invalid_channel_type",
            "Unsupported notification channel type.",
        ));
    }
    let name = validated_name(&input.name, "Channel")?;
    validate_channel_target(
        &input.channel_type,
        &input.target,
        state.config.allow_private_notification_targets,
    )?;
    validate_signing_secret(&input.channel_type, input.signing_secret.as_deref())?;
    validate_token(&input.channel_type, input.token.as_deref())?;
    validate_zulip_config(
        &input.channel_type,
        input.bot_email.as_deref(),
        input.stream.as_deref(),
        input.topic.as_deref(),
    )?;
    let display_target = redact_target(&input.target);
    let channel_id = Uuid::new_v4();
    let key = state
        .config
        .encryption_key_bytes()
        .map_err(ApiError::internal)?;
    let config = secrets::ChannelConfig {
        target: input.target,
        signing_secret: input.signing_secret,
        token: input.token,
        bot_email: input.bot_email,
        stream: input.stream,
        topic: input.topic,
    };
    let encrypted = secrets::seal(
        &key,
        &format!("channel:{channel_id}:{}", input.channel_type),
        &config.encode().map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let row = sqlx::query_as::<_, ChannelRow>("INSERT INTO notification_channels (id, name, channel_type, encrypted_config, display_target) VALUES ($1, $2, $3, $4, $5) RETURNING id, name, channel_type, display_target, enabled, last_tested_at")
        .bind(channel_id).bind(name).bind(&input.channel_type).bind(encrypted).bind(display_target).fetch_one(&mut *tx).await.map_err(channel_write_error)?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata) VALUES ($1, 'channel.created', 'notification_channel', $2, '{}'::jsonb)").bind(user.id).bind(row.id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub(super) async fn channel_delete(
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
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM notification_channels WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("channel not found"))?;
    let referenced = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM alert_rule_channels link JOIN alert_rules rule ON rule.id = link.alert_rule_id WHERE link.channel_id = $1 AND rule.deleted_at IS NULL)")
        .bind(id).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    if referenced {
        return Err(ApiError::conflict(
            "channel_in_use",
            "Remove this channel from its alert rules before deleting it.",
        ));
    }
    let result = sqlx::query("UPDATE notification_channels SET enabled = false, deleted_at = now(), updated_at = now() WHERE id = $1 AND deleted_at IS NULL")
        .bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("channel not found"));
    }
    sqlx::query("UPDATE notification_deliveries SET status = 'failed', lease_owner = NULL, lease_until = NULL, last_error = 'channel removed before delivery' WHERE channel_id = $1 AND status IN ('pending', 'retrying', 'ambiguous', 'held')")
        .bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'channel.deleted', 'notification_channel', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChannelUpdate {
    name: Option<String>,
    channel_type: Option<String>,
    target: Option<String>,
    signing_secret: Option<String>,
    token: Option<String>,
    bot_email: Option<String>,
    stream: Option<String>,
    topic: Option<String>,
    enabled: Option<bool>,
}

pub(super) async fn channel_update(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<ChannelUpdate>,
) -> Result<Json<ChannelRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let existing = sqlx::query_as::<_, StoredChannelConfig>("SELECT name, channel_type, encrypted_config, display_target, enabled FROM notification_channels WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await.map_err(ApiError::internal)?.ok_or_else(|| ApiError::not_found("channel not found"))?;
    let config_changed = input.channel_type.is_some()
        || input.target.is_some()
        || input.signing_secret.is_some()
        || input.token.is_some()
        || input.bot_email.is_some()
        || input.stream.is_some()
        || input.topic.is_some();
    let channel_type = input
        .channel_type
        .unwrap_or_else(|| existing.channel_type.clone());
    if !matches!(
        channel_type.as_str(),
        "webhook" | "discord" | "slack" | "mattermost" | "gotify" | "ntfy" | "zulip"
    ) {
        return Err(ApiError::bad_request(
            "invalid_channel_type",
            "Unsupported notification channel type.",
        ));
    }
    let (encrypted, display_target) = if config_changed {
        let key = state
            .config
            .encryption_key_bytes()
            .map_err(ApiError::internal)?;
        let plaintext = secrets::open(
            &key,
            &format!("channel:{id}:{}", existing.channel_type),
            &existing.encrypted_config,
        )
        .map_err(ApiError::internal)?;
        let existing_config =
            secrets::ChannelConfig::decode(&plaintext).map_err(ApiError::internal)?;
        let target = input.target.unwrap_or(existing_config.target);
        let signing_secret = input.signing_secret.or(existing_config.signing_secret);
        let token = input.token.or(existing_config.token);
        let bot_email = input.bot_email.or(existing_config.bot_email);
        let stream = input.stream.or(existing_config.stream);
        let topic = input.topic.or(existing_config.topic);
        validate_channel_target(
            &channel_type,
            &target,
            state.config.allow_private_notification_targets,
        )?;
        validate_signing_secret(&channel_type, signing_secret.as_deref())?;
        validate_token(&channel_type, token.as_deref())?;
        validate_zulip_config(
            &channel_type,
            bot_email.as_deref(),
            stream.as_deref(),
            topic.as_deref(),
        )?;
        let config = secrets::ChannelConfig {
            target: target.clone(),
            signing_secret,
            token,
            bot_email,
            stream,
            topic,
        };
        let encrypted = secrets::seal(
            &key,
            &format!("channel:{id}:{channel_type}"),
            &config.encode().map_err(ApiError::internal)?,
        )
        .map_err(ApiError::internal)?;
        (encrypted, redact_target(&target))
    } else {
        (existing.encrypted_config, existing.display_target)
    };
    let name = validated_name(input.name.as_deref().unwrap_or(&existing.name), "Channel")?;
    let enabled = input.enabled.unwrap_or(existing.enabled);
    let row = sqlx::query_as::<_, ChannelRow>("UPDATE notification_channels SET name = $2, channel_type = $3, encrypted_config = $4, display_target = $5, enabled = $6, updated_at = now() WHERE id = $1 RETURNING id, name, channel_type, display_target, enabled, last_tested_at")
        .bind(id).bind(name).bind(&channel_type).bind(encrypted).bind(display_target).bind(enabled)
        .fetch_one(&mut *tx).await.map_err(channel_write_error)?;
    if existing.enabled && !enabled {
        sqlx::query("UPDATE notification_deliveries SET status = 'cancelled', last_error = 'delivery cancelled because channel was disabled' WHERE channel_id = $1 AND status IN ('pending', 'retrying', 'ambiguous', 'held')")
            .bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    }
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'channel.updated', 'notification_channel', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(row))
}

pub(super) async fn channel_test(
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
    // Keep the eligibility check and enqueue ordered with channel disable/delete.
    let enabled = sqlx::query_scalar::<_, bool>(
        "SELECT enabled FROM notification_channels WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("channel not found"))?;
    if !enabled {
        return Err(ApiError::bad_request(
            "channel_disabled",
            "Enable the channel before sending a test.",
        ));
    }
    let event_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO notification_events (event_type, entity_id, semantic_hash, payload) VALUES ('system.test', $1, $2, $3) RETURNING id")
        .bind(id).bind(semantic_hash(&(id, Utc::now()))).bind(json!({"event_type":"system.test", "title":"StatusDeck test notification", "status":"test", "message":"This is a test notification. Provider updates will appear here for incident notifications."}))
        .fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    let delivery_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO notification_deliveries (notification_event_id, alert_rule_id, channel_id) VALUES ($1, NULL, $2) RETURNING id")
        .bind(event_id).bind(id).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query(
        "UPDATE notification_channels SET last_tested_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'channel.tested', 'notification_channel', $2)").bind(user.id).bind(id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"delivery_id": delivery_id, "event_id": event_id, "status": "queued"})),
    ))
}

fn redact_target(target: &str) -> String {
    url::Url::parse(target)
        .ok()
        .and_then(|url| Some(format!("{}://{}/[REDACTED]", url.scheme(), url.host_str()?)))
        .unwrap_or_else(|| "[invalid]".into())
}

fn channel_write_error(error: sqlx::Error) -> ApiError {
    if error.as_database_error().is_some_and(|error| {
        error.constraint() == Some("notification_channels_active_name_unique_idx")
    }) {
        ApiError::conflict(
            "channel_name_in_use",
            "A delivery channel with this name already exists.",
        )
    } else {
        ApiError::internal(error)
    }
}

fn validate_channel_target(
    channel_type: &str,
    target: &str,
    allow_private: bool,
) -> Result<(), ApiError> {
    if target.len() > 2048 {
        return Err(ApiError::bad_request(
            "invalid_channel_target",
            "The channel destination must not exceed 2,048 bytes.",
        ));
    }
    let url = url::Url::parse(target).map_err(|_| {
        ApiError::bad_request(
            "invalid_channel_target",
            "The channel destination must be a valid HTTPS URL.",
        )
    })?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(ApiError::bad_request(
            "invalid_channel_target",
            "The channel destination must use HTTPS and include a host.",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(ApiError::bad_request(
            "invalid_channel_target",
            "The channel destination cannot contain credentials or a fragment.",
        ));
    }
    if !is_valid_channel_target(channel_type, &url) {
        return Err(ApiError::bad_request(
            "invalid_channel_target",
            "The destination URL does not match the selected channel type.",
        ));
    }
    let private_host = url.host_str().is_some_and(|host| {
        let normalized = host.trim_matches(['[', ']']);
        let lowercase = normalized.to_ascii_lowercase();
        normalized.eq_ignore_ascii_case("localhost")
            || normalized.eq_ignore_ascii_case("metadata.google.internal")
            || lowercase.ends_with(".localhost")
            || lowercase.ends_with(".internal")
            || normalized
                .parse::<IpAddr>()
                .is_ok_and(crate::notifications::dispatcher::is_non_public_ip)
    });
    if !allow_private && private_host {
        return Err(ApiError::bad_request(
            "private_channel_target",
            "Private notification destinations are blocked.",
        ));
    }
    Ok(())
}

fn validate_signing_secret(channel_type: &str, secret: Option<&str>) -> Result<(), ApiError> {
    if channel_type == "webhook"
        && secret.is_some_and(|secret| !(32..=4096).contains(&secret.chars().count()))
    {
        return Err(ApiError::bad_request(
            "invalid_signing_secret",
            "Generic webhook signing secrets must contain between 32 and 4,096 characters.",
        ));
    }
    Ok(())
}

fn validate_token(channel_type: &str, token: Option<&str>) -> Result<(), ApiError> {
    if matches!(channel_type, "gotify" | "zulip")
        && !token.is_some_and(|token| {
            token.trim() == token && (1..=4096).contains(&token.chars().count())
        })
    {
        return Err(ApiError::bad_request(
            "invalid_channel_token",
            "This channel requires an API/application token of at most 4,096 characters.",
        ));
    }
    if channel_type == "ntfy"
        && token.is_some_and(|token| {
            token.trim() != token || !(1..=4096).contains(&token.chars().count())
        })
    {
        return Err(ApiError::bad_request(
            "invalid_channel_token",
            "Ntfy access tokens must not exceed 4,096 characters.",
        ));
    }
    Ok(())
}

fn validate_zulip_config(
    channel_type: &str,
    bot_email: Option<&str>,
    stream: Option<&str>,
    topic: Option<&str>,
) -> Result<(), ApiError> {
    if channel_type != "zulip" {
        return Ok(());
    }
    let valid = |value: Option<&str>, maximum: usize| {
        value.is_some_and(|value| {
            value.trim() == value
                && (1..=maximum).contains(&value.chars().count())
                && !value.chars().any(char::is_control)
        })
    };
    if !valid(bot_email, 254)
        || !bot_email.is_some_and(|email| {
            email.split_once('@').is_some_and(|(local, host)| {
                !local.is_empty() && !host.is_empty() && !host.contains('@')
            }) && !email.contains(':')
                && !email.chars().any(char::is_whitespace)
        })
        || !valid(stream, 60)
        || !valid(topic, 60)
    {
        return Err(ApiError::bad_request(
            "invalid_zulip_config",
            "Zulip requires a bot email, channel name (1–60 characters), and topic (1–60 characters), without surrounding whitespace or control characters.",
        ));
    }
    Ok(())
}
