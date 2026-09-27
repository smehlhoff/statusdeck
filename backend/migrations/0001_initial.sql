CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email text NOT NULL UNIQUE,
    password_hash text NOT NULL,
    role text NOT NULL DEFAULT 'admin' CHECK (role = 'admin'),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    display_name text NOT NULL DEFAULT 'Administrator'
        CHECK (char_length(display_name) BETWEEN 1 AND 100),
    preferences jsonb NOT NULL DEFAULT '{"theme":"light","time_zone":"browser","date_format":"locale","time_format":"locale","timestamp_format":"relative","landing_page":"/","refresh_interval_ms":0}'::jsonb,
    credential_version bigint NOT NULL DEFAULT 0,
    oidc_generation bigint NOT NULL DEFAULT 0
);

CREATE TABLE provider_sources (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_key text NOT NULL UNIQUE,
    adapter text NOT NULL,
    base_url text NOT NULL,
    public_config jsonb NOT NULL DEFAULT '{}'::jsonb,
    enabled boolean NOT NULL DEFAULT true,
    poll_interval_seconds integer NOT NULL CHECK (poll_interval_seconds > 0),
    next_poll_at timestamptz NOT NULL,
    history_next_poll_at timestamptz NOT NULL DEFAULT now(),
    lease_owner text,
    lease_until timestamptz,
    etag text,
    last_modified text,
    last_attempt_at timestamptz,
    last_success_at timestamptz,
    consecutive_failures integer NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0),
    stale_episode_started_at timestamptz,
    stale_episode_event_id uuid,
    history_refreshed_at timestamptz,
    history_available_from timestamptz,
    last_error text,
    history_last_attempt_at timestamptz,
    history_consecutive_failures integer NOT NULL DEFAULT 0
        CHECK (history_consecutive_failures >= 0),
    history_last_error text
);

CREATE TABLE poll_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_source_id uuid NOT NULL REFERENCES provider_sources(id) ON DELETE RESTRICT,
    started_at timestamptz NOT NULL,
    finished_at timestamptz,
    outcome text NOT NULL,
    http_status integer,
    duration_ms integer,
    component_count integer NOT NULL DEFAULT 0,
    incident_count integer NOT NULL DEFAULT 0,
    error_message text,
    adapter_version text NOT NULL DEFAULT 'unknown',
    error_class text,
    poll_kind text NOT NULL DEFAULT 'status'
        CHECK (poll_kind IN ('status', 'history')),
    CONSTRAINT poll_runs_failure_class_check CHECK (
        outcome <> 'failure'
        OR error_class IN (
            'transport',
            'rate_limit',
            'http',
            'parse',
            'response_too_large',
            'configuration',
            'database',
            'reconciliation',
            'abandoned',
            'internal'
        )
    )
);

CREATE TABLE poll_payloads (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    poll_run_id uuid NOT NULL UNIQUE REFERENCES poll_runs(id) ON DELETE CASCADE,
    provider_source_id uuid NOT NULL REFERENCES provider_sources(id) ON DELETE RESTRICT,
    payload jsonb NOT NULL,
    captured_at timestamptz NOT NULL DEFAULT now(),
    content_type text,
    truncated boolean NOT NULL DEFAULT false
);

CREATE TABLE providers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_source_id uuid NOT NULL REFERENCES provider_sources(id) ON DELETE RESTRICT,
    slug text NOT NULL,
    name text NOT NULL,
    upstream_provider_key text NOT NULL,
    official_url text NOT NULL,
    description text NOT NULL DEFAULT '',
    tags text[] NOT NULL DEFAULT '{}',
    active boolean NOT NULL DEFAULT true,
    UNIQUE (provider_source_id, upstream_provider_key),
    UNIQUE (slug)
);

CREATE TABLE components (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_id uuid NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    upstream_component_id text NOT NULL,
    name text NOT NULL,
    group_name text,
    description text,
    position integer NOT NULL DEFAULT 0,
    active boolean NOT NULL DEFAULT true,
    UNIQUE (provider_id, upstream_component_id),
    UNIQUE (id, provider_id)
);

