# Before/after test of the storage work (docs/adr/0002)

Two isolated stacks side by side, fed the same data, then measured in alternation. The results are in
`IMPLEMENTATION_PLAN.md` ("Final test: before/after"). Local only: fictional orgs and keys, a test user with a
local-only password, ports on loopback.

| | old (e.g. 83d6fe6, release 0.3.2) | new (this branch) |
|---|---|---|
| ClickHouse 26.8, 2 vCPU / 6 GiB, chart tuning and ingest profile | `ocfin-old-ch` :18201 | `ocfin-new-ch` :18301 |
| Collector 0.163.2 with that version's `deploy/otel/collector.yaml` | :18203 | :18303 |
| Vector (that version's `deploy/vector`) + NATS | :18204 / :18205 | :18304 / :18305 |
| Release `oc-intake` / `observecraft-server` | :18210 / :18220 (indexer :18221) | :18310 / :18320 (indexer :18321) |
| Postgres (the existing `ocv-pg` container, :25433) | `oc_final_old` | `oc_final_new` |
| Valkey | — | `ocfin-valkey` :16393 |

```bash
git worktree add ../oc-old <old commit>                 # build both: cargo build --release (server, intake; tracegen in new)
docker exec ocv-pg psql -U oc -c 'CREATE DATABASE oc_final_old' -c 'CREATE DATABASE oc_final_new'
scripts/final-test/stack.sh old containers; scripts/final-test/stack.sh new containers
docker run -d --name ocfin-valkey -p 16393:6379 valkey/valkey:8.1.11-alpine
scripts/final-test/stack.sh old server; scripts/final-test/stack.sh new server     # migrations
docker exec -i ocv-pg psql -U oc -d oc_final_old -v orgs=4 -f - < scripts/load-orgs.sql   # = make load-orgs (and _new)
# load-orgs.sql makes no log index and no user: add the catch-all index `main` (what org provisioning does) and a
# test user in load-org-1, then restart the servers, which install the integration dashboards for orgs with users.
scripts/final-test/stack.sh old intake; scripts/final-test/stack.sh new intake
scripts/final-test/feed.sh old load;   scripts/final-test/feed.sh new load     # same seed and settings
TRACES_PER_SEC=200 scripts/final-test/feed.sh old traces   # and new: oc-tracegen's traces and logs
scripts/final-test/feed.sh old k8s;    scripts/final-test/feed.sh new k8s      # k8s-objects.mjs
# warm up, then: make storage-report on each ClickHouse
OC_CLICKHOUSE_URL=http://localhost:18201 OC_CLICKHOUSE_USER=default OC_CLICKHOUSE_PASSWORD= node scripts/storage-report.mjs --window 1h
node scripts/final-test/measure.mjs run --runs 5 && node scripts/final-test/measure.mjs summary
```

Pid files and logs are in `.ft.local/` (git-ignored); `stack.sh <v> stop server|intake`, `feed.sh <v> stop
load|traces|k8s` stop a process by its pid, `stack.sh <v> rm` removes that stack's containers, network and volumes.

## What a page load is (`measure.mjs`)

The requests the web app sends in live mode, with a browser's 6 connections per origin and a session cookie
(`POST /api/v1/auth/login`, `Origin` as the server's public URL):

- Kubernetes - Overview (`load-org-1-kubernetes-kubernetes-dashboard`): `GET /api/v1/me` (the app renders after
  it), then `GET /api/v1/dashboards/{id}` and `/api/v1/me/orgs`, then together: the metric widgets as the page's
  live stream `POST /api/v1/query/dashboard/live` (53 queries planned by the web's own `collectDashboardQueries` +
  `resolveQuery`, every variable `*`, last hour snapped to the minute; done at the first `snapshot` frame), the
  values of the first five template variables (`GET /api/v1/tags/{key}/values`, the scope variable's empty key
  included, as the page sends it), `POST /api/v1/events/search` (`source:kubernetes`) and six
  `POST /api/v1/orchestration/search` (pods, containers, deployments, daemonsets, statefulsets, cronjobs; limit
  50). Total = until the last of them answered.
- Template variable values: each of the dashboard's nine keys alone.
- Metrics Explorer: `GET /api/v1/metrics` with `POST /api/v1/query/timeseries` (`avg:system.cpu.user{*} by {host}`
  and the load profile's `avg:kubernetes.cpu.usage.total{*} by {host}`), 1 h and 24 h.
- Kubernetes Explorer, pods: `POST /api/v1/orchestration/search` (pods, sort by name, 50).
- Errors: issues search + issue facets + users search (1 day).
- Log Explorer facets: `POST /api/v1/logs/facets` (1 h).

Cold: server restarted, Valkey flushed, ClickHouse mark / index mark / uncompressed / query caches dropped, 10 s
to settle. Warm: the same load right after. ClickHouse work per load: `system.query_log` SELECTs of the server's
user started within the load.
