# Contributing to ObserveCraft

Thanks for your interest in ObserveCraft! Bug reports, feature ideas, documentation fixes and pull requests are all
welcome. This guide explains how to get a development environment running and what we look for in a change.

- [Ways to contribute](#ways-to-contribute)
- [Development setup](#development-setup)
- [Project layout](#project-layout)
- [Making a change](#making-a-change)
- [Coding guidelines](#coding-guidelines)
- [Commit messages](#commit-messages)
- [Pull requests](#pull-requests)
- [Releases](#releases)
- [Reporting security issues](#reporting-security-issues)
- [License](#license)

## Ways to contribute

- **Report a bug.** Open an issue with what you did, what you expected and what happened. Include versions, logs and
  screenshots where they help.
- **Suggest a feature.** Open an issue describing the problem you want solved before writing a large change, so we can
  agree on the approach first.
- **Improve the docs.** Typos, missing steps and unclear explanations are worth a PR on their own.
- **Pick up an issue.** Issues labelled `good first issue` or `help wanted` are a good place to start. Comment on the
  issue so others know you are working on it.

## Development setup

### Prerequisites

| Tool | Version |
|---|---|
| Docker | Docker Desktop, OrbStack, Colima or Podman with Compose v2 |
| Rust | ≥ 1.89 (`rustup update stable`) |
| Node.js | ≥ 20 |
| libxmlsec1 / libxslt | macOS: `brew install libxmlsec1 libxslt`; Debian/Ubuntu: `apt install libxmlsec1-dev libxml2-dev libxslt1-dev pkg-config` |

`libxmlsec1` is needed for SAML signature verification. On macOS, `server/.cargo/config.toml` points the build at
Homebrew's libxml2.

### Run everything locally

```bash
git clone https://github.com/observecraft/observecraft.git
cd observecraft
cp .env.example .env     # optional: every setting has a development default
(cd web && npm install)

make up        # ClickHouse, Postgres, NATS, Vector, OTel Collector, RustFS and synthetic logs
make server    # API on :8080 (applies migrations on startup)
make intake    # telemetry intake on :4318 (API-key authenticated)
make tracegen  # synthetic traces, logs, metrics and CI pipelines (through the intake)
make web       # UI on http://localhost:5173
```

The first `cargo build` compiles DuckDB (used by Archive Search) from source; it takes a few minutes once.

On first start the server logs a one-time link to set the admin password. Open it, choose a password, and sign in.

No backend handy? `make web-mock` runs the UI against in-browser mocks.

### Useful commands

| Command | What it does |
|---|---|
| `make test` | Rust tests (`cargo test`) + web typecheck (`tsc`) |
| `OC_TEST_API_URL=http://localhost:8080 OC_TEST_ADMIN_PASSWORD=… cargo test -p oc-api --test tenancy` | Organization-isolation test against a running server (instance admin account) |
| `make openapi` | Regenerate the OpenAPI document and the web's TypeScript API types |
| `make logs` / `make ps` | Tail / list the Docker stack |
| `make down` | Stop the stack, keeping data |
| `make clean-data` | Stop the stack and delete ClickHouse/Postgres volumes |
| `make build` | Production builds of the server and the web app |

## Project layout

| Path | Contents |
|---|---|
| `server/crates/oc-query` | Search syntax → ClickHouse / DuckDB `WHERE` clauses, in-memory matcher, metric query parser |
| `server/crates/oc-store` | ClickHouse + Postgres clients, migrations, retention |
| `server/crates/oc-api` | The `observecraft-server` binary: axum routes, auth & tenancy, monitor evaluator, notifier, log archiver, demo seeds |
| `server/crates/oc-intake` | The telemetry gateway: ingest-key auth, per-org rate limits, OTLP HTTP and gRPC, Datadog logs and metrics APIs, forwarding to the Collector / Vector |
| `server/crates/oc-tracegen` | Synthetic OTLP traces, logs, metrics (incl. histograms) and CI pipeline data |
| `server/migrations/` | ClickHouse (telemetry) and Postgres (metadata) schemas |
| `web/src/api/` | The only data-access layer of the UI: typed clients + in-browser mocks |
| `web/src/pages/` | One folder per product area (logs, apm, metrics, monitors, …) |
| `deploy/` | Docker Compose stack, Helm charts, Vector and OpenTelemetry Collector configuration |

## Making a change

1. Fork the repository and create a branch from `main`: `git checkout -b feat/span-point-plot`.
2. Make your change, with tests where it makes sense.
3. Run `make test` and make sure it passes.
4. If you changed an API endpoint, run `make openapi` and commit the regenerated files under `web/src/api/generated/`.
5. If you changed the UI, check it in the browser in both live (`make web`) and, where relevant, mock mode
   (`make web-mock`).
6. Open a pull request.

### Database migrations

- Add a new numbered file under `server/migrations/postgres/` or `server/migrations/clickhouse/`. Never edit a migration
  that has already been merged; write a new one instead.
- Every Postgres table carries `org_id`, and every query must filter on it.
- ClickHouse telemetry is per org too: `oc.logs` filters on `org_id = ?`, the `otel.*` tables on
  `OrgId = {oc_org:String}` through the client from `AppState::ch(org)`. New queries on those tables must do the same;
  the `telemetry_queries_filter_by_org` test fails on a query that names a telemetry table without an org filter.
- ClickHouse tables must work on a single server and on a cluster. Write them with the placeholders documented at the
  top of `server/migrations/clickhouse/0001_schema.sql`:
  - `${ON_CLUSTER}`, `${LOCAL}` and `${REPLICATED}` on every table and materialized view;
  - a `${DISTRIBUTED db.table sharding_key}` statement after each table;
  - a sort key that starts with the org (`org_id` / `OrgId`);
  - partitions by day, plus by retention when rows carry their own.
- DDL and mutations from code target the table that holds the rows: use `AppState::tables.local(table)` and
  `.on_cluster()`. Reads and inserts use the plain names.
- `cargo test -p oc-store` checks the rendered schema in both modes. To run it on a real cluster, start one with Keeper
  (a cluster named `oc` with two shards), then run
  `OC_TEST_CLICKHOUSE_CLUSTER_URL=http://localhost:18123 cargo test -p oc-store --test cluster`.

### Helm charts and the agent

- `deploy/helm/observecraft` (the platform) and `deploy/helm/observecraft-agent`. The platform chart's Collector and
  Vector configs are symlinks to `deploy/otel` and `deploy/vector`: Compose and Helm share one copy, and endpoints come
  from environment variables. Keep compose-only parts (like `otel/hostmetrics.yaml`) out of those files.
- New values go in `values.yaml` with a comment and in `values.schema.json`.
- Check a change with `helm lint` (also with `values-ha.yaml`) and `helm template`, and install it in a local cluster
  (kind): build the image (`docker build -f server/Dockerfile -t observecraft:dev .`), `kind load docker-image` it,
  install with
  `-f ci/default-values.yaml --set image.repository=observecraft --set image.tag=dev`, then `helm test`.
- `observecraft-agent` runs the official `otel/opentelemetry-collector-contrib` image: keep its tag on the collector
  release the platform chart uses. We don't build our own agent or SDK; senders are the Datadog Agent and libraries,
  or OpenTelemetry.

### API changes

- Handlers are documented with `utoipa`; the OpenAPI document and web types are generated from them.
- `cargo test` fails on duplicate OpenAPI operation ids or schema type names across route modules. Prefix handler
  names that would collide (e.g. `ci_overview`, `span_facets`), and add new route modules to the guard test's
  `include_str!` list in `server/crates/oc-api/src/routes/mod.rs`.
- Organization and user identity come only from the session or application key, never from request input.
- Bind every user-supplied value as a query parameter; never interpolate it into SQL.

## Coding guidelines

### Rust

- `cargo fmt` before committing; `cargo clippy --all-targets` should not add new warnings.
- Prefer returning `Result` with the crate's error types over `unwrap()` outside tests.
- Keep handlers thin: parsing and validation in the route, query building in `oc-query`, storage in `oc-store`.

### TypeScript / React

- Format with Prettier: `npx prettier --single-quote --print-width 200 --write <files>`.
- `npx tsc -p .` must pass with no errors.
- Fetch data only through `web/src/api/*/client.ts` and TanStack Query hooks; components never call `fetch` directly.
- Follow the patterns of the area you are working in: reuse existing components and CSS classes before adding new ones.

### Demo and test data

Seeds, generators, mocks, fixtures and screenshots must use **fictional data only**: the e-commerce demo services
(`api-gateway`, `storefront-api`, `order-service`, `billing`, …), `example-org/*` repositories, `i-0a1b2c3d4e5f…` hosts
and `@example.com` people. Never include real company, customer, service or person names, real hostnames or IPs,
or credentials.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(apm): add point plot visualization to the span explorer
fix(monitors): keep downtime windows in the org's timezone
docs: document OIDC setup with Keycloak
```

Common types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`. Keep the subject under 72
characters and explain the *why* in the body when it isn't obvious.

## Pull requests

- Keep PRs focused: one feature or fix per PR is much easier to review.
- Describe what changed and why, and link the issue it addresses (`Closes #123`).
- Include screenshots or a short recording for UI changes.
- Note any migration, configuration or breaking API change explicitly.
- A maintainer will review your PR. Please respond to feedback with new commits rather than force-pushes while the
  review is in progress; we squash on merge.
- The PR title becomes the commit on `main`, so it must be a Conventional Commit (a check enforces it). Releases are
  generated from those titles.

## Releases

Releases are automated with [release-please](https://github.com/googleapis/release-please). After each merge to `main`
it updates a release pull request with the next version (from the `feat`/`fix` titles since the last release) and the
`CHANGELOG.md` entry. Merging that pull request tags `vX.Y.Z`, publishes the multi-arch image and both Helm charts to
GHCR, signs them, and installs the published chart in a kind cluster as a final check
(`.github/workflows/release.yml`). Every merge to `main` also publishes `:main` and `:sha-<commit>` images.

Before 1.0, a breaking change bumps the minor version (`0.1.x` → `0.2.0`). Mark it with `!` (`feat(helm)!: …`) and
describe the upgrade in the PR.

## Reporting security issues

Please **do not** open a public issue for security vulnerabilities. Use GitHub's
[private vulnerability reporting](https://github.com/observecraft/observecraft/security/advisories/new) instead. We
will acknowledge your report and keep you updated as we work on a fix.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
