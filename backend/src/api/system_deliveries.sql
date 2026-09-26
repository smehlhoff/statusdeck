WITH queue AS (
    SELECT d.*, c.enabled AND c.deleted_at IS NULL
        AND (d.resend_requested_at IS NOT NULL OR d.alert_rule_id IS NULL OR (r.enabled AND r.deleted_at IS NULL)) AS eligible
    FROM notification_deliveries d
    JOIN notification_channels c ON c.id = d.channel_id
    LEFT JOIN alert_rules r ON r.id = d.alert_rule_id
    WHERE d.status IN ('pending', 'retrying', 'ambiguous', 'held')
), current_queue AS (
    SELECT count(*) FILTER (WHERE eligible AND status <> 'held') AS queued,
           count(*) FILTER (WHERE NOT eligible) AS paused,
           count(*) FILTER (WHERE eligible AND status = 'held') AS held,
           count(*) FILTER (WHERE eligible AND next_attempt_at <= $2
                AND (lease_until IS NULL OR lease_until < $2)) AS due_now,
           count(*) FILTER (WHERE eligible AND next_attempt_at < $2 - $3 * interval '1 second'
                AND (lease_until IS NULL OR lease_until < $2)) AS overdue,
           max(EXTRACT(EPOCH FROM ($2 - next_attempt_at))) FILTER (
                WHERE eligible AND next_attempt_at < $2 - $3 * interval '1 second'
                AND (lease_until IS NULL OR lease_until < $2)) AS oldest_overdue_seconds,
           min(next_attempt_at) FILTER (WHERE eligible AND status IN ('retrying', 'ambiguous')
                AND next_attempt_at > $2) AS next_retry_at,
           count(*) FILTER (WHERE eligible AND status = 'retrying') AS retrying,
           count(*) FILTER (WHERE eligible AND status = 'ambiguous') AS ambiguous
    FROM queue
), recent_issues AS (
    SELECT count(*) FILTER (WHERE d.status = 'failed') AS failed,
           count(*) FILTER (WHERE d.status = 'ambiguous') AS ambiguous
    FROM notification_deliveries d
    JOIN LATERAL (SELECT attempted_at FROM notification_attempts a WHERE a.delivery_id = d.id
                  ORDER BY attempted_at DESC, id DESC LIMIT 1) latest ON true
    WHERE d.status IN ('failed', 'ambiguous') AND latest.attempted_at >= $1 AND latest.attempted_at <= $2
), recent_delivered AS (
    SELECT count(*) AS delivered,
           count(*) FILTER (WHERE quiet_until IS NULL) AS latency_samples,
           count(*) FILTER (WHERE quiet_until IS NOT NULL) AS delivered_after_hold,
           percentile_cont(0.5) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (delivered_at - queued_at)))
               FILTER (WHERE quiet_until IS NULL) AS median_seconds,
           percentile_cont(0.95) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (delivered_at - queued_at)))
               FILTER (WHERE quiet_until IS NULL) AS p95_seconds
    FROM notification_deliveries WHERE status = 'delivered' AND delivered_at >= $1 AND delivered_at <= $2
), history AS (
    SELECT jsonb_object_agg(status, total) AS totals FROM (
        SELECT status, count(*) AS total FROM notification_deliveries GROUP BY status
    ) counts
), configuration AS (
    SELECT count(*) AS enabled_channels FROM notification_channels WHERE enabled AND deleted_at IS NULL
), work AS (
    SELECT max(attempted_at) AS last_attempt_at,
           count(*) FILTER (WHERE attempted_at >= $1) AS attempts
    FROM notification_attempts WHERE attempted_at <= $2
)
SELECT jsonb_build_object('current', to_jsonb(current_queue),
    'recent', to_jsonb(recent_issues) || to_jsonb(recent_delivered) || jsonb_build_object('attempts', work.attempts),
    'history', COALESCE(history.totals, '{}'::jsonb),
    'enabled_channels', configuration.enabled_channels, 'last_attempt_at', work.last_attempt_at)
FROM current_queue CROSS JOIN recent_issues CROSS JOIN recent_delivered
    CROSS JOIN history CROSS JOIN configuration CROSS JOIN work
