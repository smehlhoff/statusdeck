# Deployment and operations

Docker Compose supplies PostgreSQL, a one-shot migrator, API, worker and Nginx
frontend. It builds application images from the checkout. The operator supplies
production TLS ingress, backups and external monitoring. See
[architecture](architecture.md) for component boundaries.

## Prerequisites and environments

| Environment | Requirements and behavior |
| --- | --- |
| Compose | Docker Engine/Compose supporting environment-backed secrets and health dependencies; Make/Bash for helpers |
| Host development | [Pinned Rust toolchain](../backend/rust-toolchain.toml), Node/npm (CI uses Node 24), reachable PostgreSQL with `pgcrypto` privileges |
| Production | HTTPS ingress, public origin/trusted hosts, secure cookies and exact build metadata |

Use [compose_base.yml](../compose_base.yml) first, followed by
[compose_dev.yml](../compose_dev.yml) or [compose_prod.yml](../compose_prod.yml).
Both use the same service topology and built static frontend, without source
mounts/hot reload. Production requires explicit origin, hosts and version/revision;
it also forwards the secure-cookie setting. Development Compose is the local
HTTP path. Host development uses Vite for hot reload.

## Configuration reference

Start from [.env/.env.example](../.env/.env.example). It contains placeholders,
not usable credentials. Compose reads the selected env-file and mounts secret
values as files; the backend reads its process environment, not dotenv files.
[config.rs](../backend/src/config.rs) owns exact validation and defaults.

| Variable | Purpose / important requirement |
| --- | --- |
| `STATUSDECK_DATABASE_URL` | Required PostgreSQL URL; Compose hostname is `postgres` |
| `STATUSDECK_DATABASE_PASSWORD` | Compose database secret; must match the URL password |
| `STATUSDECK_SECRET_KEY` | Required independent session-hashing key, at least 32 bytes; rotation invalidates sessions |
| `STATUSDECK_ENCRYPTION_KEY` | Required 32-byte key, raw or base64; preserve to decrypt channel and OIDC settings |
| `STATUSDECK_ADMIN_EMAIL`, `STATUSDECK_ADMIN_PASSWORD` | Email required by API/worker; password needed for first bootstrap, 12–1,024 characters; does not reset an existing account |
| `STATUSDECK_BASE_URL` | Public HTTP(S) origin, without path/query/fragment; used for Origin checks and application links |
| `STATUSDECK_TRUSTED_HOSTS` | Comma-separated hostnames; empty disables host filtering |
| `STATUSDECK_SESSION_COOKIE_SECURE` | Must be `true` for HTTPS; the example's `false` is only for local HTTP |
| `STATUSDECK_TRUST_PROXY` | Trust forwarded host/client headers; Compose fixes this to `true` for the private API |
| `STATUSDECK_ALLOW_PRIVATE_NOTIFICATION_TARGETS` | Default `false`; opt-in allows private targets while retaining HTTPS requirements |
| `STATUSDECK_BIND`, `STATUSDECK_PORT` | Backend listen address (host default `127.0.0.1:8080`) and Compose-published Nginx port (default 80) |
| `STATUSDECK_DEFAULT_POLL_INTERVAL`, `STATUSDECK_STALE_MULTIPLIER` | Collection interval in seconds (default 300) and freshness multiplier (default 3) |
| `STATUSDECK_GLOBAL_POLL_CONCURRENCY`, `STATUSDECK_PER_HOST_CONCURRENCY` | Per-worker source-job limits (defaults 10 and 2), not limits on every adapter subrequest |
| `STATUSDECK_RAW_PAYLOAD_RETENTION_DAYS` | Raw evidence retention (default 7); does not age out all history |
| `STATUSDECK_LOG_LEVEL`, `STATUSDECK_LOG_FORMAT` | Tracing filter and `pretty`/`json`; Compose defaults to JSON |
| `STATUSDECK_VERSION`, `STATUSDECK_REVISION` | Image-label build metadata; application version still comes from Cargo |
| `ENV_FILE` | Makefile env-file override; defaults to `.env/.env` |

