# Deployment and operations

StatusDeck runs as one API, one worker, PostgreSQL and a static frontend served by
Nginx. Docker Compose manages the supplied stack; production TLS termination and
backups are operator responsibilities. See [architecture](architecture.md) for
system boundaries and [backend](backend.md) for processing behavior.

## Prerequisites and environments

| Environment   | Requirements                                                                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Containers    | Docker Engine and Compose with environment-backed secrets and health dependencies; Make/Bash for helper commands                                              |
| Host backend  | Rust toolchain from [rust-toolchain.toml](../backend/rust-toolchain.toml), PostgreSQL with `pgcrypto` available and privileges to create the extension/schema |
| Host frontend | Node/npm; CI and container builds use Node 24 and `npm ci`                                                                                                    |
| Production    | HTTPS ingress, DNS, public origin and matching trusted-host configuration                                                                                     |

Examples run from the repository root unless stated otherwise.
Compose uses [shared service definitions](../compose_base.yml) followed by either
[development](../compose_dev.yml) or [production](../compose_prod.yml) overrides.
Always pass the base file first, as shown below. Both build static frontend
assets; use host development for Vite hot reload.

Production requires version/revision metadata and an explicit public origin.
The example sets secure cookies to false for HTTP: set
`STATUSDECK_SESSION_COOKIE_SECURE=true` for production HTTPS. Development Compose
does not forward that setting, so use production Compose for the HTTPS path.
Makefile Compose targets combine the base and development files.
Neither environment mounts application source for hot reload. Both build the
same application code with the same service/resource topology; production mainly
requires explicit origin/host/build metadata and exposes the secure-cookie setting.

## Configuration reference

[config.rs](../backend/src/config.rs) reads the process environment; it does not
load dotenv files. Compose reads `.env/.env` through `--env-file` and turns secret
values into `/run/secrets/...` files. Keep real values out of Git and logs.
[.env/.env.example](../.env/.env.example) is a template, not runnable credentials.

For the five names marked “file supported,” the corresponding exact
`<NAME>_FILE` variable takes precedence over the direct value. The supplied
Compose files wire secret files for database URL, session key,
encryption key and administrator password; administrator email is passed directly.

| Variable                                        | Backend default / requirement            | Meaning and constraints                                                                                           |
| ----------------------------------------------- | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `STATUSDECK_DATABASE_URL`                       | Required; file supported                 | SQLx PostgreSQL URL. `migrate` and `healthcheck-worker` need only this setting                                    |
| `STATUSDECK_SECRET_KEY`                         | Required; file supported                 | At least 32 bytes; HMAC key for stored session-token hashes. Rotation invalidates lookup of existing sessions     |
| `STATUSDECK_ENCRYPTION_KEY`                     | Required; file supported                 | Base64 decoding to 32 bytes, or a raw 32-byte value; encrypts notification channel secrets. Preserve for restores |
| `STATUSDECK_ADMIN_EMAIL`                        | Required for API/worker; file supported  | Normalized email; 3–254 bytes/basic format validation; bootstrap identity, not a startup reset                    |
| `STATUSDECK_ADMIN_PASSWORD`                     | Optional after bootstrap; file supported | Required to create first administrator; when supplied must be 12–1,024 characters                                 |
| `STATUSDECK_BASE_URL`                           | `http://localhost`                       | HTTP(S) origin only, no credentials/path/query/fragment; origin validation and outbound application links         |
| `STATUSDECK_BIND`                               | `127.0.0.1:8080`                         | Socket address; Compose fixes it to `0.0.0.0:8080` inside API/worker containers                                   |
| `STATUSDECK_LOG_LEVEL`                          | `info`                                   | Tracing filter; invalid syntax falls back to info                                                                 |
| `STATUSDECK_LOG_FORMAT`                         | `pretty`                                 | `pretty` or `json`; Compose/example default JSON                                                                  |
| `STATUSDECK_DEFAULT_POLL_INTERVAL`              | `300` seconds                            | Positive, at most 86,400; startup reconciliation applies it to catalog sources                                    |
| `STATUSDECK_STALE_MULTIPLIER`                   | `3`                                      | Integer 1–100; effective stale threshold has a 15-minute minimum                                                  |
| `STATUSDECK_GLOBAL_POLL_CONCURRENCY`            | `10`                                     | Integer 1–100; per worker, also influences pool size                                                              |
| `STATUSDECK_PER_HOST_CONCURRENCY`               | `2`                                      | Integer 1–100, cannot exceed global concurrency; source jobs per base-URL host per worker batch, not individual HTTP requests |
| `STATUSDECK_RAW_PAYLOAD_RETENTION_DAYS`         | `7`                                      | Integer 1–3,650; applies to poll payloads, not all history                                                        |
| `STATUSDECK_TRUSTED_HOSTS`                      | Empty                                    | Comma-separated hostnames, not origins; empty disables host filtering. Compose appends localhost                  |
| `STATUSDECK_SESSION_COOKIE_SECURE`              | `false`                                  | Boolean `true`/`false`; must be true if base URL is HTTPS                                                         |
| `STATUSDECK_TRUST_PROXY`                        | `false`                                  | Trust forwarded host/client address; API Compose setting fixed true, not controlled by the env-file value         |
| `STATUSDECK_ALLOW_PRIVATE_NOTIFICATION_TARGETS` | `false`                                  | Allows otherwise blocked private destination names/IPs; does not relax HTTPS                                      |

