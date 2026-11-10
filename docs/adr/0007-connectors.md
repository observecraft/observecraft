# ADR-0007: API integrations are connector packages run by one engine

Status: accepted (2026-11-10).

## Context

Datadog collects about 76 of its catalog's integrations by calling the vendor's API from its own service (`collection`
`crawler` or `oauth` in `web/src/api/integrations/catalog.generated.json`). ObserveCraft has none of them, and its six
API integrations (authentik, NetBird, FleetDM, GitLab Audit Events, Google Workspace, Entra ID) are each a Rust module,
a route file, a Postgres table, a page, a mock and wiring in five places (`main.rs`, `routes/mod.rs`,
`integration_installs::CONNECTIONS`, `dashboards::GATED` and `enabled_integrations`, `collection.ts`). Most of each is
the same: an HTTP API polled with a token, a cursor, records turned into logs or gauges, a test button, a status.

Datadog itself ships integrations as packages (`manifest.json`, `assets/` with dashboards, log pipelines, monitors,
`metadata.csv`) in integrations-core, the community's integrations-extras and the Marketplace. We want the same for
API integrations: adding one should be a package, not code, and operators should be able to add their own
(third-party or private) packages without rebuilding ObserveCraft.

## Decision

1. **A connector is a package**: a directory `integrations/connectors/<id>/` with `connector.yaml` (the manifest,
   validated against `integrations/connectors/connector.schema.json`), `fixtures/` (recorded answers of the vendor's
   API, fictional data), and `assets/` (dashboards in Datadog's widget JSON, a log pipeline in Datadog's pipeline JSON,
   monitors and Cloud SIEM rules in Datadog's JSON) plus `dist/` (the dashboards converted by
   `scripts/connector.mjs build`). The manifest declares:
   - the configuration form (`config`: url, string, text, number, boolean, select, multiselect fields; `secret: true`
     fields are sealed at rest and never answered),
   - authentication (`auth.kind`: `none`, `api_key` (header or query), `bearer`, `basic`,
     `oauth2_client_credentials`, `oauth2_refresh_token` (rotated refresh tokens are saved), `jwt_bearer` (Google
     service accounts)),
   - streams: a request (method, URL, query, headers, JSON or form body, all templates), pagination (`none`,
     `cursor`, `next_url`, `link_header`, `offset`, `page`), where the records are, their id, the incremental strategy
     (`none`: a snapshot each run; `new_records`: only ids not seen before; `time`: a time window from the cursor with
     first-sync lookback, late-data overlap, settle delay, optional slicing into sub-windows, oldest-first or
     newest-first order with a lossless backlog; `token`: an opaque since-token), rate limits, and the output (`logs`,
     `metrics` (gauge or count), `events`),
   - the connection test (one request, a success condition and messages per error),
   - more shapes the research over the 76 tiles and popular Datadog-only integrations asked for: lookups and
     discovery requests cached across runs, `for_each` fan-out (child streams), index-then-download (`expand`),
     asynchronous exports, CSV / NDJSON / Prometheus answers, request quotas per connection and per stream,
     one-off setup calls (`on_connect`), push-only packages with their intake URL, headers, signature and an optional
     request creating the push job on the vendor's side, and packages that add their own catalog tile.
2. **Templates and expressions** are one small, documented language (`{{ expr }}` in strings; a string that is a
   single `{{ expr }}` keeps the value's type): paths (`record.a.b[0]`), literals, `?? + - * / % == != < <= > >= in
   && || ! ?:`, and filters (`| lower`, `| split('.')`, `| rfc3339`, …). It has no loops, I/O or side effects, so a
   package can't do more than its manifest says. Secrets are only readable in `auth` and `request` templates.
3. **One engine** (`server/crates/oc-connectors`, no database or Kafka code) runs a stream: requests through a
   `Transport` (the server's is SSRF-guarded: the host resolved once and checked, no redirects, capped answers,
   timeouts, `OC_PORTAL_ALLOW_PRIVATE` for internal hosts; pagination never leaves the first request's host, so
   credentials never go elsewhere), pages, records, mapping, and the next state. The same crate has the validator and
   the replay harness (the fixtures served by a fake transport), and the `oc-connector` CLI (`validate`, `test`,
   `schema`) that `scripts/connector.mjs` calls, so packages are checked by the code that runs them.
4. **The server** (`oc-api/src/connectors/`) loads the bundled packages (compiled in by `build.rs`) and the
   operator's (`OC_CONNECTORS_DIR`, one directory per package, marked third-party with their publisher). An invalid
   package is skipped with one error line naming the package and the problem; it never stops the server. A bundled id
   can't be overridden. Connections are generic: Postgres `connector_connections` (org, connector, name, config,
   sealed secrets, enabled streams) and `connector_stream_state` (cursor, token, backlog, seen ids), polled on the
   cloud crawlers' jobs (`cloud_crawl_jobs`, provider = connector id, account = connection id, scope = stream id;
   `for_each` items share the stream's job, each with its own state), records published to Kafka with the org's headers (`oc.logs.json`,
   `oc.otlp-json.metrics`, `oc.otlp-json.logs` for events) before the state moves. Generic routes
   (`/api/v1/connectors…`) list packages, save, test, sync and delete connections and show each stream's status;
   writes are admins', audited.
5. **Native connectors** implement the same `Connector` trait for what is generic (catalog entry, has-a-connection,
   connections and their jobs' status, sync now) while keeping their own setup pages and code (authentik, FleetDM,
   GitLab Audit Events, Google Workspace, Entra ID). Installed state (`integration_installs`), dashboard gating
   (`dashboards::enabled_integrations`) and the catalog's links (`collection.ts`) read the registry instead of lists.
6. **NetBird moves to a package** as the proof: migration 0102 copies `netbird_connections` into
   `connector_connections` with the same ids, so its jobs and cursors carry on; its log identity (instance + event id)
   is reproduced by the engine's identity rule, so events read again are stored once.
7. **Assets**: a connection's save installs the package's log pipeline (as `integration_installs::install_pipelines`
   does) and enables its dashboards (seeded for every org, listed once the org has a connection). Monitors and SIEM rules
   are listed on the connector page as recommended; they are created on request.

## Consequences

- Adding an API integration is a package plus fixtures, reviewed like a dashboard; no route, table or page.
- The language is deliberately small: APIs needing request signing (AWS SigV4, HMAC), multi-step logins, SOAP or
  binary formats stay native connectors behind the same trait.
- Operator packages run with the server's network access, through the same SSRF guard and only to the hosts their
  configuration names; they can't read other orgs' data (the engine only sees one connection) or the server's
  environment.
- The schema is a contract for package authors: changes are additive (new optional fields, kinds and filters);
  `schema_version` in the manifest guards future breaking changes.
- Push-only vendors' intake side reads the same packages: the intake asks the server's internal listener for the
  push definitions and an org's signing secrets (cached a minute, org from the ingest key), checks signatures and maps
  webhooks with `oc-connectors::push`, and publishes through Kafka. Long-lived streams (WebSockets, SQS) remain an
  extension point: `native` names a connector compiled into the server.
- `oauth` tiles need an OAuth app per install (client id and secret from the operator, redirect to ObserveCraft);
  until the authorization-code flow lands, their packages take a refresh token or an API key the user creates.
