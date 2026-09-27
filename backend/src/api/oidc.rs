use std::sync::Arc;

use axum::{
    Json,
    extract::{ConnectInfo, Form, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{AppendHeaders, IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, AppState, profile,
    shared::{authenticated_user, require_csrf},
};
use crate::{
    auth::{
        self, User,
        oidc::{Configuration, Runtime, Settings},
    },
    notifications::secrets,
};

const BINDING_COOKIE: &str = "statusdeck_oidc";
const PROFILE_PATH: &str = "/profile?section=sso";

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "oidc_unavailable",
        "Single sign-on is unavailable. Use email and password.",
    )
}
fn invalid() -> ApiError {
    ApiError::bad_request(
        "oidc_failed",
        "The SSO flow expired or was rejected. Start again.",
    )
}
async fn runtime(state: &AppState) -> Result<Arc<Runtime>, ApiError> {
    state
        .oidc
        .get(&state.config, &state.database.pool)
        .await
        .map_err(ApiError::internal)
}
fn settings(runtime: &Runtime) -> Result<&Settings, ApiError> {
    runtime.settings().map_err(|_| unavailable())
}
fn hash(state: &AppState, token: &str) -> Result<Vec<u8>, ApiError> {
    auth::hash_token(token, state.config.secret_key.expose().as_bytes()).map_err(ApiError::internal)
}
fn browser(state: &AppState, headers: &HeaderMap) -> Result<Vec<u8>, ApiError> {
    hash(
        state,
        &auth::cookie(headers, BINDING_COOKIE).ok_or_else(invalid)?,
    )
}
fn cookie(state: &AppState, name: &str, token: &str, age: i64) -> String {
    format!(
        "{}; Max-Age={age}",
        auth::set_cookie(name, token, true, state.config.session_cookie_secure)
    )
}
fn private(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}
async fn lock_config(
    tx: &mut Transaction<'_, Postgres>,
    runtime: &Runtime,
) -> Result<(), ApiError> {
    let current = sqlx::query_scalar::<_, Option<String>>(
        "SELECT flow_key FROM oidc_configuration WHERE singleton FOR UPDATE",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::internal)?;
    if current.as_ref() != Some(&settings(runtime)?.flow_key) {
        return Err(unavailable());
    }
    sqlx::query("SELECT id FROM users ORDER BY id FOR UPDATE")
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    user: Option<Uuid>,
    action: &str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, metadata) VALUES ($1, $2, 'oidc', $3)")
        .bind(user).bind(action).bind(json!({"method":"oidc", "correlation_id": super::request::correlation_id()}))
        .execute(&mut **tx).await.map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn methods(State(state): State<AppState>) -> Result<Response, ApiError> {
    let runtime = runtime(&state).await?;
    let available = if let Some(config) = &runtime.settings {
        let linked = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM oidc_identities i JOIN users u ON u.id = i.user_id JOIN oidc_configuration c ON c.singleton WHERE u.enabled AND NOT i.needs_relink AND i.configuration_key = $1 AND c.flow_key = $2)")
            .bind(&config.identity_key).bind(&config.flow_key).fetch_one(&state.database.pool).await.map_err(ApiError::internal)?;
        linked && runtime.metadata().await.is_ok()
    } else {
        false
    };
    Ok(private(Json(json!({"mode": if runtime.enabled { "local_and_oidc" } else { "local" },
        "local": true, "oidc": {"enabled":runtime.enabled, "label":runtime.label, "available":available}})).into_response()))
}