The file-variable names are
`STATUSDECK_DATABASE_URL_FILE`, `STATUSDECK_SECRET_KEY_FILE`,
`STATUSDECK_ENCRYPTION_KEY_FILE`, `STATUSDECK_ADMIN_EMAIL_FILE`, and
`STATUSDECK_ADMIN_PASSWORD_FILE`.

| Compose/build variable                                   | Purpose                                                                                                                                           |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `STATUSDECK_DATABASE_PASSWORD`                           | Environment-backed `postgres_password` secret; must match password encoded in database URL                                                        |
| `STATUSDECK_PORT`                                        | Published Nginx host port; default 80, example 8080                                                                                               |
| `STATUSDECK_VERSION`, `STATUSDECK_REVISION`              | Docker build arguments used in OCI image labels; set exact source revision for releases. They do not replace Cargo's compiled application version |
| `ENV_FILE`                                               | Makefile override for `.env/.env`; backup/restore scripts do not honor this override                                                              |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD_FILE` | Container configuration fixed to `statusdeck`, `statusdeck`, `/run/secrets/postgres_password`                                                     |

There are no frontend `VITE_*` settings, SMTP credentials, OAuth secrets or
provider API keys in this deployment. Notification URLs/signing secrets are
configured after login and stored encrypted in the database. Changing
`STATUSDECK_DATABASE_PASSWORD` does not rotate a password inside an already
initialized PostgreSQL volume; plan database credential changes separately.

## Container setup and initialization

Copy the template without replacing existing settings:

```sh
cp -n .env/.env.example .env/.env
```

Replace all placeholders, supply independent session/encryption keys, and ensure
the database password matches the URL. URL-encode special characters in URL
credentials. The `postgres` hostname is internal to Compose.

For local HTTP at the example address `http://localhost:8080`:

```sh
make dev
# Or run detached:
make docker
```

