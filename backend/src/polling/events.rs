use anyhow::Result;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    event_type: &str,
    entity_id: Uuid,
    lifecycle_generation: i32,
    hash: &str,
    mut payload: Value,
    poll_run: Uuid,
) -> Result<Option<Uuid>> {
    if let Value::Object(map) = &mut payload {
        map.insert("event_type".into(), Value::String(event_type.to_owned()));
        map.insert("poll_run_id".into(), Value::String(poll_run.to_string()));
    }
    let event_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO notification_events (event_type, entity_id, lifecycle_generation, semantic_hash, payload) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING RETURNING id")
        .bind(event_type)
        .bind(entity_id)
        .bind(lifecycle_generation)
        .bind(hash)
        .bind(payload)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(event_id)
}

pub(super) async fn cover_same_poll_events(
    tx: &mut Transaction<'_, Postgres>,
    poll_run: Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE notification_events AS status
         SET covered_by_event_id = incident.id
         FROM notification_events AS incident
         JOIN incidents covered ON covered.id = incident.entity_id AND covered.within_provider_scope
         WHERE status.payload->>'poll_run_id' = $1 AND incident.payload->>'poll_run_id' = $1
           AND status.covered_by_event_id IS NULL AND incident.event_type LIKE 'incident.%'
           AND (
               (status.event_type = 'provider.status_changed' AND EXISTS (
                   SELECT 1 FROM incident_providers link
                   WHERE link.incident_id = incident.entity_id AND link.provider_id = status.entity_id
               ))
               OR (status.event_type = 'component.status_changed' AND EXISTS (
                   SELECT 1 FROM incident_components link
                   WHERE link.incident_id = incident.entity_id AND link.component_id = status.entity_id
               ))
           )",
    )
        .bind(poll_run.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn fanout_poll_events(
    tx: &mut Transaction<'_, Postgres>,
    poll_run: Uuid,
) -> Result<()> {
    let events = sqlx::query_as::<_, (Uuid, String, Uuid, i32, Value, Option<Uuid>)>(
        "SELECT id, event_type, entity_id, COALESCE(lifecycle_generation, 1), payload, covered_by_event_id FROM notification_events WHERE payload->>'poll_run_id' = $1 ORDER BY created_at, id",
    )
    .bind(poll_run.to_string())
    .fetch_all(&mut **tx)
    .await?;
    for (event_id, event_type, entity_id, generation, payload, covered_by) in events {
        if covered_by.is_some() {
            continue;
        }
        fanout_event(tx, &event_type, event_id, entity_id, generation, &payload).await?;
    }
    Ok(())
}

pub(crate) async fn fanout_resolved_incident_after_entry_delivery(
    tx: &mut Transaction<'_, Postgres>,
    incident_id: Uuid,
    lifecycle_generation: i32,
) -> Result<()> {
    let resolution_events = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM notification_events WHERE event_type = 'incident.resolved' AND entity_id = $1 AND lifecycle_generation = $2 ORDER BY created_at, id",
    )
    .bind(incident_id)
    .bind(lifecycle_generation)
    .fetch_all(&mut **tx)
    .await?;
    for (event_id, payload) in resolution_events {
        fanout_event(
            tx,
            "incident.resolved",
            event_id,
            incident_id,
            lifecycle_generation,
            &payload,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn fanout_event(
    tx: &mut Transaction<'_, Postgres>,
    event_type: &str,
    event_id: Uuid,
    entity_id: Uuid,
    generation: i32,
    payload: &Value,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO notification_deliveries (notification_event_id, alert_rule_id, channel_id)
        SELECT $2, rule.id, link.channel_id
        FROM alert_rules rule
        JOIN alert_rule_channels link ON link.alert_rule_id = rule.id
        JOIN notification_channels channel ON channel.id = link.channel_id AND channel.enabled AND channel.deleted_at IS NULL
        WHERE rule.enabled AND rule.deleted_at IS NULL AND (
          (
            $1 LIKE 'incident.%' AND rule.rule_kind = 'provider'
            AND EXISTS (
              SELECT 1 FROM incident_providers affected
              JOIN incidents incident
                ON incident.id = affected.incident_id AND incident.within_provider_scope
              JOIN alert_rule_providers scoped ON scoped.provider_id = affected.provider_id
              JOIN monitored_providers monitored ON monitored.provider_id = affected.provider_id AND monitored.enabled
              WHERE affected.incident_id = $3 AND scoped.alert_rule_id = rule.id
                AND (
                  monitored.monitor_all_components
                  OR NOT EXISTS (
                    SELECT 1 FROM incident_components incident_component
                    WHERE incident_component.incident_id = $3
                  )
                  OR EXISTS (
                    SELECT 1 FROM incident_components incident_component
                    JOIN monitored_components selected_monitor
                      ON selected_monitor.component_id = incident_component.component_id
                    WHERE incident_component.incident_id = $3
                      AND selected_monitor.monitored_provider_id = monitored.id
                  )
                )
                AND (
                  NOT EXISTS (
                    SELECT 1 FROM alert_rule_components selected
                    WHERE selected.alert_rule_id = rule.id
                      AND selected.provider_id = affected.provider_id
                  )
                  OR NOT EXISTS (
                    SELECT 1 FROM incident_components incident_component
                    WHERE incident_component.incident_id = $3
                  )
                  OR EXISTS (
                    SELECT 1 FROM incident_components incident_component
                    JOIN alert_rule_components selected
                      ON selected.component_id = incident_component.component_id
                    WHERE incident_component.incident_id = $3
                      AND selected.alert_rule_id = rule.id
                      AND selected.provider_id = affected.provider_id
                  )
                )
            )
            AND CASE $1
              WHEN 'incident.detected' THEN rule.notify_detected
              WHEN 'incident.started' THEN rule.notify_started
              WHEN 'incident.updated' THEN rule.notify_updated
              WHEN 'incident.resolved' THEN rule.notify_resolved
              WHEN 'incident.reopened' THEN rule.notify_reopened
              ELSE false
            END
            AND (($4::jsonb ->> 'kind') <> 'maintenance' OR rule.notify_maintenance)
            AND (CASE $4::jsonb ->> 'severity' WHEN 'critical' THEN 4 WHEN 'major' THEN 3 WHEN 'minor' THEN 2 ELSE 1 END)
              >= (CASE rule.min_severity WHEN 'critical' THEN 4 WHEN 'major' THEN 3 WHEN 'minor' THEN 2 ELSE 1 END)
            AND ($1 <> 'incident.resolved' OR EXISTS (
              SELECT 1 FROM notification_deliveries previous
              JOIN notification_events previous_event ON previous_event.id = previous.notification_event_id
              WHERE (previous.status IN ('delivered', 'held', 'summarized')
                       OR previous.delivered_at IS NOT NULL OR previous.summary_delivery_id IS NOT NULL)
                AND previous.alert_rule_id = rule.id
                AND previous.channel_id = link.channel_id
                AND previous_event.entity_id = $3
                AND previous_event.lifecycle_generation = $5
                AND previous_event.event_type IN ('incident.detected', 'incident.started', 'incident.reopened')
            ))
          ) OR (
            $1 IN ('provider.status_changed', 'component.status_changed')
            AND rule.rule_kind = 'provider'
            AND (
              ($1 = 'provider.status_changed' AND EXISTS (
                SELECT 1 FROM alert_rule_providers scoped
                JOIN monitored_providers monitored ON monitored.provider_id = scoped.provider_id AND monitored.enabled
                WHERE scoped.alert_rule_id = rule.id AND scoped.provider_id = $3
                  AND monitored.monitor_all_components
                  AND NOT EXISTS (
                    SELECT 1 FROM alert_rule_components selected
                    WHERE selected.alert_rule_id = rule.id AND selected.provider_id = scoped.provider_id
                  )
              ))
              OR ($1 = 'component.status_changed' AND EXISTS (
                SELECT 1 FROM components component
                JOIN alert_rule_providers scoped ON scoped.provider_id = component.provider_id
                JOIN monitored_providers monitored ON monitored.provider_id = component.provider_id AND monitored.enabled
                LEFT JOIN monitored_components selected_monitor
                  ON selected_monitor.monitored_provider_id = monitored.id
                 AND selected_monitor.component_id = component.id
                WHERE component.id = $3 AND scoped.alert_rule_id = rule.id
                  AND (monitored.monitor_all_components OR selected_monitor.component_id IS NOT NULL)
                  AND (
                    NOT EXISTS (
                      SELECT 1 FROM alert_rule_components selected
                      WHERE selected.alert_rule_id = rule.id AND selected.provider_id = component.provider_id
                    )
                    OR EXISTS (
                      SELECT 1 FROM alert_rule_components selected
                      WHERE selected.alert_rule_id = rule.id AND selected.component_id = $3
                    )
                  )
              ))
            )
            AND (($4::jsonb ->> 'status') <> 'operational' OR rule.notify_recovered)
            AND (
              ($4::jsonb ->> 'status') = 'operational'
              OR (CASE $4::jsonb ->> 'status' WHEN 'major_outage' THEN 4 WHEN 'partial_outage' THEN 3 WHEN 'degraded' THEN 2 ELSE 1 END)
                >= (CASE rule.min_severity WHEN 'critical' THEN 4 WHEN 'major' THEN 3 WHEN 'minor' THEN 2 ELSE 1 END)
            )
          ) OR (
            $1 IN ('source.stale', 'source.recovered')
            AND rule.rule_kind = 'system_health'
            AND (
              NOT EXISTS (SELECT 1 FROM alert_rule_providers scoped WHERE scoped.alert_rule_id = rule.id)
              OR EXISTS (
                SELECT 1 FROM alert_rule_providers scoped
                JOIN providers provider ON provider.id = scoped.provider_id
                JOIN monitored_providers monitored ON monitored.provider_id = provider.id AND monitored.enabled
                WHERE scoped.alert_rule_id = rule.id AND provider.provider_source_id = $3
              )
            )
            AND ($1 <> 'source.recovered' OR rule.notify_recovered)
          )
        )
        FOR SHARE OF rule, channel
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(event_type)
    .bind(event_id)
    .bind(entity_id)
    .bind(payload)
    .bind(generation)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