pub(super) async fn status(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let runtime = runtime(&state).await?;
    let method =
        sqlx::query_scalar::<_, String>("SELECT authentication_method FROM sessions WHERE id = $1")
            .bind(user.session_id)
            .fetch_one(&state.database.pool)
            .await
            .map_err(ApiError::internal)?;
    let linked = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT issuer, subject, needs_relink FROM oidc_identities WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    let pending = if let Ok(browser) = browser(&state, &headers) {
        sqlx::query_scalar::<_, String>("SELECT candidate_subject FROM oidc_login_attempts a JOIN users u ON u.id = a.user_id JOIN oidc_configuration c ON c.singleton WHERE a.browser_hash = $1 AND a.session_id = $2 AND a.user_id = $3 AND a.expires_at > now() AND a.candidate_subject IS NOT NULL AND a.flow_key = c.flow_key AND a.credential_version = u.credential_version AND a.generation = u.oidc_generation")
            .bind(browser).bind(user.session_id).bind(user.id).fetch_optional(&state.database.pool).await.map_err(ApiError::internal)?
    } else {
        None
    };
    let available = runtime.metadata().await.is_ok();
    Ok(private(Json(json!({"enabled":runtime.enabled, "available":available,
        "label":runtime.label, "authentication_method":method,
        "revision": runtime.revision,
        "callback_url":state.config.base_url.join("/api/v1/auth/oidc/callback").map_err(ApiError::internal)?.as_str(),
        "configuration": {
            "enabled": runtime.configuration.enabled,
            "label": runtime.configuration.label,
            "issuer_url": runtime.configuration.issuer_url,
            "discovery_url": runtime.configuration.discovery_url,
            "client_id": runtime.configuration.client_id,
            "has_client_secret": !runtime.configuration.client_secret.is_empty(),
            "token_auth_method": runtime.configuration.token_auth_method,
            "allowed_endpoint_origins": runtime.configuration.allowed_endpoint_origins,
            "session_max_age_seconds": runtime.configuration.session_max_age_seconds,
            "ca_certificate_pem": runtime.configuration.ca_certificate_pem
        },
        "linked":linked.map(|(issuer,subject,needs_relink)| json!({"issuer":issuer,"subject":subject,"needs_relink":needs_relink})),
        "pending":pending.map(|subject| json!({"issuer":runtime.settings.as_ref().map(|s| &s.issuer),"subject":subject}))})).into_response()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsUpdate {
    current_password: String,
    revision: i64,
    configuration: Configuration,
}

pub(super) async fn update_settings(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<SettingsUpdate>,
) -> Result<Response, ApiError> {
    change_settings(
        headers,
        state,
        input.current_password,
        input.revision,
        Some(input.configuration),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsClear {
    current_password: String,
    revision: i64,
}

pub(super) async fn clear_settings(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<SettingsClear>,
) -> Result<Response, ApiError> {
    change_settings(headers, state, input.current_password, input.revision, None).await
}

async fn change_settings(
    headers: HeaderMap,
    state: AppState,
    current_password: String,
    expected_revision: i64,
    configuration: Option<Configuration>,
) -> Result<Response, ApiError> {
    let user = local_user(&state, &headers).await?;
    let (version, _) =
        profile::verify_credentials(&state, &headers, &user, current_password, None).await?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let (revision, encrypted) = sqlx::query_as::<_, (i64, Option<String>)>(
        "SELECT revision, configuration FROM oidc_configuration WHERE singleton FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if revision != expected_revision {
        return Err(ApiError::conflict(
            "settings_changed",
            "SSO settings changed. Reload the page before saving.",
        ));
    }
    profile::lock_user(&mut tx, &user).await?;
    let current_version =
        sqlx::query_scalar::<_, i64>("SELECT credential_version FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    if current_version != version {
        return Err(invalid());
    }
    let (configuration, encrypted) = match configuration {
        Some(mut configuration) => {
            if configuration.client_secret.is_empty() {
                configuration.client_secret =
                    Configuration::decode(&state.config, encrypted.as_deref())
                        .map_err(ApiError::internal)?
                        .client_secret;
            }
            configuration.validate(&state.config).map_err(|error| {
                ApiError::bad_request("invalid_oidc_settings", error.to_string())
            })?;
            let encrypted = configuration
                .encode(&state.config)
                .map_err(ApiError::internal)?;
            (configuration, Some(encrypted))
        }
        None => (Configuration::default(), None),
    };
    let runtime = Runtime::new(&state.config, configuration);
    if runtime.enabled && runtime.settings.is_none() {
        return Err(unavailable());
    }
    runtime.apply(&mut tx).await.map_err(ApiError::internal)?;
    if encrypted.is_none() {
        sqlx::query("UPDATE users SET oidc_generation = oidc_generation + 1")
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let removed = sqlx::query("DELETE FROM oidc_identities")
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        if removed.rows_affected() > 0 {
            audit(&mut tx, Some(user.id), "oidc.disconnected").await?;
        }
    }
    sqlx::query(
        "UPDATE oidc_configuration SET configuration = $1, revision = revision + 1 WHERE singleton",
    )
    .bind(encrypted)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    audit(&mut tx, Some(user.id), "oidc.settings_updated").await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(private(StatusCode::NO_CONTENT.into_response()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Password {
    current_password: String,
}

async fn local_user(state: &AppState, headers: &HeaderMap) -> Result<User, ApiError> {
    require_csrf(headers)?;
    let user = authenticated_user(state, headers).await?;
    if user.role != "admin" {
        return Err(ApiError::forbidden("Administrator access is required."));
    }
    let local = sqlx::query_scalar::<_, bool>(
        "SELECT authentication_method = 'local' FROM sessions WHERE id = $1",
    )
    .bind(user.session_id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    if local != Some(true) {
        return Err(ApiError::forbidden(
            "Sign in with your local email and password first.",
        ));
    }
    Ok(user)
}

pub(super) async fn start(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    require_csrf(&headers)?;
    start_flow(&state, &headers, None).await
}
pub(super) async fn link(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<Password>,
) -> Result<Response, ApiError> {
    let mut user = local_user(&state, &headers).await?;
    let (version, _) =
        profile::verify_credentials(&state, &headers, &user, input.current_password, None).await?;
    user.credential_version = version;
    start_flow(&state, &headers, Some(user)).await
}

async fn start_flow(
    state: &AppState,
    headers: &HeaderMap,
    local: Option<User>,
) -> Result<Response, ApiError> {
    super::session::check_login_rate_limit(state, headers, "oidc:start").await?;
    let runtime = runtime(state).await?;
    let settings = settings(&runtime)?;
    let (url, csrf, nonce, verifier) = runtime.authorization().await.map_err(|_| unavailable())?;
    let binding = auth::cookie(headers, BINDING_COOKIE)
        .filter(|s| s.len() == 43)
        .unwrap_or_else(auth::random_token);
    let encrypted = secrets::seal(
        &state
            .config
            .encryption_key_bytes()
            .map_err(ApiError::internal)?,
        &csrf,
        &verifier,
    )
    .map_err(ApiError::internal)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_config(&mut tx, &runtime).await?;
    let (user_id, version, generation) = sqlx::query_as::<_, (Uuid, i64, i64)>(
        "SELECT id, credential_version, oidc_generation FROM users WHERE enabled FOR UPDATE",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(invalid)?;
    let identity = sqlx::query_scalar::<_, Uuid>("SELECT id FROM oidc_identities WHERE user_id = $1 AND configuration_key = $2 AND NOT needs_relink")
        .bind(user_id).bind(&settings.identity_key).fetch_optional(&mut *tx).await.map_err(ApiError::internal)?;
    let session_id = if let Some(user) = &local {
        if user.id != user_id || user.credential_version != version {
            return Err(invalid());
        }
        let session = profile::lock_user(&mut tx, user).await?;
        let already_linked = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM oidc_identities WHERE user_id = $1)",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if already_linked {
            return Err(ApiError::conflict(
                "already_linked",
                "Disconnect the existing identity before linking another.",
            ));
        }
        Some(session)
    } else {
        if identity.is_none() {
            return Err(unavailable());
        }
        None
    };
    sqlx::query("DELETE FROM oidc_login_attempts WHERE state_hash IN (SELECT state_hash FROM oidc_login_attempts WHERE expires_at <= now() LIMIT 1000) OR browser_hash = $1")
        .bind(hash(state, &binding)?).execute(&mut *tx).await.map_err(ApiError::internal)?;
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM oidc_login_attempts")
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    if count >= 1000 {
        return Err(ApiError::too_many_requests(
            "Too many pending SSO requests.",
        ));
    }
    sqlx::query("INSERT INTO oidc_login_attempts (state_hash, browser_hash, nonce, verifier, purpose, user_id, session_id, credential_version, generation, flow_key, identity_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(hash(state, &csrf)?).bind(hash(state, &binding)?).bind(nonce).bind(encrypted)
        .bind(if local.is_some() { "link" } else { "login" }).bind(user_id).bind(session_id).bind(version).bind(generation)
        .bind(&settings.flow_key).bind(identity).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(private(
        (
            [(
                header::SET_COOKIE,
                cookie(state, BINDING_COOKIE, &binding, 600),
            )],
            Json(json!({"authorization_url":url})),
        )
            .into_response(),
    ))
}

#[derive(Deserialize)]
pub(super) struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}
#[derive(FromRow)]
struct Attempt {
    user_id: Uuid,
    session_id: Option<Uuid>,
    credential_version: i64,
    generation: i64,
    nonce: String,
    verifier: Option<String>,
    identity_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    candidate_subject: Option<String>,
    candidate_sid: Option<String>,
}

async fn recheck(tx: &mut Transaction<'_, Postgres>, attempt: &Attempt) -> Result<(), ApiError> {
    let current = sqlx::query_as::<_, (i64, i64)>("SELECT credential_version, oidc_generation FROM users WHERE id = $1 AND enabled FOR UPDATE")
        .bind(attempt.user_id).fetch_optional(&mut **tx).await.map_err(ApiError::internal)?;
    if current != Some((attempt.credential_version, attempt.generation)) {
        return Err(invalid());
    }
    if let Some(session) = attempt.session_id {
        let live = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1 AND user_id = $2 AND authentication_method = 'local' AND expires_at > now() AND last_seen_at > now() - interval '7 days')")
            .bind(session).bind(attempt.user_id).fetch_one(&mut **tx).await.map_err(ApiError::internal)?;
        if !live {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(super) async fn callback(
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState>,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    let result = match query {
        Ok(Query(input)) => finish_callback(&state, &headers, peer, input).await,
        Err(_) => Err(invalid()),
    };
    let response = match result {
        Ok(response) => response,
        Err(_) => {
            // The token/library error may contain secrets. Only a fixed audit action is persisted.
            if let Ok(mut tx) = state.database.pool.begin().await {
                let user = sqlx::query_scalar::<_, Uuid>("SELECT id FROM users LIMIT 1")
                    .fetch_optional(&mut *tx)
                    .await
                    .ok()
                    .flatten();
                if audit(&mut tx, user, "oidc.login_denied").await.is_ok() {
                    let _ = tx.commit().await;
                }
            }
            let target = if authenticated_user(&state, &headers).await.is_ok() {
                "/profile?section=sso&oidc=failed"
            } else {
                "/login?oidc=failed"
            };
            Redirect::to(target).into_response()
        }
    };
    private(response)
}

async fn finish_callback(
    state: &AppState,
    headers: &HeaderMap,
    peer: std::net::SocketAddr,
    input: Callback,
) -> Result<Response, ApiError> {
    super::session::check_login_rate_limit(state, headers, "oidc:callback").await?;
    let runtime = runtime(state).await?;
    let _permit = runtime
        .verification
        .try_acquire()
        .map_err(|_| unavailable())?;
    let config = settings(&runtime)?;
    let csrf = input.state.filter(|s| s.len() <= 256).ok_or_else(invalid)?;
    let state_hash = hash(state, &csrf)?;
    let browser_hash = browser(state, headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_config(&mut tx, &runtime).await?;
    let attempt = sqlx::query_as::<_, Attempt>("SELECT * FROM oidc_login_attempts WHERE state_hash = $1 AND browser_hash = $2 AND expires_at > now() AND verifier IS NOT NULL AND flow_key = $3 FOR UPDATE")
        .bind(&state_hash).bind(&browser_hash).bind(&config.flow_key).fetch_optional(&mut *tx).await.map_err(ApiError::internal)?.ok_or_else(invalid)?;
    recheck(&mut tx, &attempt).await?;
    if let Some(session_id) = attempt.session_id {
        let session_token = auth::cookie(headers, "statusdeck_session").ok_or_else(invalid)?;
        let same_session = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1 AND token_hash = $2)",
        )
        .bind(session_id)
        .bind(hash(state, &session_token)?)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if !same_session {
            return Err(invalid());
        }
    }
    sqlx::query("UPDATE oidc_login_attempts SET verifier = NULL WHERE state_hash = $1")
        .bind(&state_hash)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    // One use even if exchange fails or the provider returned an error.
    if input.error.is_some() {
        return Err(invalid());
    }
    let code = input
        .code
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or_else(invalid)?;
    let verifier = secrets::open(
        &state
            .config
            .encryption_key_bytes()
            .map_err(ApiError::internal)?,
        &csrf,
        attempt.verifier.as_deref().ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    let identity = runtime
        .exchange(code, verifier, attempt.nonce.clone())
        .await
        .map_err(|_| invalid())?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_config(&mut tx, &runtime).await?;
    recheck(&mut tx, &attempt).await?;
    let still_current = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM oidc_login_attempts WHERE state_hash = $1 AND browser_hash = $2 AND expires_at > now() AND flow_key = $3)")
        .bind(&state_hash).bind(&browser_hash).bind(&config.flow_key).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    if !still_current {
        return Err(invalid());
    }
    check_revoked(
        &mut tx,
        config,
        &attempt,
        &identity.subject,
        identity.sid.as_deref(),
    )
    .await?;
    if attempt.session_id.is_some() {
        sqlx::query("UPDATE oidc_login_attempts SET candidate_subject = $2, candidate_sid = $3 WHERE state_hash = $1")
            .bind(&state_hash).bind(identity.subject).bind(identity.sid).execute(&mut *tx).await.map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(Redirect::to(PROFILE_PATH).into_response());
    }
    let linked = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM oidc_identities WHERE id = $1 AND user_id = $2 AND issuer = $3 AND subject = $4 AND configuration_key = $5 AND NOT needs_relink)")
        .bind(attempt.identity_id).bind(attempt.user_id).bind(&config.issuer).bind(&identity.subject).bind(&config.identity_key).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    if !linked {
        return Err(invalid());
    }
    let token = auth::random_token();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect::<String>()
        });
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND (expires_at <= now() OR last_seen_at <= now() - interval '7 days')")
        .bind(attempt.user_id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM sessions WHERE id IN (SELECT id FROM sessions WHERE user_id = $1 ORDER BY created_at DESC, id DESC OFFSET 19)")
        .bind(attempt.user_id).execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO sessions (user_id, token_hash, created_at, last_seen_at, expires_at, user_agent, authentication_method, oidc_identity_id, oidc_sid, ip_address) VALUES ($1,$2,now(),now(),now() + make_interval(secs => $3),$4,'oidc',$5,$6,$7::text::inet)")
        .bind(attempt.user_id).bind(hash(state, &token)?).bind(config.max_age as f64).bind(user_agent).bind(attempt.identity_id).bind(identity.sid)
        .bind(super::session::client_ip(state, headers, peer).map(|ip| ip.to_string()))
        .execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM oidc_login_attempts WHERE state_hash = $1")
        .bind(state_hash)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    let landing = sqlx::query_scalar::<_, String>(
        "SELECT preferences->>'landing_page' FROM users WHERE id = $1",
    )
    .bind(attempt.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    let landing = match landing.as_str() {
        "/catalog" | "/incidents" | "/analytics" | "/notifications" | "/system" | "/profile" => {
            landing.as_str()
        }
        _ => "/",
    };
    audit(&mut tx, Some(attempt.user_id), "oidc.login_succeeded").await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((
        AppendHeaders([
            (
                header::SET_COOKIE,
                cookie(state, "statusdeck_session", &token, config.max_age),
            ),
            (
                header::SET_COOKIE,
                auth::clear_cookie(BINDING_COOKIE, state.config.session_cookie_secure),
            ),
        ]),
        Redirect::to(landing),
    )
        .into_response())
}

pub(super) async fn confirm(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let user = local_user(&state, &headers).await?;
    let runtime = runtime(&state).await?;
    let config = settings(&runtime)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_config(&mut tx, &runtime).await?;
    let attempt = sqlx::query_as::<_, Attempt>("SELECT * FROM oidc_login_attempts WHERE browser_hash = $1 AND session_id = $2 AND user_id = $3 AND candidate_subject IS NOT NULL AND expires_at > now() AND flow_key = $4 FOR UPDATE")
        .bind(browser(&state, &headers)?).bind(user.session_id).bind(user.id).bind(&config.flow_key).fetch_optional(&mut *tx).await.map_err(ApiError::internal)?.ok_or_else(invalid)?;
    recheck(&mut tx, &attempt).await?;
    check_revoked(
        &mut tx,
        config,
        &attempt,
        attempt.candidate_subject.as_deref().ok_or_else(invalid)?,
        attempt.candidate_sid.as_deref(),
    )
    .await?;
    sqlx::query("INSERT INTO oidc_identities (user_id, issuer, subject, configuration_key) VALUES ($1,$2,$3,$4)")
        .bind(user.id).bind(&config.issuer).bind(attempt.candidate_subject).bind(&config.identity_key).execute(&mut *tx).await
        .map_err(|error| if error.as_database_error().is_some_and(|e| e.is_unique_violation()) { ApiError::conflict("already_linked", "An identity is already linked. Disconnect it first.") } else { ApiError::internal(error) })?;
    sqlx::query("UPDATE users SET oidc_generation = oidc_generation + 1 WHERE id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM oidc_login_attempts WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user.id)
        .bind(user.session_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    let token = auth::random_token();
    sqlx::query("UPDATE sessions SET token_hash = $2, created_at = now(), last_seen_at = now(), expires_at = now() + interval '30 days' WHERE id = $1")
        .bind(user.session_id).bind(hash(&state, &token)?).execute(&mut *tx).await.map_err(ApiError::internal)?;
    audit(&mut tx, Some(user.id), "oidc.linked").await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(private(
        (
            StatusCode::NO_CONTENT,
            AppendHeaders([
                (
                    header::SET_COOKIE,
                    cookie(&state, "statusdeck_session", &token, 2_592_000),
                ),
                (
                    header::SET_COOKIE,
                    auth::clear_cookie(BINDING_COOKIE, state.config.session_cookie_secure),
                ),
            ]),
        )
            .into_response(),
    ))
}

pub(super) async fn disconnect(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(input): Json<Password>,
) -> Result<StatusCode, ApiError> {
    let user = local_user(&state, &headers).await?;
    let (version, _) =
        profile::verify_credentials(&state, &headers, &user, input.current_password, None).await?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    // Disconnect must work even with invalid configuration or an unreachable provider.
    sqlx::query("SELECT singleton FROM oidc_configuration WHERE singleton FOR UPDATE")
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    profile::lock_user(&mut tx, &user).await?;
    let changed = sqlx::query("UPDATE users SET oidc_generation = oidc_generation + 1 WHERE id = $1 AND credential_version = $2")
        .bind(user.id).bind(version).execute(&mut *tx).await.map_err(ApiError::internal)?;
    if changed.rows_affected() != 1 {
        return Err(invalid());
    }
    sqlx::query("DELETE FROM oidc_login_attempts WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND authentication_method = 'oidc'")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM oidc_identities WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    audit(&mut tx, Some(user.id), "oidc.disconnected").await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogoutForm {
    logout_token: String,
}
pub(super) async fn backchannel(
    State(state): State<AppState>,
    Form(input): Form<LogoutForm>,
) -> Result<StatusCode, ApiError> {
    let runtime = runtime(&state).await?;
    // Invalid requests must not spend a shared IP quota needed by provider revocations.
    let _permit = runtime
        .logout_verification
        .try_acquire()
        .map_err(|_| unavailable())?;
    if input.logout_token.len() > 16_384 {
        return Err(invalid());
    }
    let claims = runtime
        .logout_claims(&input.logout_token)
        .await
        .map_err(|_| invalid())?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    lock_config(&mut tx, &runtime).await?;
    let user =
        sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM oidc_identities WHERE issuer = $1")
            .bind(&claims.iss)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    if let Some(user) = user {
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    }
    sqlx::query("DELETE FROM oidc_logout_events WHERE (issuer,jti) IN (SELECT issuer,jti FROM oidc_logout_events WHERE expires_at <= now() LIMIT 1000)")
        .execute(&mut *tx).await.map_err(ApiError::internal)?;
    let replay = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM oidc_logout_events WHERE issuer = $1 AND jti = $2)",
    )
    .bind(&claims.iss)
    .bind(&claims.jti)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    if replay {
        tx.commit().await.map_err(ApiError::internal)?;
        return Ok(StatusCode::OK);
    }
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM oidc_logout_events")
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    if count >= 5000 {
        return Err(ApiError::too_many_requests(
            "Logout verification is busy. Retry shortly.",
        ));
    }
    sqlx::query("INSERT INTO oidc_logout_events (issuer,jti,sid,subject) VALUES ($1,$2,$3,$4)")
        .bind(&claims.iss)
        .bind(claims.jti)
        .bind(&claims.sid)
        .bind(&claims.sub)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM sessions s USING oidc_identities i WHERE s.oidc_identity_id = i.id AND i.issuer = $1 AND ($2::text IS NULL OR s.oidc_sid = $2) AND ($3::text IS NULL OR i.subject = $3)")
        .bind(&claims.iss).bind(claims.sid).bind(claims.sub).execute(&mut *tx).await.map_err(ApiError::internal)?;
    audit(&mut tx, user, "oidc.provider_revoked").await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::OK)
}

async fn check_revoked(
    tx: &mut Transaction<'_, Postgres>,
    config: &Settings,
    attempt: &Attempt,
    subject: &str,
    sid: Option<&str>,
) -> Result<(), ApiError> {
    let revoked = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM oidc_logout_events WHERE issuer = $1 AND expires_at > now() AND ((sid IS NOT NULL AND sid = $2 AND (subject IS NULL OR subject = $3)) OR (sid IS NULL AND subject = $3 AND received_at >= $4)))")
        .bind(&config.issuer).bind(sid).bind(subject).bind(attempt.created_at).fetch_one(&mut **tx).await.map_err(ApiError::internal)?;
    if revoked {
        return Err(invalid());
    }
    Ok(())
}