For production, configure HTTPS ingress, public origin, trusted hosts, secure
cookies and release metadata, then run:

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml config --quiet
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml up -d --build --wait
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml ps
```

Only Nginx is published. PostgreSQL persists in the named `postgres18-data`
volume. Startup waits for PostgreSQL and successful migrations before the API
and worker; readiness also depends on worker heartbeats. Sign in and subscribe
to providers to begin collection.

The volume's actual Docker name includes the Compose project prefix. PostgreSQL
18 mounts it at `/var/lib/postgresql`. The one-shot migrator applies embedded
SQLx migrations, including `pgcrypto`; API and worker independently bootstrap the
administrator under an advisory lock and reconcile the catalog. Repeated startup
preserves account credentials and subscriptions. Keep the same Compose project
identity when switching files or upgrading to avoid selecting a different volume.

| Helper                             | Effect                                                                                            |
| ---------------------------------- | ------------------------------------------------------------------------------------------------- |
| `make infra-up`, `make infra-down` | Start/wait for PostgreSQL, or stop/remove the development stack without deleting its named volume |
| `make migrate`                     | Build/run the one-shot migrator using development Compose                                         |
| `make api`, `make worker`          | Start the selected development service and dependencies                                           |
| `make frontend`                    | Start built Nginx/frontend, worker and dependencies; not the Vite dev server                      |

All helpers use [Makefile](../Makefile); `ENV_FILE` selects a different env-file.
The [catalog helper](../scripts/validate-catalog.sh) runs Cargo from the repository
root, so it uses the shell's selected Rust toolchain; the explicit-toolchain
commands below avoid that ambiguity.

### Upgrades

The [initial migration](../backend/migrations/0001_initial.sql) creates the complete
schema, including resend accounting. Run `migrate` before starting the API and
worker; those processes check migration state but do not apply migrations.

Only `0001_initial.sql` is supplied; it includes Zulip delivery channels.
SQLx's migrator checks applied migration
checksums; changing an already-applied file is not an upgrade path. In contrast,
[startup/readiness](../backend/src/db/mod.rs) checks only the maximum recorded
version and success flags, not checksums or actual table definitions. A ready
API is therefore not evidence that a database created by a different variant of
`0001` is compatible. A database with a different applied checksum or additional
migration versions needs explicit schema/migration-history reconciliation.
No conversion or down migration is provided. The available repository does not
establish which legacy schema variants operators may have deployed; maintainers
must supply that inventory before defining an upgrade path.

For a database compatible with the checked-in migration history:

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml build
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml stop backend worker
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml run --rm migrate
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml up -d --wait
```

Use `compose_dev.yml` instead for the development stack. Back up before upgrading
and retain the matching encryption key and prior application version.

## Local development and checks

Use a host-accessible PostgreSQL database; Compose does not publish its database
port. Export backend settings yourself with a host database URL,
`STATUSDECK_BASE_URL=http://localhost:5173`, a matching trusted host and secure
cookies disabled for HTTP. The backend does not load dotenv files automatically.
For `api` and `worker`, required exports are `STATUSDECK_DATABASE_URL`,
`STATUSDECK_SECRET_KEY`, `STATUSDECK_ENCRYPTION_KEY` and `STATUSDECK_ADMIN_EMAIL`,
plus `STATUSDECK_ADMIN_PASSWORD` for the first account. Use the configuration
table for valid values. An env-file intended for Compose is not automatically
a host-process environment, and its internal `postgres` hostname usually cannot
be used by a host process.

```sh
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- migrate
npm ci --prefix frontend

# Run each in a separate terminal:
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- api
cargo +1.97.0 run --manifest-path backend/Cargo.toml --locked -- worker
npm run dev --prefix frontend
```

Vite serves port 5173 and proxies API/health requests to port 8080. Rust checks
should run from `backend/` to use the pinned toolchain; there is no root Cargo
workspace.

Keep the browser origin equal to `STATUSDECK_BASE_URL`: Vite can choose another
port when 5173 is occupied, but mutation Origin validation still uses the
configured value. Vite does not apply the production Nginx CSP, login rate limit
or health-route restrictions, so host development does not verify those controls.

```sh
# Working directory: backend/
cargo fmt -- --check
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
cargo run --locked -- validate-catalog
```

```sh
# Working directory: frontend/
npm run check
npm run lint
npm run format:check
npm run build
```

Frontend build includes TypeScript checks. Catalog validation needs no database
or live provider access. There are no Rust or frontend test suites.
Passing build/static checks does not establish runtime
coverage.

