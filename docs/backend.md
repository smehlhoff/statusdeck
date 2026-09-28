# Backend

One Rust executable supplies the API, background worker and database commands.
[Architecture](architecture.md) covers system boundaries; [deployment](deployment.md)
owns environment settings, commands and operations.

## Runtime and module boundaries

[main.rs](../backend/src/main.rs) selects `api` (the default), `worker`, `migrate`,
`healthcheck-worker` or `validate-catalog`. [lib.rs](../backend/src/lib.rs) assembles
process roles. API and worker load validated configuration, verify migrations,
bootstrap the administrator and reconcile the catalog before serving work.
Migrations are explicit; startup does not apply them.

| Subsystem | Responsibility and interface |
| --- | --- |
| [config.rs](../backend/src/config.rs), [db/](../backend/src/db/) | Environment → validated configuration, PostgreSQL pools and migration checks |
| [api/](../backend/src/api/) | HTTP requests → queries/mutations and responses; feature handlers use SQLx directly |
| [auth/](../backend/src/auth/) | Bootstrap, credential verification, sessions and optional OIDC identity verification |
| [domain.rs](../backend/src/domain.rs) | Shared health/lifecycle types, provider snapshots and semantic fingerprints |
| [providers/](../backend/src/providers/) | Reviewed catalog configuration → normalized snapshots or typed fetch failures |
| [polling/](../backend/src/polling/) | Scheduled sources → persisted observations, incidents and matching alert work; freshness and cleanup |
| [notifications/](../backend/src/notifications/) | Durable deliveries + encrypted channel settings → outbound requests and recorded attempts |
| [logging.rs](../backend/src/logging.rs), [retry_after.rs](../backend/src/retry_after.rs) | Tracing configuration and shared retry-header interpretation |

## Authentication and request boundaries

The schema and bootstrap enforce one administrator. Monitoring and notification
configuration are installation-wide; profile, sessions, bookmarks and personal
comment listings are account-scoped. This is not a multi-tenant permission model;
comment editing relies on the single-administrator assumption.

Passwords use Argon2id; random session tokens are stored as keyed hashes. Browser
requests use a session cookie, and mutations require the matching CSRF
cookie/header, including login. Sessions have idle and absolute expiry. Credential
changes require the current password and revoke other sessions; bootstrap does
not overwrite credentials saved through the application.

