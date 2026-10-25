# ObserveCraft

Observability platform: logs, metrics, traces, errors, dashboards and monitors.

```
 apps / hosts ──► OTel Collector / Vector agents ──► Vector aggregator ──► ClickHouse (oc.logs, otel.*)
                                                            └──────────► NATS (oc.logs.live.<org>)
 Browser (React SPA) ◄── /api (JSON, SSE) ──► observecraft-server (Rust/axum) ──► ClickHouse · Postgres · NATS
```

| Path | What |
|---|---|
| `web/` | React + Vite + TypeScript UI. `src/api/*/client.ts` is the only data-access layer. |
| `server/` | Rust workspace: `oc-query` (search syntax → SQL / matcher), `oc-store` (ClickHouse + Postgres, migrations), `oc-api` (`observecraft-server`), `oc-tracegen` (synthetic traces). |
| `server/migrations/` | ClickHouse (telemetry) and Postgres (metadata) schemas. |
| `deploy/` | Docker Compose stack, Vector aggregator + load generator, OTel Collector config. |

## Quickstart

Requires Docker (OrbStack/Docker Desktop), Rust ≥ 1.89, Node ≥ 20, and on macOS `brew install libxmlsec1 libxslt`
(SAML signature verification; `server/.cargo/config.toml` points the build at Homebrew's libxml2).

```bash
make up        # ClickHouse, Postgres, NATS, Vector aggregator, OTel Collector, synthetic logs
make server    # API on :8080 (applies migrations on startup)
make tracegen  # synthetic OTLP traces for APM (8 services, DB/cache/queue deps, N+1s, errors, rolling deploys)
make web       # UI on :5173 in live mode → http://localhost:5173/logs/livetail, /apm
```

`docker compose -f deploy/docker-compose.yml --profile app up -d` runs the API and the trace generator in
containers instead.

On first start the server logs a one-time link to set the admin password (`alex.morgan@example.com`):
`No one can sign in yet. Set the admin password for … : http://localhost:5173/login/link/…`.

`make web-mock` runs the UI with in-browser mocks (no backend). `make test` runs Rust tests and the web typecheck.

## Contract

The API's OpenAPI document is generated from the Rust handlers (utoipa) and the web types are generated
from it: **`make openapi`** after changing an endpoint. `web/src/api/http.ts` checks paths, bodies and
responses against `web/src/api/generated/schema.d.ts` at compile time. Live spec: `GET /api/openapi.json`.

## What is live vs mocked

| Area | Backend |
|---|---|
| Logs: search, facets, histogram, suggestions, insights, live tail | **Live** (ClickHouse + NATS) |
| APM: services, endpoints, recommendations, anomalies, service page, trace explorer, trace waterfall | **Live** (ClickHouse `otel.otel_traces`; single-tenant until spans carry an org) |
| Metrics: query engine (`avg:system.cpu.user{env:prod} by {host}`), catalog, tag values | **Live** (ClickHouse `otel_metrics_*`; host-metric aliases + span-derived `trace.*` metrics) |
| Dashboards: list, lists, starring, CRUD, built-in Host / APM dashboards | **Live** (Postgres; seeds in `server/seeds/dashboards`) |
| Infrastructure: host list, host panel, metrics, processes | **Live** (host + process metrics, spans, logs). Pods/containers need k8s/docker receivers — empty for now |
| Personal & organization settings: profile, security (password, MFA, sessions), application keys, users, roles, teams, SAML group mappings, login methods, audit trail | **Live** (Postgres) |
| Authentication: password + MFA, SAML, OIDC, sessions, RBAC | **Live** |
| Incidents: list, analytics, declare, detail, timeline, impacts, follow-ups | **Live** (Postgres) |
| Error tracking, integrations | Mocked in the browser (`web/src/api/mock/*`) |

Host metrics come from the OTel Collector `hostmetrics` receiver (the Docker VM appears as `docker-dev-host`) and from
`oc-tracegen`, which reports CPU/memory/load/network/process metrics for the hosts its synthetic services run on.
With `OC_SEED_DEMO=true` (default in development) an empty org gets fictional demo users, teams, SAML mappings and
incidents; set it to `false` in production.

## Authentication

- **Password** sign-in (Argon2id) with optional **authenticator-app MFA** (TOTP; seeds encrypted with `OC_SECRET_KEY`),
  account lockout after 10 failures (15 min), and one-time **invite / reset links** (Organization Settings → Users).
- **SSO**, configured per org in Organization Settings → **Login Methods**:
  - **SAML 2.0** (Entra Enterprise app, Okta, ADFS, …): SP-initiated, signed responses required (SHA-256+), InResponseTo
    checked, assertion replay blocked. SP metadata: `/api/v1/auth/saml/<org>/metadata`.
  - **OpenID Connect** (Entra app registration, Google, Okta, Keycloak, …): authorization code + PKCE, nonce, ID-token
    verification via discovery/JWKS.
  - Users are created on first SSO sign-in (optional); **SAML Group Mappings** turn IdP groups into team memberships and
    roles (role mappings replace roles at each sign-in). "Require SSO" keeps password access for admins as break-glass.
- **Sessions**: server-side (hashed token in an HttpOnly, SameSite=Lax cookie), sliding idle timeout, 7-day maximum;
  "Log out of all sessions" and per-session revoke. Cookie-authenticated writes must come from an allowed Origin.
- **Application keys** (Personal Settings → Application Keys): `Authorization: Bearer ocak_…` for scripts and Terraform.
- **Roles**: Admin (org settings, users, SSO, audit), Standard (create/edit), Read Only — enforced by the API.
- **Audit trail** (Organization Settings → Audit Trail): sign-ins and failures, MFA/key changes, user and SSO admin.

Test SSO locally: `docker compose -f deploy/docker-compose.yml --profile sso up -d` starts a SAML IdP (SimpleSAMLphp on
:8081, users `user1`/`user1pass`; metadata URL `http://localhost:8081/simplesaml/saml2/idp/metadata.php`) and an OIDC
provider (issuer `http://localhost:8082/default`, any client id/secret). Configure them under Login Methods.

## Notes

- **Tenancy & identity:** every table carries `org_id`; the org and user of a request come only from its session or
  application key (`server/crates/oc-api/src/auth/`), never from client input.
- **API contract guards:** `cargo test` fails on duplicate OpenAPI operation ids or schema type names across route
  modules (either would silently merge or replace generated web types).
- **Security of queries:** all search values *and* tag keys are bound parameters (`oc-query::build_where`), never
  interpolated into SQL.
- **Retention:** `oc.logs` keeps 15 days (TTL); OTel tables 72h. Archive-to-S3 (Parquet) + DuckDB search is planned.
- **Live tail** is best-effort: the NATS sink drops when full so it never back-pressures storage; ClickHouse
  writes use a disk buffer.
