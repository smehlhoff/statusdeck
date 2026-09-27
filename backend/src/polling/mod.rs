pub(crate) mod events;
mod maintenance;

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    config::Config,
    db::Database,
    domain::{
        IncidentLifecycle, NormalizedStatus, ProviderIncident, ProviderSnapshot, semantic_hash,
    },
    providers::{FetchContext, FetchOutcome, ProviderError, adapter_version, registry},
};
use events::{cover_same_poll_events, fanout_poll_events, insert_event};
use maintenance::{cleanup_expired_sessions, cleanup_raw_payloads, freshness_sweep, heartbeat};

pub async fn run(config: Config, database: Database) -> Result<()> {
    let instance = Uuid::new_v4().to_string();
    let result = {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let poller = poller_loop(&config, &database, &instance, shutdown_rx.clone());
        let dispatcher = dispatcher_loop(&config, &database, &instance, shutdown_rx);
        tokio::pin!(poller, dispatcher);
        tokio::select! {
            result = &mut poller => result,
            result = &mut dispatcher => result,
            _ = crate::shutdown_signal() => {
                tracing::info!(worker_instance_id = %instance, "worker shutdown requested");
                let _ = shutdown_tx.send(true);
                match tokio::time::timeout(Duration::from_secs(30), async {
                    tokio::join!(&mut poller, &mut dispatcher)
                }).await {
                    Ok((poller_result, dispatcher_result)) => poller_result.and(dispatcher_result),
                    Err(_) => Err(anyhow!("worker shutdown exceeded the 30-second deadline")),
                }
            },
        }
    };
    database.close().await;
    result
}

async fn poller_loop(
    config: &Config,
    database: &Database,
    instance: &str,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let started_at = Utc::now();
    let mut last_completed_at = None;
    while !*shutdown.borrow() {
        heartbeat(
            &database.pool,
            "poller",
            instance,
            started_at,
            last_completed_at,
        )
        .await?;
        poller_iteration(config, database, instance).await?;
        last_completed_at = Some(Utc::now());
    }
    Ok(())
}

