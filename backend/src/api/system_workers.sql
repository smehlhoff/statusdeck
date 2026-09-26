SELECT heartbeat.role, heartbeat.instance_id, heartbeat.version,
       heartbeat.heartbeat_at, heartbeat.started_at, heartbeat.last_completed_at,
       heartbeat.heartbeat_at > $1 - $2 * interval '1 second' AS fresh,
       activity.in_progress, activity.expired_claims, activity.oldest_started_at
FROM worker_heartbeats heartbeat
CROSS JOIN LATERAL (
    SELECT count(*) FILTER (WHERE lease_until > $1) AS in_progress,
           count(*) FILTER (WHERE lease_until <= $1) AS expired_claims,
           min(work_started_at) FILTER (WHERE lease_until > $1) AS oldest_started_at
    FROM (
        SELECT source.lease_until, source.last_attempt_at AS work_started_at
        FROM provider_sources source
        WHERE heartbeat.role = 'poller' AND source.lease_owner = heartbeat.instance_id
            AND source.lease_until IS NOT NULL
        UNION ALL
        SELECT delivery.lease_until, delivery.lease_until - $3 * interval '1 second' AS work_started_at
        FROM notification_deliveries delivery
        WHERE heartbeat.role = 'dispatcher' AND delivery.lease_owner = heartbeat.instance_id
            AND delivery.lease_until IS NOT NULL
    ) claimed
) activity
ORDER BY heartbeat.role
