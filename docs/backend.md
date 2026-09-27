# Backend

The Rust application provides an Axum API and a Tokio worker backed by SQLx and
PostgreSQL. Its commands are `api`, `worker`, `migrate`, `healthcheck-worker` and
`validate-catalog`. See [architecture](architecture.md) for system boundaries
and [deployment](deployment.md) for configuration and runnable commands.

## Runtime and module boundaries

[main.rs](../backend/src/main.rs) defaults to `api` when no command is supplied.
`migrate` and `healthcheck-worker` load only the database URL; `validate-catalog`
needs neither configuration secrets nor a database. API/worker load Config,
initialize tracing, connect a pool, check migration state, ensure the administrator
and reconcile the catalog before serving work.

| Subsystem | Inputs → outputs; collaborators |
| --- | --- |
| [config.rs](../backend/src/config.rs), [lib.rs](../backend/src/lib.rs), [db/](../backend/src/db/) | Environment → validated Config, SQLx pool and selected process role; explicit migration entry point |
| [api/](../backend/src/api/) | HTTP headers/path/query/JSON → JSON or empty status response; handlers use SQLx and shared authentication/validation directly |
| [auth/](../backend/src/auth/) | Credentials/cookies → administrator/session identity; bootstrap and credential operations serialize changes through database locks |
| [domain.rs](../backend/src/domain.rs) | Shared normalized health, severity, lifecycle, snapshots and semantic fingerprints; preserves upstream values separately |
| [providers/](../backend/src/providers/) | Reviewed source configuration + fetch context → snapshot, unchanged metadata or typed ProviderError |
| [polling/](../backend/src/polling/) | Due enabled sources → reconciled observations, events and delivery rows; also heartbeat, freshness and cleanup work |
| [notifications/](../backend/src/notifications/) | Eligible delivery rows + encrypted channel configuration → outbound HTTP and persisted attempt outcome |
| [logging.rs](../backend/src/logging.rs), [retry_after.rs](../backend/src/retry_after.rs) | Tracing setup and shared Retry-After parsing (seconds or HTTP date) |

API and worker each have their own pool, capped at `min(poll_concurrency, 23) + 7`
connections. API shutdown drains requests; worker shutdown allows up to 30 seconds
for its polling/dispatch loops. There is no separate cron process: cleanup and
freshness checks run inside the poller loop.

## Authentication and request boundaries

StatusDeck has one local administrator. Bootstrap creates that account only when
none exists; environment settings do not reset credentials changed through the
application. Credential changes require the current password, preserve the
current session and revoke other sessions.

The browser authenticates with a session cookie. Mutations also require a CSRF
cookie and matching header. Sessions have absolute and idle expiry, passwords
are hashed, and login attempts are rate-limited. Business APIs require a session;
health and session-bootstrap endpoints are available before sign-in.

Passwords use Argon2id. Random session tokens are stored as HMAC-SHA256 hashes;
`statusdeck_session` is HttpOnly, both it and the readable `statusdeck_csrf` cookie
use `Path=/; SameSite=Lax`, and Secure is configuration-controlled. The CSRF
header is `x-csrf-token`, including for login/logout. Sessions expire after
30 days absolutely or 7 days idle; authenticated reads refresh activity.
At most 20 live session rows are kept per user, replacing the oldest at capacity.

The database permits only the `admin` role; bootstrap rejects multiple existing
administrators. Monitoring and alert configuration are installation-wide.
Bookmarks, My Comments and profile/session operations are account-scoped;
comment edit/delete handlers match incident/comment IDs without an author check,
relying on the single-administrator model.
There is no tenant or configurable permission model. Login throttling is local
to the API process (five attempts per five minutes per client/email key), with
two concurrent password verification slots; Nginx adds a separate login limit.
Without proxy trust, the backend groups direct clients under one key per email;
it does not use the peer address for throttling. Successful verification clears
that key. Password verification runs on Tokio's blocking pool.

Host and origin validation, request limits and structured error responses are
shared API boundaries. Errors include a correlation ID for diagnostics. There
is no service-token, signup or account-recovery API.

The API limits bodies to 256 KiB and requests to 30 seconds. Nonempty bodies
must use JSON media types. A supplied Origin on a mutation must match the
configured base origin; host filtering is optional through trusted hosts.
There is no CORS layer. Do not expose a proxy-trusting API to arbitrary clients.

`ApiError` serializes `{type, title, status, code, detail, correlation_id}` as
JSON. `x-request-id` is a valid supplied UUID or a generated UUID and is returned
on responses. Internal errors log their cause but return a generic detail.
Invalid input, missing authentication, forbidden access, missing records,
conflicts and throttling normally use 400/401/403/404/409/429. Proxy-generated
errors and some framework rejections (for example 405) need not use that envelope.

The authoritative implementation is in [auth/](../backend/src/auth/),
[request.rs](../backend/src/api/request.rs) and
[shared.rs](../backend/src/api/shared.rs).

