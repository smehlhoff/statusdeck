use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, require_csrf, validated_name},
};
use crate::auth::{self, User};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum DateFormat {
    Locale,
    Iso,
    DayFirst,
    MonthFirst,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum TimeFormat {
    Locale,
    TwelveHour,
    TwentyFourHour,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum TimestampFormat {
    Relative,
    Absolute,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Theme {
    Light,
    Dark,
    System,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum LandingPage {
    #[serde(rename = "/")]
    Overview,
    #[serde(rename = "/catalog")]
    Catalog,
    #[serde(rename = "/incidents")]
    Incidents,
    #[serde(rename = "/analytics")]
    Analytics,
    #[serde(rename = "/notifications")]
    Notifications,
    #[serde(rename = "/system")]
    System,
    #[serde(rename = "/profile")]
    Profile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    theme: Theme,
    time_zone: String,
    date_format: DateFormat,
    time_format: TimeFormat,
    timestamp_format: TimestampFormat,
    landing_page: LandingPage,
    refresh_interval_ms: i32,
}

#[derive(Debug, Serialize, FromRow)]
pub(super) struct ProfileRow {
    id: Uuid,
    email: String,
    role: String,
    display_name: String,
    preferences: sqlx::types::Json<Preferences>,
}

pub(super) async fn fetch_profile(pool: &sqlx::PgPool, id: Uuid) -> Result<ProfileRow, ApiError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT id, email, role, display_name, preferences FROM users WHERE id = $1 AND enabled",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::unauthorized("authentication required"))
}

pub(super) async fn profile(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<ProfileRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    Ok(Json(fetch_profile(&state.database.pool, user.id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferencesPatch {
    theme: Option<Theme>,
    time_zone: Option<String>,
    date_format: Option<DateFormat>,
    time_format: Option<TimeFormat>,
    timestamp_format: Option<TimestampFormat>,
    landing_page: Option<LandingPage>,
    refresh_interval_ms: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfilePatch {
    display_name: Option<String>,
    preferences: Option<PreferencesPatch>,
}

// Profile/security mutations serialize on the user and recheck the session after locking.
pub(super) async fn lock_user(
    tx: &mut Transaction<'_, Postgres>,
    user: &User,
) -> Result<Uuid, ApiError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 AND enabled FOR UPDATE")
        .bind(user.id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    let session_id = user
        .session_id
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    let active = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1 AND user_id = $2 AND expires_at > now() AND last_seen_at > now() - interval '7 days')")
        .bind(session_id).bind(user.id).fetch_one(&mut **tx).await.map_err(ApiError::internal)?;
    if !active {
        return Err(ApiError::unauthorized(
            "Your session has ended. Sign in again.",
        ));
    }
    Ok(session_id)
}

pub(super) async fn profile_update(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<ProfilePatch>,
) -> Result<Json<ProfileRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let display_name = input
        .display_name
        .as_deref()
        .map(|name| validated_name(name, "Display"))
        .transpose()?;
    if display_name
        .as_ref()
        .is_some_and(|name| name.chars().any(char::is_control))
    {
        return Err(ApiError::bad_request(
            "invalid_name",
            "Display names cannot contain control characters.",
        ));
    }
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_user(&mut tx, &user).await?;
    let mut preferences = sqlx::query_scalar::<_, sqlx::types::Json<Preferences>>(
        "SELECT preferences FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if let Some(patch) = input.preferences {
        if let Some(theme) = patch.theme {
            preferences.theme = theme;
        }
        if let Some(time_zone) = patch.time_zone {
            let valid = time_zone == "browser"
                || sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)",
                )
                .bind(&time_zone)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            if !valid
                || time_zone.len() > 100
                || time_zone.starts_with("posix/")
                || time_zone.starts_with("right/")
            {
                return Err(ApiError::bad_request(
                    "invalid_time_zone",
                    "Choose a valid time zone.",
                ));
            }
            preferences.time_zone = time_zone;
        }
        if let Some(value) = patch.date_format {
            preferences.date_format = value;
        }
        if let Some(value) = patch.time_format {
            preferences.time_format = value;
        }
        if let Some(value) = patch.timestamp_format {
            preferences.timestamp_format = value;
        }
        if let Some(value) = patch.landing_page {
            preferences.landing_page = value;
        }
        if let Some(value) = patch.refresh_interval_ms {
            if ![0, 5_000, 15_000, 60_000].contains(&value) {
                return Err(ApiError::bad_request(
                    "invalid_refresh_interval",
                    "Choose off, 5 seconds, 15 seconds, or 1 minute.",
                ));
            }
            preferences.refresh_interval_ms = value;
        }
    }
    let row = sqlx::query_as::<_, ProfileRow>("UPDATE users SET display_name = COALESCE($2, display_name), preferences = $3, updated_at = now() WHERE id = $1 RETURNING id, email, role, display_name, preferences")
        .bind(user.id).bind(display_name).bind(preferences).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    audit(&mut tx, user.id, "profile.updated", user.id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(row))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EmailRequest {
    email: String,
    current_password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PasswordRequest {
    current_password: String,
    new_password: String,
}

pub(super) async fn verify_credentials(
    state: &AppState,
    headers: &HeaderMap,
    user: &User,
    current_password: String,
    new_password: Option<String>,
) -> Result<(i64, Option<String>), ApiError> {
    let password = Zeroizing::new(current_password);
    let new_password = new_password.map(Zeroizing::new);
    if password.chars().count() > 1024 {
        return Err(ApiError::bad_request(
            "incorrect_password",
            "The current password is incorrect.",
        ));
    }
    let rate_key = format!("profile:{}", user.id);
    super::session::check_login_rate_limit(state, headers, &rate_key).await?;
    let permit = state
        .password_verifications
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::too_many_requests("Password verification is busy. Try again shortly.")
        })?;
    let (hash, version) = sqlx::query_as::<_, (String, i64)>(
        "SELECT password_hash, credential_version FROM users WHERE id = $1 AND enabled",
    )
    .bind(user.id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    let (valid, new_hash) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _permit = permit;
        let valid = auth::verify_password(&password, &hash);
        let new_hash = if valid {
            new_password
                .as_ref()
                .map(|value| auth::hash_password(value))
                .transpose()?
        } else {
            None
        };
        Ok((valid, new_hash))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    if !valid {
        return Err(ApiError::bad_request(
            "incorrect_password",
            "The current password is incorrect.",
        ));
    }
    super::session::clear_login_attempts(state, headers, &rate_key).await;
    Ok((version, new_hash))
}

async fn change_credentials(
    state: &AppState,
    user: &User,
    version: i64,
    email: Option<String>,
    password_hash: Option<String>,
) -> Result<ProfileRow, ApiError> {
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let session_id = lock_user(&mut tx, user).await?;
    let action = if password_hash.is_some() {
        "profile.password_changed"
    } else {
        "profile.email_changed"
    };
    let row = sqlx::query_as::<_, ProfileRow>("UPDATE users SET email = COALESCE($2, email), password_hash = COALESCE($3, password_hash), credential_version = credential_version + 1, oidc_generation = oidc_generation + 1, updated_at = now() WHERE id = $1 AND credential_version = $4 RETURNING id, email, role, display_name, preferences")
        .bind(user.id).bind(email).bind(password_hash).bind(version).fetch_optional(&mut *tx).await
        .map_err(|error| {
            if error.as_database_error().is_some_and(|error| error.is_unique_violation()) {
                ApiError::conflict("email_in_use", "This email address is already in use.")
            } else { ApiError::internal(error) }
        })?.ok_or_else(|| ApiError::conflict("credentials_changed", "Your credentials changed during this request. Enter your current password and try again."))?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user.id)
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    audit(&mut tx, user.id, action, user.id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(row)
}

pub(super) async fn email_update(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<EmailRequest>,
) -> Result<Json<ProfileRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let email = crate::config::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("invalid_email", "Enter a valid email address."))?;
    let (version, _) =
        verify_credentials(&state, &headers, &user, input.current_password, None).await?;
    Ok(Json(
        change_credentials(&state, &user, version, Some(email), None).await?,
    ))
}

pub(super) async fn password_update(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<PasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    if !(12..=1024).contains(&input.new_password.chars().count()) {
        return Err(ApiError::bad_request(
            "invalid_password",
            "Passwords must contain between 12 and 1,024 characters.",
        ));
    }
    let (version, hash) = verify_credentials(
        &state,
        &headers,
        &user,
        input.current_password,
        Some(input.new_password),
    )
    .await?;
    change_credentials(&state, &user, version, None, hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, FromRow)]
pub(super) struct SecurityActivityRow {
    id: Uuid,
    action: String,
    created_at: DateTime<Utc>,
}

pub(super) async fn security_activity(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<SecurityActivityRow>>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, SecurityActivityRow>(
        "SELECT id, action, created_at FROM audit_log
         WHERE actor_user_id = $1 AND action IN (
             'session.login_succeeded', 'session.login_failed', 'session.logout',
             'profile.password_changed', 'profile.email_changed',
             'profile.session_revoked', 'profile.other_sessions_revoked',
             'oidc.login_succeeded', 'oidc.login_denied', 'oidc.linked', 'oidc.disconnected', 'oidc.provider_revoked', 'oidc.settings_updated'
         ) ORDER BY created_at DESC, id DESC LIMIT 100",
    )
    .bind(user.id)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(rows))
}

#[derive(Serialize, FromRow)]
pub(super) struct SessionRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    user_agent: Option<String>,
    ip_address: Option<String>,
    status: String,
    ended_at: Option<DateTime<Utc>>,
    current: bool,
    authentication_method: String,
}

pub(super) async fn sessions(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionRow>>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, SessionRow>(r#"
        SELECT * FROM (
            SELECT id, created_at, last_seen_at, expires_at, user_agent, authentication_method,
                host(ip_address) AS ip_address, id = $2 AS current,
                CASE WHEN expires_at > now() AND last_seen_at > now() - interval '7 days'
                    THEN 'active' ELSE 'expired' END AS status,
                CASE WHEN expires_at <= now() OR last_seen_at <= now() - interval '7 days'
                    THEN LEAST(expires_at, last_seen_at + interval '7 days') END AS ended_at
            FROM sessions WHERE user_id = $1
            UNION ALL
            SELECT id, created_at, last_seen_at, expires_at, user_agent, authentication_method,
                host(ip_address) AS ip_address, false AS current, status, ended_at
            FROM session_history WHERE user_id = $1 AND ended_at > now() - interval '90 days'
        ) history ORDER BY current DESC, (status = 'active') DESC, last_seen_at DESC, id DESC LIMIT 120
    "#)
        .bind(user.id).bind(user.session_id).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    Ok(Json(rows))
}

pub(super) async fn session_revoke(
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
    let current = lock_user(&mut tx, &user).await?;
    if current == id {
        return Err(ApiError::bad_request(
            "current_session",
            "Use Log out to end this session.",
        ));
    }
    let result = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("session not found"));
    }
    invalidate_oidc_attempts(&mut tx, user.id).await?;
    audit(&mut tx, user.id, "profile.session_revoked", id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn sessions_revoke_others(
    headers: HeaderMap,
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
    let current = lock_user(&mut tx, &user).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user.id)
        .bind(current)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    invalidate_oidc_attempts(&mut tx, user.id).await?;
    audit(&mut tx, user.id, "profile.other_sessions_revoked", user.id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    action: &str,
    entity_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, $2, CASE WHEN $2 = 'profile.session_revoked' THEN 'session' ELSE 'user' END, $3)")
        .bind(user_id).bind(action).bind(entity_id).execute(&mut **tx).await.map_err(ApiError::internal)?;
    Ok(())
}

async fn invalidate_oidc_attempts(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE users SET oidc_generation = oidc_generation + 1 WHERE id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}