async fn dispatcher_loop(
    config: &Config,
    database: &Database,
    instance: &str,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let started_at = Utc::now();
    let mut last_completed_at = None;
    while !*shutdown.borrow() {
        heartbeat(
            &database.pool,
            "dispatcher",
            instance,
            started_at,
            last_completed_at,
        )
        .await?;
        match dispatcher_iteration(config, database, instance).await {
            Ok(dispatched) => {
                last_completed_at = Some(Utc::now());
                if dispatched {
                    tokio::task::yield_now().await;
                } else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            Err(error) => {
                tracing::error!(worker_instance_id = %instance, %error, "notification dispatch iteration failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Ok(())
}

async fn dispatcher_iteration(
    config: &Config,
    database: &Database,
    instance: &str,
) -> Result<bool> {
    const DELIVERY_CONCURRENCY: usize = 5;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..DELIVERY_CONCURRENCY {
        let pool = database.pool.clone();
        let config = config.clone();
        let owner = instance.to_owned();
        tasks.spawn(async move {
            crate::notifications::dispatcher::dispatch_once(&pool, &config, &owner).await
        });
    }
    let mut dispatched = false;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(did_dispatch)) => dispatched |= did_dispatch,
            Ok(Err(error)) => {
                tracing::error!(worker_instance_id = %instance, %error, "notification dispatch task failed");
            }
            Err(error) => return Err(anyhow!("notification dispatch task failed: {error}")),
        }
    }
    Ok(dispatched)
}

async fn poller_iteration(config: &Config, database: &Database, instance: &str) -> Result<()> {
    let mut excluded = Vec::new();
    let mut per_host = HashMap::<String, usize>::new();
    let mut claimed = Vec::new();
    while claimed.len() < config.global_poll_concurrency {
        let Some(source) = claim_source(&database.pool, instance, &excluded).await? else {
            break;
        };
        let host = url::Url::parse(&source.base_url)
            .ok()
            .and_then(|url| url.host_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| source.id.to_string());
        let host_count = per_host.entry(host).or_default();
        if *host_count >= config.per_host_concurrency {
            excluded.push(source.id);
            release_source(&database.pool, source.id, instance).await?;
            continue;
        }
        *host_count += 1;
        claimed.push(source);
    }
    if claimed.is_empty() {
        tokio::time::sleep(Duration::from_secs(2)).await;
    } else {
        let mut tasks = tokio::task::JoinSet::new();
        for source in claimed {
            let pool = database.pool.clone();
            let owner = instance.to_owned();
            tasks.spawn(async move { poll_source(&pool, &source, &owner).await });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                return Err(anyhow!("poll task failed: {error}"));
            }
        }
    }
    freshness_sweep(&database.pool, config).await?;
    if let Err(error) =
        cleanup_raw_payloads(&database.pool, config.raw_payload_retention_days).await
    {
        tracing::error!(%error, "raw payload cleanup failed");
    }
    if let Err(error) = cleanup_expired_sessions(&database.pool).await {
        tracing::error!(%error, "expired session cleanup failed");
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct Source {
    id: Uuid,
    adapter: String,
    base_url: String,
    official_urls: Vec<String>,
    public_config: serde_json::Value,
    etag: Option<String>,
    last_modified: Option<String>,
    last_success_at: Option<DateTime<Utc>>,
    poll_kind: PollKind,
}

#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
enum PollKind {
    Status,
    History,
}

impl PollKind {
    const fn refresh_history(self) -> bool {
        matches!(self, Self::History)
    }
}

enum FailureSchedule {
    Retry(Option<u64>),
    Regular,
}

struct PollFailure {
    message: String,
    error_class: &'static str,
    http_status: Option<i32>,
    content_type: Option<String>,
    schedule: FailureSchedule,
}

impl PollFailure {
    fn regular(error_class: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error_class,
            http_status: None,
            content_type: None,
            schedule: FailureSchedule::Regular,
        }
    }

    fn retry(error_class: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error_class,
            http_status: None,
            content_type: None,
            schedule: FailureSchedule::Retry(None),
        }
    }
}

async fn claim_source(pool: &PgPool, owner: &str, excluded: &[Uuid]) -> Result<Option<Source>> {
    let mut tx = pool.begin().await?;
    let source = sqlx::query_as::<_, Source>("
        SELECT id, adapter, base_url,
               ARRAY(SELECT DISTINCT provider.official_url FROM providers provider WHERE provider.provider_source_id = provider_sources.id) AS official_urls,
               public_config, etag, last_modified, last_success_at,
               CASE WHEN next_poll_at <= now() THEN 'status' ELSE 'history' END AS poll_kind
        FROM provider_sources
        WHERE enabled AND (next_poll_at <= now() OR history_next_poll_at <= now())
          AND (lease_until IS NULL OR lease_until < now())
          AND NOT (id = ANY($1))
          AND EXISTS (SELECT 1 FROM providers s JOIN monitored_providers m ON m.provider_id = s.id WHERE s.provider_source_id = provider_sources.id AND m.enabled)
        ORDER BY CASE WHEN next_poll_at <= now() THEN 0 ELSE 1 END,
                 LEAST(next_poll_at, history_next_poll_at), id
        FOR UPDATE SKIP LOCKED LIMIT 1
    ").bind(excluded).fetch_optional(&mut *tx).await?;
    let Some(source) = source else {
        tx.rollback().await?;
        return Ok(None);
    };
    sqlx::query("UPDATE poll_runs SET finished_at = now(), outcome = 'failure', duration_ms = LEAST(EXTRACT(EPOCH FROM (now() - started_at)) * 1000, $2)::integer, error_class = 'abandoned', error_message = 'worker stopped before finalizing poll' WHERE provider_source_id = $1 AND outcome = 'running' AND started_at < now() - interval '2 minutes'")
        .bind(source.id)
        .bind(i32::MAX)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE provider_sources SET lease_owner = $2, lease_until = now() + interval '2 minutes', last_attempt_at = CASE WHEN $3 = 'status' THEN now() ELSE last_attempt_at END, history_last_attempt_at = CASE WHEN $3 = 'history' THEN now() ELSE history_last_attempt_at END WHERE id = $1")
        .bind(source.id).bind(owner).bind(source.poll_kind).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(source))
}

async fn release_source(pool: &PgPool, source_id: Uuid, owner: &str) -> Result<()> {
    sqlx::query(
        "UPDATE provider_sources SET lease_owner = NULL, lease_until = NULL WHERE id = $1 AND lease_owner = $2",
    )
    .bind(source_id)
    .bind(owner)
    .execute(pool)
    .await?;
    Ok(())
}

#[tracing::instrument(
    skip(pool, source, owner),
    fields(source_id = %source.id, adapter = %source.adapter, poll_kind = ?source.poll_kind, worker_instance_id = %owner)
)]
async fn poll_source(pool: &PgPool, source: &Source, owner: &str) {
    let started = Utc::now();
    let poll_run = match sqlx::query_scalar::<_, Uuid>("INSERT INTO poll_runs (provider_source_id, started_at, outcome, adapter_version, poll_kind) VALUES ($1, $2, 'running', $3, $4) RETURNING id")
        .bind(source.id).bind(started).bind(adapter_version(&source.adapter)).bind(source.poll_kind).fetch_one(pool).await {
        Ok(id) => id,
        Err(error) => { tracing::error!(source_id = %source.id, error = %error, "could not create poll run"); return; }
    };
    let known = match known_incidents(pool, source.id).await {
        Ok(known) => known,
        Err(error) => {
            finish_failure(
                pool,
                source,
                poll_run,
                started,
                PollFailure::retry("database", error.to_string()),
                owner,
            )
            .await;
            return;
        }
    };
    let base_url = match url::Url::parse(&source.base_url) {
        Ok(url) => url,
        Err(error) => {
            finish_failure(
                pool,
                source,
                poll_run,
                started,
                PollFailure::regular("configuration", format!("invalid source URL: {error}")),
                owner,
            )
            .await;
            return;
        }
    };
    let provider = match registry(&source.adapter) {
        Ok(provider) => provider,
        Err(error) => {
            finish_failure(
                pool,
                source,
                poll_run,
                started,
                PollFailure::regular("configuration", error.to_string()),
                owner,
            )
            .await;
            return;
        }
    };
    let mut provider_config = source.public_config.clone();
    if let Value::Object(config) = &mut provider_config {
        config.insert("base_url".into(), Value::String(source.base_url.clone()));
    }
    if let Err(error) = provider.validate_config(&provider_config) {
        finish_failure(
            pool,
            source,
            poll_run,
            started,
            PollFailure::regular("configuration", error.to_string()),
            owner,
        )
        .await;
        return;
    }
    let context = FetchContext {
        base_url,
        public_config: provider_config,
        etag: source.etag.clone(),
        last_modified: source.last_modified.clone(),
        known_active_incidents: known,
        refresh_history: source.poll_kind.refresh_history(),
        deadline: std::time::Instant::now() + Duration::from_secs(20),
    };
    let fetched =
        tokio::time::timeout_at(context.deadline.into(), provider.fetch_snapshot(&context))
            .await
            .unwrap_or_else(|_| {
                Err(ProviderError::Transport(
                    "provider request deadline elapsed".into(),
                ))
            });
    match fetched {
        Ok(FetchOutcome::NotModified {
            status,
            etag,
            last_modified,
        }) => {
            let update = finish_source_success(
                pool,
                source,
                owner,
                etag.or_else(|| source.etag.clone()),
                last_modified.or_else(|| source.last_modified.clone()),
                None,
            )
            .await;
            match update {
                Ok(result) if result.rows_affected() == 1 => {}
                Ok(_) => {
                    finish_abandoned_run(pool, poll_run, started).await;
                    return;
                }
                Err(error) => {
                    tracing::error!(source_id = %source.id, %error, "could not record conditional poll success");
                    return;
                }
            }
            finish_run(
                pool,
                poll_run,
                started,
                "not_modified",
                Some(i32::from(status)),
                0,
                0,
            )
            .await;
        }
        Ok(FetchOutcome::Fetched(mut snapshot)) => {
            for incident in snapshot
                .incidents
                .iter_mut()
                .chain(snapshot.maintenance.iter_mut())
            {
                crate::providers::normalize_incident_timestamps(incident, &source.adapter);
            }
            crate::providers::limit_history(&mut snapshot, &context);
            let history_available_from = snapshot
                .incidents
                .iter()
                .chain(&snapshot.maintenance)
                .filter_map(|incident| {
                    incident
                        .started_at
                        .or(incident.created_at)
                        .or(incident.planned_start_at)
                })
                .min();
            sanitize_incident_links(
                &mut snapshot,
                &source.adapter,
                &context.base_url,
                &source.official_urls,
            );
            match reconcile(pool, source, poll_run, &snapshot, owner).await {
                Ok((components, incidents)) => {
                    let update = finish_source_success(
                        pool,
                        source,
                        owner,
                        snapshot.response.etag,
                        snapshot.response.last_modified,
                        history_available_from,
                    )
                    .await;
                    match update {
                        Ok(result) if result.rows_affected() == 1 => {}
                        Ok(_) => {
                            finish_abandoned_run(pool, poll_run, started).await;
                            return;
                        }
                        Err(error) => {
                            tracing::error!(source_id = %source.id, %error, "could not record poll success");
                            return;
                        }
                    }
                    finish_run(
                        pool,
                        poll_run,
                        started,
                        "success",
                        Some(i32::from(snapshot.response.status)),
                        components,
                        incidents,
                    )
                    .await;
                }
                Err(error) => {
                    finish_failure(
                        pool,
                        source,
                        poll_run,
                        started,
                        PollFailure::retry("reconciliation", error.to_string()),
                        owner,
                    )
                    .await
                }
            }
        }
        Err(error) => {
            let schedule = if error.is_retryable() {
                FailureSchedule::Retry(error.retry_after_seconds())
            } else {
                FailureSchedule::Regular
            };
            let failure = PollFailure {
                message: error.to_string(),
                error_class: error.class(),
                http_status: error.http_status().map(i32::from),
                content_type: error.response_content_type().map(ToOwned::to_owned),
                schedule,
            };
            finish_failure(pool, source, poll_run, started, failure, owner).await
        }
    }
}

