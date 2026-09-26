use anyhow::Result;
use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuietHours {
    pub enabled: bool,
    pub timezone: String,
    pub days: Vec<u8>,
    pub start: String,
    pub end: String,
    pub critical_override: bool,
}

impl QuietHours {
    pub async fn validate(&self, connection: &mut PgConnection) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.days.is_empty()
            || self.days.len() > 7
            || self.days.iter().any(|day| !(1..=7).contains(day))
        {
            return Err("Choose at least one day, using Monday through Sunday.".into());
        }
        let mut days = self.days.clone();
        days.sort_unstable();
        days.dedup();
        if days.len() != self.days.len() {
            return Err("Quiet-hours days must not repeat.".into());
        }
        for time in [&self.start, &self.end] {
            if time.len() != 5 || NaiveTime::parse_from_str(time, "%H:%M").is_err() {
                return Err("Quiet-hours times must use HH:MM in 24-hour format.".into());
            }
        }
        if self.start == self.end {
            return Err("Quiet-hours start and end times must differ.".into());
        }
        if self.timezone.len() > 100 {
            return Err("Choose a valid timezone.".into());
        }
        let valid = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1 AND name NOT LIKE 'posix/%' AND name NOT LIKE 'right/%')")
            .bind(&self.timezone).fetch_one(connection).await.map_err(|_| "Timezone validation is temporarily unavailable.".to_owned())?;
        if !valid {
            return Err("Choose a recognized timezone, such as America/New_York or UTC.".into());
        }
        Ok(())
    }
}