CREATE TABLE provider_status_current (
    provider_id uuid PRIMARY KEY REFERENCES providers(id) ON DELETE RESTRICT,
    normalized_status text NOT NULL,
    original_status text NOT NULL,
    provider_normalized_status text NOT NULL DEFAULT 'unknown',
    provider_original_status text NOT NULL DEFAULT 'unknown',
    observed_at timestamptz NOT NULL,
    semantic_hash text NOT NULL,
    poll_run_id uuid REFERENCES poll_runs(id) ON DELETE RESTRICT
);

CREATE TABLE component_status_current (
    component_id uuid PRIMARY KEY REFERENCES components(id) ON DELETE RESTRICT,
    provider_id uuid NOT NULL,
    normalized_status text NOT NULL,
    original_status text NOT NULL,
    observed_at timestamptz NOT NULL,
    semantic_hash text NOT NULL,
    poll_run_id uuid REFERENCES poll_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY (component_id, provider_id) REFERENCES components(id, provider_id)
);

CREATE TABLE status_changes (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_id uuid REFERENCES providers(id) ON DELETE RESTRICT,
    component_id uuid REFERENCES components(id) ON DELETE RESTRICT,
    old_status text,
    new_status text NOT NULL,
    original_status text NOT NULL,
    observed_at timestamptz NOT NULL,
    semantic_hash text NOT NULL,
    poll_run_id uuid REFERENCES poll_runs(id) ON DELETE RESTRICT,
    CHECK ((provider_id IS NOT NULL) <> (component_id IS NOT NULL))
);

CREATE TABLE incidents (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_source_id uuid NOT NULL REFERENCES provider_sources(id) ON DELETE RESTRICT,
    upstream_incident_id text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('incident', 'maintenance')),
    title text NOT NULL,
    official_url text,
    lifecycle text NOT NULL CHECK (lifecycle IN ('open', 'resolution_pending', 'resolved')),
    original_phase text NOT NULL,
    severity text NOT NULL,
    original_impact text,
    provider_created_at timestamptz,
    provider_started_at timestamptz,
    provider_updated_at timestamptz,
    provider_monitoring_at timestamptz,
    provider_resolved_at timestamptz,
    provider_metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    planned_start_at timestamptz,
    planned_end_at timestamptz,
    within_provider_scope boolean NOT NULL DEFAULT true,
    first_observed_at timestamptz NOT NULL,
    last_observed_at timestamptz NOT NULL,
    lifecycle_generation integer NOT NULL DEFAULT 1 CHECK (lifecycle_generation > 0),
    current_fingerprint text NOT NULL,
    absence_count integer NOT NULL DEFAULT 0 CHECK (absence_count >= 0),
    UNIQUE (provider_source_id, upstream_incident_id)
);

CREATE TABLE incident_updates (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE RESTRICT,
    upstream_update_id text,
    body text NOT NULL,
    original_status text NOT NULL,
    provider_created_at timestamptz,
    provider_updated_at timestamptz,
    provider_display_at timestamptz,
    semantic_hash text NOT NULL,
    synthesized boolean NOT NULL DEFAULT false,
    UNIQUE (incident_id, upstream_update_id)
);

CREATE TABLE incident_comments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE RESTRICT,
    author_user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    body text NOT NULL CHECK (char_length(body) BETWEEN 1 AND 5000),
    created_at timestamptz NOT NULL DEFAULT now(),
    edited_at timestamptz
);

CREATE INDEX incident_comments_feed_idx
    ON incident_comments (incident_id, created_at DESC, id DESC);

CREATE TABLE incident_providers (
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE RESTRICT,
    provider_id uuid NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    PRIMARY KEY (incident_id, provider_id)
);

CREATE TABLE incident_components (
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE RESTRICT,
    component_id uuid NOT NULL REFERENCES components(id) ON DELETE RESTRICT,
    PRIMARY KEY (incident_id, component_id)
);

CREATE TABLE incident_update_components (
    incident_update_id uuid NOT NULL REFERENCES incident_updates(id) ON DELETE RESTRICT,
    component_id uuid NOT NULL REFERENCES components(id) ON DELETE RESTRICT,
    PRIMARY KEY (incident_update_id, component_id)
);

CREATE TABLE incident_affected_scopes (
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    scope_type text NOT NULL,
    upstream_id text NOT NULL,
    display_name text NOT NULL,
    normalized_status text,
    original_status text,
    PRIMARY KEY (incident_id, scope_type, upstream_id)
);

