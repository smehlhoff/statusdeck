WITH clients AS (
    SELECT pid, state, wait_event_type, xact_start
    FROM pg_stat_activity
    WHERE datname = current_database() AND backend_type = 'client backend'
), activity AS (
    SELECT count(*) AS connections,
           bool_and(state IS NOT NULL AND state <> 'disabled') AS visible,
           count(*) FILTER (WHERE state = 'active' AND pid <> pg_backend_pid()) AS active_queries,
           count(*) FILTER (WHERE wait_event_type = 'Lock') AS lock_waits,
           count(*) FILTER (WHERE state IN ('idle in transaction', 'idle in transaction (aborted)')) AS idle_transactions,
           COALESCE(max(EXTRACT(EPOCH FROM (statement_timestamp() - xact_start)))
               FILTER (WHERE pid <> pg_backend_pid()), 0) AS longest_transaction_seconds
    FROM clients
)
SELECT jsonb_build_object(
    'size', pg_size_pretty(pg_database_size(current_database())),
    'connections', connections,
    'active_queries', CASE WHEN visible THEN active_queries END,
    'lock_waits', CASE WHEN visible THEN lock_waits END,
    'idle_transactions', CASE WHEN visible THEN idle_transactions END,
    'longest_transaction_seconds', CASE WHEN visible THEN longest_transaction_seconds END
)
FROM activity