// Serialize scheduling and summary creation across dispatcher instances. The lock
// is held only for database work, never for DNS or outbound HTTP.
pub(super) async fn prepare(tx: &mut Transaction<'_, Postgres>) -> Result<bool> {
    sqlx::query("SELECT pg_advisory_xact_lock(748291603)")
        .execute(&mut **tx)
        .await?;
    let held = sqlx::query(r#"
        WITH candidates AS (
            SELECT d.id, e.event_type,
                   GREATEST(quiet_hours_end(r.quiet_hours, statement_timestamp()),
                       CASE WHEN d.attempt_count = 0 AND e.event_type <> 'system.quiet_summary'
                            THEN quiet_hours_end(r.quiet_hours, d.queued_at) END) AS until
            FROM notification_deliveries d
            JOIN alert_rules r ON r.id = d.alert_rule_id AND r.enabled AND r.deleted_at IS NULL
            JOIN notification_channels c ON c.id = d.channel_id AND c.enabled AND c.deleted_at IS NULL
            JOIN notification_events e ON e.id = d.notification_event_id
            WHERE d.status IN ('pending', 'retrying', 'ambiguous') AND d.next_attempt_at <= now()
              AND d.resend_requested_at IS NULL
              AND (d.lease_until IS NULL OR d.lease_until < now())
              AND NOT (COALESCE((r.quiet_hours->>'critical_override')::boolean, false)
                       AND (COALESCE(e.payload->>'severity', '') = 'critical' OR COALESCE(e.payload->>'status', '') = 'major_outage'))
        ), due AS (
            SELECT d.id, c.until, c.event_type FROM candidates c JOIN notification_deliveries d ON d.id = c.id
            WHERE c.until IS NOT NULL ORDER BY d.id LIMIT 100 FOR UPDATE OF d SKIP LOCKED
        )
        UPDATE notification_deliveries d SET
            status = CASE WHEN due.event_type = 'system.quiet_summary' THEN d.status ELSE 'held' END,
            quiet_until = due.until, next_attempt_at = due.until,
            lease_owner = NULL, lease_until = NULL
        FROM due WHERE d.id = due.id
    "#).execute(&mut **tx).await?.rows_affected();
    // Drain eligible events before constructing summaries, including after downtime.
    if held > 0 {
        return Ok(true);
    }

    let group = sqlx::query_as::<_, (Uuid, Uuid, DateTime<Utc>, String)>(
        r#"
        SELECT d.alert_rule_id, d.channel_id, d.quiet_until, r.name
        FROM notification_deliveries d
        JOIN alert_rules r ON r.id = d.alert_rule_id AND r.enabled AND r.deleted_at IS NULL
        JOIN notification_channels c ON c.id = d.channel_id AND c.enabled AND c.deleted_at IS NULL
        WHERE d.status = 'held' AND d.quiet_until <= now()
        ORDER BY d.quiet_until, d.id LIMIT 1 FOR UPDATE OF d SKIP LOCKED
    "#,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some((rule_id, channel_id, until, name)) = group else {
        return Ok(false);
    };
    let (event_count, entity_count) = sqlx::query_as::<_, (i64, i64)>(r#"
        SELECT count(*), count(DISTINCT (e.entity_id, e.lifecycle_generation, split_part(e.event_type, '.', 1)))
        FROM notification_deliveries d JOIN notification_events e ON e.id = d.notification_event_id
        WHERE d.status = 'held' AND d.alert_rule_id = $1 AND d.channel_id = $2 AND d.quiet_until = $3
    "#).bind(rule_id).bind(channel_id).bind(until).fetch_one(&mut **tx).await?;
    let entries = sqlx::query_scalar::<_, Value>(r#"
        WITH latest AS (
            SELECT DISTINCT ON (e.entity_id, e.lifecycle_generation, split_part(e.event_type, '.', 1))
                   e.entity_id, e.lifecycle_generation, e.event_type, e.payload, e.created_at,
                   min(e.created_at) OVER (PARTITION BY e.entity_id, e.lifecycle_generation, split_part(e.event_type, '.', 1)) AS first_at
            FROM notification_deliveries d JOIN notification_events e ON e.id = d.notification_event_id
            WHERE d.status = 'held' AND d.alert_rule_id = $1 AND d.channel_id = $2 AND d.quiet_until = $3
            ORDER BY e.entity_id, e.lifecycle_generation, split_part(e.event_type, '.', 1), e.created_at DESC, e.id DESC
        ), entries AS (
            SELECT jsonb_build_object(
                'entity_id', l.entity_id, 'event_type', l.event_type,
                'title', COALESCE(i.title, l.payload->>'title', l.payload#>>'{provider,name}', l.payload#>>'{component,name}', l.payload#>>'{source,source_key}', l.event_type),
                'status', COALESCE(i.lifecycle, l.payload->>'lifecycle', l.payload->>'status', l.payload->>'freshness', l.event_type),
                'started_at', COALESCE(i.provider_started_at, l.first_at),
                'resolved_at', CASE WHEN i.lifecycle = 'resolved' THEN COALESCE(i.provider_resolved_at, i.last_observed_at) END,
                'last_event_at', l.created_at
            ) AS entry,
            COALESCE(i.lifecycle IN ('open', 'resolution_pending'), false) AS ongoing
            FROM latest l LEFT JOIN incidents i ON l.event_type LIKE 'incident.%' AND i.id = l.entity_id AND i.lifecycle_generation = l.lifecycle_generation
        ) SELECT entry FROM entries ORDER BY ongoing DESC, entry->>'title', entry->>'entity_id' LIMIT 50
    "#).bind(rule_id).bind(channel_id).bind(until).fetch_all(&mut **tx).await?;
    let mut lines = Vec::with_capacity(entries.len() + 1);
    for entry in &entries {
        let title = entry["title"].as_str().unwrap_or("Event");
        let status = entry["status"].as_str().unwrap_or("changed");
        let start = entry["started_at"].as_str().unwrap_or("unknown time");
        let timing = if let Some(end) = entry["resolved_at"].as_str() {
            format!("{start} → {end}")
        } else if matches!(status, "open" | "resolution_pending") {
            format!("still active; since {start}")
        } else {
            format!(
                "last event {}",
                entry["last_event_at"].as_str().unwrap_or(start)
            )
        };
        lines.push(format!("{title}: {status} ({timing})"));
    }
    let omitted = entity_count.saturating_sub(i64::try_from(entries.len())?);
    if omitted > 0 {
        lines.push(format!(
            "{omitted} more entries; see delivery history in StatusDeck."
        ));
    }
    let payload = json!({"title": format!("Quiet-hours summary: {name}"), "status": "summary", "message": lines.join("\n"), "quiet_until": until, "event_count": event_count, "entries": entries, "omitted_entries": omitted});
    let event_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO notification_events (event_type, entity_id, semantic_hash, payload) VALUES ('system.quiet_summary', $1, $2, $3) RETURNING id")
        .bind(rule_id).bind(Uuid::new_v4().to_string()).bind(payload).fetch_one(&mut **tx).await?;
    let summary_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO notification_deliveries (notification_event_id, alert_rule_id, channel_id) VALUES ($1, $2, $3) RETURNING id")
        .bind(event_id).bind(rule_id).bind(channel_id).fetch_one(&mut **tx).await?;
    sqlx::query("UPDATE notification_deliveries SET status = 'summarized', summary_delivery_id = $4 WHERE status = 'held' AND alert_rule_id = $1 AND channel_id = $2 AND quiet_until = $3")
        .bind(rule_id).bind(channel_id).bind(until).bind(summary_id).execute(&mut **tx).await?;
    Ok(true)
}