CREATE TABLE incident_update_affected_scopes (
    incident_update_id uuid NOT NULL REFERENCES incident_updates(id) ON DELETE CASCADE,
    scope_type text NOT NULL,
    upstream_id text NOT NULL,
    display_name text NOT NULL,
    normalized_status text,
    original_status text,
    PRIMARY KEY (incident_update_id, scope_type, upstream_id)
);

CREATE TABLE monitored_providers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_id uuid NOT NULL UNIQUE REFERENCES providers(id) ON DELETE RESTRICT,
    monitor_all_components boolean NOT NULL DEFAULT true,
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE monitored_components (
    monitored_provider_id uuid NOT NULL REFERENCES monitored_providers(id) ON DELETE CASCADE,
    provider_id uuid NOT NULL,
    component_id uuid NOT NULL,
    PRIMARY KEY (monitored_provider_id, component_id),
    FOREIGN KEY (component_id, provider_id) REFERENCES components(id, provider_id)
);

CREATE TABLE oidc_configuration (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    identity_key text,
    flow_key text,
    configuration text,
    revision bigint NOT NULL DEFAULT 0
);
INSERT INTO oidc_configuration DEFAULT VALUES;

CREATE TABLE oidc_identities (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    issuer text NOT NULL,
    subject text NOT NULL,
    configuration_key text NOT NULL,
    needs_relink boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (issuer, subject)
);

CREATE TABLE sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE,
    created_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    user_agent text,
    ip_address inet,
    authentication_method text NOT NULL DEFAULT 'local' CHECK (authentication_method IN ('local', 'oidc')),
    oidc_identity_id uuid REFERENCES oidc_identities(id),
    oidc_sid text,
    CONSTRAINT sessions_oidc_identity CHECK ((authentication_method = 'oidc') = (oidc_identity_id IS NOT NULL))
);

CREATE INDEX sessions_user_created_idx ON sessions (user_id, created_at DESC, id DESC);

CREATE TABLE oidc_login_attempts (
    state_hash bytea PRIMARY KEY,
    browser_hash bytea NOT NULL UNIQUE,
    nonce text NOT NULL,
    verifier text,
    purpose text NOT NULL CHECK (purpose IN ('login', 'link')),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id uuid REFERENCES sessions(id) ON DELETE CASCADE,
    credential_version bigint NOT NULL,
    generation bigint NOT NULL,
    flow_key text NOT NULL,
    identity_id uuid REFERENCES oidc_identities(id) ON DELETE CASCADE,
    candidate_subject text,
    candidate_sid text,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '10 minutes',
    CHECK ((purpose = 'link') = (session_id IS NOT NULL))
);
CREATE INDEX oidc_attempt_expiry_idx ON oidc_login_attempts (expires_at);

-- Also act as short-lived revocation markers for callbacks already exchanging codes.
CREATE TABLE oidc_logout_events (
    issuer text NOT NULL,
    jti text NOT NULL,
    sid text,
    subject text,
    received_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '11 minutes',
    PRIMARY KEY (issuer, jti),
    CHECK (sid IS NOT NULL OR subject IS NOT NULL)
);
CREATE INDEX oidc_logout_expiry_idx ON oidc_logout_events (expires_at);

-- History contains display metadata only, never authentication tokens.
CREATE TABLE session_history (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    ended_at timestamptz NOT NULL,
    user_agent text,
    ip_address inet,
    status text NOT NULL CHECK (status IN ('expired', 'ended')),
    authentication_method text NOT NULL DEFAULT 'local',
    oidc_identity_id uuid,
    oidc_sid text
);
CREATE INDEX session_history_user_seen_idx ON session_history (user_id, last_seen_at DESC, id DESC);
CREATE INDEX session_history_ended_idx ON session_history (ended_at);

