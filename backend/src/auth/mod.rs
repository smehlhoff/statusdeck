pub mod bootstrap;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::http::{HeaderMap, header};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::{RngCore, rng};
use sha2::Sha256;
use sqlx::PgPool;
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::ApiError;

const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$KAwzHhGxH2cuwsbKFc/e4g$UD/U2Oy9wx5WCASDDOxcTKaIWcVwj4MPTjgdCc3oEJI";

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub session_id: Option<Uuid>,
    pub credential_version: i64,
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt =
        argon2::password_hash::SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_string())
}

#[must_use]
pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub async fn authenticate(
    pool: &PgPool,
    email: &str,
    password: &str,
    verification_permit: OwnedSemaphorePermit,
) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, (Uuid, String, String, bool, i64)>(
        r#"
        SELECT id, email, password_hash, enabled, credential_version FROM users WHERE email = $1
    "#,
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    let hash = row
        .as_ref()
        .map_or_else(|| DUMMY_PASSWORD_HASH.to_owned(), |row| row.2.clone());
    let password = Zeroizing::new(password.to_owned());
    let valid = tokio::task::spawn_blocking(move || {
        let _verification_permit = verification_permit;
        verify_password(password.as_str(), &hash)
    })
    .await
    .map_err(|error| anyhow::anyhow!("password verification task failed: {error}"))?;
    let Some((id, stored_email, _, enabled, credential_version)) = row else {
        return Ok(None);
    };
    Ok((enabled && valid).then_some(User {
        id,
        email: stored_email,
        role: "admin".to_owned(),
        session_id: None,
        credential_version,
    }))
}

type HmacSha256 = Hmac<Sha256>;

pub async fn create_session(
    pool: &PgPool,
    user: &User,
    key: &[u8],
    user_agent: Option<&str>,
    ip_address: Option<std::net::IpAddr>,
) -> anyhow::Result<String> {
    let token = random_token();
    let token_hash = hash_token(&token, key)?;
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    // Serialize session creation with credential changes and session revocation.
    let version = sqlx::query_scalar::<_, i64>(
        "SELECT credential_version FROM users WHERE id = $1 AND enabled FOR UPDATE",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    anyhow::ensure!(
        version == Some(user.credential_version),
        "credentials changed during login; sign in again"
    );
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND (expires_at <= now() OR last_seen_at <= now() - interval '7 days')")
        .bind(user.id).execute(&mut *tx).await?;
    // Keep a bounded list: a successful login replaces the oldest session at capacity.
    sqlx::query("DELETE FROM sessions WHERE id IN (SELECT id FROM sessions WHERE user_id = $1 ORDER BY created_at DESC, id DESC OFFSET 19)")
        .bind(user.id).execute(&mut *tx).await?;
    let user_agent = user_agent.map(|value| {
        value
            .chars()
            .filter(|c| !c.is_control())
            .take(512)
            .collect::<String>()
    });
    sqlx::query("INSERT INTO sessions (user_id, token_hash, created_at, last_seen_at, expires_at, user_agent, ip_address) VALUES ($1, $2, $3, $3, $4, $5, $6::text::inet)")
        .bind(user.id).bind(token_hash).bind(now).bind(now + Duration::days(30)).bind(user_agent)
        .bind(ip_address.map(|ip| ip.to_string()))
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(token)
}

pub async fn lookup_session(
    pool: &PgPool,
    token: &str,
    key: &[u8],
) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, (Uuid, String, String, Uuid, i64)>(
        "UPDATE sessions s SET last_seen_at = now() FROM users u
         WHERE u.id = s.user_id AND s.token_hash = $1 AND s.expires_at > now()
           AND s.last_seen_at > now() - interval '7 days' AND u.enabled
         RETURNING u.id, u.email, u.role, s.id, u.credential_version",
    )
    .bind(hash_token(token, key)?)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(id, email, role, session_id, credential_version)| User {
            id,
            email,
            role,
            session_id: Some(session_id),
            credential_version,
        }),
    )
}

pub async fn revoke_session(pool: &PgPool, token: &str, key: &[u8]) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(hash_token(token, key)?)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn user_from_headers(
    pool: &PgPool,
    headers: &HeaderMap,
    key: &[u8],
) -> Result<User, ApiError> {
    let token = cookie(headers, "statusdeck_session")
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    lookup_session(pool, &token, key)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::unauthorized("authentication required"))
}

#[must_use]
pub fn csrf_cookie(headers: &HeaderMap) -> Option<String> {
    cookie(headers, "statusdeck_csrf")
}

#[must_use]
pub fn csrf_valid(headers: &HeaderMap) -> bool {
    let cookie_value = csrf_cookie(headers);
    let header_value = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok());
    cookie_value
        .as_deref()
        .zip(header_value)
        .is_some_and(|(cookie_value, header_value)| {
            cookie_value == header_value && !cookie_value.is_empty()
        })
}

#[must_use]
pub fn set_cookie(name: &str, value: &str, http_only: bool, secure: bool) -> String {
    let mut cookie = format!("{name}={value}; Path=/; SameSite=Lax");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_cookie(name: &str, secure: bool) -> String {
    let mut cookie = format!("{name}=; Path=/; Max-Age=0; SameSite=Lax");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str, key: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("session key could not initialize HMAC"))?;
    mac.update(token.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}
