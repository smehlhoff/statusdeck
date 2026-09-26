use anyhow::{Result, bail};
use chrono::Utc;
use sqlx::PgPool;

use crate::{auth::hash_password, config::Config};

pub async fn ensure_admin(pool: &PgPool, config: &Config) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(783_443_201)")
        .execute(&mut *tx)
        .await?;
    let existing = sqlx::query_as::<_, (String, bool)>(
        "SELECT email, enabled FROM users WHERE role = 'admin' ORDER BY created_at",
    )
    .fetch_all(&mut *tx)
    .await?;
    if existing.len() > 1 {
        bail!("the database contains more than one administrator");
    }
    if let Some((_, enabled)) = existing.first() {
        if !enabled {
            bail!("the configured administrator account is disabled");
        }
        tx.commit().await?;
        return Ok(());
    }
    let password = config.admin_password.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "STATUSDECK_ADMIN_PASSWORD or STATUSDECK_ADMIN_PASSWORD_FILE is required to create the administrator"
        )
    })?;
    let password = zeroize::Zeroizing::new(password.expose().to_owned());
    let hash = tokio::task::spawn_blocking(move || hash_password(&password)).await??;
    let user_id = sqlx::query_scalar::<_, uuid::Uuid>("INSERT INTO users (email, password_hash, role, enabled, created_at, updated_at) VALUES ($1, $2, 'admin', true, $3, $3) RETURNING id")
        .bind(&config.admin_email)
        .bind(hash)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id) VALUES ($1, 'administrator.bootstrapped', 'user', $1)")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(email = %config.admin_email, "bootstrap administrator ensured");
    Ok(())
}