## Images, ingress and resource limits

The [Dockerfiles](../deploy/) build the backend and frontend from pinned base
images. Compose defines resource limits, container restrictions and log rotation;
these are configured budgets rather than measured capacity recommendations.
Review the Compose files when changing deployment capacity.

| Container             | CPU / memory limit | Runtime boundary                                           |
| --------------------- | ------------------ | ---------------------------------------------------------- |
| PostgreSQL            | 2 / 1 GiB          | Persistent named volume, internal port only                |
| Migrator              | 1 / 512 MiB        | One-shot backend image, database URL secret only           |
| API and worker (each) | 1 / 512 MiB        | Backend image runs as UID 10001; separate commands/pools   |
| Frontend/Nginx        | 0.5 / 128 MiB      | UID 101, read-only root filesystem and 64 MiB `/tmp` tmpfs |

Application containers drop capabilities and disable privilege escalation.
Compose's local log driver rotates three 10 MiB files per service. Backend builds
use the locked Rust dependencies and a Debian runtime; frontend builds use Node
24, `npm ci` and an unprivileged Nginx runtime. The supplied topology has no
Kubernetes manifests, managed database provisioning or registry image references
for the application: Compose builds it from the local checkout.

[Nginx](../deploy/nginx.conf) serves the application, proxies API/health routes
and applies request limits and browser security headers. External ingress must
preserve the intended host/client-address boundary. No real-IP configuration is
supplied for another proxy in front of Nginx, so rate limiting may otherwise see
the ingress as one client. Keep the API private to the intended proxy.
Nginx resolves the backend through Docker DNS, uses SPA fallback for browser
routes, and applies CSP, frame denial, MIME-sniffing protection and referrer
policy. It does not configure TLS certificates or HSTS. Its login limiter is
five requests/minute per observed remote address with burst five, independent of
the backend's in-memory limit.

## Health, logging and troubleshooting

| Check                           | Meaning                                                            |
| ------------------------------- | ------------------------------------------------------------------ |
| `GET /health/live`              | API process responds                                               |
| `GET /health/ready`             | Database, recorded migration state and both worker heartbeats pass |
| `statusdeck healthcheck-worker` | Poller and dispatcher heartbeats are recent                        |
| System page                     | Feed freshness, worker/database evidence and delivery outcomes     |

The public Nginx endpoint exposes `/health/live` only. Docker calls
`/health/ready` directly on the backend's internal network so unauthenticated
internet traffic cannot amplify its database checks.

API readiness requires the worker even with no subscriptions. A Compose unhealthy
status alone does not restart a process; restart policies act on process exit.

```sh
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml logs --tail=100 backend worker
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml exec -T backend wget -qO- http://localhost:8080/health/ready
docker compose --env-file .env/.env -f compose_base.yml -f compose_prod.yml exec -T worker statusdeck healthcheck-worker
```

Migration/startup failures appear in service logs. Source diagnostics distinguish
provider HTTP, transport and parsing failures. Queued deliveries need an enabled
channel/rule and a working dispatcher; held deliveries await quiet-hours release.
Ambiguous sends may already have reached the destination. Eligible completed
records can be explicitly resent from delivery diagnostics.

Structured logs include request and worker identities; business changes also
produce audit records. Source-stale alerts depend on the worker, so they cannot
independently detect a completely stopped worker. External monitoring and log
collection are operator responsibilities.
There is no Prometheus scrape endpoint, distributed tracing exporter or bundled
monitoring/logging service. Readiness checks have a two-second overall deadline
and accept worker heartbeats newer than two minutes; inspect actual progress in
System when a heartbeat alone is insufficient.

### Post-deployment verification

Check public health URLs, then sign in and verify current worker heartbeats,
source collection and retained incident history. If testing notifications, a
queued response only confirms acceptance; inspect the later delivery outcome.
Channel tests send real messages to the configured destination.