## HTTP interfaces

Business routes are under `/api/v1`. The [router](../backend/src/api/mod.rs) and
linked handlers define exact request fields, validation and response shapes.
[Frontend types](../frontend/src/api/types.ts) mirror those contracts manually.

The paths below omit `/api/v1`. `{id}` denotes a UUID parameter. All business
routes require a session and all mutations require CSRF. Reads return 200 JSON
unless stated otherwise; deletion/revocation generally returns 204. Request
structs reject unknown fields. Configuration PATCH fields are optional; omitted
fields preserve stored values. Credential changes require the fields listed
below. Arrays are replaced when supplied, not appended.

### Account

Implemented by [session.rs](../backend/src/api/session.rs) and
[profile.rs](../backend/src/api/profile.rs).

| Method and path | Input → response |
| --- | --- |
| `GET /csrf` | No session required → 204 and readable CSRF cookie |
| `GET /session` | Optional cookie → profile or JSON `null` |
| `POST /session` | `{email, password}` + CSRF → profile and session cookie |
| `DELETE /session` | CSRF, optional session → 204 and cleared cookie |
| `GET /profile`, `PATCH /profile` | Read or update `display_name`, `preferences` → profile (`id`, `email`, `role`, display name/preferences) |
| `PATCH /profile/email` | `{email, current_password}` → updated profile |
| `PATCH /profile/password` | `{current_password, new_password}` → 204 |
| `GET /profile/sessions` | Current/recent session metadata array, including `current`, `status`, `ended_at`; never tokens |
| `DELETE /profile/sessions`, `DELETE /profile/sessions/{id}` | Revoke others or a selected session; use logout for the current session |
| `GET /profile/security-activity` | Up to 100 recent account security actions |

Names are trimmed and limited to 1–100 characters; new passwords require
12–1,024 characters. Preferences include theme, timezone (`browser` or a database
timezone), date/time/timestamp formats, landing page and refresh interval.
Allowed values are defined by the profile enums and
[frontend preferences](../frontend/src/api/types.ts).

### Catalog, monitoring and analysis

| Method and path | Input → response; source |
| --- | --- |
| `GET /catalog/providers` | Catalog provider array; [catalog.rs](../backend/src/api/catalog.rs) |
| `GET /catalog/components` | Required comma-separated `provider_ids` (1–100 IDs) → active component array with provider and monitored flag; catalog handler |
| `GET /catalog/providers/{id}` | Provider metadata, current/provider-wide status, freshness, monitor, components and active incidents; catalog handler |
| `GET /monitors`, `POST /monitors` | Array, or `{provider_id, monitor_all_components, component_ids}` → upserted monitor (200); [monitors.rs](../backend/src/api/monitors.rs) |
| `PATCH /monitors/{id}`, `DELETE /monitors/{id}` | Update `monitor_all_components`, `component_ids`, `enabled`, or disable monitor; monitor ID differs from provider ID |
| `GET /dashboard` | `{counts, providers}` for enabled monitors; [dashboard.rs](../backend/src/api/dashboard.rs) |
| `GET /analytics` | `period=7d\|30d\|90d\|365d`, `scope=monitored\|all`, `include_maintenance` → period, filters, summary, trend, impacted components, providers and data quality; [analytics.rs](../backend/src/api/analytics.rs) |
| `GET /catalog/providers/{id}/reliability` | `days=90\|180\|365`, optional `component_id` and `time_zone` → local date range, coverage metadata and daily records; [reliability.rs](../backend/src/api/reliability.rs) |

Analytics defaults to 30 days, monitored scope and no maintenance. Reliability
defaults to 365 days including today and requires an active catalog provider,
not an enabled subscription. A monitor's all-components mode requires an empty component
array; selected mode requires at least one component belonging to its provider.

### Incident records and account annotations

| Method and path | Input → response; source |
| --- | --- |
| `GET /incidents` | Filters below, including `scope=monitored\|provider\|all` → cursor page `{items, next_cursor}`; [incidents.rs](../backend/src/api/incidents.rs) |
| `GET /incidents/{id}` | `{incident, providers, components, scopes, updates}`; incident handler |
| `GET /incidents/{id}/comments`, `POST /incidents/{id}/comments` | Cursor page with `total_count`, or `{body}` → created comment (201); [comments.rs](../backend/src/api/comments.rs) |
| `PATCH /incidents/{id}/comments/{comment_id}`, `DELETE /incidents/{id}/comments/{comment_id}` | Replace `{body}` or delete an incident comment |
| `PUT /incidents/{id}/bookmark`, `DELETE /incidents/{id}/bookmark` | Idempotent save/remove (204); [bookmarks.rs](../backend/src/api/bookmarks.rs) |
| `GET /bookmarks`, `GET /my-comments` | `q`, `provider_id`, `cursor` → `{items, next_cursor, providers}` for the account; bookmarks and [my_comments.rs](../backend/src/api/my_comments.rs) |

