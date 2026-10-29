# Datadog API contract fixtures

Recorded scenarios of Datadog's management API, replayed against ObserveCraft's Datadog-compatible routes
(`src/routes/datadog_api/`, see `docs/adr/0003-datadog-compatible-management-api.md`).

## Source

- Repository: [`DataDog/datadog-api-client-go`](https://github.com/DataDog/datadog-api-client-go), Apache License 2.0
  (`LICENSE` and `NOTICE` in this directory are that repository's).
- Commit: `1a5c98969c6a269710149bfa7ca7e39320a1e644` (2026-10-29).
- Paths, kept as they are there:
  - `tests/scenarios/features/v1/{authentication,monitors,downtimes,dashboards,dashboard_lists,service_level_objectives,service_level_objective_corrections}.feature`
    → `features/v1/`, with the request bodies those features read from files (`*.json`, `dashboards_json_payload/`);
  - `tests/scenarios/features/v2/{downtimes,dashboard_lists,users,roles,teams,key_management}.feature` → `features/v2/`;
  - `tests/scenarios/cassettes/TestScenarios/v1/Feature_{Authentication,Monitors,Downtimes,Dashboards,Dashboard_Lists,Service_Level_Objectives,Service_Level_Objective_Corrections}/`
    and `.../v2/Feature_{Downtimes,Dashboard_Lists,Users,Roles,Teams,Key_Management}/` → `cassettes/TestScenarios/`.

## Changes made here

- Left out: the shared (public) dashboard cassettes of `Feature_Dashboards` (a separate API, out of scope, and their
  recordings carry share tokens), and two cassettes no scenario refers to any more.
- Users, Roles, Teams and Key Management: only the cassettes of the scenarios we support. Left out: personal access
  tokens and scoped application keys (not supported), and in `Feature_Teams` everything but teams and memberships
  (links, hierarchy, connections, notification rules, sync, permission settings) as well as the scenarios that set
  avatars, banners or modules. Their cassettes were re-serialised by the script that replaced the people's data below
  (same content, YAML formatting changed), which also replaced the generated client ids in some application key names
  (`dd_auth_cli-…`) and two people's first names in key names by placeholders. Keys in these recordings are already
  redacted upstream (`xxxx…`).
- People: every `…@datadoghq.com` address in the cassettes is replaced by a fictional `userN@example.com`, the names
  derived from those addresses by `Example User`, and Gravatar hashes by zeros. Nothing else is edited.
- The cassettes carry no credentials: the Go client records requests without its key headers.

## Use

`src/routes/datadog_api/replay.rs` loads a feature and the cassette of each scenario, sends every recorded request
to an in-process server with test keys (`DD-API-KEY` / `DD-APPLICATION-KEY` as the scenario's background says),
maps ids from earlier answers onto later requests, and checks the status class and that every recorded response
field is there with the same JSON type. Each resource's module adds a test replaying its feature
(`validate.rs` replays `v1/authentication`).
