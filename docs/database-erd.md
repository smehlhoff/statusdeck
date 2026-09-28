# Database entity-relationship diagram

This document reflects the repository's PostgreSQL schema in
[`0001_initial.sql`](../backend/migrations/0001_initial.sql). It describes the
source schema, not the state of any deployed database. It excludes SQLx's internal
`_sqlx_migrations` table. `incident_timing` is a derived view, not a stored table.

The diagrams show every application table and foreign-key relationship. Columns
are limited to keys and fields that explain each entity's role; JSON payload,
lease, audit and timestamp details remain authoritative in the migration.

## Provider collection and monitoring

```mermaid
erDiagram
    provider_sources {
        uuid id PK
        text source_key UK
        text adapter
        text base_url
        jsonb public_config
        boolean enabled
        int poll_interval_seconds
        timestamptz next_poll_at
        timestamptz history_next_poll_at
        text lease_owner
        timestamptz lease_until
        int consecutive_failures
        int history_consecutive_failures
    }

    providers {
        uuid id PK
        uuid provider_source_id FK
        text slug UK
        text upstream_provider_key
        text name
        text official_url
        text_array tags
        boolean active
    }

    components {
        uuid id PK
        uuid provider_id FK
        text upstream_component_id
        text name
        text group_name
        int position
        boolean active
    }

    poll_runs {
        uuid id PK
        uuid provider_source_id FK
        timestamptz started_at
        timestamptz finished_at
        text poll_kind
        text outcome
        int http_status
        int component_count
        int incident_count
        text error_class
    }

    poll_payloads {
        uuid id PK
        uuid poll_run_id FK,UK
        uuid provider_source_id FK
        jsonb payload
        text content_type
        boolean truncated
        timestamptz captured_at
    }

    provider_status_current {
        uuid provider_id PK,FK
        text normalized_status
        text original_status
        timestamptz observed_at
        text semantic_hash
        uuid poll_run_id FK
    }

    component_status_current {
        uuid component_id PK,FK
        uuid provider_id FK
        text normalized_status
        text original_status
        timestamptz observed_at
        text semantic_hash
        uuid poll_run_id FK
    }

    status_changes {
        uuid id PK
        uuid provider_id FK
        uuid component_id FK
        text old_status
        text new_status
        text original_status
        timestamptz observed_at
        uuid poll_run_id FK
    }

    monitored_providers {
        uuid id PK
        uuid provider_id FK,UK
        boolean monitor_all_components
        boolean enabled
    }

    monitored_components {
        uuid monitored_provider_id PK,FK
        uuid component_id PK,FK
        uuid provider_id FK
    }

    provider_sources ||--o{ providers : supplies
    provider_sources ||--o{ poll_runs : schedules
    provider_sources ||--o{ poll_payloads : captures
    providers ||--o{ components : contains
    providers ||--o| provider_status_current : has_current_status
    components ||--o| component_status_current : has_current_status
    poll_runs ||--o| poll_payloads : captures
    poll_runs o|--o{ provider_status_current : observed_in
    poll_runs o|--o{ component_status_current : observed_in
    providers o|--o{ status_changes : changes
    components o|--o{ status_changes : changes
    poll_runs o|--o{ status_changes : observed_in
    providers ||--o| monitored_providers : subscription
    monitored_providers ||--o{ monitored_components : selects
    components ||--o{ monitored_components : selected_component
```

`component_status_current` and `monitored_components` also use composite foreign
keys `(component_id, provider_id) -> components(id, provider_id)` to prevent a
component from being paired with the wrong provider.

## Incidents and account annotations