OIDC links an exact issuer/subject to the existing account, without provisioning.
Configuration/linking require a local session and password confirmation. The
browser redirects to the provider; the API exchanges the returned code and
verifies the identity before issuing an application session. The callback
validates state, nonce and PKCE; signed back-channel logout revokes
matching OIDC sessions. These public protocol routes have their own validation
instead of ordinary authenticated JSON-mutation handling. See
[OIDC setup](deployment.md#optional-oidc-authentication).

[request.rs](../backend/src/api/request.rs) and [shared.rs](../backend/src/api/shared.rs)
centralize request limits, host/origin checks, authentication and CSRF helpers.
Login is rate-limited. Proxy trust must be confined to the intended ingress.
`ApiError` returns `{type, title, status, code, detail, correlation_id}`; internal
causes are logged while clients receive generic errors. Proxy/framework errors
may have a different format. There is no signup, recovery or service-token API.

## HTTP interfaces

The [router](../backend/src/api/mod.rs) is the endpoint index. Paths below are
under `/api/v1`; linked handlers own exact methods, fields and validation, and
[frontend types](../frontend/src/api/types.ts) mirror response shapes manually.
Business routes require a session; health/session bootstrap and OIDC protocol
routes are the deliberate exceptions.

| API family | Inputs → outputs and implementation |
| --- | --- |
| `/csrf`, `/session`, `/auth/*` | Credentials/cookies or OIDC protocol data → session/bootstrap state; [session](../backend/src/api/session.rs), [OIDC](../backend/src/api/oidc.rs) |
| `/profile/*` | Preferences, password-confirmed changes, session IDs and SSO settings → account/security state; [profile](../backend/src/api/profile.rs), [OIDC](../backend/src/api/oidc.rs) |
| `/catalog/providers`, `/catalog/components`, `/monitors`, `/dashboard` | Catalog IDs and component selections → available/monitored health; [catalog](../backend/src/api/catalog.rs), [monitors](../backend/src/api/monitors.rs), [dashboard](../backend/src/api/dashboard.rs) |
| `/incidents/*`, `/bookmarks`, `/my-comments` | Scope/search/cursors and account notes → history/detail or saved records; [incidents](../backend/src/api/incidents.rs), [comments](../backend/src/api/comments.rs), [bookmarks](../backend/src/api/bookmarks.rs), [personal comments](../backend/src/api/my_comments.rs) |
| `/analytics`, `/catalog/providers/{id}/reliability` | Period, timezone and coverage → reported-impact aggregates; [analytics](../backend/src/api/analytics.rs), [reliability](../backend/src/api/reliability.rs) |
| `/notification-channels`, `/alert-rules`, `/quiet-hours/preview`, `/notifications/summaries/{id}` | Destinations, scope and quiet-hour rules → redacted configuration, schedule preview or saved summary; [channels](../backend/src/api/channels.rs), [rules](../backend/src/api/rules.rs), [summaries](../backend/src/api/notification_summaries.rs) |
| `/system/*` | Diagnostic filters or delivery ID → source/queue/database evidence or queued resend; [diagnostics](../backend/src/api/diagnostics.rs), [summary](../backend/src/api/system_summary.rs), [resend](../backend/src/api/delivery_resend.rs) |

JSON contracts use UUIDs, RFC3339 timestamps and null for unknown timing. Paged
history uses bounded opaque cursors; retain the same filters when advancing.
Mutations validate ownership/scope at the server. Queued operations return
acceptance, not proof that background work succeeded. Exact defaults and limits
belong to handler definitions rather than a second field-by-field reference.

## Provider catalog and adapter contracts

The compiled [catalog](../backend/src/providers/catalog/) defines approved origins,
adapters and collection scope; [PROVIDERS.md](../PROVIDERS.md) describes supported
coverage. A `StatusProvider` accepts a fetch context and returns a normalized
snapshot, unchanged metadata or an error. Original source classifications remain
available alongside normalized status. Upstream requests have bounded deadlines,
payload limits and no redirects.

Catalog scope and monitoring selection are independent. Listed catalog scope
requires reviewed IDs; discoverable scope accepts adapter inventory within its
configured filters. Whole-provider monitoring includes new collected components;
explicit selections do not. Missing components become inactive only after a
successful scoped snapshot; retained IDs/history allow later reactivation.
Explicitly selected inactive components report unknown health.

All enabled sources are collected before subscription. Source-specific parsing,
supplemental feeds and history completeness live in the adapters. Adding a source
requires catalog registration and origin validation; a new adapter also needs a
registry entry. Update provider coverage documentation and run `validate-catalog`.

## Polling and reconciliation

The worker leases due source jobs with bounded concurrency. Status and history
refreshes have separate schedules/retry state but share a source lease. HTTP
validators avoid unchanged transfers; retryable failures use backoff. Freshness
measures collection health independently of reported service health.

Reconciliation transactionally updates components/current status, incident
history and matching event/delivery rows. Semantic deduplication suppresses
repeated observations; reopened incidents start a new lifecycle generation.
Complete-feed absence requires confirmation before resolution; partial feeds
require explicit terminal evidence. Missing events do not invent recovery times.

Subscriptions govern dashboard and alert scope, not whether catalog collection
continues. Creating rules or changing monitoring can include existing active
opening events; editing a rule does not replay historical notifications.
Resolution alerts require corresponding opening-delivery evidence. The worker
also runs session/payload cleanup and publishes poller/dispatcher heartbeats.

## Quiet hours and delivery

Rules narrow monitored providers/components by severity and event kind, with
separate rules for collection-health episodes. Channels support generic webhooks,
Discord, Slack, Mattermost, Gotify, ntfy and Zulip. Channel settings are encrypted;
API reads never return stored plaintext secrets.

Quiet hours use local weekdays/timezones, including overnight windows. Eligible
events are held and combined into a saved summary at release; critical override
can bypass the hold. The dispatcher validates HTTPS destinations and DNS addresses
before sending. Private targets require explicit opt-in; redirects and environment
HTTP proxies are not used.

Deliveries are durable and leased. Transient failures retry; permanent failures
stop. Ambiguous sends and interruptions after remote acceptance can duplicate
messages. Disabling rules/channels cancels pending automatic work, but cannot
recall an HTTP request already in flight. Manual resend uses the current channel,
retains attempt history/event ID and bypasses rule filters and quiet hours.

Generic webhook payloads include stable `event_id`, `event_type`, `occurred_at`,
`application_url`, readable content and structured event data. Headers carry
`x-statusdeck-event-id` and `x-statusdeck-timestamp`. Optional
`x-statusdeck-signature` is `sha256=` plus HMAC-SHA256 of
`timestamp + "." + exact JSON body bytes`. Receivers should validate signatures,
reject stale timestamps and deduplicate event IDs. [Notification code](../backend/src/notifications/)
owns payload details and destination-specific formatting/size limits.

## Persistence and analytics

The [initial migration](../backend/migrations/0001_initial.sql) defines identity,
monitoring, observations/incidents, account annotations, notifications and
operational evidence. Constraints protect upstream identity and delivery
uniqueness; SQL supplies shared timing/quiet-hour rules and session archival.
The [ERD](database-erd.md) is the detailed schema reference. PostgreSQL is the
source of truth; process caches and browser caches are disposable.

Raw payloads and session history are cleaned up; incident, poll, audit and
delivery history have no general automatic retention policy. Unsubscribing
preserves history, bookmarks and comments. Migration/backup constraints are in
[deployment](deployment.md#upgrades).

### Analytics and reliability

Shared `incident_timing` rules use provider impact evidence, with publication
fallback for incident starts; import time never establishes impact duration.
Maintenance needs actual impact evidence rather than planned dates. Unknown or
inconsistent bounds remain unknown, and ongoing durations end at query time.

Analytics clips intervals to a rolling period and merges overlaps per provider;
summing across providers can exceed wall-clock time. Restoration statistics use
resolved events with known duration. Reliability groups observations into local
calendar days and keeps maintenance separate from incident impact. A healthy day
is observed evidence, not measured uptime. Exact calculations live in
[analytics.rs](../backend/src/api/analytics.rs) and [reliability.sql](../backend/src/api/reliability.sql).

## Observability and validation

Structured logs correlate requests, sources and deliveries; audit records capture
account/configuration changes. Poll runs, heartbeats and delivery attempts power
System diagnostics. Liveness and readiness are different from provider health or
delivery success; [operations](deployment.md#health-logging-and-troubleshooting)
explains their use. There are no backend/frontend test suites; available build,
lint and catalog checks are listed in [development](deployment.md#local-development-and-checks).