-- All existing logout, revocation, capacity and expiry paths delete sessions.
CREATE FUNCTION archive_session() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO session_history
        (id, user_id, created_at, last_seen_at, expires_at, ended_at, user_agent, ip_address, status,
         authentication_method, oidc_identity_id, oidc_sid)
    SELECT OLD.id, OLD.user_id, OLD.created_at, OLD.last_seen_at, OLD.expires_at,
        LEAST(now(), OLD.expires_at, OLD.last_seen_at + interval '7 days'),
        OLD.user_agent, OLD.ip_address,
        CASE WHEN OLD.expires_at <= now() OR OLD.last_seen_at <= now() - interval '7 days'
            THEN 'expired' ELSE 'ended' END,
        OLD.authentication_method, OLD.oidc_identity_id, OLD.oidc_sid
    FROM users WHERE id = OLD.user_id;
    DELETE FROM session_history WHERE id IN (
        SELECT id FROM session_history WHERE user_id = OLD.user_id
        ORDER BY last_seen_at DESC, id DESC OFFSET 100
    );
    RETURN OLD;
END;
$$;

CREATE TRIGGER sessions_archive AFTER DELETE ON sessions
    FOR EACH ROW EXECUTE FUNCTION archive_session();

CREATE TABLE notification_channels (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    channel_type text NOT NULL CHECK (channel_type IN ('webhook', 'discord', 'slack', 'mattermost', 'gotify', 'ntfy', 'zulip')),
    encrypted_config text NOT NULL,
    display_target text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    last_tested_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);

CREATE TABLE alert_rules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    rule_kind text NOT NULL DEFAULT 'provider' CHECK (rule_kind IN ('provider', 'system_health')),
    min_severity text NOT NULL DEFAULT 'minor',
    notify_detected boolean NOT NULL DEFAULT true,
    notify_started boolean NOT NULL DEFAULT true,
    notify_updated boolean NOT NULL DEFAULT true,
    notify_resolved boolean NOT NULL DEFAULT true,
    notify_reopened boolean NOT NULL DEFAULT true,
    notify_maintenance boolean NOT NULL DEFAULT false,
    notify_recovered boolean NOT NULL DEFAULT true,
    quiet_hours jsonb NOT NULL DEFAULT '{"enabled":false,"timezone":"UTC","days":[1,2,3,4,5],"start":"22:00","end":"07:00","critical_override":false}',
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);

CREATE TABLE alert_rule_channels (
    alert_rule_id uuid NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    channel_id uuid NOT NULL REFERENCES notification_channels(id) ON DELETE RESTRICT,
    PRIMARY KEY (alert_rule_id, channel_id)
);

CREATE TABLE alert_rule_providers (
    alert_rule_id uuid NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    provider_id uuid NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    PRIMARY KEY (alert_rule_id, provider_id)
);

CREATE TABLE alert_rule_components (
    alert_rule_id uuid NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    provider_id uuid NOT NULL,
    component_id uuid NOT NULL,
    PRIMARY KEY (alert_rule_id, component_id),
    FOREIGN KEY (component_id, provider_id) REFERENCES components(id, provider_id)
);

CREATE TABLE notification_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type text NOT NULL,
    entity_id uuid NOT NULL,
    lifecycle_generation integer,
    semantic_hash text NOT NULL,
    payload jsonb NOT NULL,
    covered_by_event_id uuid REFERENCES notification_events(id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (event_type, entity_id, lifecycle_generation, semantic_hash),
    CHECK (covered_by_event_id IS NULL OR covered_by_event_id <> id)
);

CREATE TABLE notification_deliveries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    notification_event_id uuid NOT NULL REFERENCES notification_events(id) ON DELETE RESTRICT,
    alert_rule_id uuid REFERENCES alert_rules(id) ON DELETE RESTRICT,
    channel_id uuid NOT NULL REFERENCES notification_channels(id) ON DELETE RESTRICT,
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'retrying', 'delivered', 'ambiguous', 'failed', 'held', 'summarized', 'cancelled')),
    attempt_count integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    lease_owner text,
    lease_until timestamptz,
    last_error text,
    delivered_at timestamptz,
    queued_at timestamptz NOT NULL DEFAULT now(),
    quiet_until timestamptz,
    summary_delivery_id uuid REFERENCES notification_deliveries(id),
    retry_attempt_count integer NOT NULL DEFAULT 0,
    resend_requested_at timestamptz,
    UNIQUE (notification_event_id, alert_rule_id, channel_id)
);

CREATE TABLE notification_attempts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_id uuid NOT NULL REFERENCES notification_deliveries(id) ON DELETE RESTRICT,
    attempted_at timestamptz NOT NULL DEFAULT now(),
    duration_ms integer,
    outcome text NOT NULL,
    response_class text,
    response_status integer,
    error_message text,
    ambiguous boolean NOT NULL DEFAULT false
);

