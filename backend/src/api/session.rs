use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use super::profile::{ProfileRow, fetch_profile};
use super::{ApiError, AppState, shared::require_csrf};
use crate::auth;

pub(super) async fn csrf(headers: HeaderMap, State(state): State<AppState>) -> Response {
    let token = auth::csrf_cookie(&headers).unwrap_or_else(auth::random_token);
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            auth::set_cookie(
                "statusdeck_csrf",
                &token,
                false,
                state.config.session_cookie_secure,
            ),
        )],
    )
        .into_response()
}

pub(super) async fn session_get(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Option<ProfileRow>>, ApiError> {
    let Some(token) = auth::cookie(&headers, "statusdeck_session") else {
        return Ok(Json(None));
    };
    let user = auth::lookup_session(
        &state.database.pool,
        &token,
        state.config.secret_key.expose().as_bytes(),
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(match user {
        Some(user) => Some(fetch_profile(&state.database.pool, user.id).await?),
        None => None,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LoginRequest {
    email: String,
    password: String,
}

pub(super) async fn session_create(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    require_csrf(&headers)?;
    let email = crate::config::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("invalid_credentials", "Invalid email or password."))?;
    if input.password.chars().count() > 1024 {
        return Err(ApiError::bad_request(
            "invalid_credentials",
            "Invalid email or password.",
        ));
    }
    check_login_rate_limit(&state, &headers, &email).await?;
    let verification_permit = state
        .password_verifications
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::too_many_requests("Login verification is busy. Try again shortly.")
        })?;
    let user = auth::authenticate(
        &state.database.pool,
        &email,
        &input.password,
        verification_permit,
    )
    .await
    .map_err(ApiError::internal)?;
    let Some(user) = user else {
        sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, metadata) VALUES ((SELECT id FROM users WHERE email = $2), 'session.login_failed', 'session', $1)")
            .bind(json!({"email": email}))
            .bind(&email)
            .execute(&state.database.pool)
            .await
            .map_err(ApiError::internal)?;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Invalid email or password.",
        ));
    };
    clear_login_attempts(&state, &headers, &email).await;
    let ip_address = client_ip(&state, &headers, peer);
    let token = auth::create_session(
        &state.database.pool,
        &user,
        state.config.secret_key.expose().as_bytes(),
        headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
        ip_address,
    )
    .await
    .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata) VALUES ($1, 'session.login_succeeded', 'session', $1, $2)")
        .bind(user.id)
        .bind(json!({"method":"local", "correlation_id":super::request::correlation_id()}))
        .execute(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let cookie = auth::set_cookie(
        "statusdeck_session",
        &token,
        true,
        state.config.session_cookie_secure,
    ) + "; Max-Age=2592000";
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(fetch_profile(&state.database.pool, user.id).await?),
    )
        .into_response())
}

fn login_key(state: &AppState, headers: &HeaderMap, email: &str) -> String {
    let client = if state.config.trust_proxy {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
    } else {
        "direct"
    };
    format!("{client}|{email}")
}

pub(super) async fn check_login_rate_limit(
    state: &AppState,
    headers: &HeaderMap,
    email: &str,
) -> Result<(), ApiError> {
    const WINDOW: Duration = Duration::from_secs(5 * 60);
    const MAX_ATTEMPTS: usize = 5;
    const MAX_TRACKED_KEYS: usize = 10_000;
    let key = login_key(state, headers, email);
    let now = Instant::now();
    let mut attempts = state.login_attempts.lock().await;
    if let Some(values) = attempts.get_mut(&key) {
        values.retain(|attempt| now.duration_since(*attempt) < WINDOW);
        if values.len() >= MAX_ATTEMPTS {
            return Err(ApiError::too_many_requests(
                "Too many login attempts. Try again later.",
            ));
        }
        values.push(now);
        return Ok(());
    }
    if attempts.len() >= MAX_TRACKED_KEYS {
        attempts.retain(|_, values| {
            values.retain(|attempt| now.duration_since(*attempt) < WINDOW);
            !values.is_empty()
        });
        if attempts.len() >= MAX_TRACKED_KEYS {
            return Err(ApiError::too_many_requests(
                "Too many login attempts. Try again later.",
            ));
        }
    }
    attempts.entry(key).or_default().push(now);
    Ok(())
}

pub(super) async fn clear_login_attempts(state: &AppState, headers: &HeaderMap, email: &str) {
    state
        .login_attempts
        .lock()
        .await
        .remove(&login_key(state, headers, email));
}

pub(super) async fn session_delete(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    require_csrf(&headers)?;
    if let Some(token) = auth::cookie(&headers, "statusdeck_session") {
        let user = auth::lookup_session(
            &state.database.pool,
            &token,
            state.config.secret_key.expose().as_bytes(),
        )
        .await
        .map_err(ApiError::internal)?;
        auth::revoke_session(
            &state.database.pool,
            &token,
            state.config.secret_key.expose().as_bytes(),
        )
        .await
        .map_err(ApiError::internal)?;
        if let Some(user) = user {
            sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'session.logout', 'session', $1)")
                .bind(user.id)
                .execute(&state.database.pool)
                .await
                .map_err(ApiError::internal)?;
        }
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            auth::clear_cookie("statusdeck_session", state.config.session_cookie_secure),
        )],
    )
        .into_response())
}

pub(super) fn client_ip(state: &AppState, headers: &HeaderMap, peer: SocketAddr) -> Option<IpAddr> {
    if state.config.trust_proxy {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
    } else {
        Some(peer.ip())
    }
}