Database URL, session key, encryption key, admin email and admin password also
accept exact `<NAME>_FILE` variables, which take precedence. Compose wires these
for all except email, which it passes directly. OIDC and notification credentials
are configured after login and encrypted in PostgreSQL; there are no frontend
`VITE_*` settings. Changing the Compose database password does not rotate an
already-initialized database's credentials.

## Container setup and initialization

Commands run from the repository root unless stated otherwise:

```sh
cp -n .env/.env.example .env/.env
# Replace placeholders before starting; keep existing settings if the file exists.
make dev
# Or start the development stack detached:
make docker
```

Use independent secret keys and matching URL/database passwords; URL-encode
special characters in credentials. The example browser origin is
`http://localhost:8080`. [Makefile](../Makefile) also provides `infra-up`,
`infra-down`, `migrate`, `api`, `worker` and `frontend` targets using development
Compose. `frontend` starts the built application, not Vite.

For production, set the public origin, trusted hosts, secure cookies and exact
version/revision, configure external HTTPS ingress, then run:

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml config --quiet
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml up -d --build --wait
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml ps
```

Only Nginx is published. The named `postgres18-data` volume persists PostgreSQL;
keep the same Compose project identity across upgrades to select the same volume.
Startup waits for PostgreSQL and migration success. API/worker bootstrap and
catalog reconciliation preserve existing credentials and selections. Collection
starts for enabled catalog sources; subscriptions determine dashboard/alert scope.

### Upgrades

Back up the database and retain its encryption key and previous application
revision first. The sole [0001_initial.sql](../backend/migrations/0001_initial.sql)
is a consolidated baseline. Older versions of `0001`, or the former multi-file
OIDC migrations, require explicit schema/history reconciliation. Do not delete
migration records or rerun rewritten SQL to bypass incompatibility.

Migrator, startup and readiness compare recorded migration versions/checksums;
they do not detect manual schema changes. No legacy conversion or down migration
is supplied. For a database compatible with the checked-in history:

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml build
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml stop backend worker
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml run --rm migrate
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml up -d --wait
```

## Local development and checks

Provide a host-accessible PostgreSQL database; Compose does not publish its port.
Export the required backend settings from the table with a host database URL,
`STATUSDECK_BASE_URL=http://localhost:5173`, matching trusted hosts and secure
cookies disabled for HTTP. The Compose env-file/internal `postgres` hostname
is not automatically usable by a host process.

```sh
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- migrate
npm ci --prefix frontend
# Run each in a separate terminal:
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- api
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- worker
npm run dev --prefix frontend
```

[Vite](../frontend/vite.config.ts) serves port 5173 and proxies API/health to 8080.
Keep the browser origin equal to `STATUSDECK_BASE_URL`, including if Vite chooses
another port. Vite does not apply Nginx's production security headers or limits.

```sh
# From backend/ (uses rust-toolchain.toml):
cargo fmt -- --check
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
cargo run --locked -- validate-catalog

# From frontend/:
npm ci
npm run check
npm run lint
npm run format:check
npm run build
```

There are no Rust/frontend test suites or frontend `test` script. These are build,
lint and static/catalog checks, not runtime coverage. Catalog validation needs
neither secrets nor a database. Frontend build includes TypeScript checks.

## Ingress and runtime boundaries

The [Dockerfiles](../deploy/) use pinned base images; [Compose](../compose_base.yml)
sets resource budgets, non-root application users, dropped capabilities and
bounded container logs. These budgets are configuration, not measured capacity.
No cloud provisioning or multi-replica deployment is supplied.

[Nginx](../deploy/nginx.conf) provides SPA fallback, API proxying, browser security
headers and login rate limits. It does not provision TLS or HSTS. Keep the API
private. If another proxy precedes Nginx, define trusted client-address forwarding;
no real-IP configuration is supplied for that extra hop. Otherwise login limits
may group users behind the ingress address.

## Health, logging and troubleshooting

