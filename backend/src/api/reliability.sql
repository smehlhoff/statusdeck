WITH intervals AS MATERIALIZED (
    SELECT i.id, i.kind, i.severity, timing.count_at, timing.start_at,
           CASE WHEN timing.end_at IS NOT NULL THEN LEAST(timing.end_at, $3) END AS end_at, timing.end_at IS NULL AS duration_unknown
    FROM incidents i JOIN incident_providers link ON link.incident_id = i.id
    JOIN incident_timing timing ON timing.id = i.id
    WHERE link.provider_id = $1 AND i.within_provider_scope
      AND ($4::uuid IS NULL OR EXISTS (
          SELECT 1 FROM incident_components ic WHERE ic.incident_id = i.id AND ic.component_id = $4
      ))
), events AS MATERIALIZED (
    SELECT * FROM intervals
    WHERE (start_at < $3 AND end_at > $2 AND end_at > start_at)
       OR (start_at = end_at AND start_at >= $2 AND start_at < $3)
       OR (count_at >= $2 AND count_at < $3 AND duration_unknown)
), poll_days AS (
    SELECT DISTINCT (finished_at AT TIME ZONE $6)::date AS day
    FROM poll_runs
    WHERE provider_source_id = $5 AND poll_kind = 'status'
      AND finished_at >= $2 AND finished_at < $3
      AND started_at >= $2 - interval '1 day'
      AND outcome IN ('success', 'not_modified')
), starts AS (
    SELECT (count_at AT TIME ZONE $6)::date AS day,
           count(*) FILTER (WHERE severity IN ('major', 'critical')) AS major,
           count(*) FILTER (WHERE severity NOT IN ('major', 'critical')) AS minor
    FROM intervals
    WHERE kind = 'incident' AND count_at >= $2 AND count_at < $3
    GROUP BY 1
), daily AS (
    SELECT bucket.day, bucket.start_at, bucket.end_at,
           count(e.id) FILTER (WHERE e.kind = 'incident') AS incident_count,
           count(e.id) FILTER (WHERE e.kind = 'maintenance') AS maintenance_count,
           count(e.id) FILTER (WHERE e.duration_unknown) AS unknown_duration_count,
           CASE
               WHEN bool_or(e.kind = 'incident' AND e.severity IN ('major', 'critical')) THEN 'major'
               WHEN bool_or(e.kind = 'incident') THEN 'minor'
               WHEN bool_or(e.kind = 'maintenance') THEN 'maintenance'
               WHEN EXISTS (SELECT 1 FROM poll_days p WHERE p.day = bucket.day) THEN 'healthy'
               ELSE 'unknown'
           END AS status,
           range_agg(tstzrange(GREATEST(e.start_at, bucket.start_at), LEAST(e.end_at, bucket.end_at), '[)'))
               FILTER (WHERE e.kind = 'incident' AND NOT e.duration_unknown) AS spans
    FROM (
        SELECT local_day::date AS day,
               local_day AT TIME ZONE $6 AS start_at,
               LEAST((local_day + interval '1 day') AT TIME ZONE $6, $3) AS end_at
        FROM generate_series(
            ($2 AT TIME ZONE $6)::date::timestamp,
            ($3 AT TIME ZONE $6)::date::timestamp,
            interval '1 day'
        ) AS series(local_day)
    ) AS bucket
    LEFT JOIN events e ON (e.start_at < bucket.end_at AND e.end_at > bucket.start_at)
        OR (e.start_at = e.end_at AND e.start_at >= bucket.start_at AND e.start_at < bucket.end_at)
        OR (e.duration_unknown AND e.count_at >= bucket.start_at AND e.count_at < bucket.end_at)
    GROUP BY bucket.day, bucket.start_at, bucket.end_at
)
SELECT jsonb_build_object(
    'from', $2::timestamptz, 'to', $3::timestamptz,
    'history_available_from', (SELECT history_available_from FROM provider_sources WHERE id = $5),
    'history_refreshed_at', (SELECT history_refreshed_at FROM provider_sources WHERE id = $5),
    'coverage_complete', false,
    'days', (SELECT jsonb_agg(jsonb_build_object(
        'date', day, 'from', start_at, 'to', end_at, 'status', status,
        'started_major_count', COALESCE((SELECT major FROM starts s WHERE s.day = daily.day), 0),
        'started_minor_count', COALESCE((SELECT minor FROM starts s WHERE s.day = daily.day), 0),
        'unknown_duration_count', unknown_duration_count, 'incident_count', incident_count, 'maintenance_count', maintenance_count,
        'affected_seconds', (SELECT COALESCE(sum(extract(epoch FROM upper(span) - lower(span))), 0) FROM unnest(spans) span)
    ) ORDER BY day) FROM daily)
)