async fn finish_source_success(
    pool: &PgPool,
    source: &Source,
    owner: &str,
    etag: Option<String>,
    last_modified: Option<String>,
    history_available_from: Option<DateTime<Utc>>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    match source.poll_kind {
        PollKind::Status => sqlx::query("UPDATE provider_sources SET next_poll_at = now() + ((poll_interval_seconds + mod((hashtext(id::text)::bigint & 2147483647), GREATEST(poll_interval_seconds / 10, 1))) * interval '1 second'), lease_owner = NULL, lease_until = NULL, etag = $2, last_modified = $3, last_success_at = now(), consecutive_failures = 0, last_error = NULL WHERE id = $1 AND lease_owner = $4")
            .bind(source.id)
            .bind(etag)
            .bind(last_modified)
            .bind(owner)
            .execute(pool)
            .await,
        PollKind::History => sqlx::query("UPDATE provider_sources SET history_next_poll_at = now() + interval '1 hour', lease_owner = NULL, lease_until = NULL, history_refreshed_at = now(), history_available_from = LEAST(history_available_from, $2), history_consecutive_failures = 0, history_last_error = NULL WHERE id = $1 AND lease_owner = $3")
            .bind(source.id)
            .bind(history_available_from)
            .bind(owner)
            .execute(pool)
            .await,
    }
}

fn sanitize_incident_links(
    snapshot: &mut ProviderSnapshot,
    adapter: &str,
    base_url: &url::Url,
    official_urls: &[String],
) {
    for incident in snapshot
        .incidents
        .iter_mut()
        .chain(snapshot.maintenance.iter_mut())
    {
        if incident.url.as_deref().is_some_and(|candidate| {
            !is_allowed_incident_link(candidate, adapter, base_url, official_urls)
        }) {
            incident.url = None;
        }
    }
}

fn is_allowed_incident_link(
    candidate: &str,
    adapter: &str,
    base_url: &url::Url,
    official_urls: &[String],
) -> bool {
    if candidate.len() > 1024 {
        return false;
    }
    let Ok(candidate) = url::Url::parse(candidate) else {
        return false;
    };
    if candidate.scheme() != "https"
        || candidate.host_str().is_none()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
    {
        return false;
    }
    candidate.host_str() == base_url.host_str()
        || official_urls.iter().any(|official_url| {
            url::Url::parse(official_url)
                .is_ok_and(|official_url| official_url.host_str() == candidate.host_str())
        })
        || (adapter == "datadog"
            && crate::providers::datadog::SITES.iter().any(|(_, origin)| {
                url::Url::parse(origin)
                    .is_ok_and(|origin| candidate.host_str() == origin.host_str())
            }))
        || (matches!(adapter, "statuspage" | "datadog") && candidate.host_str() == Some("stspg.io"))
}

