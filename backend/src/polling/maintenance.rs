use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::events::{fanout_event, insert_event};
use crate::{
    config::Config,
    domain::{semantic_hash, stale_threshold},
};

#[derive(Debug, FromRow)]
struct FreshnessSource {
    id: Uuid,
    source_key: String,
    adapter: String,
    poll_interval_seconds: i32,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    first_failure_at: Option<DateTime<Utc>>,
    stale_episode_started_at: Option<DateTime<Utc>>,
    stale_episode_event_id: Option<Uuid>,
}

pub(super) async fn freshness_sweep(pool: &PgPool, config: &Config) -> Result<()> {
    let mut tx = pool.begin().await?;
    let sources = sqlx::query_as::<_, FreshnessSource>("SELECT ps.id, ps.source_key, ps.adapter, ps.poll_interval_seconds, ps.last_attempt_at, ps.last_success_at, CASE WHEN ps.last_success_at IS NULL THEN (SELECT min(run.started_at) FROM poll_runs run WHERE run.provider_source_id = ps.id AND run.outcome = 'failure') END AS first_failure_at, ps.stale_episode_started_at, ps.stale_episode_event_id FROM provider_sources ps WHERE ps.enabled AND EXISTS (SELECT 1 FROM providers s JOIN monitored_providers m ON m.provider_id = s.id WHERE s.provider_source_id = ps.id AND m.enabled) FOR UPDATE OF ps")
        .fetch_all(&mut *tx)
        .await?;

    for source in sources {
        let Some(reference) = source
            .last_success_at
            .or(source.first_failure_at)
            .or(source.last_attempt_at)
        else {
            continue;
        };
        let poll_interval =
            Duration::from_secs(u64::try_from(source.poll_interval_seconds).unwrap_or(1));
        let threshold = stale_threshold(poll_interval, config.stale_multiplier);
        let is_stale = Utc::now()
            .signed_duration_since(reference)
            .to_std()
            .unwrap_or_default()
            > threshold;

        if is_stale && source.stale_episode_started_at.is_none() {
            let hash = semantic_hash(&(source.id, "stale", reference));
            let event_id = insert_event(
                &mut tx,
                "source.stale",
                source.id,
                1,
                &hash,
                json!({
                    "source_id": source.id,
                    "source_key": source.source_key,
                    "adapter": source.adapter,
                    "status": "stale",
                    "last_success_at": source.last_success_at,
                    "last_attempt_at": source.last_attempt_at,
                }),
                Uuid::nil(),
            )
            .await?;
            sqlx::query("UPDATE provider_sources SET stale_episode_started_at = now(), stale_episode_event_id = $2 WHERE id = $1")
                .bind(source.id)
                .bind(event_id)
                .execute(&mut *tx)
                .await?;
            if let Some(event_id) = event_id {
                let payload = sqlx::query_scalar::<_, Value>(
                    "SELECT payload FROM notification_events WHERE id = $1",
                )
                .bind(event_id)
                .fetch_one(&mut *tx)
                .await?;
                fanout_event(&mut tx, "source.stale", event_id, source.id, 1, &payload).await?;
            }
        } else if !is_stale && source.stale_episode_started_at.is_some() {
            let hash = semantic_hash(&(source.id, "recovered", source.stale_episode_event_id));
            let event_id = insert_event(
                &mut tx,
                "source.recovered",
                source.id,
                1,
                &hash,
                json!({
                    "source_id": source.id,
                    "source_key": source.source_key,
                    "adapter": source.adapter,
                    "status": "recovered",
                    "last_success_at": source.last_success_at,
                    "last_attempt_at": source.last_attempt_at,
                }),
                Uuid::nil(),
            )
            .await?;
            sqlx::query("UPDATE provider_sources SET stale_episode_started_at = NULL, stale_episode_event_id = NULL WHERE id = $1")
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            if let Some(event_id) = event_id {
                let payload = sqlx::query_scalar::<_, Value>(
                    "SELECT payload FROM notification_events WHERE id = $1",
                )
                .bind(event_id)
                .fetch_one(&mut *tx)
                .await?;
                fanout_event(
                    &mut tx,
                    "source.recovered",
                    event_id,
                    source.id,
                    1,
                    &payload,
                )
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn cleanup_raw_payloads(pool: &PgPool, retention_days: u32) -> Result<()> {
    sqlx::query("DELETE FROM poll_payloads WHERE id IN (SELECT id FROM poll_payloads WHERE captured_at < now() - ($1::text || ' days')::interval ORDER BY captured_at LIMIT 1000)")
        .bind(retention_days.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn cleanup_expired_sessions(pool: &PgPool) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id IN (SELECT id FROM sessions WHERE expires_at <= now() OR last_seen_at <= now() - interval '7 days' ORDER BY expires_at LIMIT 1000)")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM session_history WHERE id IN (SELECT id FROM session_history WHERE ended_at <= now() - interval '90 days' ORDER BY ended_at LIMIT 1000)")
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn heartbeat(
    pool: &PgPool,
    role: &str,
    instance: &str,
    started_at: DateTime<Utc>,
    last_completed_at: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query("INSERT INTO worker_heartbeats (role, instance_id, version, heartbeat_at, started_at, last_completed_at)
        VALUES ($1, $2, $3, now(), $4, $5)
        ON CONFLICT (role) DO UPDATE SET instance_id = EXCLUDED.instance_id,
            version = EXCLUDED.version, heartbeat_at = EXCLUDED.heartbeat_at,
            started_at = EXCLUDED.started_at, last_completed_at = EXCLUDED.last_completed_at")
        .bind(role).bind(instance).bind(env!("CARGO_PKG_VERSION"))
        .bind(started_at).bind(last_completed_at).execute(pool).await?;
    Ok(())
}