CREATE TABLE worker_heartbeats (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    role text NOT NULL UNIQUE CHECK (role IN ('poller', 'dispatcher')),
    instance_id text NOT NULL,
    version text NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    started_at timestamptz,
    last_completed_at timestamptz
);

CREATE TABLE audit_log (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    action text NOT NULL,
    entity_type text NOT NULL,
    entity_id uuid,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX provider_sources_due_idx ON provider_sources (enabled, next_poll_at);
CREATE INDEX provider_sources_history_due_idx
    ON provider_sources (enabled, history_next_poll_at);
CREATE INDEX provider_sources_lease_idx ON provider_sources (lease_until);
CREATE INDEX providers_source_idx ON providers (provider_source_id, active);
CREATE INDEX component_provider_idx ON components (provider_id, active, position);
CREATE INDEX status_changes_provider_idx ON status_changes (provider_id, observed_at DESC);
CREATE INDEX status_changes_component_idx ON status_changes (component_id, observed_at DESC);
CREATE INDEX incidents_active_idx ON incidents (lifecycle, last_observed_at DESC);
CREATE INDEX incidents_feed_idx ON incidents (last_observed_at DESC, id DESC);
CREATE INDEX incidents_scope_schedule_idx
    ON incidents (within_provider_scope, kind, planned_start_at, planned_end_at);
CREATE INDEX incident_updates_feed_idx ON incident_updates (incident_id, provider_created_at DESC NULLS LAST);
CREATE UNIQUE INDEX incident_updates_fallback_identity_idx
    ON incident_updates (
        incident_id,
        semantic_hash,
        COALESCE(provider_created_at, '-infinity'::timestamptz)
    )
    WHERE upstream_update_id IS NULL;
CREATE INDEX poll_runs_source_idx ON poll_runs (provider_source_id, started_at DESC);
CREATE INDEX poll_runs_source_kind_idx
    ON poll_runs (provider_source_id, poll_kind, started_at DESC, id DESC);
CREATE INDEX poll_payloads_retention_idx ON poll_payloads (captured_at);
CREATE INDEX deliveries_due_idx ON notification_deliveries (status, next_attempt_at);
CREATE INDEX deliveries_quiet_idx ON notification_deliveries (quiet_until, alert_rule_id, channel_id) WHERE status = 'held';
CREATE INDEX attempts_delivery_idx ON notification_attempts (delivery_id, attempted_at DESC);
CREATE INDEX audit_recent_idx ON audit_log (created_at DESC);
CREATE INDEX audit_log_user_security_idx ON audit_log (actor_user_id, created_at DESC, id DESC)
WHERE action IN (
    'session.login_succeeded', 'session.login_failed', 'session.logout',
    'profile.password_changed', 'profile.email_changed',
    'profile.session_revoked', 'profile.other_sessions_revoked',
    'oidc.login_succeeded', 'oidc.login_denied', 'oidc.linked',
    'oidc.disconnected', 'oidc.provider_revoked', 'oidc.settings_updated'
);
CREATE INDEX alert_rule_components_component_idx ON alert_rule_components (component_id);
CREATE INDEX notification_deliveries_event_idx
    ON notification_deliveries (notification_event_id, id)
    WHERE alert_rule_id IS NULL;
CREATE INDEX notification_channels_active_idx
    ON notification_channels (name, id)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX notification_channels_active_name_unique_idx
    ON notification_channels (lower(name))
    WHERE deleted_at IS NULL;
CREATE INDEX alert_rules_active_idx
    ON alert_rules (name, id)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX alert_rules_active_name_unique_idx
    ON alert_rules (lower(name))
    WHERE deleted_at IS NULL;
CREATE INDEX poll_runs_history_idx
    ON poll_runs (started_at DESC, id DESC);
CREATE INDEX poll_runs_source_history_idx
    ON poll_runs (provider_source_id, started_at DESC, id DESC);
CREATE INDEX poll_runs_outcome_history_idx
    ON poll_runs (outcome, started_at DESC, id DESC);
CREATE INDEX notification_events_history_idx
    ON notification_events (created_at DESC, id DESC);
CREATE INDEX poll_runs_running_source_idx
    ON poll_runs (provider_source_id, started_at)
    WHERE outcome = 'running';

-- Days are ISO weekdays of the start date. PostgreSQL resolves nonexistent local
-- times using the pre-transition offset and ambiguous times using standard time.
CREATE FUNCTION quiet_hours_end(schedule jsonb, instant timestamptz)
RETURNS timestamptz LANGUAGE plpgsql STABLE STRICT AS $$
DECLARE
    local_day date;
    start_time time;
    end_time time;
    begins timestamptz;
    ends timestamptz;
    candidate date;
    offset_days integer;
BEGIN
    IF NOT (schedule->>'enabled')::boolean THEN RETURN NULL; END IF;
    local_day := (instant AT TIME ZONE (schedule->>'timezone'))::date;
    start_time := (schedule->>'start')::time;
    end_time := (schedule->>'end')::time;
    FOR offset_days IN 0..1 LOOP
        candidate := local_day - offset_days;
        IF NOT (schedule->'days' @> to_jsonb(extract(isodow FROM candidate)::integer)) THEN CONTINUE; END IF;
        begins := (candidate + start_time) AT TIME ZONE (schedule->>'timezone');
        ends := (candidate + CASE WHEN end_time <= start_time THEN 1 ELSE 0 END + end_time) AT TIME ZONE (schedule->>'timezone');
        IF instant >= begins AND instant < ends THEN RETURN ends; END IF;
    END LOOP;
    RETURN NULL;
END;
$$;

-- Publication and observation locate records but never establish outage duration.
CREATE VIEW incident_timing AS
WITH bounds AS (
    SELECT i.id,
           COALESCE(NULLIF(i.provider_started_at, 'epoch'::timestamptz),
               CASE WHEN i.kind = 'maintenance' THEN NULLIF(i.planned_start_at, 'epoch'::timestamptz) END,
               NULLIF(i.provider_created_at, 'epoch'::timestamptz), i.first_observed_at) AS count_at,
           -- Publication is the reported start when an incident has no explicit impact start.
           COALESCE(
               CASE WHEN i.provider_started_at > 'epoch'::timestamptz THEN i.provider_started_at END,
               CASE WHEN i.kind = 'incident' AND i.provider_created_at > 'epoch'::timestamptz
                   THEN i.provider_created_at END
           ) AS start_at,
           CASE
               WHEN i.lifecycle = 'resolved' THEN
                   CASE WHEN i.provider_resolved_at > 'epoch'::timestamptz THEN i.provider_resolved_at END
               WHEN i.kind = 'incident' THEN now()
               -- A scheduled window alone is not evidence of actual maintenance.
               WHEN lower(i.original_phase) IN ('in_progress', 'in progress', 'started', 'ongoing', 'verifying')
                   AND (i.planned_end_at IS NULL OR i.planned_end_at > now()) THEN now()
               WHEN lower(i.original_phase) IN ('in_progress', 'in progress', 'started', 'ongoing', 'verifying')
                   AND i.provider_updated_at > i.planned_end_at THEN LEAST(i.provider_updated_at, now())
           END AS candidate_end,
           i.kind = 'maintenance' AND i.lifecycle <> 'resolved'
               AND i.planned_end_at <= now() AS maintenance_uncertain
    FROM incidents i
)
SELECT id, count_at, start_at,
       CASE WHEN candidate_end >= start_at THEN candidate_end END AS end_at,
       COALESCE(maintenance_uncertain, false) AS maintenance_uncertain
FROM bounds;

CREATE INDEX poll_runs_finished_idx ON poll_runs (finished_at DESC)
    WHERE finished_at IS NOT NULL;
CREATE INDEX deliveries_delivered_idx ON notification_deliveries (delivered_at DESC)
    WHERE delivered_at IS NOT NULL;

CREATE TABLE incident_bookmarks (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    incident_id uuid NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, incident_id)
);

CREATE INDEX incident_bookmarks_recent_idx
    ON incident_bookmarks (user_id, created_at DESC, incident_id DESC);
CREATE INDEX incident_bookmarks_incident_idx ON incident_bookmarks (incident_id);

CREATE INDEX incident_comments_author_feed_idx
    ON incident_comments (author_user_id, created_at DESC, id DESC);
