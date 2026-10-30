# Datadog API contract fixtures

The contract tests of the Datadog-compatible management API (`src/routes/datadog_api/`, see
`docs/adr/0003-datadog-compatible-management-api.md`) replay Datadog's own recorded API scenarios. Those recordings
are **not in this repository**: they are downloaded at test time.

## Source

- Repository: [`DataDog/datadog-api-client-go`](https://github.com/DataDog/datadog-api-client-go), Apache License 2.0.
  Its `LICENSE` and `NOTICE` are downloaded with the recordings; ObserveCraft does not redistribute them.
- Commit: `1a5c98969c6a269710149bfa7ca7e39320a1e644`, pinned with its archive's SHA-256 in
  `scripts/datadog-fixtures/fetch.sh`.
- Features: v1 Authentication, Monitors, Downtimes, Dashboards, Dashboard Lists, Service Level Objectives and
  Service Level Objective Corrections; v2 Downtimes, Dashboard Lists, Users, Roles, Teams, Key Management and Cloud Cost Management. The
  script takes their `tests/scenarios/features/{v1,v2}/*.feature` files (with `given.json` and the request bodies
  they read from files) and `tests/scenarios/cassettes/TestScenarios/{v1,v2}/Feature_*/`, unchanged.

## Fetching and running

```bash
make datadog-fixtures   # once per pinned commit: ~/.cache/observecraft/datadog-api-client-go/<commit>
cd server && OC_TEST_POSTGRES_URL=… OC_TEST_CLICKHOUSE_URL=… cargo test -p oc-api datadog_api
```

`OC_DATADOG_FIXTURES` sets another location (the script and the tests both read it). Without the fixtures the
replay tests are skipped with a message saying how to fetch them; `OC_REQUIRE_DATADOG_FIXTURES=1` (set in CI) makes
them fail instead. Which upstream scenarios are not replayed, and why, is in `src/routes/datadog_api/replay.rs`
(`not_replayed`) and the tests of each resource.