```mermaid
erDiagram
    provider_sources {
        uuid id PK
        text source_key UK
    }

    providers {
        uuid id PK
        text slug UK
        text name
    }

    components {
        uuid id PK
        uuid provider_id FK
        text name
    }

    incidents {
        uuid id PK
        uuid provider_source_id FK
        text upstream_incident_id
        text kind
        text title
        text lifecycle
        text original_phase
        text severity
        timestamptz provider_created_at
        timestamptz provider_updated_at
        timestamptz provider_resolved_at
        timestamptz planned_start_at
        timestamptz planned_end_at
        boolean within_provider_scope
        int lifecycle_generation
        text current_fingerprint
    }

    incident_providers {
        uuid incident_id PK,FK
        uuid provider_id PK,FK
    }

    incident_components {
        uuid incident_id PK,FK
        uuid component_id PK,FK
    }

    incident_affected_scopes {
        uuid incident_id PK,FK
        text scope_type PK
        text upstream_id PK
        text display_name
        text normalized_status
        text original_status
    }

    incident_updates {
        uuid id PK
        uuid incident_id FK
        text upstream_update_id
        text body
        text original_status
        timestamptz provider_display_at
        text semantic_hash
        boolean synthesized
    }

    incident_update_components {
        uuid incident_update_id PK,FK
        uuid component_id PK,FK
    }

    incident_update_affected_scopes {
        uuid incident_update_id PK,FK
        text scope_type PK
        text upstream_id PK
        text display_name
        text normalized_status
        text original_status
    }

    users {
        uuid id PK
        text email UK
        text display_name
    }

    incident_bookmarks {
        uuid user_id PK,FK
        uuid incident_id PK,FK
        timestamptz created_at
    }

    incident_comments {
        uuid id PK
        uuid incident_id FK
        uuid author_user_id FK
        text body
        timestamptz created_at
        timestamptz edited_at
    }

    incident_timing {
        uuid id
        timestamptz count_at
        timestamptz start_at
        timestamptz end_at
        boolean maintenance_uncertain
    }

    provider_sources ||--o{ incidents : reports
    incidents ||--o{ incident_providers : affects
    providers ||--o{ incident_providers : affected_provider
    incidents ||--o{ incident_components : affects
    components ||--o{ incident_components : affected_component
    incidents ||--o{ incident_affected_scopes : has_scope
    incidents ||--o{ incident_updates : has_update
    incident_updates ||--o{ incident_update_components : affects
    components ||--o{ incident_update_components : update_component
    incident_updates ||--o{ incident_update_affected_scopes : has_scope
    users ||--o{ incident_bookmarks : saves
    incidents ||--o{ incident_bookmarks : bookmarked
    users ||--o{ incident_comments : authors
    incidents ||--o{ incident_comments : has_comment
    incidents ||--o| incident_timing : derives_view_row
```

Incident identity is unique by `(provider_source_id, upstream_incident_id)`.
Update identity is unique by `(incident_id, upstream_update_id)` when an upstream
ID exists.

## Alerts and notification delivery

```mermaid
erDiagram
    providers {
        uuid id PK
        text name
    }

    components {
        uuid id PK
        uuid provider_id FK
        text name
    }

    alert_rules {
        uuid id PK
        text name
        text rule_kind
        text min_severity
        boolean notify_detected
        boolean notify_started
        boolean notify_updated
        boolean notify_resolved
        boolean notify_reopened
        boolean notify_maintenance
        boolean notify_recovered
        jsonb quiet_hours
        boolean enabled
        timestamptz deleted_at
    }

    alert_rule_providers {
        uuid alert_rule_id PK,FK
        uuid provider_id PK,FK
    }

    alert_rule_components {
        uuid alert_rule_id PK,FK
        uuid provider_id FK
        uuid component_id PK,FK
    }

    notification_channels {
        uuid id PK
        text name
        text channel_type
        text encrypted_config
        text display_target
        boolean enabled
        timestamptz last_tested_at
        timestamptz deleted_at
    }

    alert_rule_channels {
        uuid alert_rule_id PK,FK
        uuid channel_id PK,FK
    }

    notification_events {
        uuid id PK
        text event_type
        uuid entity_id
        int lifecycle_generation
        text semantic_hash
        jsonb payload
        uuid covered_by_event_id FK
        timestamptz created_at
    }

    notification_deliveries {
        uuid id PK
        uuid notification_event_id FK
        uuid alert_rule_id FK
        uuid channel_id FK
        text status
        int attempt_count
        int retry_attempt_count
        timestamptz next_attempt_at
        timestamptz quiet_until
        uuid summary_delivery_id FK
        timestamptz delivered_at
        text last_error
    }

    notification_attempts {
        uuid id PK
        uuid delivery_id FK
        timestamptz attempted_at
        int duration_ms
        text outcome
        text response_class
        int response_status
        text error_message
        boolean ambiguous
    }

    alert_rules ||--o{ alert_rule_providers : scopes
    providers ||--o{ alert_rule_providers : selected_provider
    alert_rules ||--o{ alert_rule_components : narrows
    components ||--o{ alert_rule_components : selected_component
    alert_rules ||--o{ alert_rule_channels : routes_to
    notification_channels ||--o{ alert_rule_channels : linked_channel
    notification_events o|--o{ notification_events : covered_by
    notification_events ||--o{ notification_deliveries : fans_out
    alert_rules o|--o{ notification_deliveries : matched_rule
    notification_channels ||--o{ notification_deliveries : destination
    notification_deliveries o|--o{ notification_deliveries : summarized_by
    notification_deliveries ||--o{ notification_attempts : attempts
```