Incident filters are `q`, `provider_id`, comma-separated `provider_ids`,
`component_id`, `severity`, `lifecycle`, `kind`, `scope`, `activity=active`,
`updated_within=24h|7d|30d`, `affected_status`, `latest_phase`,
`maintenance_window=active|upcoming`, `upcoming_within=7d|30d`, `from`, `to`,
`cursor` and `limit`. `upcoming_within` implies upcoming maintenance unless a
window is supplied. The handler defines enum values and time-bound semantics;
`affected_status` checks current affected-component health. Search is trimmed,
limited to 500 characters and excludes null characters. Comments require
1–5,000 trimmed characters and reject null characters.

### Alerts and diagnostics

| Method and path | Input → response; source |
| --- | --- |
| `GET /notification-channels`, `POST /notification-channels` | Array, or `{name, channel_type, target, signing_secret?, token?, bot_email?, stream?, topic?}` → redacted channel (201); [channels.rs](../backend/src/api/channels.rs) |
| `PATCH /notification-channels/{id}`, `DELETE /notification-channels/{id}` | Update channel fields/enabled, or soft-delete; deletion returns 409 while a non-deleted rule references the channel |
| `POST /notification-channels/{id}/test` | Queue real test → 202 `{delivery_id, event_id, status: "queued"}` |
| `GET /alert-rules`, `POST /alert-rules` | Array, or rule configuration → rule with provider/component/channel IDs (201); [rules.rs](../backend/src/api/rules.rs) |
| `PATCH /alert-rules/{id}`, `DELETE /alert-rules/{id}` | Update rule fields/enabled or soft-delete |
| `POST /quiet-hours/preview` | QuietHours object → `{quiet_until}` timestamp or null; rules handler |
| `GET /notifications/summaries/{id}` | Summary event ID → `{event_id, created_at, payload}`; [notification_summaries.rs](../backend/src/api/notification_summaries.rs) |
| `GET /system/summary` | `window=1h\|24h\|7d` (default 24h) → API, database, worker, polling and delivery metrics; [system_summary.rs](../backend/src/api/system_summary.rs) |
| `GET /system/sources`, `GET /system/data-metrics` | Source evidence array including 24-hour completed status-check and failure counts, or `{record_type, count}` array; [diagnostics.rs](../backend/src/api/diagnostics.rs) |
| `GET /system/poll-runs` | `source_id`, `outcome`, `started_after`, `started_before`, `cursor`, `limit` → cursor page; diagnostics handler |
| `GET /system/deliveries` | `view=all\|overdue\|issues`, `window`, `status`, `channel_id`, `event_id`, `event_type`, `cursor`, `limit` → cursor page with rendered payload/last-attempt evidence; diagnostics handler |
| `GET /system/deliveries/{id}/attempts` | Array of attempts with response class/status, error and ambiguity; diagnostics handler |
| `POST /system/deliveries/{id}/resend` | Requeue eligible delivery → same 202 shape as channel test, or 409; [delivery_resend.rs](../backend/src/api/delivery_resend.rs) |

Channel types are `webhook`, `discord`, `slack`, `mattermost`, `gotify`,
`ntfy` and `zulip`.
Generic signing secrets, when supplied, require 32–4,096 characters; Gotify
requires an application token; ntfy permits an optional bearer token. Zulip
requires `token` (bot API key), `bot_email`, `stream` (channel name), and `topic`.
The bot email is limited to 254 characters; channel and topic to 60 each. Zulip
fields reject blank values, surrounding whitespace and control characters. Tokens
must contain 1–4,096 characters without leading/trailing whitespace. API updates
preserve omitted or null secrets; blank strings do not clear them and fail
validation for the applicable channel type. The browser omits blank replacement
fields. Channel and rule names are case-insensitively unique among non-deleted
records. Rule creation requires `name`, `quiet_hours` and
`channel_ids`; `rule_kind` defaults to `provider` and `min_severity` to `minor`.
Provider rules require monitored `provider_ids`; optional `component_ids` narrow
that scope. System-health rules accept optional providers and no components.
Notification flags are `notify_detected`, `notify_started`, `notify_updated`,
`notify_resolved`, `notify_reopened`, `notify_maintenance`, `notify_recovered`;
all default true except maintenance. QuietHours is
`{enabled, timezone, days, start, end, critical_override}` with ISO weekdays 1–7
and distinct `HH:MM` start/end when enabled. New rule selections must refer to
valid, enabled channels and monitored coverage; existing disabled channel
associations may be retained while editing a rule.

Delivery diagnostics also synthesizes `status: "suppressed"` records for uncovered
events with no delivery rows. Their `id` and channel fields are null and
`can_resend` is false; `suppressed` is not a stored delivery state. Events covered
by another event from the same poll are omitted. `view=all` is not time-window
limited; `window` bounds recent attempts for `view=issues`, while `view=overdue`
examines currently eligible work. The generic suppression reason is not a
persisted explanation of which rule predicate failed.