## Backup, restore and rollback

Run the scripts from the repository root. They use production Compose and
`.env/.env`:

```sh
./scripts/backup.sh /secure/path/backup.dump
./scripts/restore.sh /secure/path/backup.dump
```

Backups use PostgreSQL custom format and are not encrypted by the script. Keep
the destination protected and retain the channel encryption key separately.
Changing that key without migrating ciphertext makes stored channels unreadable.

Restore requires typing `RESTORE`. It validates the archive, stops application
services, replaces archive objects in one database transaction, then migrates and
restarts. Database restore failure rolls back the replacement and leaves services
stopped. Later migration/restart failure does not undo a committed restore.
Restored queues can produce duplicate downstream notifications.

Verify readiness, login, channel decryption and provider state after recovery.
There is no automated backup schedule, restore drill or schema rollback. An
application-only rollback requires a prior artifact compatible with the database;
otherwise restore the matching database and configuration.

**Do not use `make reset` for rollback.** Its script removes all containers,
images and volumes on the Docker host, including unrelated projects.

## CI and release process

[CI](../.github/workflows/ci.yml) builds and lints both applications, validates
the catalog and migrations, audits dependencies, scans the full Git history for
secrets, and builds container images. The secret scan uses a versioned,
checksum-verified Gitleaks binary and redacts detected values from logs. Runtime
smoke/browser tests and production deployment are separate work. Release
publishing, artifact retention and recovery schedules are not automated. The
RustSec exception for `RUSTSEC-2023-0071` covers `rsa` in the lockfile through
SQLx's optional MySQL driver. This PostgreSQL-only build does not enable MySQL
or compile `rsa`; `cargo tree --all-features --target all -i rsa` has no entries.
Revisit the exception if database features change.

The workflow runs on pushes and pull requests. Backend gates are formatting,
locked check/Clippy/release build, offline catalog validation, migration against
PostgreSQL 18 and a RustSec audit. Frontend gates are `npm ci`, typecheck, lint,
format check, build and npm audit at high severity. Container gates render both
Compose files using the example and build both Dockerfiles; they do not start
the stack.

Use an exact source revision and preserve the matching database backup/configuration
for a release. OCI labels record requested version/revision, but application logs
and worker heartbeat versions come from Cargo. Maintainers still need to define
release tags/artifact retention, a legacy-schema upgrade path, external health
alerts, backup frequency and restore-time/data-loss objectives.

## System diagnostics

Diagnostics separates service outages from problems collecting or delivering
StatusDeck data. Recent statistics use a selected time window; queue counts and
current worker/database state describe the present snapshot. Delivery history
shows recent events independently of the summary window.

| Signal           | Interpretation                                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Freshness        | Failed or old snapshots are stale; missing evidence is unknown, not healthy                                                      |
| Provider feeds   | Delayed, failing and never-checked feeds differ from provider outages; no subscriptions is unconfigured                          |
| Polling backlog  | Scheduled work overdue beyond its polling interval, excluding active leases; retry backoff is already part of the schedule       |
| Worker health    | Heartbeats establish liveness; completed work and attempts establish progress. Idle cycles can be healthy                        |
| Database         | Size/activity are current samples; query timing includes API pool wait and a round trip, not whole-application performance       |
| Delivery backlog | Eligible work past its attempt/release time; active leases and paused configuration are accounted for separately                 |
| Delivery health  | Recent terminal failures/ambiguous outcomes and overdue eligible work; older failures remain visible in history                  |
| Delivery latency | Queue-to-confirmation time including retries; deliveries with `quiet_until` are excluded from percentile samples and counted separately, rather than having their hold time subtracted |

Counts and lists can differ briefly as work progresses. A successful delivery
means the destination accepted the HTTP request. Empty samples and uncertain
outcomes should not be interpreted as zero failures or proven availability.
Exact queries and thresholds live in
[system summary](../backend/src/api/system_summary.rs) and its adjacent SQL files.
