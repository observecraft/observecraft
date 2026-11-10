# Connector packages

A **connector** collects an integration's data by calling the vendor's API (what Datadog does "from its own service"
for its crawler and OAuth tiles). In ObserveCraft a connector is a **package**: a directory with a `connector.yaml`
manifest, recorded answers of the vendor's API and optional assets. The server's engine (`server/crates/oc-connectors`)
runs every package the same way: it polls the API on a schedule, keeps a cursor so nothing is lost or sent twice, maps
the records to logs, metrics, events or costs, and publishes them through Kafka like the intake does. Adding an
integration is writing a package, not code. The design is [ADR-0007](adr/0007-connectors.md).

- Bundled packages live in `integrations/connectors/<id>/` and are compiled into the server.
- Operators add their own (community, third-party, private) in `OC_CONNECTORS_DIR` (see [Third-party
  packages](#third-party-packages)).
- The schema is `integrations/connectors/connector.schema.json` (editors complete and check `connector.yaml` with
  it: start the file with `# yaml-language-server: $schema=../connector.schema.json`).
- Two complete examples: [`netbird`](../integrations/connectors/netbird/connector.yaml) (time cursor over a single
  list, logs with standard attributes, gauges, its own dashboard and pipeline) and
  [`have-i-been-pwned`](../integrations/connectors/have-i-been-pwned/connector.yaml) (discovery, `for_each`, lookups,
  `new_records`, the tile's integrations-core pipeline and dashboard).

Contents: [Layout](#layout) · [Writing one](#writing-a-package) · [Manifest](#manifest) · [Config](#config) ·
[Templates](#templates-and-expressions) · [Auth](#auth) · [Requests](#requests) · [Pagination](#pagination) ·
[Incremental reading](#incremental-reading) · [Records](#records) · [Output](#output) · [Errors, rate limits,
quotas](#errors-rate-limits-and-quotas) · [Test](#the-connection-test) · [Assets](#assets) · [Push-only
vendors](#push-only-vendors-and-webhooks) · [Testing](#testing-a-package) · [Recipes](#recipes) · [Third-party
packages](#third-party-packages) · [Licensing](#licensing) · [Checklist](#checklist)

## Layout

```
integrations/connectors/<id>/
  connector.yaml                 the manifest
  fixtures/*.json                recorded answers (fictional data) and golden outputs
  assets/dashboards/*.json       dashboards in Datadog's JSON (optional)
  assets/logs/pipeline.json      a log pipeline in Datadog's pipeline JSON (optional)
  assets/monitors/*.json         recommended monitors in Datadog's monitor JSON (optional)
  assets/security_rules/*.json   Cloud SIEM detection rules in Datadog's rule JSON (optional)
  dist/dashboards.json           the dashboards converted by `node scripts/connector.mjs build` (committed)
```

Only text files are read. Packages ship no images: dashboards' banner images become the package's logo tile, and
logos come from the catalog (`logo`, see the manifest).

## Writing a package

1. `node scripts/connector.mjs new <id>` writes a skeleton in `integrations/connectors/<id>/`. Use the catalog tile's
   id (`web/src/api/integrations/catalog.generated.json`) as the package id and in `tiles`.
2. Read the vendor's API documentation and, where Datadog has a tile, `integrations-core`'s
   `<integration>/assets/logs/<source>_tests.yaml` and dashboards: they show the log shape Datadog sends
   (`source`, `service`, the attributes the pipeline and dashboard read). Reproduce that shape so the tile's
   integrations-core pipeline and dashboard work unchanged.
3. Fill in `config`, `auth`, the `streams` and the `test`.
4. Record the API's answers into `fixtures/` with fictional data (`example.com`, the e-commerce demo names, AWS account
   `123456789012`), list them under `mock.responses`, and state what each stream must give under `mock.expect`.
5. `node scripts/connector.mjs test integrations/connectors/<id>` until it passes; `OC_CONNECTOR_UPDATE=1` writes the
   golden files, which you then read.
6. `node scripts/connector.mjs build` (dashboards and the web's mock catalog), then `cargo test -p oc-api connectors`
   (every bundled package is loaded and replayed).

## Manifest

| Field | Required | Meaning |
|---|---|---|
| `schema_version` | yes | `1`. |
| `id` | yes | Lowercase letters, digits, `-`, `_`, `.`. The tile's id when it implements one. Also the jobs' provider: never change it once published. |
| `name`, `publisher`, `vendor`, `description` | yes | Shown in the catalog. `publisher` is `ObserveCraft` for bundled packages. |
| `version` | | The package's semver (default `1.0.0`). |
| `tiles` | | Catalog tile ids it makes available ("Not available" tiles become available and link to the connector page). A vendor without a tile (no Datadog integration, or a third-party package): leave `tiles` empty and the package adds its own tile with its `id`, `name`, `categories`, `description` and `logo`. |
| `logo` | | Logo name (`web/src/api/integrations/integrationLogos.generated.ts`); default the first tile. Dashboards are gated by it. |
| `categories` | | Catalog categories. |
| `source` | | Logs' `ddsource`; default `id`. Use the tile's `logSource` when Datadog has one. |
| `docs_url`, `setup`, `links` | | The vendor's API docs; Markdown setup steps; links (`url` may hold `{{ config.x }}`). |
| `instance` | | Template over `config` naming the connected account (`{{ config.base_url \| host }}`): logs' `host`, `connection.instance`, part of every log's identity. Default the id. |
| `config` | | The setup form ([Config](#config)). |
| `auth` | | Default authentication ([Auth](#auth)); default `none`. |
| `tls` | | `certificate`, `key`: a client certificate (PEM templates, usually secrets) for mutual TLS. |
| `headers` | | Headers on every request (templates). |
| `errors` | | Error rules for every request ([Errors](#errors-rate-limits-and-quotas)). |
| `rate_limit`, `budget` | | Pacing and request quota of the connection. |
| `tables` | | Static lookup tables for mappings (`tables.event_types[record.type \| str]`). |
| `discover` | | Requests run before every stream, cached ([Records](#records)). |
| `on_connect` | | One-off requests sent when a connection is saved, before anything is stored (starting a subscription the API needs: Microsoft 365's Management Activity `start`). Each: `request`, `message`, `errors`, `ok_status` (statuses that also mean done, e.g. `[409]`). One failing saves nothing. |
| `streams` | yes¹ | What is polled ([Requests](#requests) onwards). |
| `test` | yes¹ | The connection test. |
| `push`, `webhook` | | Push-only vendors ([below](#push-only-vendors-and-webhooks)). |
| `native` | | A connector compiled into the server runs the streams (non-HTTP sources); see [ADR-0007](adr/0007-connectors.md). |
| `assets` | | Dashboards, pipeline, monitors, rules ([Assets](#assets)). |
| `mock` | yes¹ | Recorded answers and expectations ([Testing](#testing-a-package)). |

¹ Unless the package is push-only.

Unknown fields are errors everywhere (typos don't pass silently).

## Config

Each field: `key` (`^[a-z][a-z0-9_]*$`), `label`, `type`, and optionally `required`, `secret`, `default`,
`placeholder`, `help`, `options` (`[{value, label}]`), `pattern` (a regular expression the whole value, or each list
item, must match), `strip_suffix` (url fields: suffixes dropped, e.g. `/api`).

| `type` | Value in templates | Notes |
|---|---|---|
| `string`, `text` | text | `text` is multi-line (a JSON key file, a PEM key). Can be `secret`. |
| `url` | text, no trailing `/` | https only (http when the server allows private hosts), no credentials/query/fragment, checked against the SSRF policy. |
| `number`, `boolean` | number, true/false | |
| `select` | text, one of `options` | |
| `multiselect`, `list` | list of text | `list` is free values (domains, project ids), entered one per line. |

Non-secret fields are `config.<key>`; secrets are `secrets.<key>` and may only be read in `auth`, `tls`, `headers`,
requests and error messages (never in mappings). Secrets are sealed one by one, never answered by the API, and scrubbed
from every error message. A saved secret is only sent to the URLs it was saved with: changing a `url` field asks for
every secret again. `default`s fill empty non-secret fields.

## Templates and expressions

Any string of the manifest may hold `{{ expr }}` parts. A string that is **exactly one** `{{ expr }}` keeps the
value's type (a list, a number, an object, `null`); any other string renders as text, `null` parts as "". JSON trees
(request bodies, log attributes) are templates at every string leaf.

**Expressions** are side-effect free and never fail at run time: a missing step is `null`.

- Literals: `'text'`, `"text"`, numbers, `true`, `false`, `null`, lists `[a, b]`, objects `{key: a, 'other key': b}`.
- Paths: `record.a.b`, `record.items[0]`, `record.items[-1]`, `record[vars.key]`, `headers['retry-after']`.
- Operators, loosest first: `c ? a : b` · `a ?? b` (b when a is `null` or "") · `||` · `&&` ·
  `== != < <= > >= in not in` · `+ -` · `* / %` · unary `! -` · then `.`, `[]` and filters `| name(args)`, which bind
  **tightest**: `1 + x | len` is `1 + (x | len)`; write `(a + b) | upper`; `-5 | max(0)` is `-(5 | max(0))`.
- `+` adds numbers, joins lists, merges objects, else concatenates text. With a `null` side it is `null`, so
  `': ' + x ?? ''` is "" when `x` is missing. `""` is not `null`: use `| nonempty` to turn "" (and `[]`, `{}`) into
  `null` (`(a ?? b) | nonempty`).
- Arithmetic on `null` (or non-numbers) is `null`; division by zero is `null`. Comparisons of different types are false.
- Truthiness: `null`, `false`, `0`, `""`, `[]`, `{}` are false. `&&`, `||`, `!` give booleans.
- `in`: an item of a list, a key of an object, or a substring.
- Times are epoch milliseconds inside expressions (`now`, `minute`, `window.start`): format them with a filter.

**Filters**

| Filter | Does |
|---|---|
| `lower` `upper` `trim` | text case and spaces |
| `split(sep)` `join(sep)` `replace(a, b)` `slice(start[, end])` | text and lists (`slice` takes negative indexes) |
| `startswith(x)` `endswith(x)` `contains(x)` `matches(regex)` | booleans |
| `str` `int` `float` `bool` | conversions (`bool` reads "false", "0", "no", "off" as false) |
| `round([digits])` `floor` `ceil` `abs` `min([x])` `max([x])` `sum` | numbers; `min`/`max` of a list, or of the value and `x` |
| `len` `first` `last` `keys` `values` `entries` `from_entries` `flatten` `compact` `unique` `sort([field])` | collections; `entries` makes `[{key, value}]` of an object |
| `get(key)` `pluck(field)` `where(field[, value])` `index_by(field)` | lists of objects: a field of each, those matching, an object by a field |
| `explode(field)` | each object of a list once per item of its list `field` (`{a, tags: [x, y]}` → `{a, tags: x}`, `{a, tags: y}`) |
| `with(field, value)` | sets `field` on each object of a list (tagging records of several arrays) |
| `default(x)` `nonempty` | `x` when null or ""; "" / `[]` / `{}` → `null` |
| `epoch_ms([zone])` `epoch_s([zone])` | a time → epoch ms / s. Reads RFC 3339 / ISO 8601 (with or without a zone), `YYYY-MM-DD`, RFC 2822, and numbers of seconds, ms, µs or ns (told apart by size). `zone` (`'Europe/Bucharest'`, `'-05:00'`) is used for times without one. |
| `rfc3339` `rfc3339_s` `strftime(fmt[, zone])` | epoch ms (or any time) → `2026-11-10T12:00:00.000Z`, without ms, or chrono's `strftime` format |
| `host` | a URL's host |
| `urlencode` `base64` `base64_decode` `json` `parse_json` `sha256` | encodings (`sha256` is hex) |
| `to_objects(columns)` | a list of arrays → objects by column names (strings, or objects with `name`) |
| `map(expr)` | each element of a list through an expression reading it as `it` (`rules \| map(it.data \| base64_decode)`) |

New filters are added over time; an unknown filter or a wrong number of arguments is a validation error.

**Variables by place** (anything else is a validation error):

| Place | Variables |
|---|---|
| everywhere | `config`, `tables`, `connection` (`id`, `name`, `instance`), `now`, `minute` (now floored to the minute, ms), `source` |
| `auth`, `tls` | + `secrets` (token exchange headers: + `auth`) |
| requests (`url`, `query`, `headers`, `body`, `form`) | + `secrets`, `auth` (token exchange captures), `discovery`, `lookups`, `item`, `page` (`cursor`, `offset`, `number`, `size`), `window` (`start`, `end`), `cursor` (`token`), `export` (`id`), `chunk` |
| answers (`records`, pagination `next`/`total`, incremental `next`, test `success`/`message`, error rules, discovery `value`) | + the request's variables and `response`, `headers` (lowercase names), `status`, `records` (the page's), `last` (its last record) |
| records (`record_id`, `filter`, `vars`, `record_time`, every output field) | `record`, `vars`, `lookups`, `item`, `window`, `discovery`, `parent` (expanded records), `entry` (metrics' `each`), and the "everywhere" ones; never `secrets` |
| `expand.request` | the request's variables + `parent` |
| `on_connect`, `push.create` | the request's variables (+ `push_url` for `push.create`), then `response`, `status` in `message` |
| `for_each` | + `discovery`, `lookups` |
| lookup `key` / `value` | `record` (the lookup's record) |

## Auth

`auth.kind`:

| Kind | Fields | Sends |
|---|---|---|
| `none` | | nothing |
| `api_key` | `name`, `value`, `in: header\|query` (default header) | `name: value`. Covers `Authorization: Token {{ secrets.token }}`, `apikey {{ config.email }}:{{ secrets.key }}`, a raw token. |
| `bearer` | `token` | `Authorization: Bearer <token>` |
| `basic` | `username`, `password` | HTTP basic |
| `headers` | `headers: {name: template}` | several headers |
| `oauth2_client_credentials` | `token_url`, `client_id`, `client_secret`, `scopes`, `client_auth: basic\|body` (default basic), `params` | fetches a token (`grant_type=client_credentials`), caches it until a minute before `expires_in`, sends it as a bearer token |
| `oauth2_refresh_token` | `token_url`, `client_id`, `client_secret?`, `refresh_token`, `scopes`, `client_auth`, `params`, `rotate` | refresh grant; a new refresh token in the answer is saved into the secret named by `rotate` |
| `oauth2_client_assertion` | `token_url`, `client_id`, `private_key`, `key_id?`, `audience?`, `scopes`, `params`, `lifetime` (5m) | client credentials with a signed client assertion (`private_key_jwt`: an RS256 JWT, `iss` = `sub` = the client id, `aud` the token URL, a random `jti`) instead of a secret: Okta service apps, Entra certificate credentials; the token is cached |
| `oauth2_authorization_code` | `authorize_url`, `token_url`, `scopes`, `scope_separator?`, `authorize_params`, `params`, `client_auth`, `pkce` (true), `client_id?`, `client_secret?` | "Connect with <vendor>" on the page: the admin consents on the vendor's page (PKCE S256, a single-use state), ObserveCraft's callback (`<OC_PUBLIC_URL>/api/v1/public/connectors/oauth/callback`) exchanges the code and saves the connection with its refresh token (else its access token) sealed; runs refresh it like `oauth2_refresh_token` (rotated tokens saved). The client is the operator's OAuth app for the package (`OC_OAUTH_<ID>_CLIENT_ID` / `_CLIENT_SECRET`, `<ID>` upper-cased with `_` for `-` and `.`), or the connection's own when `client_id` / `client_secret` name its fields (`{{ config.client_id }}`, `{{ secrets.client_secret }}`). For the `oauth` tiles and user-authorized apps (Zoom, Atlassian, Slack) |
| `jwt` | `private_key`, `claims`, `key_id`, `lifetime`, `headers` | a JWT signed with the connection's key (RS256), renewed before it expires, sent as `Authorization: Bearer` or in `headers` (templates over `auth.jwt`). Claims may read `key.fingerprint` (`SHA256:` + base64 SHA-256 of the public key: Snowflake key-pair auth, `iss: "{{ config.account + '.' + config.user + '.' + key.fingerprint }}"`) |
| `jwt_bearer` | `service_account` (a Google JSON key) or `private_key` (PEM, PKCS#8 or PKCS#1) + `token_url`, `key_id`, `claims`, `scopes`, `lifetime` (default 1h), `params` | an RS256 assertion (`iat`, `exp` added; with a service account `iss` and `aud` too) exchanged at `token_url` (`grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`); covers Google service accounts with domain-wide delegation (`claims: {sub: '{{ config.admin_email }}'}`) and customer-registered OAuth apps (DocuSign: `claims: {iss: integration key, sub: user id, aud: account host}`; show the consent URL in `links`) |
| `token_exchange` | `request`, `capture: {name: template}`, `expires_in?`, `ttl` (default 1h), `headers`, `jwt?` | a login request; the values captured from its answer (`response`, `headers`) are `auth.<name>` in later requests (a token, a base URL, a cookie), and `headers` (default `Authorization: Bearer {{ auth.token }}`) go on every request. Covers short-lived JWTs (Greenhouse), logon calls returning a manager URL (CyberArk EPM), password → token (ZeroFox), cookie sessions (`capture: {cookie: "{{ headers['set-cookie'] }}"}`). With `jwt` (a `jwt` block as above, without `headers`), the login request can read `auth.jwt`: GitHub Apps (`Authorization: Bearer {{ auth.jwt }}` on `POST /app/installations/{id}/access_tokens`, `capture: {token: "{{ response.token }}"}`) |
| `signer` | `signer`, `params` | a signer compiled into the server signs each request: `duo_hmac_v2` / `duo_hmac_v5` (`ikey`, `skey`), `cortex_xdr_advanced` (`key_id`, `key`; `level: standard` sends a Standard key as it is), `trend_micro_apex_central` (`application_id`, `api_key`), `akamai_edgegrid` (`client_token`, `client_secret`, `access_token`). A new scheme is a small Rust function in `oc-connectors/src/auth.rs` plus its name in the schema. |

**Choices.** `{kind: choice, field: method, options: {token: {kind: api_key, …}, oauth: {kind:
oauth2_client_credentials, …}}}` lets the connection pick (a select config field, every option of it covered): an Okta
SSWS token or OAuth, a Shopify token or an app.

A stream can override it with its own `auth` (an API needing two credentials). Tokens are cached per connection and
forgotten when its secrets change; tests never reuse a cached token.

## Requests

```yaml
request:
  method: POST                 # GET (default), POST, PUT, PATCH
  url: "{{ config.base_url }}/api/v2/search"
  query: { since: "{{ window.start | rfc3339 }}", limit: 100, cursor: "{{ page.cursor }}", tag: [a, b] }
  headers: { X-Tenant-Id: "{{ discovery.whoami.id }}" }
  body: { query: "…", variables: { after: "{{ page.cursor }}" } }   # or form: {k: v}
  format: json                 # json (default), ndjson, csv, prometheus, text
```

- Query values rendering to `null` or "" are left out; lists repeat the parameter.
- `format: ndjson` makes `response` the list of lines; `csv` the list of rows as objects by the header row's names
  (RFC 4180 quoting); `prometheus` a list of samples `{name, family, labels, value,
  type, help, timestamp}` (NaN and infinite values left out); `text` the text.
- Every request goes through the SSRF guard (the host resolved once and checked, no redirects, 32 MB and 30 s per
  answer). `User-Agent: ObserveCraft-Connectors/1.0` is sent unless the manifest sets one.

## Pagination

`pagination.kind` (templates read the answer: `response`, `headers`, `records`, `page`):

| Kind | Fields | Next page |
|---|---|---|
| `none` | | one page |
| `cursor` | `next` | `page.cursor` = `next` (any JSON value: `{pit: headers['pit-id'], after: headers['search-after']}` works); stops when it is `null`/"" or the page has no records |
| `next_url` | `next`, `method?` | the URL `next` gives (absolute or relative); stops when `null`/"" or no records. `method: GET` after a first `POST` (a query whose result chunks are links) sends the next pages without the body |
| `link_header` | | the `Link` header's `rel="next"` |
| `offset` | `size`, `start` (0), `total?`, `max?` | `page.offset` += `size` (`page.size` = `size`); stops on a short page or when the offset reaches `total`. `max`: the largest offset the API accepts; a time window reaching it is split in two and read again (Cisco Umbrella) |
| `page` | `size`, `start` (1), `total_pages?` | `page.number` += 1; stops on a short page or after `total_pages` |
| `async_export` | `create`, `id`, `status`, `ready`, `failed?`, `chunks`, `chunk` | an export flow across runs: `create` starts it (`export.id` = `id` of its answer), each run polls `status` until `ready`, then reads every chunk of `chunks` with `chunk` (variable `chunk`); the stream's `request` isn't used (Tenable). Records outside the export's window are skipped, and ids sent stay remembered across exports |

A next page never leaves the first request's host (credentials never go elsewhere); one that would is refused with a
warning. A run reads at most `max_pages` pages (default 20) per partition.

## Incremental reading

`incremental.kind` decides what a run reads again and what it skips:

| Kind | Fields | For |
|---|---|---|
| `none` | | snapshots (state as gauges): everything every run |
| `new_records` | `max_seen` (100 000), `order?` | lists without times: only records whose `record_id` wasn't sent before (breached accounts, findings, finished test runs). `order: newest_first`: a page with a record sent before ends the paging |
| `time` | `record_time` (may read `vars`), `order`, `lookback` (1d), `overlap` (0s), `settle` (0s), `slice?`, `max_window?`, `cursor: window_end\|last_record` | event logs |
| `token` | `next`, `initial?` | APIs that hand out a checkpoint (`since` tokens, `nextpagetoken` kept across runs): `cursor.token` |

**Time windows.** A run reads the window `window.start` = cursor − `overlap` (the first run: now − `lookback`, and never
further back) to `window.end` = now − `settle`, cut into `slice`s read oldest first, at most `max_window` long. Records
whose `record_time` is outside the window are skipped; so are records already sent (their `record_id`s are kept while
they are inside the overlap). A window read to its end moves the cursor to its end (`cursor: last_record`: to its
newest record). A window not read to its end (out of pages or quota, a page failed) carries on next run, never losing
records:

- `order: newest_first` or `unordered`: the run keeps the window and the page it stopped at (the backlog); the next
  run carries on from there, and the cursor moves only once the window is read.
- `order: oldest_first`: the cursor moves to the last record read; the next window starts there.
- `order: newest_first` lists the request can't filter by time stop at the first record older than the window: the
  window is complete, nothing older is paged through.

Without `record_id`, the record's JSON is its identity. The cursor and the backlog are saved only after the run's
records are stored in Kafka, so a failed publish reads them again.

`for_each` gives every item its own cursor (a partition); items that disappear from the list lose their state.

**Bucketed APIs revising settled buckets** (usage per hour, still counted for a while): emit a `gauge` per bucket
(`time: "{{ record.start }}"`), read the recent buckets again with `overlap`, and put the value in the `record_id`
(`"{{ record.start + ':' + record.count }}"`): a revised bucket is sent again at its own time, an unchanged one never.
Don't use `type: count` for buckets (a delta over the run).

## Records

```yaml
discover:                          # before every stream, cached for ttl (default 1h)
  - { name: whoami, request: { url: "…/whoami" }, value: "{{ response.apiHosts.dataRegion }}" }
  - { name: tenants, request: { url: "…/tenants", query: { page: "{{ page.number }}" } }, pagination: { kind: page, size: 100 },
      records: "{{ response.data }}", value: "{{ records | pluck('id') }}" }   # every page's records as `records`
streams:
  - id: events                     # ^[a-z][a-z0-9_]*$, the job's scope
    name: Events
    description: …
    interval: 5m                   # at least 1m (the default)
    default_enabled: true          # admins turn streams on and off per connection
    for_each: "{{ config.projects ?? discovery.projects }}"     # item
    lookups:                       # read once per run, cached for ttl: lookups.<name>[key]
      - { name: members, request: { url: "…/members" }, records: "{{ response.data }}", key: "{{ record.id }}", value: "{{ record.name }}" }
      # reading `item`: per for_each item; `when` (over config, item) false: not read, lookups.<name> is {}
      - { name: owners, when: "{{ config.with_owners }}", request: { url: "…/projects/{{ item }}/owners" }, key: "{{ record.id }}" }
    request: …
    records: "{{ response.data }}" # default: the answer (an object is one record)
    record_id: "{{ record.id }}"
    filter: "{{ record.type != 'heartbeat' }}"
    vars: { actor: "{{ lookups.members[record.actorId] ?? record.actorId }}" }   # a var can't read another var
```

- Several arrays in one answer: `records: "{{ (response.clicksBlocked | with('kind', 'click_blocked')) + (response.messagesDelivered | with('kind', 'message_delivered')) }}"`.
- An object of lists: `records: "{{ response | entries | explode('value') }}"`.
- Wrapped records: `records: "{{ response.items | pluck('record') }}"`.
- Child streams (jobs of each recipe, logs of each run): a lookup lists the parents, `for_each` iterates them:
  `for_each: "{{ lookups.recipes | keys }}"`.
- Index-then-download (Salesforce EventLogFile CSVs, Microsoft 365 content blobs): the pages list index entries and
  `expand: {request: {url: "{{ config.base_url + parent.LogFile }}", format: csv}, records: …}` reads each one; its
  records take the entry's place, with the entry as `parent` in the record's templates (`record_id: "{{ parent.Id +
  ':' + record.REQUEST_ID }}"`). With a `record_id` (and `record_time`) reading only `parent`, entries sent before or
  outside the window are skipped before their download (per-record detail requests aren't sent again every run).
- YAML anchors and merge keys (`- <<: *events`) work for streams that share most of their fields.
- Positional rows (Snowflake's SQL API): `records: "{{ response.data | to_objects(response.resultSetMetaData.rowType) }}"`.

## Output

A stream's `output` has any of `logs`, `metrics` (a list), `events`, `costs`.

**Logs** (`oc.logs.json`, normalized like the intake's Datadog logs, then your pipeline):

```yaml
logs:
  message: "{{ record.action }} by {{ vars.actor }}"
  status: "{{ record.severity == 'high' ? 'error' : 'info' }}"   # default info
  service: audit                   # default the manifest's source
  source: …                        # default the manifest's source
  host: …                          # default connection.instance
  date: "{{ record.created_at }}"  # default the record's time, else the run's
  tags: { env: "{{ record.env }}" }                              # ddtags
  attributes:                      # null, "" and empty objects are left out
    evt: { name: "{{ record.action }}", outcome: "{{ record.success ? 'success' : 'failure' }}" }
    usr: { id: "{{ record.actor.id }}", email: "{{ record.actor.email }}" }
    network: { client: { ip: "{{ record.ip }}" } }
  keep_record_as: vendor           # the whole record under @vendor
```

Use Datadog's standard attributes (`usr.*`, `network.client.ip`, `http.*`, `evt.name`, `evt.outcome`,
`error.*`) so facets, Cloud SIEM rules and the tile's dashboard work. `attributes` may also be one template giving an
object (`"{{ record + {user_details: {…}} }}"`), pruned the same way. A record's own `status` attribute stays
`@status`, apart from the log's status (it travels in the log's JSON message, whose fields the log ingester keeps as
attributes). Every log gets an identity from the instance, the `record_id` and the
connector, timed at its date: a record read again rewrites the same row.

**Metrics** (`oc.otlp-json.metrics`, resource `cloud.provider = <id>`):

```yaml
metrics:
  - name: vendor.queue.depth       # a template for families: "streamnative.{{ record.name }}"
    type: gauge                    # or count (a delta since the last run)
    value: "{{ record.depth }}"    # records whose value is null are skipped
    tags: { queue: "{{ record.name }}" }       # empty values left out
    tags_from: "{{ record.labels }}"           # an object of more tags (Prometheus labels)
    each: "{{ record.usage }}"     # one point per entry (an object's {key, value}, or a list's items): entry
    host: …                        # none by default: accounts aren't hosts
    time: "{{ record.timestamp }}" # default the run's minute
    filter: …
    unit: second                   # Datadog unit names (a template for families: "{{ tables.units[entry.key] }}"), sent as the points' unit
    aggregate: max                 # sum, min, max, avg, count, p50, p75, p90, p95, p99: the run's points of a series and time as one
    examples: [vendor.queue.depth] # the names a templated name gives, for Data Collected (default: a tables.<t>[…] name's table values)
    description: …
```

Keep Datadog's metric names when the tile has them (`metadata.csv`), so users' dashboards and monitors keep working.

**Events** (Event Management, `otel.otel_events`): `title`, `text`, `alert_type` (error, warning, info, success),
`priority`, `host`, `tags`, `aggregation_key`, `date`.

**Costs** (Cloud Cost Management, `otel.cloud_costs`): `provider` (a fixed name: `anthropic`, `databricks`), `date`,
`cost`, `currency` (USD), `service`, `account`, `usage_type`, `usage_amount`, `tags`. One file per connection and
stream: a day read again replaces its rows.

## Errors, rate limits and quotas

```yaml
errors:                            # the stream's, then the manifest's, then the defaults
  - { status: [403], when: "{{ response.code == 'PLAN' }}", kind: access_denied, message: "The audit log needs the Enterprise plan" }
rate_limit: { requests_per_minute: 60 }        # or min_interval: 2s
budget: { requests: 10000, per: day }          # hour, day, month; the manifest's for the connection, a stream's for itself
```

**Truncated runs** are warnings on the stream's status: a list read again every run (`none`, `new_records`) with more
pages than `max_pages`, and an answer the stream's `truncated` template says was cut short (`truncated: "{{ records |
len >= 10000 }}"` for an API answering its newest 10,000 records) that doesn't reach back past the window. Time
windows out of pages aren't truncated: they carry on next run.

A rule with `reset: true` drops the stream's saved position (its token, backlog, export) when it matches: an offset
the API no longer has (HTTP 416) starts again from the initial token or the cursor's window, with a warning.

A rule naming a 2xx status (`status: [200]`) with a `when` reads successful answers too: GraphQL APIs answering
errors with 200 (`when: "{{ response.errors | len > 0 }}"`).

Kinds: `auth` (credentials refused: retried at the stream's interval), `access_denied` (permission or plan: interval),
`throttled` (exponential backoff), `disabled` (daily), `other` (exponential). Defaults: 401 `auth`, 403 and 404
`access_denied`, 429 `throttled` (with `Retry-After`), others `other`, each with the vendor's own message when it has
one. A first request that fails fails the run (nothing published); a page after the first keeps what was read as the
run's warning. A used-up quota stops the run like running out of pages.

## The connection test

```yaml
test:
  request: { url: "{{ config.base_url }}/api/v1/me" }   # or stream: events (its first page; `records` is set)
  success: "{{ response.role in ['admin', 'auditor'] }}" # default: the request succeeded
  message: "Connected as {{ response.email }}"
  failure: "{{ response.email }} can't read the audit log: give it the Auditor role"
  errors: [ … ]
```

## Assets

- **The tile's integrations-core assets come for free.** Saving a connection installs the tile's log pipeline and
  dashboards from integrations-core (as Integrations → Install does). Don't copy them into the package: produce the
  logs they expect.
- `assets.dashboards`: our own dashboards (vendors Datadog has none for) in Datadog's JSON; `node scripts/connector.mjs
  build` converts them into `dist/dashboards.json` (commit both). Their id is `<id>-<file name>`, their logo the
  package's; they are seeded for every org and listed once it has a connection.
- `assets.pipeline`: a log pipeline in Datadog's pipeline JSON (`{name, filter: {query}, processors}`), installed with
  the first connection (`connector:<id>`), editable in Logs → Pipelines.
- `assets.monitors` (Datadog's monitor JSON, or integrations-core's `{definition: …}`), `assets.security_rules`
  (Cloud SIEM rule JSON) and `assets.log_metrics` (`{id, compute, filter, group_by}` or the API's `{data: …}`): created
  for the org with its first connection, through the same code as Datadog's API (monitors and rules tagged
  `connector:<id>`). Each is created once (Postgres `connector_assets`): saving again, a second connection or an asset
  the org deleted create nothing; a log-based metric whose name the org has is left alone. One that can't be created
  is skipped (the server logs why). Removing the org's last connection removes them and the pipeline; dashboards
  stay. `assets.metadata` is validated and shipped.

## Push-only vendors and webhooks

Vendors that send to us (Cloudflare Logpush, Auth0 log streams, Fastly logging, Akamai DataStream, Zscaler NSS,
Vercel drains, Stripe / PagerDuty / LaunchDarkly webhooks) have no streams:

```yaml
push:
  kind: logs                     # logs (Datadog's logs API), otlp, webhook (the intake's /intake/webhook/<name>), splunk_hec
  instructions: |                # Markdown; {{ intake }}, {{ source }}, {{ api_key }} are replaced on the page
    In Cloudflare, create a Logpush job with an HTTP destination …
  url: "{{ intake }}/api/v2/logs?ddsource={{ source }}&dd-api-key={{ api_key }}"    # default by kind
  headers: { DD-API-KEY: "{{ api_key }}" }
  signature: { header: Stripe-Signature, algorithm: stripe, secret: signing_secret } # hmac_sha1, hmac_sha256, stripe, pagerduty_v3
  create:                        # optional: the page creates the push job with an API token the admin enters
    request: { method: POST, url: "https://api.cloudflare.com/client/v4/zones/{{ config.zone_id }}/logpush/jobs", body: { destination_conf: "{{ push_url }}", dataset: http_requests } }
    message: "Logpush job {{ response.result.id }} created"
```

They appear in the catalog as available and are installed from their tile (the tile's pipeline and dashboards); the
page shows the URL and headers, and with `create` a form (the package's `config`) that runs the request once
(`POST /api/v1/connectors/{id}/push`, nothing stored).

`push.endpoints` lists more endpoints the vendor sends to (`{label: Traces, kind: otlp, url: …, headers, instructions}`),
each shown with its URL. OTLP logs take `?ddsource=<source>&service=…` on `/v1/logs` (`url: "{{ intake }}/v1/logs?ddsource={{
source }}"`): every record without its own gets them, so the package's pipeline matches. Vendors that take only a base
URL but let the user add headers (Modal's `OTEL_HEADER_<name>` secrets) send `X-OC-Source` / `X-OC-Service` instead;
`/api/v2/logs` reads them too, and the query's win over them.

**Signatures.** `push.signature` names the header, the algorithm and the secret config field holding the signing
secret; a package with one is connected like a polled one (the page's "Save the signing secret": a connection with
that sealed field, no streams). The intake checks every request against the org's saved secrets (any of them, so a
secret can be rotated) and refuses unsigned or wrongly signed ones with 401; until a secret is saved the ingest key
alone admits them.

| `algorithm` | Header value | Vendors |
|---|---|---|
| `hmac_sha256` | hex HMAC-SHA256 of the body (`prefix: "sha256="` for GitHub) | GitHub `X-Hub-Signature-256`, LaunchDarkly `X-LD-Signature` |
| `hmac_sha1` | hex HMAC-SHA1 of the body | Vercel `x-vercel-signature` |
| `stripe` | `t=<unix>,v1=<hex>…`: HMAC-SHA256 of `t.body`, at most `tolerance` old (default `5m`) | Stripe `Stripe-Signature` |
| `pagerduty_v3` | `v1=<hex>,v1=<hex>…`: HMAC-SHA256 of the body, any one matching | PagerDuty `X-PagerDuty-Signature` |
| `token` | the secret itself, after `prefix` (`"Bearer "`) | log streams sending a shared token (Auth0) |

**Webhooks.** `/intake/webhook/<name>` (`webhook.name`, default the id; the ingest key as `?dd-api-key=`) maps the
body as `webhook` says:

```yaml
webhook:
  records: "{{ body.data }}"       # default the body (each item of an array)
  record_id: "{{ record.id }}"     # with a log date, a delivery retried is stored once
  filter: "{{ record.type != 'ping' }}"
  logs: { message: "{{ record.type }}", date: "{{ record.created }}", attributes: { evt: { name: "{{ record.type }}" } } }
  events: { title: "{{ record.summary }}", text: "{{ record.id }}", alert_type: "{{ record.urgency == 'high' ? 'error' : 'warning' }}" }
```

Templates read `body`, `record`, `headers` (the request's, lowercase names, keys left out) and `tables`. Logs go to
Kafka like every other log; events to Event Management. A `push.kind: logs` package with a signature has its
`/api/v2/logs?ddsource=<source>` requests checked the same way. The intake reads the definitions and the org's
signing secrets from the server's internal listener (`/ingest/connectors/push`, cached for a minute; the org is the
ingest key's), so the operator's packages work there too.

**Splunk HEC and Fastly.** Vendors that only stream to Splunk (GitHub Enterprise's audit log streaming) send to the
intake's HEC-compatible endpoint: `https://<intake>/services/collector` with the org's ingest key as the HEC token
(`Authorization: Splunk <key>`, `/services/collector/raw` for lines); events become JSON logs (`sourcetype` as
`ddsource` unless `?ddsource=` says otherwise, `fields` as tags). Fastly's HTTPS logging endpoints check
`/.well-known/fastly/logging/challenge` first: the intake answers `*`, or the SHA-256 of each service id in
`OC_INTAKE_FASTLY_SERVICES`. A package's `push.url` points there:
`{{ intake }}/services/collector?ddsource={{ source }}`.

## Testing a package

```yaml
mock:
  config: { base_url: "https://api.example.com" }
  secrets: { token: test-token }
  now: "2026-11-10T12:00:00Z"
  responses:                       # the most specific (query, body_contains) unused match, else the most specific: reruns get the first run's answers
    - { url: "https://api.example.com/v1/events", query: { cursor: c2 }, file: fixtures/events-2.json }
    - { url: "https://api.example.com/v1/events", file: fixtures/events-1.json, headers: { Link: "…" } }
    - { method: POST, url: "https://login.example.com/token", body_contains: "client_credentials", body: { access_token: t } }
    - { url: "https://api.example.com/v1/me", status: 200, body: { email: admin@example.com } }
  expect:
    events: { records: 4, logs: 4, rerun: 0, golden: fixtures/events.expected.json }
    peers: { metrics: 21 }
    test: { ok: true, message: "admin@example.com" }
```

- Each stream runs once from an empty state at `now`, then (`rerun`) again a minute later with the state it left,
  against the same answers: `rerun: 0` proves incremental reading skips what was sent.
- `golden` files hold the run's logs, metrics, events, costs and state; `OC_CONNECTOR_UPDATE=1` rewrites them.
- `dist/` is committed (the server compiles it in): run `node scripts/connector.mjs build` after changing a dashboard.
- `node scripts/connector.mjs validate <dir>` checks the manifest; `test <dir>` replays; both run the server's own
  code (`oc-connector`). `cargo test -p oc-connectors` covers the engine; `cargo test -p oc-api connectors` replays
  every bundled package.
- The web's mock mode (`make web-mock`) lists bundled packages from `web/src/api/connectors/packages.generated.json`
  (`node scripts/connector.mjs build`) with their mock connection.

## Recipes

- **GraphQL**: `method: POST`, `body: {query: '…', variables: {first: 100, after: '{{ page.cursor }}'}}`,
  `pagination: {kind: cursor, next: '{{ response.data.x.pageInfo.endCursor }}'}`.
- **A search body with paging fields**: put `{{ page.offset }}` / `{{ page.cursor }}` in the body.
- **An API limiting a request's range**: `incremental.time.slice: 1h`.
- **An API capping offsets**: `pagination: {kind: offset, size: 1000, max: 9000}` with a time window in the request.
- **Data regions / tenants**: `discover` the region, then `url: "{{ discovery.whoami.apiHosts.dataRegion }}/…"`;
  partners: `for_each: "{{ discovery.tenants | pluck('id') }}"` and a `X-Tenant-ID: "{{ item }}"` header.
- **Times without a zone**: `record_time: "{{ record.time | epoch_ms(config.time_zone) }}"`.
- **Large metric name maps**: `tables: {metrics: {nOffered: 'genesys.conversations.offered', …}}` and
  `each: "{{ record.metrics }}"`, `name: "{{ tables.metrics[entry.key] }}"` (points with an empty name are skipped).
- **Prometheus endpoints**: `format: prometheus`, `filter: "{{ record.type != 'histogram' }}"`,
  `name: "{{ 'vendor.' + record.name }}"`, `tags_from: "{{ record.labels }}"`.

## Third-party packages

Operators add packages without rebuilding ObserveCraft: one directory per package under `OC_CONNECTORS_DIR` (the
same layout). At startup the server validates them; an invalid one is left out with one error line naming it and every
problem (instance admins also see them in `GET /api/v1/connectors`), and an id clashing with a bundled or native
connector is refused. Valid ones appear in the catalog marked third-party with their `publisher`, and run like bundled
ones (same SSRF guard, quotas, tenancy).

Helm: put the packages in a ConfigMap (or any volume) and set `connectors.configMap` (or `connectors.volume`) in
`deploy/helm/observecraft/values.yaml`; the chart mounts it and sets `OC_CONNECTORS_DIR`. A ConfigMap holds files
without directories, so name its keys `<package>__<path with __ for />` (`acme-status__connector.yaml`,
`acme-status__fixtures__incidents.json`): the server reads such flat files as packages. Build `dist/` before
packaging:

```bash
kubectl create configmap my-connectors $(for f in $(cd acme-status && find . -type f | sed 's|^\./||'); do echo "--from-file=acme-status__${f//\//__}=acme-status/$f"; done)
```

## Licensing

ObserveCraft is MIT; the rules of [CLAUDE.md](../CLAUDE.md) apply to packages:

- Write manifests, mappings and fixtures yourself from the vendor's public API documentation; don't paste Datadog's
  documentation text. Fixtures are fictional (no customer data, no real names, hosts or ids).
- integrations-core / integrations-extras assets (BSD-3-Clause) may be adapted with their copyright and the source
  path noted; the tile's own assets are installed from the catalog, so they rarely need copying.
- Datadog's proprietary frontend, images and logos are never used; packages ship no images.
- Third-party packages carry their own license; bundled ones are MIT.

## Checklist

- [ ] `id` and `tiles` are the catalog tile's; `source` is the tile's `logSource`.
- [ ] The logs have the shape Datadog's integration sends (its `_tests.yaml`), with standard attributes.
- [ ] Metric names, types and units are Datadog's (`metadata.csv`) when the tile has metrics.
- [ ] Every stream has a `record_id` (or a reason not to) and an incremental strategy; event logs use `time` with an
      `overlap` for late data and the right `order`.
- [ ] Error rules say what to fix in the vendor's words (role, plan, scope).
- [ ] Rate limits and quotas from the vendor's documentation.
- [ ] `mock` covers every stream and the test, with `rerun: 0` for incremental streams, and golden files read.
- [ ] `node scripts/connector.mjs test` and `cargo test -p oc-api connectors` pass.