async fn reconcile(
    pool: &PgPool,
    source: &Source,
    poll_run: Uuid,
    snapshot: &ProviderSnapshot,
    owner: &str,
) -> Result<(i32, i32)> {
    let mut tx = pool.begin().await?;
    let lease_extended = sqlx::query("UPDATE provider_sources SET lease_until = now() + interval '2 minutes' WHERE id = $1 AND lease_owner = $2")
        .bind(source.id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    if lease_extended.rows_affected() != 1 {
        return Err(anyhow!("poll lease was lost before reconciliation"));
    }
    let mut semantic_change = false;
    let providers = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, slug, name FROM providers WHERE provider_source_id = $1 AND active",
    )
    .bind(source.id)
    .fetch_all(&mut *tx)
    .await?;
    let upstream_component_ids = snapshot
        .components
        .iter()
        .map(|component| component.upstream_id.clone())
        .collect::<Vec<_>>();
    for (provider_id, _, _) in &providers {
        for component in &snapshot.components {
            sqlx::query(
                "INSERT INTO components (provider_id, upstream_component_id, name, group_name, description, position, active) VALUES ($1, $2, $3, $4, $5, $6, true) ON CONFLICT (provider_id, upstream_component_id) DO UPDATE SET name = EXCLUDED.name, group_name = EXCLUDED.group_name, description = EXCLUDED.description, position = EXCLUDED.position, active = true",
            )
            .bind(provider_id)
            .bind(&component.upstream_id)
            .bind(&component.name)
            .bind(&component.group)
            .bind(&component.description)
            .bind(component.position)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE components SET active = false WHERE provider_id = $1 AND NOT (upstream_component_id = ANY($2))",
        )
        .bind(provider_id)
        .bind(&upstream_component_ids)
        .execute(&mut *tx)
        .await?;
    }
    let mut component_count = 0;
    for (component_id, provider_id, upstream) in sqlx::query_as::<_, (Uuid, Uuid, String)>("SELECT c.id, c.provider_id, c.upstream_component_id FROM components c JOIN providers s ON s.id = c.provider_id WHERE s.provider_source_id = $1 AND c.active").bind(source.id).fetch_all(&mut *tx).await? {
        if let Some(component) = snapshot.components.iter().find(|item| item.upstream_id == upstream) {
            let hash = semantic_hash(&(component.status, &component.original_status));
            let previous = sqlx::query_as::<_, (String, String)>("SELECT semantic_hash, normalized_status FROM component_status_current WHERE component_id = $1").bind(component_id).fetch_optional(&mut *tx).await?;
            sqlx::query("INSERT INTO component_status_current (component_id, provider_id, normalized_status, original_status, observed_at, semantic_hash, poll_run_id) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (component_id) DO UPDATE SET normalized_status = EXCLUDED.normalized_status, original_status = EXCLUDED.original_status, observed_at = EXCLUDED.observed_at, semantic_hash = EXCLUDED.semantic_hash, poll_run_id = EXCLUDED.poll_run_id")
                .bind(component_id).bind(provider_id).bind(component.status.key()).bind(&component.original_status).bind(snapshot.observed_at).bind(&hash).bind(poll_run).execute(&mut *tx).await?;
            if previous.as_ref().is_none_or(|(previous_hash, _)| previous_hash != &hash) {
                semantic_change = true;
            }
            if let Some((_, old_status)) = previous.filter(|(previous_hash, _)| previous_hash != &hash) {
                let provider = providers.iter().find(|(id, _, _)| id == &provider_id);
                sqlx::query("INSERT INTO status_changes (component_id, old_status, new_status, original_status, observed_at, semantic_hash, poll_run_id) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                    .bind(component_id).bind(old_status).bind(component.status.key()).bind(&component.original_status).bind(snapshot.observed_at).bind(&hash).bind(poll_run).execute(&mut *tx).await?;
                insert_event(
                    &mut tx,
                    "component.status_changed",
                    component_id,
                    1,
                    &semantic_hash(&(&hash, poll_run)),
                    json!({
                        "component_id": component_id,
                        "status": component.status,
                        "original_status": component.original_status,
                        "component": {"id": component_id, "name": component.name},
                        "provider": provider.map(|(id, slug, name)| json!({"id": id, "slug": slug, "name": name})),
                    }),
                    poll_run,
                )
                .await?;
            }
            component_count += 1;
        }
    }
    for (provider_id, slug, name) in providers {
        let hash = semantic_hash(&(snapshot.overall, &snapshot.original_overall));
        let previous = sqlx::query_as::<_, (String, String)>(
            "SELECT semantic_hash, normalized_status FROM provider_status_current WHERE provider_id = $1",
        )
        .bind(provider_id)
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO provider_status_current (provider_id, normalized_status, original_status, provider_normalized_status, provider_original_status, observed_at, semantic_hash, poll_run_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (provider_id) DO UPDATE SET normalized_status = EXCLUDED.normalized_status, original_status = EXCLUDED.original_status, provider_normalized_status = EXCLUDED.provider_normalized_status, provider_original_status = EXCLUDED.provider_original_status, observed_at = EXCLUDED.observed_at, semantic_hash = EXCLUDED.semantic_hash, poll_run_id = EXCLUDED.poll_run_id")
            .bind(provider_id).bind(snapshot.overall.key()).bind(&snapshot.original_overall).bind(snapshot.provider_overall.key()).bind(&snapshot.provider_original_overall).bind(snapshot.observed_at).bind(&hash).bind(poll_run).execute(&mut *tx).await?;
        if previous
            .as_ref()
            .is_none_or(|(previous_hash, _)| previous_hash != &hash)
        {
            semantic_change = true;
        }
        if let Some((_, old_status)) = previous.filter(|(previous_hash, _)| previous_hash != &hash)
        {
            sqlx::query("INSERT INTO status_changes (provider_id, old_status, new_status, original_status, observed_at, semantic_hash, poll_run_id) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                .bind(provider_id).bind(old_status).bind(snapshot.overall.key()).bind(&snapshot.original_overall).bind(snapshot.observed_at).bind(&hash).bind(poll_run).execute(&mut *tx).await?;
            insert_event(
                &mut tx,
                "provider.status_changed",
                provider_id,
                1,
                &semantic_hash(&(&hash, poll_run)),
                json!({"provider_id": provider_id, "status": snapshot.overall, "original_status": snapshot.original_overall, "provider": {"id": provider_id, "slug": slug, "name": name}}),
                poll_run,
            )
            .await?;
        }
    }
    let mut incident_count = 0;
    let seen_incidents = snapshot
        .incidents
        .iter()
        .chain(snapshot.maintenance.iter())
        .map(|incident| incident.upstream_id.as_str())
        .collect::<HashSet<_>>();
    for incident in snapshot.incidents.iter().chain(snapshot.maintenance.iter()) {
        semantic_change |= reconcile_incident(
            &mut tx,
            source.id,
            poll_run,
            source.last_success_at.is_none(),
            incident,
        )
        .await?;
        incident_count += 1;
    }
    if snapshot.active_incident_set_complete {
        semantic_change |=
            reconcile_absences(&mut tx, source.id, poll_run, &seen_incidents).await?;
    }
    cover_same_poll_events(&mut tx, poll_run).await?;
    fanout_poll_events(&mut tx, poll_run).await?;
    if semantic_change && let Some(raw_payload) = &snapshot.raw_payload {
        sqlx::query("INSERT INTO poll_payloads (poll_run_id, provider_source_id, payload, content_type) VALUES ($1, $2, $3, $4) ON CONFLICT (poll_run_id) DO NOTHING")
            .bind(poll_run)
            .bind(source.id)
            .bind(raw_payload)
            .bind(&snapshot.response.content_type)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((component_count, incident_count))
}

async fn reconcile_incident(
    tx: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    poll_run: Uuid,
    baseline: bool,
    incident: &ProviderIncident,
) -> Result<bool> {
    let historical_import = incident
        .metadata
        .get("_statusdeck_history")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut provider_metadata = incident.metadata.clone();
    if let Some(metadata) = provider_metadata.as_object_mut() {
        metadata.remove("_statusdeck_history");
    }
    let providers_snapshot = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name, 'slug', provider.slug) ORDER BY provider.name), '[]'::jsonb) FROM providers provider WHERE provider.provider_source_id = $1 AND provider.active")
        .bind(source_id)
        .fetch_one(&mut **tx)
        .await?;
    let components_snapshot = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', component.id, 'upstream_id', component.upstream_component_id, 'name', component.name, 'status', current.normalized_status) ORDER BY component.position, component.name), '[]'::jsonb) FROM components component JOIN providers provider ON provider.id = component.provider_id LEFT JOIN component_status_current current ON current.component_id = component.id WHERE provider.provider_source_id = $1 AND component.upstream_component_id = ANY($2)")
        .bind(source_id)
        .bind(&incident.affected_components)
        .fetch_one(&mut **tx)
        .await?;
    let fingerprint = incident_fingerprint(incident);
    let existing = sqlx::query_as::<_, (Uuid, String, String, i32)>("SELECT id, lifecycle, current_fingerprint, lifecycle_generation FROM incidents WHERE provider_source_id = $1 AND upstream_incident_id = $2")
        .bind(source_id).bind(&incident.upstream_id).fetch_optional(&mut **tx).await?;
    let mut changed = false;
    let id = if let Some((id, lifecycle, previous, generation)) = existing {
        if lifecycle == "resolved" && incident.lifecycle != IncidentLifecycle::Resolved {
            changed = true;
            sqlx::query("UPDATE incidents SET kind = $2, title = $3, official_url = $4, lifecycle = $18, original_phase = $5, severity = $6, original_impact = $7, provider_created_at = $8, provider_updated_at = $9, provider_resolved_at = $10, planned_start_at = $11, planned_end_at = $12, within_provider_scope = $13, lifecycle_generation = lifecycle_generation + 1, current_fingerprint = $14, provider_started_at = $15, provider_monitoring_at = $16, provider_metadata = $17, last_observed_at = now(), absence_count = 0 WHERE id = $1")
                .bind(id).bind(incident.kind.key()).bind(&incident.title).bind(&incident.url).bind(&incident.original_phase).bind(incident.severity.key()).bind(&incident.original_impact).bind(incident.created_at).bind(incident.updated_at).bind(incident.resolved_at).bind(incident.planned_start_at).bind(incident.planned_end_at).bind(incident.within_provider_scope).bind(&fingerprint).bind(incident.started_at).bind(incident.monitoring_at).bind(&provider_metadata).bind(incident.lifecycle.key()).execute(&mut **tx).await?;
            insert_event(
                tx,
                "incident.reopened",
                id,
                generation + 1,
                &fingerprint,
                incident_event_payload(json!({"incident_id": id, "title": incident.title, "kind": incident.kind, "severity": incident.severity, "lifecycle": incident.lifecycle, "provider_phase": incident.original_phase, "message": incident.updates.iter().max_by_key(|update| update.created_at).map(|update| update.body.as_str()), "official_url": incident.url, "generation": generation + 1}), &providers_snapshot, &components_snapshot),
                poll_run,
            )
            .await?;
        } else {
            sqlx::query("UPDATE incidents SET kind = $2, title = $3, official_url = $4, lifecycle = $5, original_phase = $6, severity = $7, original_impact = $8, provider_created_at = $9, provider_updated_at = $10, provider_resolved_at = $11, planned_start_at = $12, planned_end_at = $13, within_provider_scope = $14, current_fingerprint = $15, provider_started_at = $16, provider_monitoring_at = $17, provider_metadata = $18, last_observed_at = now(), absence_count = 0 WHERE id = $1")
                .bind(id).bind(incident.kind.key()).bind(&incident.title).bind(&incident.url).bind(incident.lifecycle.key()).bind(&incident.original_phase).bind(incident.severity.key()).bind(&incident.original_impact).bind(incident.created_at).bind(incident.updated_at).bind(incident.resolved_at).bind(incident.planned_start_at).bind(incident.planned_end_at).bind(incident.within_provider_scope).bind(&fingerprint).bind(incident.started_at).bind(incident.monitoring_at).bind(&provider_metadata).execute(&mut **tx).await?;
            changed |= previous != fingerprint;
            if previous != fingerprint && (!historical_import || lifecycle != "resolved") {
                let event_type = if lifecycle != "resolved"
                    && matches!(incident.lifecycle, IncidentLifecycle::Resolved)
                {
                    "incident.resolved"
                } else {
                    "incident.updated"
                };
                insert_event(
                    tx,
                    event_type,
                    id,
                    generation,
                    &fingerprint,
                    incident_event_payload(json!({"incident_id": id, "title": incident.title, "kind": incident.kind, "severity": incident.severity, "lifecycle": incident.lifecycle, "provider_phase": incident.original_phase, "message": incident.updates.iter().max_by_key(|update| update.created_at).map(|update| update.body.as_str()), "official_url": incident.url}), &providers_snapshot, &components_snapshot),
                    poll_run,
                )
                .await?;
            }
        }
        id
    } else {
        changed = true;
        let id = sqlx::query_scalar::<_, Uuid>("INSERT INTO incidents (provider_source_id, upstream_incident_id, kind, title, official_url, lifecycle, original_phase, severity, original_impact, provider_created_at, provider_updated_at, provider_resolved_at, planned_start_at, planned_end_at, within_provider_scope, first_observed_at, last_observed_at, current_fingerprint, provider_started_at, provider_monitoring_at, provider_metadata) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, now(), now(), $16, $17, $18, $19) RETURNING id")
            .bind(source_id).bind(&incident.upstream_id).bind(incident.kind.key()).bind(&incident.title).bind(&incident.url).bind(incident.lifecycle.key()).bind(&incident.original_phase).bind(incident.severity.key()).bind(&incident.original_impact).bind(incident.created_at).bind(incident.updated_at).bind(incident.resolved_at).bind(incident.planned_start_at).bind(incident.planned_end_at).bind(incident.within_provider_scope).bind(&fingerprint).bind(incident.started_at).bind(incident.monitoring_at).bind(&provider_metadata).fetch_one(&mut **tx).await?;
        // A record first seen after recovery is history, not a new opening.
        if !historical_import && incident.lifecycle != IncidentLifecycle::Resolved {
            let event_type = if baseline {
                "incident.detected"
            } else {
                "incident.started"
            };
            insert_event(
                tx,
                event_type,
                id,
                1,
                &fingerprint,
                incident_event_payload(json!({"incident_id": id, "title": incident.title, "severity": incident.severity, "kind": incident.kind, "lifecycle": incident.lifecycle, "provider_phase": incident.original_phase, "message": incident.updates.iter().max_by_key(|update| update.created_at).map(|update| update.body.as_str()), "official_url": incident.url}), &providers_snapshot, &components_snapshot),
                poll_run,
            )
            .await?;
        }
        id
    };
    sqlx::query("DELETE FROM incident_providers WHERE incident_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO incident_providers (incident_id, provider_id) SELECT $1, id FROM providers WHERE provider_source_id = $2 AND active")
        .bind(id)
        .bind(source_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM incident_components WHERE incident_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO incident_components (incident_id, component_id) SELECT $1, c.id FROM components c JOIN providers s ON s.id = c.provider_id WHERE s.provider_source_id = $2 AND c.upstream_component_id = ANY($3)")
        .bind(id)
        .bind(source_id)
        .bind(&incident.affected_components)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM incident_affected_scopes WHERE incident_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for scope in &incident.affected_scopes {
        sqlx::query("INSERT INTO incident_affected_scopes (incident_id, scope_type, upstream_id, display_name, normalized_status, original_status) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(id).bind(&scope.scope_type).bind(&scope.upstream_id).bind(&scope.display_name).bind(scope.normalized_status.map(NormalizedStatus::key)).bind(&scope.original_status).execute(&mut **tx).await?;
    }
    for update in &incident.updates {
        let hash = semantic_hash(&(update.status.clone(), update.body.clone()));
        let update_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO incident_updates (incident_id, upstream_update_id, body, original_status, provider_created_at, provider_updated_at, provider_display_at, semantic_hash) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING RETURNING id")
            .bind(id).bind(&update.upstream_id).bind(&update.body).bind(&update.status).bind(update.created_at).bind(update.updated_at).bind(update.display_at).bind(&hash).fetch_optional(&mut **tx).await?;
        let update_id = if let Some(update_id) = update_id {
            changed = true;
            update_id
        } else {
            let (update_id, previous_hash) = sqlx::query_as::<_, (Uuid, String)>("SELECT id, semantic_hash FROM incident_updates WHERE incident_id = $1 AND ((upstream_update_id IS NOT NULL AND upstream_update_id = $2) OR (upstream_update_id IS NULL AND $2::text IS NULL AND semantic_hash = $3 AND provider_created_at IS NOT DISTINCT FROM $4)) ORDER BY id LIMIT 1")
                .bind(id).bind(&update.upstream_id).bind(&hash).bind(update.created_at).fetch_one(&mut **tx).await?;
            changed |= previous_hash != hash;
            update_id
        };
        sqlx::query("UPDATE incident_updates SET body = $2, original_status = $3, provider_created_at = $4, provider_updated_at = $5, provider_display_at = $6, semantic_hash = $7 WHERE id = $1")
            .bind(update_id).bind(&update.body).bind(&update.status).bind(update.created_at).bind(update.updated_at).bind(update.display_at).bind(&hash).execute(&mut **tx).await?;
        sqlx::query("DELETE FROM incident_update_components WHERE incident_update_id = $1")
            .bind(update_id)
            .execute(&mut **tx)
            .await?;
        let component_scope_ids = update
            .affected_scopes
            .iter()
            .filter(|scope| scope.scope_type == "component")
            .map(|scope| scope.upstream_id.as_str())
            .collect::<Vec<_>>();
        sqlx::query("INSERT INTO incident_update_components (incident_update_id, component_id) SELECT $1, c.id FROM components c JOIN providers s ON s.id = c.provider_id WHERE s.provider_source_id = $2 AND c.upstream_component_id = ANY($3) ON CONFLICT DO NOTHING")
            .bind(update_id)
            .bind(source_id)
            .bind(&component_scope_ids)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM incident_update_affected_scopes WHERE incident_update_id = $1")
            .bind(update_id)
            .execute(&mut **tx)
            .await?;
        for scope in &update.affected_scopes {
            sqlx::query("INSERT INTO incident_update_affected_scopes (incident_update_id, scope_type, upstream_id, display_name, normalized_status, original_status) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(update_id).bind(&scope.scope_type).bind(&scope.upstream_id).bind(&scope.display_name).bind(scope.normalized_status.map(NormalizedStatus::key)).bind(&scope.original_status).execute(&mut **tx).await?;
        }
    }
    Ok(changed)
}

fn incident_fingerprint(incident: &ProviderIncident) -> String {
    let mut affected_components = incident
        .affected_components
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    affected_components.sort_unstable();
    affected_components.dedup();

    let mut affected_scopes = incident
        .affected_scopes
        .iter()
        .map(|scope| {
            (
                scope.scope_type.as_str(),
                scope.upstream_id.as_str(),
                scope.display_name.as_str(),
                scope.normalized_status,
                scope.original_status.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    affected_scopes.sort_unstable();
    affected_scopes.dedup();

    semantic_hash(&(
        &incident.title,
        incident.kind,
        incident.lifecycle,
        &incident.original_phase,
        incident.severity,
        &incident.original_impact,
        affected_components,
        incident
            .updates
            .iter()
            .max_by_key(|update| update.created_at)
            .map(|update| {
                (
                    &update.upstream_id,
                    &update.status,
                    &update.body,
                    update.created_at,
                )
            }),
        incident.created_at,
        incident.resolved_at,
        incident.planned_start_at,
        incident.planned_end_at,
        affected_scopes,
        incident.within_provider_scope,
    ))
}

fn incident_event_payload(mut payload: Value, providers: &Value, components: &Value) -> Value {
    if let Value::Object(map) = &mut payload {
        map.insert("providers".into(), providers.clone());
        map.insert("components".into(), components.clone());
    }
    payload
}

async fn reconcile_absences(
    tx: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    poll_run: Uuid,
    seen: &HashSet<&str>,
) -> Result<bool> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, i32, i32, String, String, String, Option<String>)>(
        "SELECT id, upstream_incident_id, lifecycle, absence_count, lifecycle_generation, kind, severity, title, official_url FROM incidents WHERE provider_source_id = $1 AND lifecycle IN ('open', 'resolution_pending')",
    )
    .bind(source_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut changed = false;
    for (
        id,
        upstream_id,
        lifecycle,
        absence_count,
        generation,
        kind,
        severity,
        title,
        official_url,
    ) in rows
    {
        if seen.contains(upstream_id.as_str()) {
            continue;
        }
        if absence_count == 0 {
            changed = true;
            sqlx::query("UPDATE incidents SET lifecycle = 'resolution_pending', absence_count = 1, last_observed_at = now() WHERE id = $1")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        } else if lifecycle == "resolution_pending" {
            changed = true;
            let hash = semantic_hash(&("absence_resolved", id, generation));
            let providers_snapshot = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name, 'slug', provider.slug) ORDER BY provider.name), '[]'::jsonb) FROM incident_providers link JOIN providers provider ON provider.id = link.provider_id WHERE link.incident_id = $1")
                .bind(id)
                .fetch_one(&mut **tx)
                .await?;
            let components_snapshot = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', component.id, 'upstream_id', component.upstream_component_id, 'name', component.name, 'status', current.normalized_status) ORDER BY component.position, component.name), '[]'::jsonb) FROM incident_components link JOIN components component ON component.id = link.component_id LEFT JOIN component_status_current current ON current.component_id = component.id WHERE link.incident_id = $1")
                .bind(id)
                .fetch_one(&mut **tx)
                .await?;
            sqlx::query("UPDATE incidents SET lifecycle = 'resolved', absence_count = 2, last_observed_at = now(), current_fingerprint = $2 WHERE id = $1")
                .bind(id)
                .bind(&hash)
                .execute(&mut **tx)
                .await?;
            sqlx::query("INSERT INTO incident_updates (incident_id, body, original_status, semantic_hash, synthesized) VALUES ($1, 'No longer present in the complete provider incident set.', 'resolved_by_absence', $2, true) ON CONFLICT DO NOTHING")
                .bind(id)
                .bind(&hash)
                .execute(&mut **tx)
                .await?;
            insert_event(
                tx,
                "incident.resolved",
                id,
                generation,
                &hash,
                incident_event_payload(json!({"incident_id": id, "generation": generation, "kind": kind, "severity": severity, "title": title, "official_url": official_url, "lifecycle": "resolved", "synthesized": true}), &providers_snapshot, &components_snapshot),
                poll_run,
            )
            .await?;
        }
    }
    Ok(changed)
}

async fn known_incidents(pool: &PgPool, source_id: Uuid) -> Result<HashSet<String>> {
    Ok(sqlx::query_scalar::<_, String>("SELECT upstream_incident_id FROM incidents WHERE provider_source_id = $1 AND lifecycle IN ('open', 'resolution_pending')").bind(source_id).fetch_all(pool).await?.into_iter().collect())
}

async fn finish_run(
    pool: &PgPool,
    id: Uuid,
    started: DateTime<Utc>,
    outcome: &str,
    http_status: Option<i32>,
    components: i32,
    incidents: i32,
) {
    let duration = elapsed_milliseconds(started);
    if let Err(error) = sqlx::query("UPDATE poll_runs SET finished_at = now(), outcome = $2, http_status = $3, duration_ms = $4, component_count = $5, incident_count = $6 WHERE id = $1")
        .bind(id).bind(outcome).bind(http_status).bind(duration).bind(components).bind(incidents).execute(pool).await
    {
        tracing::error!(poll_run_id = %id, error = %error, "could not finalize poll run");
    } else {
        tracing::info!(poll_run_id = %id, %outcome, ?http_status, duration_ms = duration, component_count = components, incident_count = incidents, "poll completed");
    }
}

async fn finish_abandoned_run(pool: &PgPool, id: Uuid, started: DateTime<Utc>) {
    let duration = elapsed_milliseconds(started);
    if let Err(error) = sqlx::query("UPDATE poll_runs SET finished_at = now(), outcome = 'failure', duration_ms = $2, error_class = 'abandoned', error_message = 'poll lease was lost before finalization' WHERE id = $1 AND outcome = 'running'")
        .bind(id)
        .bind(duration)
        .execute(pool)
        .await
    {
        tracing::error!(poll_run_id = %id, %error, "could not mark poll run abandoned");
    } else {
        tracing::warn!(poll_run_id = %id, "poll lease was lost before finalization");
    }
}

async fn finish_failure(
    pool: &PgPool,
    source: &Source,
    poll_run: Uuid,
    started: DateTime<Utc>,
    failure: PollFailure,
    owner: &str,
) {
    let duration = elapsed_milliseconds(started);
    if let Err(update_error) = sqlx::query("UPDATE poll_runs SET finished_at = now(), outcome = 'failure', duration_ms = $2, error_message = $3, http_status = $4, error_class = $5 WHERE id = $1").bind(poll_run).bind(duration).bind(&failure.message).bind(failure.http_status).bind(failure.error_class).execute(pool).await {
        tracing::error!(poll_run_id = %poll_run, error = %update_error, "could not finalize failed poll run");
    }
    if let Err(evidence_error) = sqlx::query("INSERT INTO poll_payloads (poll_run_id, provider_source_id, payload, content_type) VALUES ($1, $2, jsonb_build_object('failure', jsonb_build_object('message', $3::text, 'http_status', $4::integer, 'class', $5::text)), $6) ON CONFLICT (poll_run_id) DO NOTHING")
        .bind(poll_run)
        .bind(source.id)
        .bind(&failure.message)
        .bind(failure.http_status)
        .bind(failure.error_class)
        .bind(&failure.content_type)
        .execute(pool)
        .await
    {
        tracing::error!(poll_run_id = %poll_run, error = %evidence_error, "could not retain poll failure evidence");
    }
    let (retry_after_seconds, retryable) = match failure.schedule {
        FailureSchedule::Retry(retry_after) => (
            retry_after.map(|seconds| i64::try_from(seconds.min(1_800)).unwrap_or(1_800)),
            true,
        ),
        FailureSchedule::Regular => (None, false),
    };
    let update = match source.poll_kind {
        PollKind::Status => sqlx::query("UPDATE provider_sources SET next_poll_at = now() + CASE WHEN $4::bigint IS NOT NULL THEN $4 * interval '1 second' WHEN $5 THEN GREATEST(1, floor(random() * LEAST(poll_interval_seconds * (2 ^ LEAST(consecutive_failures + 1, 6)), 1800))::bigint) * interval '1 second' ELSE (poll_interval_seconds + mod((hashtext(id::text)::bigint & 2147483647), GREATEST(poll_interval_seconds / 10, 1))) * interval '1 second' END, lease_owner = NULL, lease_until = NULL, consecutive_failures = consecutive_failures + 1, last_error = $2 WHERE id = $1 AND lease_owner = $3")
            .bind(source.id).bind(&failure.message).bind(owner).bind(retry_after_seconds).bind(retryable).execute(pool).await,
        PollKind::History => sqlx::query("UPDATE provider_sources SET history_next_poll_at = now() + CASE WHEN $4::bigint IS NOT NULL THEN $4 * interval '1 second' WHEN $5 THEN GREATEST(1, floor(random() * LEAST(300 * (2 ^ LEAST(history_consecutive_failures + 1, 6)), 1800))::bigint) * interval '1 second' ELSE interval '1 hour' END, lease_owner = NULL, lease_until = NULL, history_consecutive_failures = history_consecutive_failures + 1, history_last_error = $2 WHERE id = $1 AND lease_owner = $3")
            .bind(source.id).bind(&failure.message).bind(owner).bind(retry_after_seconds).bind(retryable).execute(pool).await,
    };
    if let Err(update_error) = update {
        tracing::error!(source_id = %source.id, error = %update_error, "could not record poll failure");
    } else {
        tracing::warn!(poll_run_id = %poll_run, source_id = %source.id, http_status = ?failure.http_status, error = %failure.message, duration_ms = duration, "poll failed");
    }
}

fn elapsed_milliseconds(started: DateTime<Utc>) -> i32 {
    let milliseconds = Utc::now()
        .signed_duration_since(started)
        .num_milliseconds()
        .clamp(0, i64::from(i32::MAX));
    i32::try_from(milliseconds).unwrap_or(i32::MAX)
}
