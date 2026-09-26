WITH active_sources AS (
    SELECT source.* FROM provider_sources source
    WHERE source.enabled AND EXISTS (
        SELECT 1 FROM providers provider JOIN monitored_providers monitor ON monitor.provider_id = provider.id
        WHERE provider.provider_source_id = source.id AND monitor.enabled
    )
), scheduling AS (
    SELECT count(*) FILTER (WHERE next_poll_at < $2 - poll_interval_seconds * interval '1 second'
                               AND (lease_until IS NULL OR lease_until < $2)) AS overdue,
           max(EXTRACT(EPOCH FROM ($2 - next_poll_at))) FILTER (
               WHERE next_poll_at < $2 AND (lease_until IS NULL OR lease_until < $2)) AS longest_delay_seconds,
           COALESCE(jsonb_agg(id) FILTER (WHERE next_poll_at < $2 - poll_interval_seconds * interval '1 second'
                               AND (lease_until IS NULL OR lease_until < $2)), '[]'::jsonb) AS overdue_source_ids
    FROM active_sources
), recent AS (
    SELECT count(*) AS completed,
           count(*) FILTER (WHERE outcome IN ('success', 'not_modified')) AS successful,
           count(*) FILTER (WHERE outcome = 'failure') AS failed,
           percentile_cont(0.5) WITHIN GROUP (ORDER BY duration_ms) AS median_ms,
           percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms) AS p95_ms
    FROM poll_runs WHERE poll_kind = 'status' AND finished_at >= $1 AND finished_at <= $2
        AND provider_source_id IN (SELECT id FROM active_sources)
), work AS (
    SELECT max(finished_at) AS last_completed_at FROM poll_runs
    WHERE poll_kind = 'status' AND provider_source_id IN (SELECT id FROM active_sources) AND finished_at <= $2
)
SELECT to_jsonb(scheduling) || to_jsonb(recent) || to_jsonb(work)
FROM scheduling CROSS JOIN recent CROSS JOIN work
