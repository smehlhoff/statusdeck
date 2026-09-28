# Frontend

The browser application presents stored provider health and lets the administrator
configure monitoring, alerts and account settings. [Architecture](architecture.md)
explains the system; [deployment](deployment.md#local-development-and-checks)
owns setup and validation commands.

## Application and routes

[main.tsx](../frontend/src/main.tsx) initializes the theme and React Router.
[App.tsx](../frontend/src/app/App.tsx) owns the query client, session gate, shared
navigation/layout and lazy-loaded pages. Authentication restores the intended
location or preferred landing page; session loss clears account caches.
The backend remains authoritative for access control.

| Routes | Subsystem and purpose |
| --- | --- |
| `/login` | [Authentication](../frontend/src/features/auth/): password or configured OIDC sign-in |
| `/` | [Dashboard](../frontend/src/features/dashboard/): monitored health, freshness, pins and wallboard |
| `/catalog`, `/catalog/:id`, `/catalog/:id/coverage` | [Catalog](../frontend/src/features/catalog/): subscriptions, provider evidence, reliability and component selection |
| `/incidents`, `/incidents/:id` | [Incidents](../frontend/src/features/incidents/): searchable history, timelines, maintenance and internal comments |
| `/bookmarks`, `/my-comments` | [Personal incident views](../frontend/src/features/incidents/): saved records and account notes |
| `/analytics` | [Analytics](../frontend/src/features/analytics/): period/scope filters and reported-impact summaries |
| `/notifications`, `/notifications/summaries/:id` | [Notifications](../frontend/src/features/notifications/): channels, alert rules, quiet hours and saved summary events |
| `/system` | [System health](../frontend/src/features/system-health/): collection/delivery diagnostics and resend actions |
| `/profile`, `/profile/sso-instructions` | [Profile](../frontend/src/features/profile/): preferences, credentials, sessions, SSO configuration/linking and setup guides |

All data views require a session, including the wallboard. Unknown authenticated
routes return to the dashboard. Nginx serves the application for browser deep
links; there is no server-rendered frontend.

## State and API boundary

| State | Owner and interaction |
| --- | --- |
| Server data | TanStack Query; [queries.ts](../frontend/src/api/queries.ts) defines keys and shared invalidation |
| Account preferences | PostgreSQL via Profile; [profileContext.ts](../frontend/src/features/profile/profileContext.ts) exposes display settings |
| Shareable filters | URL query parameters for incidents, bookmarks, comments and analytics |
| Drafts and local navigation | Component state; drafts are not persisted across reloads |
| Browser preferences | Per-account pins, initial theme cache and selected display controls in local storage |

[client.ts](../frontend/src/api/client.ts) sends same-origin requests with cookies,
adds CSRF headers to mutations, handles empty responses and translates API errors
into `ApiFailure`. Session-expiry responses notify the application shell.
[types.ts](../frontend/src/api/types.ts) mirrors server contracts manually; it is
not runtime response validation. Update both sides when contracts change.

Query functions pass cancellation signals. Pages use periodic refetching or
explicit invalidation; reading data does not force an upstream provider poll.
[useRefreshInterval](../frontend/src/hooks/useRefreshInterval.ts) manages the
account's optional overview/feed refresh preference, while detail and diagnostics
pages have their own schedules. [usePinnedProviders](../frontend/src/hooks/usePinnedProviders.ts)
keeps browser pins in sync across tabs. Monitoring changes invalidate related
catalog, dashboard, incident and analytics data together.

## Feature behavior and forms

Forms combine React state/native constraints with authoritative server validation.
Pending states prevent repeat submission; field/panel errors and toasts report
outcomes. Loading, empty results, missing records and failed refreshes have
separate presentation. Cached results can remain visible after a refresh failure.

- **Monitoring:** whole-provider coverage includes newly collected components;
  explicit selections do not expand automatically. Coverage edits retain missing
  components for review. Bulk subscription saves can partially fail; unsuccessful
  optimistic updates revert. Unsubscribing retains history and account notes.
- **History and analysis:** timelines preserve unknown source timing. Charts use
  backend calculations and distinguish missing history from known zero impact.
  [display.ts](../frontend/src/utils/display.ts) owns preference-aware date/time
  formatting; [searchParams.ts](../frontend/src/utils/searchParams.ts) selects URL
  filters. Reliability uses local calendar days; Analytics uses rolling periods.
- **Notifications:** forms expose redacted destinations and preserve saved secrets
  when replacement inputs are blank. Queuing a channel test or resend does not
  prove delivery; System shows the outcome. Saved summary URLs use event IDs.
- **Account:** local login remains available alongside OIDC. Credential and SSO
  administration follow the [backend security rules](backend.md#authentication-and-request-boundaries).
  There is no signup or password-reset flow.

Coverage, notification drafts, profile preferences and comment drafts guard
unsaved navigation. This protection is not universal and cannot preserve drafts
across reloads. Exact form inputs and validation live in each linked feature.

## Shared presentation

[components/](../frontend/src/components/) provides search, confirmation dialogs,
help controls, toasts, loading/empty states, status badges and account-aware dates.
Feature components compose these controls rather than owning separate fetching
or authentication layers. Chart.js visualizes API-derived metrics.

Provider updates and comments share [MarkdownContent](../frontend/src/components/MarkdownContent.tsx).
It sanitizes HTML, supports code/math and uses [MermaidDiagram](../frontend/src/components/MermaidDiagram.tsx)
for bounded, non-interactive diagrams. Remote Markdown images are not loaded;
invalid diagrams retain readable source. Provider content remains untrusted.

[global.css](../frontend/src/styles/global.css) owns theme tokens and common
responsive/reduced-motion rules; feature CSS covers local layouts.
[theme.ts](../frontend/src/features/profile/theme.ts) resolves light/dark/system
preferences before rendering. Branding comes from [public/](../frontend/public/);
fonts and renderer assets are bundled dependencies. See [STYLE.md](../frontend/STYLE.md)
for contribution conventions, rather than duplicating them here.

## Build and development boundaries

[package.json](../frontend/package.json) defines Vite, TypeScript, ESLint and
Prettier commands. [vite.config.ts](../frontend/vite.config.ts) proxies local API
and health requests; [tsconfig.app.json](../frontend/tsconfig.app.json) and
[tsconfig.node.json](../frontend/tsconfig.node.json) cover application/build code.
There are no configured frontend environment variables or automated test suites.
The app has no service worker or offline data store, and no browser support matrix
is declared.

The [Authentik](authentik.md), [Keycloak](keycloak.md) and [Authelia](authelia.md)
guides are imported by the SSO instructions page and copied into frontend image
builds. They are application inputs as well as documentation; preserve that
integration when editing or moving them.
