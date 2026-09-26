# Frontend

The React and TypeScript application presents stored provider health and manages
monitoring, alerts and the administrator account. It reads the API; opening or
refreshing a page does not poll providers directly.

See [architecture](architecture.md) for system boundaries, [backend](backend.md)
for processing and API behavior, and [deployment](deployment.md#local-development-and-checks)
for setup and validation commands.

## Application structure

[App.tsx](../frontend/src/app/App.tsx) owns the authenticated shell, routing and
shared providers. Features live under [features/](../frontend/src/features/),
with reusable presentation in [components/](../frontend/src/components/).

| Routes | Subsystem and primary components | API dependency / responsibility |
| --- | --- | --- |
| `/login` | [Login](../frontend/src/features/auth/Login.tsx) | CSRF bootstrap and session creation; unauthenticated requests to other paths also show this screen |
| `/` | [Dashboard](../frontend/src/features/dashboard/Dashboard.tsx) | `/dashboard`; monitored health, filters, pins and rotating authenticated wallboard |
| `/catalog`, `/catalog/:id` | [Catalog](../frontend/src/features/catalog/Catalog.tsx), [ProviderDetail](../frontend/src/features/catalog/ProviderDetail.tsx), [ReliabilityHistory](../frontend/src/features/catalog/ReliabilityHistory.tsx) | `/catalog/providers`, `/monitors`, provider detail/reliability; discovery and component selection |
| `/incidents`, `/incidents/:id` | [Incidents](../frontend/src/features/incidents/Incidents.tsx), [IncidentDetail](../frontend/src/features/incidents/IncidentDetail.tsx), [IncidentComments](../frontend/src/features/incidents/IncidentComments.tsx) | Incident feed/detail and comments; provider evidence, maintenance and internal notes |
| `/bookmarks`, `/my-comments` | [Bookmarks](../frontend/src/features/incidents/Bookmarks.tsx), [MyComments](../frontend/src/features/incidents/MyComments.tsx), [PersonalIncidentFilters](../frontend/src/features/incidents/PersonalIncidentFilters.tsx) | `/bookmarks`, `/my-comments`; paginated account records and URL filters |
| `/analytics` | [Analytics](../frontend/src/features/analytics/Analytics.tsx) | `/analytics`; period/scope/maintenance filters and impact statistics |
| `/notifications` | [Notifications](../frontend/src/features/notifications/Notifications.tsx), [ChannelPanel](../frontend/src/features/notifications/ChannelPanel.tsx), [AlertRulePanel](../frontend/src/features/notifications/AlertRulePanel.tsx), [QuietHoursFields](../frontend/src/features/notifications/QuietHoursFields.tsx) | Channels, rules, catalog component selections and quiet-hours preview |
| `/notifications/summaries/:id` | [NotificationSummary](../frontend/src/features/notifications/NotificationSummary.tsx) | `/notifications/summaries/:id`; saved summary **event** ID, not delivery ID |
| `/system` | [SystemHealth](../frontend/src/features/system-health/SystemHealth.tsx), [RuntimeOverview](../frontend/src/features/system-health/RuntimeOverview.tsx), [DataMetrics](../frontend/src/features/system-health/DataMetrics.tsx) | System sources/summary/deliveries/attempts/resend/data-metrics; pipeline diagnostics |
| `/profile` | [Profile](../frontend/src/features/profile/Profile.tsx), [Appearance](../frontend/src/features/profile/Appearance.tsx), [SecurityActivity](../frontend/src/features/profile/SecurityActivity.tsx) | `/profile` and its credential/session/activity subroutes; account settings |

API paths in this table omit `/api/v1`. Browser paths use React Router; `:id`
is a route parameter. Unknown authenticated paths redirect to `/`. Nginx falls
back to `index.html` for browser deep links.

[main.tsx](../frontend/src/main.tsx) initializes the theme and mounts StrictMode
and RouterProvider using `createBrowserRouter`. Its catch-all route mounts
AppRoot, which supplies the query client and toasts; App
checks CSRF/session state, supplies ProfileContext, then mounts the shared shell.
Feature pages are lazy imports with a Suspense loading fallback. Navigation
updates the page title and focuses the main region. Client gating is a usability
boundary; backend handlers enforce the
[single-administrator security model](backend.md#authentication-and-request-boundaries).

The application assumes one local administrator. Sign-in restores the requested
page or the account's preferred landing page; session loss clears cached account
data. The wallboard remains part of the authenticated application.
There is no browser role/permission hierarchy, signup, OAuth or password reset.

## State and data fetching

TanStack Query owns server state. [client.ts](../frontend/src/api/client.ts)
centralizes same-origin requests, session cookies, CSRF headers and API errors;
[queries.ts](../frontend/src/api/queries.ts) owns cache keys and shared invalidation.
[types.ts](../frontend/src/api/types.ts) mirrors backend contracts manually and
does not validate responses at runtime.

`api<T>(path, RequestInit)` includes cookies, adds the CSRF header for mutations,
handles empty 204 responses, and converts unsuccessful JSON responses into
`ApiFailure(status, code, message)`. A 401 other than `invalid_credentials`
dispatches `statusdeck:unauthorized`. Query functions pass AbortSignal to fetch;
the client has no independent request timeout. Default queries stay fresh for
15 seconds and retry once except on 4xx errors. Session reads refresh every
60 seconds while the page is visible.

Account preferences are stored on the server. Pins, the initial theme cache and
the incident advanced-filter expansion preference are local to the browser.
Shareable incident, bookmark, personal-comment and analytics filters live in the
URL; unsaved forms and catalog filters use component state.

[usePinnedProviders](../frontend/src/hooks/usePinnedProviders.ts) stores per-user
provider IDs and listens for storage changes in other tabs.
[useRefreshInterval](../frontend/src/hooks/useRefreshInterval.ts) saves the shared
preference (`0`, `5000`, `15000` or `60000` milliseconds) through Profile; zero
disables optional refresh. Monitoring changes call `invalidateMonitoringState`
to refresh related dashboard, catalog, incident, analytics and diagnostic caches.

Overview and incident-feed refresh use the account preference. Detail and
diagnostics pages have their own refresh schedules. Loading additional history
can pause refresh to keep the list stable. Cached data remains visible after
refresh failures with a stale/unavailable indication; mutations are not
implicitly retried.

The incident page translates the browser-only `provider_tag` filter into catalog
provider IDs before calling the API. [searchParams.ts](../frontend/src/utils/searchParams.ts)
selects allowed query parameters; browser filters are not an independent API
contract. Native date/time fields are converted using the account timezone by
[display.ts](../frontend/src/utils/display.ts).

Forms use controlled React state and native input constraints, with backend
validation authoritative. Pending states prevent repeated submissions; field or
panel errors and shared toasts report outcomes. Comments enforce a 5,000-character
limit and share the sanitized renderer for preview. Empty history, missing
records, initial load failure and failed background refresh have distinct states.

## Monitoring and reliability

Subscriptions select whole providers or specific components. Provider-wide
status, selected coverage and feed freshness remain distinct so a healthy
selection does not hide broader provider trouble or stale information.

Bulk subscription actions apply to the full catalog, independent of visible
filters. They report partial failures rather than acting as one transaction.
Removing a subscription preserves collected history, comments and bookmarks.

Reliability and Analytics describe provider-reported history, not measured uptime.
Missing timing and incomplete history remain explicit. Maintenance distinguishes
planned windows, actual activity and unconfirmed completion. Calculation rules
belong in [backend analytics](backend.md#analytics-and-reliability).

## Incidents and comments

Incident details combine original provider classifications, affected components
and scopes, source timing, and updates. Repeated equivalent updates are grouped
for readability while provider-supplied and synthesized observations remain
distinguishable. Missing provider data remains explicitly unknown.

Comments are internal notes: saving one does not modify provider records or send
notifications. Bookmarks reference the current incident record, and both saved
incidents and personal comments remain accessible after unsubscription.

## Notification and account forms

Channel forms never retrieve stored plaintext secrets. Leaving replacement URL,
signing-secret or Gotify/ntfy/Zulip token fields blank preserves existing values.
Zulip also requires a bot email, channel name and topic on creation; blank
replacement fields preserve these values when editing. Its destination is the
Zulip server’s HTTPS `/api/v1/messages` endpoint.
There is no form action to clear a saved secret. A channel test queues a delivery;
its outcome is available in System diagnostics. Destination and token constraints
are documented with the [backend channel contract](backend.md#alerts-and-diagnostics).

Alert rules narrow monitored coverage by provider, component, severity and event
kind. An empty component selection includes all monitored components of the
selected providers. Unavailable selections remain visible for correction.
Cloning opens an unsaved draft. Editing an existing rule applies to future events
without replaying historical notifications.

Quiet hours defer eligible notifications into summaries. The summary page shows
a saved snapshot with links to related records. See
[delivery behavior](backend.md#quiet-hours-and-delivery) for retries and resends.

Profile manages display preferences and account security. Credential changes
require the current password and revoke other sessions. The installation has no
signup or account-recovery flow.

Notifications blocks route changes and browser unload while channel/rule drafts
are dirty; Profile does this for unsaved display preferences, and incident
comments for new drafts or changed edits. Successful comment saves clear the
warning; failed saves retain the draft and protection. This protection
uses React Router's data router and is not shared by every form. Drafts are not
persisted across reloads. [TimeZoneSelect](../frontend/src/features/profile/TimeZoneSelect.tsx)
lists browser-supported timezones; the API validates the saved value.

## Diagnostics and reusable UI

System diagnostics separates provider service health from the health of
StatusDeck's collection and delivery pipeline. It shows recent deliveries,
attempt history and payloads alongside source, worker and database evidence.
For each source it shows consecutive failures and failed completed status checks
out of all completed status checks in the preceding 24 hours; sources without
checks show that explicitly. History polls do not contribute.
Summary windows affect recent statistics, not the underlying delivery history.
See [metric definitions](deployment.md#system-diagnostics).

Delivery history can include `suppressed` events with no delivery ID or channel.
These have no attempt history and cannot be resent; see the
[diagnostic record contract](backend.md#alerts-and-diagnostics).

Manual resend queues an existing event on its original channel and preserves
attempt history. It is an explicit action that bypasses rule filters and quiet
hours; successful queueing is not proof of delivery.

Shared components provide search, dialogs, loading/error states, status labels,
content rendering and account-aware timestamps. Prefer those conventions when
adding features. Styling and accessibility behavior are centralized in
[global.css](../frontend/src/styles/global.css); coding conventions are in
[STYLE.md](../frontend/STYLE.md).

| Shared subsystem | Inputs, output and constraints |
| --- | --- |
| [QuickSearch](../frontend/src/components/QuickSearch.tsx) | Searches catalog names and API incident results; keyboard navigation routes to the chosen record |
| [ConfirmDialog](../frontend/src/components/ConfirmDialog.tsx), [ToastProvider](../frontend/src/components/ToastProvider.tsx), [LoadingSkeleton](../frontend/src/components/LoadingSkeleton.tsx), [EmptyState](../frontend/src/components/EmptyState.tsx) | Confirmation, feedback and loading/empty presentation reused by features |
| [StatusBadge](../frontend/src/components/StatusBadge.tsx), [MaintenanceWindow](../frontend/src/components/MaintenanceWindow.tsx), [MetricCard](../frontend/src/components/MetricCard.tsx) | Display normalized health, timing and metrics without fetching provider data |
| [RelativeDateTime](../frontend/src/components/RelativeDateTime.tsx), [display.ts](../frontend/src/utils/display.ts), [profileContext](../frontend/src/features/profile/profileContext.ts) | Format timestamps/durations using account preferences; preserve unknown timing |
| [ReliabilityCharts](../frontend/src/features/catalog/ReliabilityCharts.tsx), [ReliabilityTrends](../frontend/src/features/catalog/ReliabilityTrends.tsx) | Consume reliability data and filter callbacks; Chart.js renders trends, with account theme colors |
| [ProviderContent](../frontend/src/components/ProviderContent.tsx), [CommentMarkdown](../frontend/src/features/incidents/CommentMarkdown.tsx) | Adapt provider text and account notes to the shared Markdown renderer |
| [BookmarkButton](../frontend/src/features/incidents/BookmarkButton.tsx), [CommentEditor](../frontend/src/features/incidents/CommentEditor.tsx) | Reusable saved-incident mutation and comment editing/preview |

[theme.ts](../frontend/src/features/profile/theme.ts) resolves light/dark/system
preferences to the document theme and caches the preference under
`statusdeck:theme` to apply it before mounting. CSS variables own colors, borders,
radius and chart tokens; responsive and reduced-motion rules live in global CSS.
Icons/avatars are code-rendered. Brand logos and the favicon are static PNGs in
[public/](../frontend/public/), referenced by CSS and
[index.html](../frontend/index.html). Provider reliability also imports its
[feature stylesheet](../frontend/src/features/catalog/reliability-preview.css).
KaTeX fonts and highlighting CSS come from npm dependencies; Vite emits bundled
assets. There is no separate design-system package or asset service.

## Markdown support

Provider updates, saved comments and previews share the sanitized
[MarkdownContent](../frontend/src/components/MarkdownContent.tsx) renderer. It
supports common Markdown and GFM, code highlighting, math and Mermaid diagrams.
Unsupported or invalid diagrams remain readable as source.

Provider HTML is sanitized, scripts and executable embeds are removed, and
Mermaid interactions are disabled. Markdown images render their alt text without
loading the referenced resource. Renderer code and fonts are bundled locally;
rendering and network restrictions are defined in the shared renderer and
[Nginx configuration](../deploy/nginx.conf).

[MermaidDiagram](../frontend/src/components/MermaidDiagram.tsx) lazy-loads Mermaid,
limits input to 10,000 characters and 200 edges, and displays the result as a
blob-backed SVG image. It retains readable source on failure and revokes object
URLs when replaced or unmounted. Sanitized HTML IDs and local references are
prefixed per rendered document to avoid collisions between comments and updates.

## Build and development configuration

[package.json](../frontend/package.json) defines Vite development/build,
TypeScript checks, ESLint and Prettier commands; there is no `test` script.
[tsconfig.app.json](../frontend/tsconfig.app.json) enables strict TypeScript and
targets ES2022, while [tsconfig.node.json](../frontend/tsconfig.node.json) covers
build configuration. [eslint.config.js](../frontend/eslint.config.js) enforces
React hooks and TypeScript conventions, and [.prettierrc.json](../frontend/.prettierrc.json)
defines formatting. The build runs TypeScript checks and emits `frontend/dist/`.

[vite.config.ts](../frontend/vite.config.ts) owns the development proxy. There
are no configured frontend environment variables, SSR server, service worker or
offline persistence. Commands and matching backend setup are maintained in
[deployment](deployment.md#local-development-and-checks). The repository has no
declared browser support matrix or automated browser/accessibility suite.