`notification_events.entity_id` is intentionally polymorphic and has no foreign
key. Test deliveries may have no `alert_rule_id`. Event coverage and quiet-hours
summaries create the two self-references shown above.

## Accounts and operations

```mermaid
erDiagram
    users {
        uuid id PK
        text email UK
        text password_hash
        text role
        boolean enabled
        text display_name
        jsonb preferences
        bigint credential_version
        bigint oidc_generation
    }

    oidc_configuration {
        boolean singleton PK
        text identity_key
        text flow_key
        text configuration
        bigint revision
    }

    oidc_identities {
        uuid id PK
        uuid user_id FK,UK
        text issuer
        text subject
        text configuration_key
        boolean needs_relink
    }

    oidc_login_attempts {
        bytea state_hash PK
        bytea browser_hash UK
        uuid user_id FK
        uuid session_id FK
        uuid identity_id FK
        text purpose
        text nonce
        text verifier
        text flow_key
        bigint credential_version
        bigint generation
        text candidate_subject
        timestamptz expires_at
    }

    oidc_logout_events {
        text issuer PK
        text jti PK
        text sid
        text subject
        timestamptz received_at
        timestamptz expires_at
    }

    sessions {
        uuid id PK
        uuid user_id FK
        bytea token_hash UK
        timestamptz created_at
        timestamptz last_seen_at
        timestamptz expires_at
        text user_agent
        inet ip_address
        text authentication_method
        uuid oidc_identity_id FK
        text oidc_sid
    }

    session_history {
        uuid id PK
        uuid user_id FK
        timestamptz created_at
        timestamptz last_seen_at
        timestamptz expires_at
        timestamptz ended_at
        text user_agent
        inet ip_address
        text status
        text authentication_method
        uuid oidc_identity_id
        text oidc_sid
    }

    audit_log {
        uuid id PK
        uuid actor_user_id FK
        text action
        text entity_type
        uuid entity_id
        jsonb metadata
        timestamptz created_at
    }

    worker_heartbeats {
        uuid id PK
        text role UK
        text instance_id
        text version
        timestamptz heartbeat_at
        timestamptz started_at
        timestamptz last_completed_at
    }

    users ||--o{ sessions : owns
    users ||--o| oidc_identities : links
    users ||--o{ oidc_login_attempts : authenticates
    sessions o|--o{ oidc_login_attempts : authorizes_link
    oidc_identities o|--o{ sessions : authenticates
    oidc_identities o|--o{ oidc_login_attempts : signs_in
    users ||--o{ session_history : archived_sessions
    users o|--o{ audit_log : acts
```

`audit_log.entity_type` and `entity_id` form an intentionally polymorphic audit
reference. `worker_heartbeats` is independent and keeps one current row per worker
role.

`oidc_configuration` is a singleton with encrypted settings. Its identity/flow
fingerprints are checked by the application, not foreign keys. The issuer/subject
pair is unique in `oidc_identities`; issuer/token ID is the composite logout-event
key. Logout events are temporary replay/revocation markers. Archived OIDC identity
IDs in `session_history` intentionally have no foreign key.

## Deletion behavior

- Rule, monitor, bookmark, session and scope join rows generally cascade with
  their owning record.
- Provider, component, incident, delivery and attempt history generally uses
  `RESTRICT` to prevent accidental loss of evidence.
- Deleting a user sets `audit_log.actor_user_id` to null but retains the audit
  entry.
- Application-level deletion is often soft deletion via `deleted_at`; referential
  actions describe physical deletion only.