IDs are UUIDs, timestamps use RFC3339, and nullable timing represents unknown
information. History endpoints use bounded cursor pagination. Incident ordering
uses provider activity, so unchanged polls do not make old incidents look new.
Treat cursors as opaque and send them back with unchanged filters. Incident pages
default to 20 items, diagnostics to 100, with `limit` clamped to 1–100. Comments
use fixed pages of 50; bookmarks and My Comments use 20. Health routes are outside
the API prefix and described under [health](#logging-health-and-verification).

### Monitoring and alert scope

A monitor selects either all components or an explicit set belonging to its
provider. Removing a monitor disables collection without deleting history.
Incident-feed and analytics `all` scope broadens event/component coverage but
still requires subscribed providers; retained incidents remain accessible by ID.
The incident feed also accepts `scope=provider`: all events within the provider's
configured geographic/product coverage, independent of selected monitor components.
Reliability-day links use this scope to match the calendar and are shown only for
enabled monitors, since the incident feed requires a subscription.

Alert rules select monitored providers and optionally narrow their components.
An empty component selection means all monitored components of the selected
providers. Provider rules filter by severity and event kind; system-health rules
cover source-stale/recovered episodes. Maintenance alerts are opt-in.

Editing an existing rule applies to future events and does not replay historical
notifications or reset previous deliveries. Creating a rule or creating/updating
a monitor can include existing active opening events. Explicit resends are a
separate operation.

Disabling a rule cancels its automatically queued, retrying, ambiguous and held
deliveries. Disabling a channel cancels all such deliveries for that destination.
Configuration associations remain in place, and re-enabling applies only to new
events; cancelled deliveries require an explicit resend. An HTTP request already
in flight may still complete. Deleting a rule or channel fails its outstanding
work.

## Provider catalog and adapter contracts

The compiled [YAML catalog](../backend/src/providers/catalog/) defines reviewed
origins, adapters and collection scope. Startup reconciles catalog metadata while
preserving historical identities. Catalog changes require a rebuild/restart;
there is no arbitrary-URL provider management API.

Collection scope and monitoring selection are separate. `component_scope: listed`
accepts only catalog IDs, so new upstream components require a reviewed catalog
update. `component_scope: all` accepts components returned by the adapter, subject
to any configured filters; for example, NetSuite discovers names beginning with
`US `. AWS discovers published `us-*` service-region pairs (including GovCloud)
and unregionalized global services. Its YAML entries are initial seeds, not a
service allowlist. Marketo discovers services whose registry environments include
Americas. A failed or empty inventory fails the poll instead of retiring components.
Monitoring all components includes newly collected components automatically.
Explicit component selections and component-specific alert rules do not expand.
All enabled catalog sources are polled before subscription, so users can select
from discovered components when configuring coverage. Discovery still respects
each source's collection scope and upstream availability; a new installation
shows seed components until its first successful poll.

Successful polls update component names and metadata by upstream ID and mark
components missing from the scoped snapshot inactive immediately. Failed polls do
not deactivate components. Inactive records and history are retained; the same ID
returning reactivates its existing record. All-component monitors exclude inactive
components, while explicitly selected inactive components contribute `unknown`
status. A replacement ID is a separate component and needs catalog review for
listed sources; selections are not transferred by matching names. Catalog startup
reconciliation seeds configured entries as active, and the next successful poll checks
their presence. There is no dedicated notification for component additions or
removals.

Adapters normalize official status and incident feeds into shared snapshots:
provider/component health, incidents and maintenance, affected scopes, timing,
raw evidence and feed completeness. Original classifications are retained
alongside normalized values. Network requests have bounded time and payload
limits. See [providers/mod.rs](../backend/src/providers/mod.rs) for the adapter
contract and shared HTTP behavior.

Marketo preserves the existing aggregate component ID for monitoring and history.
Native service health follows explicit service IDs in Americas incident updates;
an active aggregate incident without mapped service IDs leaves individual service
health unknown rather than assigning the outage to every service. Registry and
event feeds are both fetched on each poll so an unchanged event feed cannot hide
catalog changes.

Some sources need supplemental component, incident or maintenance feeds.
Statuspage-compatible feeds merge native Incident.io records, including incidents
and maintenance absent from the shorter compatibility archive. The native data
supplies affected components, impact severity, timelines, actual impact starts,
and separately published postmortems. Postmortems without a publication timestamp
retain an unknown timestamp. Intercom uses the same importer for its US hosting feed.
Explicit update scopes can fill missing incident scopes;
component names are not guessed from titles or prose. Structured provider HTML
is preserved where needed for links and sanitized by the frontend.

The [provider list](../PROVIDERS.md) owns supported coverage. Adapter code owns
source-specific parsing and history rules. Configured coverage does not guarantee
upstream availability or completeness.

`StatusProvider::validate_config` checks adapter inputs;
`fetch_snapshot(&FetchContext)` returns `Fetched(ProviderSnapshot)` or
`NotModified` metadata. FetchContext supplies origin, public configuration,
ETag/Last-Modified, known active IDs, history-refresh intent and a shared deadline.

| Adapter source | Feed strategy and constraint |
| --- | --- |
| [statuspage.rs](../backend/src/providers/statuspage.rs) | Summary plus optional component/incident/maintenance paths; Incident.io-compatible separate history is treated as incomplete for absence resolution |
| [datadog.rs](../backend/src/providers/datadog.rs) | One provider combining five official US Statuspage sites; site-qualified component/incident IDs, grouped components, and atomic polling across all sites |
| [statusio.rs](../backend/src/providers/statusio.rs) | Status.io current status endpoint with required catalog `page_id`; listed component coverage and no separate resolved-event archive importer |
| [intercom.rs](../backend/src/providers/intercom.rs) | Native Incident.io US hosting summary and bounded recent history; the compatible API ignores region selection, so only native regional feeds are used |
| [slack.rs](../backend/src/providers/slack.rs) | Current status, history/detail lookup for missing active records and resolution-note enrichment |
| [google_cloud.rs](../backend/src/providers/google_cloud.rs) | Product and incident JSON; shared adapter for Google Cloud and Gemini catalogs |
| [aws.rs](../backend/src/providers/aws.rs) | Published S3 service inventory restricted to US/GovCloud/global; current events and public S3 history, UTF-16 decoding and scoped service recovery |
| [salesforce.rs](../backend/src/providers/salesforce.rs) | Services, active incidents and paginated maintenance/history from Trust API; date-windowed maintenance requires explicit resolution |
| [pagerduty.rs](../backend/src/providers/pagerduty.rs) | Services, impacts, enum dictionaries and post details/history |
| [adobe.rs](../backend/src/providers/adobe.rs) | Adobe registry and StatusEvents for Marketo Americas services plus the existing aggregate; explicit service IDs determine individual impact |
| [okta.rs](../backend/src/providers/okta.rs) | Structured arrays and the published cellList embedded in status-page HTML, US cells and postmortem timing; depends on upstream page structure |

Adding YAML alone does not register a source: update the compiled catalog list and
approved source tuple in [catalog/mod.rs](../backend/src/providers/catalog/mod.rs).
New adapter keys also require registry/version entries. Catalog configuration
supports listed IDs or supported name-prefix scope and selected endpoint overrides;
the validator enforces adapter-specific combinations. Keep
[PROVIDERS.md](../PROVIDERS.md) aligned and run the offline catalog check.

Statuspage component groups are resolved from the upstream `group_id` when the
feed does not supply `group_name`, preserving regional identity for repeated
service names. Okta reads its published US production/preview cell list during
ordinary polls; malformed, duplicate, or empty cell inventories fail the poll
without replacing stored components. Historical incidents can still reference
older US cells. Catalog synchronization does not enable monitoring subscriptions,
and existing component selections are preserved.

New resolved historical imports share a 365-day lookback. Ongoing events and
known active events needed for recovery reconciliation are exempt. This limits
imports, not retained history, and does not promise a full year of source data.
An hourly history refresh requests the archive available through that adapter;
it does not crawl every public history page. Compatibility feeds can expose fewer
records than native feeds, which is why the Incident.io paths merge both.

Salesforce maintenance includes the Trust API's 30-day lookback and future windows,
paginated in batches of up to 1,000 records. Incident history uses batches of 100,
with two page requests at a time to stay within the shared polling deadline.
Both stop after 100 pages and fail the fetch on truncation or repeated IDs rather
than accepting an incomplete result. Maintenance falling outside the date window
does not prove recovery, so Salesforce records require an explicit terminal status;
both `Canceled` and `Cancelled` are treated as resolved while retaining the original
provider phase.

## Polling and reconciliation

The worker polls all due enabled catalog sources, including unsubscribed providers.
This refreshes component inventories, status and available incident history on the
existing schedules. Subscriptions control dashboard coverage and alert eligibility;
unsubscribing retains collection and history. Subscribing schedules a background
refresh without waiting for a source already locked by polling; that poll or the
next scheduled poll supplies fresh data. Leases coordinate claims,
concurrency limits bound work, and transient failures use backoff. Unchanged
responses refresh source freshness without creating new observations.

Claims use `FOR UPDATE SKIP LOCKED` and two-minute leases. A fetch has a 20-second
total deadline, up to five seconds to connect, no redirects and adapter-specific
response-size limits. Stored HTTP validators reduce transfers when possible;
active incident reconciliation and history refresh can force full responses.
Status polling and hourly history refreshes have independent schedules and failure
counters, but share the source lease to prevent concurrent reconciliation.
History failure does not increment status failure counters or move the status
schedule. An in-flight history fetch and the worker's batch completion can still
delay a due status check. Status work wins when both lanes are due. Normal
scheduling adds deterministic jitter; retryable transport/429/408/5xx failures use lane-local
backoff capped at 30 minutes, while parse/configuration failures return to that
lane's normal interval.
The per-host limit counts source jobs by their configured base-URL host; it is
not a semaphore around every HTTP request. Adapters such as Datadog and Salesforce
can make parallel subrequests within a single job's deadline.

Reconciliation transactionally updates components and current status, records
semantic changes, stores incident updates, and creates matching delivery work.
Duplicate observations are suppressed; reopened incidents have a new lifecycle
generation. New records already resolved are retained as history without opening
alerts.

An incident missing from a complete feed becomes resolution-pending, then resolves
after a second complete absence. Partial feeds require explicit resolution.
Absence does not invent a provider recovery timestamp, so a resolved record can
still have unknown duration. Resolution alerts require a corresponding opening
delivery for the same rule, channel and generation.
That opening may already be delivered, held for a summary or summarized; prior
delivery/summary evidence is retained through resends. Provider/component status
events covered by an incident event from the same poll are suppressed during
fan-out to avoid duplicate alerts for the same observation.

History refreshes supplement ordinary status polling where supported. Source
stale/recovered episodes describe collection health separately from service
outages. Poll outcomes and worker heartbeats provide operational evidence, but
failed finalization can leave committed observations beside incomplete poll logs.

See [polling/](../backend/src/polling/) for scheduling, reconciliation and event
fan-out. Per-worker limits and role-level heartbeats assume the supplied single
worker deployment.

## Quiet hours and delivery

Quiet hours use a timezone, weekdays and local start/end times. Overnight windows
belong to their start weekday. Eligible events are held and combined into a
summary when the window ends; they are not individually replayed. Critical
override can bypass the hold. Saved summaries are snapshots with links to related
records, and large summaries can omit entries.
Discord quiet-hours embeds link to the saved summary without including incident
descriptions. Their titles use `Quiet-Hours Summary: <rule name>`.

The dispatcher claims durable work, decrypts the destination and validates it
before sending. Destinations require HTTPS; private targets are blocked unless
explicitly enabled. Redirects are disabled. Channel configuration is encrypted
at rest and API reads expose only redacted targets.

Secrets use XChaCha20-Poly1305 with the channel ID/type as authenticated context.
Destination DNS is checked at send time and an accepted address is pinned for
the request. The delivery client ignores environment HTTP proxies. Discord
targets require the standard `discord.com` webhook path and no query string.
Slack accepts standard Slack or GovSlack webhook URLs. Mattermost accepts an
HTTPS URL ending in `/hooks/<token>`. Gotify accepts an HTTPS URL ending in
`/message` and sends its encrypted application token in `X-Gotify-Key`. ntfy
accepts an HTTPS topic URL and optionally authenticates with an encrypted bearer
token. Its final path segment must contain 1–64 ASCII letters, digits, `_` or `-`,
with no trailing slash or query. The dispatcher removes the topic segment from
the request URL and includes it in the JSON publish body. Zulip requires an HTTPS
`/api/v1/messages` URL without a query. It sends
form-encoded `type=stream`, JSON-encoded `to` (channel name), `topic` and `content`,
using HTTP Basic authentication with the encrypted bot email/API key. All alerts
for a delivery channel use its configured topic; direct messages are not supported.
The bot must have permission to post to its configured channel. Omitted or null
`bot_email`, `stream`, and `topic` updates preserve saved values; channel-field
validation is described in [the API contract](#alerts-and-diagnostics).
Private-target opt-in still requires HTTPS with a valid certificate; notification HTTP proxies are not
supported.

A successful HTTP response confirms delivery. Transient failures are retried,
while permanent failures stop. Ambiguous timeouts and crashes after remote
success can produce duplicates. Receivers should deduplicate by event ID.
A queued channel test is not proof of successful delivery.

Five dispatch tasks run per iteration. Each send allows five seconds for destination
resolution and 15 seconds for HTTP. Ordinary 4xx responses fail immediately;
408, 429, other non-success responses and transport errors consume a retry budget
of five recorded attempts. Retry jitter uses 60/300/900/3,600-second bounds and
Retry-After is capped at one hour. In-flight work is represented by a lease, not
a separate `sending` database status. Delivery states are `pending`, `retrying`,
`ambiguous`, `delivered`, `failed`, `held`, `summarized` and `cancelled`.

### Webhook contract

Generic webhooks use schema version `"1"` and include `event_id`, `event_type`,
`occurred_at`, `application_url`, readable `content` and event-specific data.
Provider IDs identify catalog entries, not polling sources. Notifications link to
the relevant StatusDeck record; Discord presents the same information as an embed,
Slack and Mattermost as attachments, and Gotify and ntfy as titled push
messages. Zulip uses a bold title and readable Markdown text with the same
status, severity, providers, components, incident description, timestamp and
StatusDeck link; provider Markdown is escaped and titles are capped at 256 characters
before escaping. Its message body shares Gotify/ntfy
truncation and quiet-summary handling. See [Zulip’s API](https://zulip.com/api/send-message).
Discord renders HTML incident updates as readable text before applying its
description length limit; stored provider content retains its original formatting.

Delivery-size policies (checked against upstream documentation/source in September 2026):

| Destination | Upstream limit and StatusDeck policy |
| --- | --- |
| [Discord](https://docs.discord.com/developers/resources/message#embed-limits) | Embed titles: 256 characters; field values: 1,024; descriptions: 4,096; combined embed text: 6,000. StatusDeck keeps its existing 256/1,024/1,500 budgets; four fields plus labels and footer fit the total. |
| [Slack](https://docs.slack.dev/legacy/legacy-messaging/legacy-secondary-message-attachments/) | Legacy attachments document a 300-character footer; the 700-character text threshold only collapses the display. StatusDeck caps attachment titles at 256, each field at 1,024, and description at 4,000. These are conservative application budgets, not Block Kit limits. |
| [Mattermost](https://developers.mattermost.com/integrate/webhooks/incoming/) | Post text supports 16,383 characters; attachments are stored separately in post properties. StatusDeck applies the same conservative attachment budgets as Slack. |
| [ntfy](https://docs.ntfy.sh/publish/#limitations) | Titles: 1,024 UTF-8 bytes; messages: 4,096 bytes by default. StatusDeck enforces both and fits the [JSON publish request](https://github.com/binwiederhier/ntfy/blob/main/server/server.go) within 8,192 bytes, including topic and JSON escaping. |
| [Zulip](https://zulip.com/api/send-message) | Server-specific message ceiling, [10,000 characters by default](https://github.com/zulip/zulip/blob/main/zproject/default_settings.py). StatusDeck caps the final escaped Markdown at 10,000 UTF-16 units, including title and link. |
| [Gotify](https://github.com/gotify/server/blob/master/model/message.go) | The current API model declares no fixed title/body length cap. StatusDeck retains its 1,900-character body policy; the title is unchanged. |
| Generic webhook | Receiver-specific limits; StatusDeck keeps the structured event intact and caps generated `content` at 1,900 characters. |

Character budgets use conservative UTF-16 counting for rich chat fields, so emoji
also fit. ntfy additionally counts UTF-8 bytes and serialized JSON. Push bodies
retain their 1,900-character presentation ceiling. Truncation reserves space for
the footer and link when they fit; an oversized link is omitted rather than cut.
An unusually large ntfy JSON request can require further shortening of its body
or title, or omission of its click URL. Self-hosted servers and reverse proxies
can impose lower limits; StatusDeck does not discover custom limits at runtime.

| Header | Meaning |
| --- | --- |
| `x-statusdeck-event-id` | Stable event UUID for receiver deduplication |
| `x-statusdeck-timestamp` | Unix seconds for the send attempt |
| `x-statusdeck-signature` | Optional `sha256=` plus hex HMAC-SHA256 of `timestamp + "." + exact JSON body bytes`, using the channel signing secret |

Exact event payloads and formatting are defined in
[notifications/](../backend/src/notifications/). Diagnostics renders stored events
with current configuration; its preview is not an archive of the original wire
bytes.

### Manual resend

Resend requeues the existing event on its original channel, retaining its delivery
ID and all previous attempts. It uses current channel configuration and formatting,
resets the automatic retry budget, and bypasses rule filters and quiet hours.
The event ID stays the same, so a receiver's deduplication may ignore it.

A disabled/deleted channel or an event already queued, held or sending on that
channel is ineligible. Only completed delivery states (`delivered`, `failed`,
`summarized`, `cancelled`) without an active lease can be resent. The delivery row
reflects the latest send; attempt history preserves prior outcomes. Resend requests
are audited.

## Persistence and analytics

PostgreSQL stores identity, subscriptions, observations, incident history,
comments/bookmarks, alert configuration and delivery evidence. Removing
subscriptions or alert configuration preserves historical records. Raw poll
payloads and session history have cleanup policies; incident and delivery history
are not automatically aged out.

| Persistence group | Tables and responsibility |
| --- | --- |
| Identity | `users`, `sessions`, `session_history`; profile JSON, hashed credentials/tokens and token-free session archive |
| Collection configuration | `provider_sources`, `providers`, `components`, `monitored_providers`, `monitored_components`; catalog identity, selection, leases and HTTP validators |
| Observations | `provider_status_current`, `component_status_current`, `status_changes`, `poll_runs`, `poll_payloads`; latest health, changes and collection evidence |
| Incident evidence | `incidents`, `incident_updates`, `incident_providers`, `incident_components`, `incident_update_components`, `incident_affected_scopes`, `incident_update_affected_scopes`; source identity, lifecycle generations, structured impact and timeline |
| Account annotations | `incident_comments`, `incident_bookmarks`; account-owned notes and saved incident references |
| Notifications | `notification_channels`, `alert_rules`, `alert_rule_channels`, `alert_rule_providers`, `alert_rule_components`, `notification_events`, `notification_deliveries`, `notification_attempts`; encrypted destinations, routing, durable work and attempts |
| Operations | `worker_heartbeats`, `audit_log`; role liveness/progress and business/security changes |

The migration enforces source/upstream identities, component ownership and
event/rule/channel delivery uniqueness. SQL functions implement quiet-hours
timezone behavior and session archival; `incident_timing` centralizes duration
bounds. Timezone transitions follow PostgreSQL's handling of ambiguous and
nonexistent local times. There is no ORM entity layer or database-independent
storage abstraction.

The worker deletes expired sessions and old raw payloads in batches of 1,000.
Session history retains at most 100 records per user and is cleaned after 90 days.
Raw payloads are saved on semantic changes or as failure evidence, not for every
poll. Poll runs, audit records, status changes and notification history have no
automatic retention policy.

[The SQLx migration](../backend/migrations/0001_initial.sql) creates the complete
schema, including resend accounting, and runs explicitly before startup. See
[upgrade guidance](deployment.md#container-setup-and-initialization).

### Analytics and reliability

Duration consumers share the database's `incident_timing` rules:

| Value | Interpretation |
| --- | --- |
| Incident start | Explicit provider impact start, falling back to provider publication time |
| Resolved end | Valid provider recovery time at or after the start |
| Ongoing end | Current query time; elapsed impact rather than final restoration time |
| Maintenance bounds | Actual impact evidence; planned dates and publication are not substitutes |
| Unknown duration | Missing or inconsistent bounds; remains in counts but contributes no affected time or restoration statistics |

Import time is never a duration fallback. Publication-based intervals describe
the reported incident window and may differ from actual outage duration.
Planned completion does not prove maintenance recovery; unconfirmed completion
remains uncertain.

Analytics clips impact to the selected period and merges overlaps per provider,
then sums across providers. Totals can therefore exceed elapsed wall time.
Restoration statistics use full durations of resolved incidents overlapping the
period; ongoing and unknown-duration events are excluded.
Percentiles use linear interpolation between adjacent sorted durations, rounded
to seconds; the median averages the two middle values for even-sized samples.
Component summaries retain unknown-duration counts instead of presenting missing
timing as zero impact. Enabling maintenance labels combined counts as events.

| Derived value | Contract |
| --- | --- |
| Known/count date | Explicit start, maintenance planned start, publication, then first observation; the last fallback locates a record but never establishes impact duration |
| Analytics period | Rolling 7/30/90/365 days ending at query time; trend buckets are 1, 7, 7 or 30 days respectively, with a shorter final bucket when needed |
| Analytics incident count | Unique events overlapping the period when duration is known, or located by their known date when duration is unknown; maintenance is included only when requested |
| Major/minor chart bands | Major includes `major` and `critical`; minor includes the remaining severities, including `info`. These are chart groupings, not new upstream classifications |
| Reliability frequency | Incident counts by known date, once per event; daily and monthly affected time instead include overlapping incident intervals |
| Timing precision | Affected-time aggregates retain fractional seconds; individual durations and restoration inputs use whole seconds, with interpolated percentiles rounded at output |

Incident detail returns duration from the same timing view. Distinct update IDs
are retained even when bodies or import timestamps coincide; timeline display
prefers source display time. Presentation grouping is owned by the frontend.

Reliability uses calendar days in the requested IANA time zone, including the
current partial day. Period and daily boundaries use PostgreSQL's time-zone
conversion so skipped or repeated local midnights are handled consistently.
A healthy day means a successful observation with no
matching recorded event, not proven continuous uptime.
Maintenance is marked separately and does not contribute affected time or
incident-duration uncertainty to Reliability history. Unknown maintenance
durations remain visible in the all-event warnings. History completeness and
period comparisons are not established. Exact calculations live in the
[migrations](../backend/migrations/), [analytics handler](../backend/src/api/analytics.rs)
and [reliability query](../backend/src/api/reliability.sql).

## Logging, health and verification

Structured logs include request IDs and worker/source/delivery context. Audit
records cover account and configuration changes. Poll runs, worker heartbeats
and delivery attempts support troubleshooting.

`/health/live` confirms the API process responds. `/health/ready` also checks the
database, recorded migration state and both worker roles. The worker has a
separate CLI health check. Readiness does not prove provider availability or
successful notification delivery.

[Deployment checks](deployment.md#local-development-and-checks) cover builds and
static validation. There are no Rust or frontend test suites. Runtime behavior
still needs operational verification.