| Signal | Meaning |
| --- | --- |
| `/health/live` | API process responds; exposed by public Nginx |
| `/health/ready` | Database, matching migration history and both worker heartbeats; check the API internally, public Nginx blocks it |
| `statusdeck healthcheck-worker` | Recent poller and dispatcher heartbeats |
| System page | Source freshness, worker progress, database state, deliveries and attempts |

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml logs --tail=100 backend worker
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml exec -T backend wget -qO- http://localhost:8080/health/ready
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml exec -T worker statusdeck healthcheck-worker
```

Readiness requires workers even without subscriptions. A heartbeat establishes
liveness, not completed work or upstream availability. A Compose unhealthy status
alone does not restart a process. Structured logs and audit records provide
request/change context; no monitoring service or Prometheus endpoint is bundled.
External alerts must detect a stopped worker, which cannot send its own alerts.

System distinguishes service outages from collection and delivery failures.
Current queue state and windowed history answer different questions. Held work
awaits quiet-hour release; ambiguous sends may already have reached the receiver.
Successful queuing is not delivery confirmation. After deployment, inspect
readiness, login, collection progress and delivery outcomes; channel tests send
real messages. Exact diagnostic definitions live in [system_summary.rs](../backend/src/api/system_summary.rs)
and adjacent SQL.

## Backup, restore and rollback

The scripts use production Compose and `.env/.env`, regardless of Make's
`ENV_FILE`. Run from the repository root:

```sh
./scripts/backup.sh /secure/path/backup.dump
./scripts/restore.sh /secure/path/backup.dump
```

[Backup](../scripts/backup.sh) writes a PostgreSQL custom-format archive without
encryption. Protect it and retain the application encryption key separately;
without that key, restored channel/OIDC settings are unreadable.
[Restore](../scripts/restore.sh) requires `RESTORE` confirmation, validates the
archive, stops application services and replaces archive objects in one database
transaction, then migrates/restarts. Restore failure leaves services stopped;
a later migration/startup failure does not undo a committed restore. Restored
queues may duplicate notifications. Verify readiness, login and secret decryption.

There is no automatic schema rollback. Application-only rollback needs a revision
compatible with the current database; otherwise restore matching data/configuration.
**Do not use `make reset` for rollback:** its script deletes containers, images
and volumes across the Docker host, including other projects.

## CI and release process

[CI](../.github/workflows/ci.yml) runs on pushes/PRs: backend formatting, locked
check/Clippy/release build, catalog validation and PostgreSQL migration; frontend
install/typecheck/lint/format/build; dependency audits, Git-history secret scan,
Compose rendering and image builds. It does not deploy or start the full stack.
The existing `RUSTSEC-2023-0071` exception is assessed against OIDC public-key
verification; reassess it if private-key operations are introduced.

Production builds require exact version/revision labels. Release publishing,
artifact retention, backup scheduling and external monitoring are not automated.

## Optional OIDC authentication

Use [Authentik](authentik.md), [Keycloak](keycloak.md) or [Authelia](authelia.md)
for provider-specific steps. Configure HTTPS for the public application origin
and provider endpoints, including localhost, with trusted certificates and secure
cookies. Sign in locally, open Profile → Single sign-on, save the provider
configuration with the current password, then link and confirm the identity.

Use a confidential Authorization Code client with S256 PKCE, RS256 ID tokens and
`openid`. Register the exact displayed callback and optional back-channel logout
URLs. Settings support discovery overrides, endpoint-origin allowlisting and
private CA trust; the issuer must match metadata exactly. Settings are encrypted
in the database and take effect without restart. There are no active
`STATUSDECK_AUTH_MODE` or `STATUSDECK_OIDC_*` environment settings.

Local login remains available for recovery. Disabling SSO revokes OIDC sessions;
changing issuer/client ID requires relinking. Discovery/signing keys expire after
one minute and failed refreshes reject OIDC validation instead of trusting expired
keys. Back-channel logout has independent bounded concurrency and is excluded
from browser login quotas; external ingress must preserve that separation and
providers must retry temporary delivery failures.

## Maintainer decisions

The repository does not establish deployed legacy-schema variants, a supported
upgrade conversion, release/artifact retention policy, backup frequency/recovery
objectives, external alert ownership or a browser support matrix. Maintainers
must supply these operational requirements; the supplied files do not imply them.
