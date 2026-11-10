//! The engine against recorded answers: every auth kind, pagination kind and incremental strategy, and that time
//! windows lose no record and send none twice across runs (backlogs, failures, splits).

use std::sync::Arc;

use base64::Engine;
use oc_connectors::engine::{self, Connection, Run, Runtime, StreamState};
use oc_connectors::http::HttpRequest;
use oc_connectors::manifest::IssueKind;
use oc_connectors::package::{Origin, Package};
use oc_connectors::replay::{self, MockTransport};
use serde_json::{json, Value};

/// 2026-11-10T12:00:00Z.
const NOW: i64 = 1_794_312_000_000;
const MIN: i64 = 60_000;
const HOUR: i64 = 3_600_000;

const HEAD: &str = "schema_version: 1\nid: example\nname: Example\npublisher: ObserveCraft\nvendor: Example Corp\ndescription: Test package.\n";
const CONFIG: &str = "config:\n  - { key: base_url, label: URL, type: url, required: true }\n  - { key: token, label: Token, type: string, secret: true }\n";
const TEST: &str = "test:\n  request: { url: 'https://api.example.com/v1/me' }\n  message: ok\n";

/// A package from manifest parts (the header, config and test are added).
fn package(body: &str) -> Package {
    let yaml = format!("{HEAD}{CONFIG}{body}\n{TEST}");
    Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"))
}

fn invalid(body: &str) -> Vec<String> {
    let yaml = format!("{HEAD}{CONFIG}{body}");
    Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).err().unwrap_or_default()
}

fn conn(pkg: &Package) -> Connection {
    let mut c = replay::connection(pkg.manifest.mock.as_ref().unwrap());
    c.config = engine::effective_config(&pkg.manifest, &c.config);
    c
}

struct Fixture {
    pkg: Package,
    transport: Arc<MockTransport>,
    rt: Runtime,
}

fn fixture(body: &str) -> Fixture {
    let pkg = package(body);
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    Fixture { pkg, transport, rt }
}

impl Fixture {
    async fn run(&self, state: StreamState, now: i64) -> Result<Run, engine::Issue> {
        let stream = &self.pkg.manifest.streams[0];
        engine::run(&self.pkg.manifest, stream, &conn(&self.pkg), state, now, &self.rt, None).await
    }

    fn requests(&self) -> Vec<HttpRequest> {
        std::mem::take(&mut *self.transport.requests.lock().unwrap())
    }
}

fn ids(run: &Run) -> Vec<String> {
    run.mapped.logs.iter().map(|l| l["message"].as_str().unwrap().to_string()).collect()
}

fn mock(responses: &str) -> String {
    format!("mock:\n  config: {{ base_url: 'https://api.example.com' }}\n  secrets: {{ token: 'tok-SECRET-123' }}\n  now: '2026-11-10T12:00:00Z'\n  responses:\n{responses}")
}

const LOGS: &str = "    output:\n      logs: { message: '{{ record.id }}' }\n";

fn stream(rest: &str) -> String {
    format!("streams:\n  - id: events\n    name: Events\n    description: Events.\n{rest}{LOGS}")
}

// ---------------------------------------------------------------- auth

#[tokio::test]
async fn keys_bearer_and_basic_go_where_the_manifest_says() {
    for (auth, check) in [
        ("auth: { kind: api_key, name: X-Api-Key, value: '{{ secrets.token }}' }", Box::new(|r: &HttpRequest| r.header("x-api-key") == Some("tok-SECRET-123")) as Box<dyn Fn(&HttpRequest) -> bool>),
        ("auth: { kind: api_key, in: query, name: api_key, value: '{{ secrets.token }}' }", Box::new(|r: &HttpRequest| r.url.ends_with("?api_key=tok-SECRET-123"))),
        ("auth: { kind: bearer, token: '{{ secrets.token }}' }", Box::new(|r: &HttpRequest| r.header("authorization") == Some("Bearer tok-SECRET-123"))),
        ("auth: { kind: basic, username: me, password: '{{ secrets.token }}' }", Box::new(|r: &HttpRequest| r.header("authorization") == Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("me:tok-SECRET-123")).as_str()))),
        ("auth: { kind: api_key, name: Authorization, value: 'apikey me@example.com:{{ secrets.token }}' }", Box::new(|r: &HttpRequest| r.header("authorization") == Some("apikey me@example.com:tok-SECRET-123"))),
    ] {
        let f = fixture(&format!("{auth}\n{}{}", stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"), mock("    - { url: 'https://api.example.com/v1/events', body: [{ id: a }] }")));
        let run = f.run(StreamState::default(), NOW).await.unwrap();
        assert_eq!(ids(&run), ["a"]);
        let reqs = f.requests();
        assert!(check(&reqs[0]), "{auth}: {:?}", reqs[0]);
        assert_eq!(reqs[0].header("user-agent"), Some(engine::USER_AGENT));
    }
}

#[tokio::test]
async fn oauth_client_credentials_tokens_are_fetched_once_and_cached() {
    let f = fixture(&format!(
        "auth: {{ kind: oauth2_client_credentials, token_url: 'https://login.example.com/token', client_id: app-1, client_secret: '{{{{ secrets.token }}}}', scopes: [read, audit] }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { method: POST, url: 'https://login.example.com/token', body: { access_token: at-1, expires_in: 3600 } }\n    - { url: 'https://api.example.com/v1/events', body: [{ id: a }] }")
    ));
    f.run(StreamState::default(), NOW).await.unwrap();
    f.run(StreamState::default(), NOW + MIN).await.unwrap();
    let reqs = f.requests();
    assert_eq!(reqs.iter().filter(|r| r.url.contains("login")).count(), 1, "cached for its lifetime");
    let token = &reqs[0];
    assert_eq!(String::from_utf8(token.body.clone().unwrap()).unwrap(), "grant_type=client_credentials&scope=read+audit");
    assert_eq!(token.header("authorization"), Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("app-1:tok-SECRET-123")).as_str()));
    assert!(reqs[1..].iter().all(|r| r.header("authorization") == Some("Bearer at-1")));
    // An hour later the token is fetched again.
    f.run(StreamState::default(), NOW + 2 * HOUR).await.unwrap();
    assert_eq!(f.requests().iter().filter(|r| r.url.contains("login")).count(), 1);
}

#[tokio::test]
async fn a_rotated_refresh_token_is_returned_to_be_saved() {
    let yaml = format!(
        "{HEAD}config:\n  - {{ key: base_url, label: URL, type: url }}\n  - {{ key: refresh_token, label: Refresh token, type: string, secret: true }}\n\
         auth: {{ kind: oauth2_refresh_token, token_url: 'https://login.example.com/token', client_id: app-1, refresh_token: '{{{{ secrets.refresh_token }}}}', rotate: refresh_token, client_auth: body }}\n\
         {}mock:\n  config: {{ base_url: 'https://api.example.com' }}\n  secrets: {{ refresh_token: rt-old-0001 }}\n  now: '2026-11-10T12:00:00Z'\n  responses:\n    - {{ method: POST, url: 'https://login.example.com/token', body: {{ access_token: at-2, refresh_token: rt-new-0002 }} }}\n    - {{ url: 'https://api.example.com/v1/events', body: [{{ id: a }}] }}\n{TEST}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n")
    );
    let pkg = Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    let run = engine::run(&pkg.manifest, &pkg.manifest.streams[0], &conn(&pkg), StreamState::default(), NOW, &rt, None).await.unwrap();
    assert_eq!(run.rotated, [("refresh_token".to_string(), "rt-new-0002".to_string())]);
    let reqs = transport.requests.lock().unwrap();
    assert_eq!(String::from_utf8(reqs[0].body.clone().unwrap()).unwrap(), "grant_type=refresh_token&refresh_token=rt-old-0001&client_id=app-1");
    assert_eq!(reqs[1].header("authorization"), Some("Bearer at-2"));
}

#[tokio::test]
async fn jwt_bearer_signs_an_rs256_assertion_with_pkcs8_or_pkcs1_keys() {
    for file in ["test-rsa-pkcs8.pem", "test-rsa-pkcs1.pem"] {
        let pem = std::fs::read_to_string(format!("{}/tests/fixtures/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let yaml = format!(
            "{HEAD}config:\n  - {{ key: base_url, label: URL, type: url }}\n  - {{ key: integration_key, label: Integration key, type: string }}\n  - {{ key: private_key, label: Key, type: text, secret: true }}\n\
             auth:\n  kind: jwt_bearer\n  token_url: 'https://account-d.example.com/oauth/token'\n  private_key: '{{{{ secrets.private_key }}}}'\n  claims: {{ iss: '{{{{ config.integration_key }}}}', sub: user-1, aud: account-d.example.com }}\n  scopes: [signature, impersonation]\n\
             {}mock:\n  config: {{ base_url: 'https://api.example.com', integration_key: ik-1 }}\n  secrets: {{ private_key: {} }}\n  now: '2026-11-10T12:00:00Z'\n  responses:\n    - {{ method: POST, url: 'https://account-d.example.com/oauth/token', body: {{ access_token: at-3 }} }}\n    - {{ url: 'https://api.example.com/v1/events', body: [{{ id: a }}] }}\n{TEST}",
            stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
            serde_json::to_string(&pem).unwrap()
        );
        let pkg = Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
        let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
        let rt = Runtime::new(transport.clone(), false);
        engine::run(&pkg.manifest, &pkg.manifest.streams[0], &conn(&pkg), StreamState::default(), NOW, &rt, None).await.unwrap();
        let reqs = transport.requests.lock().unwrap();
        let form: Vec<(String, String)> = url::form_urlencoded::parse(reqs[0].body.as_ref().unwrap()).into_owned().collect();
        assert_eq!(form[0], ("grant_type".into(), "urn:ietf:params:oauth:grant-type:jwt-bearer".into()));
        let jwt = &form[1].1;
        let parts: Vec<&str> = jwt.split('.').collect();
        let claims: Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims, json!({ "iss": "ik-1", "sub": "user-1", "aud": "account-d.example.com", "scope": "signature impersonation", "iat": NOW / 1000, "exp": NOW / 1000 + 3600 }));
        assert_eq!(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[2]).unwrap().len(), 256, "{file}: a 2048-bit RSA signature");
        assert_eq!(reqs[1].header("authorization"), Some("Bearer at-3"));
    }
}

#[tokio::test]
async fn a_token_exchange_captures_a_base_url_and_a_token() {
    let f = fixture(&format!(
        "auth:\n  kind: token_exchange\n  request: {{ method: POST, url: 'https://login.example.com/logon', body: {{ username: me, password: '{{{{ secrets.token }}}}' }} }}\n  capture: {{ token: '{{{{ response.ManagerToken }}}}', base_url: '{{{{ response.ManagerURL }}}}' }}\n  headers: {{ Authorization: 'basic {{{{ auth.token }}}}' }}\n{}{}",
        stream("    request: { url: '{{ auth.base_url }}/v1/events' }\n"),
        mock("    - { method: POST, url: 'https://login.example.com/logon', body: { ManagerToken: mt-1, ManagerURL: 'https://eu.manager.example.com' } }\n    - { url: 'https://eu.manager.example.com/v1/events', body: [{ id: a }] }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a"]);
    let reqs = f.requests();
    assert_eq!(serde_json::from_slice::<Value>(reqs[0].body.as_ref().unwrap()).unwrap(), json!({ "username": "me", "password": "tok-SECRET-123" }));
    assert_eq!(reqs[1].header("authorization"), Some("basic mt-1"));
}

#[tokio::test]
async fn refused_credentials_fail_the_run_without_echoing_secrets() {
    let f = fixture(&format!(
        "auth: {{ kind: bearer, token: '{{{{ secrets.token }}}}' }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', status: 401, body: { message: 'token tok-SECRET-123 expired' } }")
    ));
    let issue = f.run(StreamState::default(), NOW).await.unwrap_err();
    assert_eq!(issue.kind, IssueKind::Auth);
    assert_eq!(issue.message, "Example refused the credentials (HTTP 401): token [secret] expired");
}

// ---------------------------------------------------------------- pagination

#[tokio::test]
async fn every_pagination_kind_reads_to_the_last_page() {
    let cases: [(&str, &str, &str); 6] = [
        (
            "cursor in the body",
            "    request: { url: '{{ config.base_url }}/v1/events', query: { cursor: '{{ page.cursor }}' } }\n    records: '{{ response.data }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n",
            "    - { url: 'https://api.example.com/v1/events', query: { cursor: c2 }, body: { data: [{ id: b }], next: null } }\n    - { url: 'https://api.example.com/v1/events', body: { data: [{ id: a }], next: c2 } }",
        ),
        (
            "cursor in headers",
            "    request: { url: '{{ config.base_url }}/v1/events', headers: { Search-After: '{{ page.cursor.after }}', Pit-Id: '{{ page.cursor.pit }}' } }\n    pagination: { kind: cursor, next: \"{{ headers['search-after'] ? { pit: headers['pit-id'], after: headers['search-after'] } : null }}\" }\n",
            "    - { url: 'https://api.example.com/v1/events', headers: { Pit-Id: p1, Search-After: s1 }, body: [{ id: a }] }\n    - { url: 'https://api.example.com/v1/events', body: [{ id: b }] }",
        ),
        (
            "next URL, relative",
            "    request: { url: '{{ config.base_url }}/v1/events' }\n    records: '{{ response.items }}'\n    pagination: { kind: next_url, next: '{{ response.links.next }}' }\n",
            "    - { url: 'https://api.example.com/v1/events', query: { page: '2' }, body: { items: [{ id: b }], links: {} } }\n    - { url: 'https://api.example.com/v1/events', body: { items: [{ id: a }], links: { next: '/v1/events?page=2' } } }",
        ),
        (
            "Link header",
            "    request: { url: '{{ config.base_url }}/v1/events' }\n    pagination: { kind: link_header }\n",
            "    - { url: 'https://api.example.com/v1/events', query: { after: x }, body: [{ id: b }] }\n    - { url: 'https://api.example.com/v1/events', headers: { Link: '<https://api.example.com/v1/events?after=x>; rel=\"next\"' }, body: [{ id: a }] }",
        ),
        (
            "offset",
            "    request: { url: '{{ config.base_url }}/v1/events', query: { offset: '{{ page.offset }}', limit: '{{ page.size }}' } }\n    pagination: { kind: offset, size: 2 }\n",
            "    - { url: 'https://api.example.com/v1/events', query: { offset: '0', limit: '2' }, body: [{ id: a }, { id: b }] }\n    - { url: 'https://api.example.com/v1/events', query: { offset: '2' }, body: [{ id: c }] }",
        ),
        (
            "page numbers with a total",
            "    request: { url: '{{ config.base_url }}/v1/events', query: { page: '{{ page.number }}' } }\n    records: '{{ response.rows }}'\n    pagination: { kind: page, size: 1, total_pages: '{{ response.pages }}' }\n",
            "    - { url: 'https://api.example.com/v1/events', query: { page: '1' }, body: { rows: [{ id: a }], pages: 2 } }\n    - { url: 'https://api.example.com/v1/events', query: { page: '2' }, body: { rows: [{ id: b }], pages: 2 } }\n    - { url: 'https://api.example.com/v1/events', query: { page: '3' }, body: { rows: [{ id: never }], pages: 2 } }",
        ),
    ];
    for (name, s, responses) in cases {
        let f = fixture(&format!("{}{}", stream(s), mock(responses)));
        let run = f.run(StreamState::default(), NOW).await.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let mut got = ids(&run);
        got.sort();
        let want: &[&str] = if name == "offset" { &["a", "b", "c"] } else { &["a", "b"] };
        assert_eq!(got, want, "{name}");
        assert!(run.warning.is_none(), "{name}: {:?}", run.warning);
    }
}

#[tokio::test]
async fn a_next_page_on_another_host_is_refused() {
    let f = fixture(&format!(
        "auth: {{ kind: bearer, token: '{{{{ secrets.token }}}}' }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n    records: '{{ response.items }}'\n    pagination: { kind: next_url, next: '{{ response.next }}' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', body: { items: [{ id: a }], next: 'https://collector.attacker.example/steal' } }\n    - { url: 'https://collector.attacker.example/steal', body: { items: [] } }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a"]);
    assert!(run.warning.unwrap().message.contains("another host"));
    assert!(f.requests().iter().all(|r| !r.url.contains("attacker")), "the token never leaves the API's host");
}

#[tokio::test]
async fn graphql_bodies_carry_the_page_and_the_window() {
    let f = fixture(&format!(
        "{}{}",
        stream(
            "    request:\n      method: POST\n      url: '{{ config.base_url }}/graphql'\n      body: { query: 'query($from: String, $after: String) { events }', variables: { from: '{{ window.start | rfc3339 }}', after: '{{ page.cursor }}' } }\n    records: '{{ response.data.events.nodes }}'\n    pagination: { kind: cursor, next: '{{ response.data.events.pageInfo.endCursor }}' }\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h }\n"
        ),
        mock("    - { method: POST, url: 'https://api.example.com/graphql', body_contains: '\"after\":\"e1\"', body: { data: { events: { nodes: [{ id: b, time: '2026-11-10T11:40:00Z' }], pageInfo: { endCursor: null } } } } }\n    - { method: POST, url: 'https://api.example.com/graphql', body: { data: { events: { nodes: [{ id: a, time: '2026-11-10T11:30:00Z' }], pageInfo: { endCursor: e1 } } } } }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a", "b"]);
    let body: Value = serde_json::from_slice(f.requests()[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["variables"], json!({ "from": "2026-11-10T11:00:00.000Z", "after": null }));
}

// ---------------------------------------------------------------- incremental

fn events(n: usize, start_ms: i64, step_ms: i64) -> String {
    let items: Vec<Value> = (0..n).map(|i| json!({ "id": format!("e{i}"), "time": start_ms + i as i64 * step_ms })).collect();
    serde_json::to_string(&items).unwrap()
}

#[tokio::test]
async fn time_windows_move_the_cursor_and_overlap_without_duplicates() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { since: '{{ window.start }}' } }\n    record_id: '{{ record.id }}'\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h, overlap: 5m }\n"),
        mock(&format!(
            "    - {{ url: 'https://api.example.com/v1/events', body: {} }}\n    - {{ url: 'https://api.example.com/v1/events', body: {} }}",
            // First run: one record older than the lookback (skipped), two inside.
            serde_json::to_string(&json!([{ "id": "e0", "time": NOW - 90 * MIN }, { "id": "e1", "time": NOW - 50 * MIN }, { "id": "e2", "time": NOW - 2 * MIN }])).unwrap(),
            // Second run: the overlap answers e2 again, plus a new one.
            serde_json::to_string(&json!([{ "id": "e2", "time": NOW - 2 * MIN }, { "id": "e3", "time": NOW + 30_000 }])).unwrap()
        ))
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["e1", "e2"], "the lookback bounds the first run");
    assert_eq!(f.requests()[0].url, format!("https://api.example.com/v1/events?since={}", NOW - HOUR));
    assert_eq!(first.state.collected_until(), Some(NOW), "the window's end");
    let second = f.run(first.state.clone(), NOW + MIN).await.unwrap();
    assert_eq!(ids(&second), ["e3"], "e2 was sent: skipped though the overlap reads it again");
    assert_eq!(f.requests()[0].url, format!("https://api.example.com/v1/events?since={}", NOW - 5 * MIN));
}

#[tokio::test]
async fn a_newest_first_window_out_of_pages_carries_on_from_its_backlog() {
    // Four pages newest first, two pages a run: the cursor moves only once the window is read to its end.
    let page = |i: usize, next: &str| format!("    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '{i}' }}, body: {{ items: [{{ id: e{i}, time: {} }}], next: {next} }} }}", NOW - (i as i64 + 1) * MIN);
    let responses = [page(1, "'2'"), page(2, "'3'"), page(3, "'4'"), page(4, "null")].join("\n");
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { p: '{{ page.cursor ?? 1 }}', until: '{{ window.end }}' } }\n    records: '{{ response.items }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n    max_pages: 2\n    incremental: { kind: time, record_time: '{{ record.time }}', order: newest_first, lookback: 1h }\n"),
        mock(&responses)
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["e1", "e2"]);
    let part = &first.state.partitions[""];
    assert_eq!(part.cursor, None, "the window isn't read to its end");
    assert_eq!(part.backlog.as_ref().map(|b| (b.end, b.page.clone())), Some((NOW, json!({ "cursor": "3" }))));
    f.requests();
    let second = f.run(first.state.clone(), NOW + 5 * MIN).await.unwrap();
    assert_eq!(ids(&second), ["e3", "e4"]);
    assert!(f.requests().iter().all(|r| r.url.contains(&format!("until={NOW}"))), "the backlog's window, not a new one");
    assert_eq!(second.state.partitions[""].cursor, Some(NOW), "read to its end: the cursor moves to the window's end");
    assert!(second.state.partitions[""].backlog.is_none());
}

#[tokio::test]
async fn an_oldest_first_window_out_of_pages_moves_the_cursor_to_its_last_record() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { offset: '{{ page.offset }}' } }\n    record_id: '{{ record.id }}'\n    pagination: { kind: offset, size: 2 }\n    max_pages: 1\n    incremental: { kind: time, record_time: '{{ record.time }}', order: oldest_first, lookback: 1h }\n"),
        mock(&format!("    - {{ url: 'https://api.example.com/v1/events', query: {{ offset: '0' }}, body: {} }}", events(2, NOW - 50 * MIN, 10 * MIN)))
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["e0", "e1"]);
    assert_eq!(run.state.partitions[""].cursor, Some(NOW - 40 * MIN));
    assert!(run.state.partitions[""].backlog.is_none());
}

#[tokio::test]
async fn a_failed_page_keeps_what_was_read_and_the_window_carries_on() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { p: '{{ page.cursor ?? 1 }}' } }\n    records: '{{ response.items }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h }\n"),
        mock(&format!(
            "    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '1' }}, body: {{ items: [{{ id: a, time: {} }}], next: '2' }} }}\n    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '2' }}, status: 503, body: {{ message: busy }} }}",
            NOW - MIN
        ))
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a"]);
    assert_eq!(run.warning.as_ref().map(|w| w.message.as_str()), Some("Example answered HTTP 503: busy"));
    let part = &run.state.partitions[""];
    assert_eq!((part.cursor, part.backlog.as_ref().map(|b| b.page.clone())), (None, Some(json!({ "cursor": "2" }))));
}

#[tokio::test]
async fn a_window_reaching_the_offset_cap_is_split_and_read_again() {
    // At most 2 records per window (offset ≤ 1): the hour holds 3, so it is read as two halves.
    let window = |since: i64, until: i64, body: &str| format!("    - {{ url: 'https://api.example.com/v1/events', query: {{ from: '{since}', to: '{until}', offset: '0' }}, body: {body} }}");
    let (start, mid) = (NOW - HOUR, NOW - HOUR / 2);
    let responses = [
        window(start, NOW, &events(2, start + MIN, MIN)),
        window(start, mid, &serde_json::to_string(&json!([{ "id": "e0", "time": start + MIN }])).unwrap()),
        window(mid, NOW, &serde_json::to_string(&json!([{ "id": "late", "time": NOW - MIN }])).unwrap()),
    ]
    .join("\n");
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { from: '{{ window.start }}', to: '{{ window.end }}', offset: '{{ page.offset }}' } }\n    record_id: '{{ record.id }}'\n    pagination: { kind: offset, size: 2, max: 1 }\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h }\n"),
        mock(&responses)
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["e0", "e1", "late"], "the first window's records are kept once, the halves add the rest");
    assert_eq!(run.state.collected_until(), Some(NOW));
}

#[tokio::test]
async fn new_records_are_sent_once() {
    let f = fixture(&format!(
        "streams:\n  - id: breaches\n    name: Breaches\n    description: Breached accounts.\n    request: {{ url: '{{{{ config.base_url }}}}/v1/breaches' }}\n    records: \"{{{{ response | entries | explode('value') }}}}\"\n    record_id: \"{{{{ record.key + '/' + record.value }}}}\"\n    incremental: {{ kind: new_records }}\n    output:\n      logs: {{ message: \"{{{{ record.key + '/' + record.value }}}}\" }}\n{}",
        mock("    - { url: 'https://api.example.com/v1/breaches', body: { jane: [Adobe, Gawker], sam: [Adobe] } }\n    - { url: 'https://api.example.com/v1/breaches', body: { jane: [Adobe, Gawker, Canva], sam: [Adobe] } }")
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["jane/Adobe", "jane/Gawker", "sam/Adobe"]);
    let second = f.run(first.state, NOW + HOUR).await.unwrap();
    assert_eq!(ids(&second), ["jane/Canva"]);
}

#[tokio::test]
async fn token_checkpoints_are_saved_after_the_run() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { since: '{{ cursor.token }}' } }\n    records: '{{ response.events }}'\n    incremental: { kind: token, initial: '0', next: '{{ response.checkpoint }}' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', query: { since: '0' }, body: { events: [{ id: a }], checkpoint: 'cp-1' } }\n    - { url: 'https://api.example.com/v1/events', query: { since: cp-1 }, body: { events: [{ id: b }], checkpoint: null } }")
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(first.state.partitions[""].token, Some(json!("cp-1")));
    let second = f.run(first.state.clone(), NOW + MIN).await.unwrap();
    assert_eq!(ids(&second), ["b"]);
    assert_eq!(second.state.partitions[""].token, Some(json!("cp-1")), "no new checkpoint: the old one stays");
}

#[tokio::test]
async fn for_each_items_have_their_own_cursor_and_lookups_enrich_records() {
    let f = fixture(&format!(
        "tables: {{ kinds: {{ '1000': 'User logged in' }} }}\n{}{}",
        stream(
            "    for_each: '{{ lookups.projects | keys }}'\n    lookups:\n      - { name: projects, request: { url: '{{ config.base_url }}/v1/projects' }, key: '{{ record.slug }}' }\n      - { name: members, request: { url: '{{ config.base_url }}/v1/members' }, records: '{{ response.data }}', key: '{{ record.id }}', value: '{{ record.name }}' }\n    request: { url: \"{{ config.base_url + '/v1/projects/' + item + '/events' }}\" }\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h }\n"
        )
        .replace("'{{ record.id }}' }", "\"{{ item + ':' + tables.kinds[record.type | str] + ' by ' + lookups.members[record.actor] }}\" }"),
        mock(&format!(
            "    - {{ url: 'https://api.example.com/v1/projects', body: [{{ slug: shop }}, {{ slug: billing }}] }}\n    - {{ url: 'https://api.example.com/v1/members', body: {{ data: [{{ id: u1, name: Jane }}] }} }}\n    - {{ url: 'https://api.example.com/v1/projects/shop/events', body: [{{ type: 1000, actor: u1, time: {} }}] }}\n    - {{ url: 'https://api.example.com/v1/projects/billing/events', status: 403, body: {{}} }}",
            NOW - MIN
        ))
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["shop:User logged in by Jane"]);
    assert_eq!(run.state.partitions["shop"].cursor, Some(NOW));
    assert_eq!(run.state.partitions["billing"].cursor, None, "its failure keeps its own state");
    assert_eq!(run.warning.map(|w| w.kind), Some(IssueKind::AccessDenied));
}

#[tokio::test]
async fn async_exports_are_created_polled_and_read_across_runs() {
    let f = fixture(&format!(
        "{}{}",
        "streams:\n  - id: vulns\n    name: Vulnerabilities\n    description: Exported.\n    pagination:\n      kind: async_export\n      create: { method: POST, url: '{{ config.base_url }}/vulns/export', body: { since: '{{ window.start }}' } }\n      id: '{{ response.export_uuid }}'\n      status: { url: \"{{ config.base_url + '/vulns/export/' + export.id + '/status' }}\" }\n      ready: \"{{ response.status == 'FINISHED' }}\"\n      chunks: '{{ response.chunks_available }}'\n      chunk: { url: \"{{ config.base_url + '/vulns/export/' + export.id + '/chunks/' + chunk }}\" }\n    record_id: '{{ record.id }}'\n    incremental: { kind: time, record_time: '{{ record.seen }}', lookback: 1d }\n    output:\n      logs: { message: '{{ record.id }}' }\n",
        mock(&format!(
            "    - {{ method: POST, url: 'https://api.example.com/vulns/export', body: {{ export_uuid: x1 }} }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/status', body: {{ status: PROCESSING, chunks_available: [] }} }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/status', body: {{ status: FINISHED, chunks_available: [1, 2] }} }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/chunks/1', body: [{{ id: v1, seen: {t} }}] }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/chunks/2', body: [{{ id: v2, seen: {t} }}] }}",
            t = NOW - HOUR
        ))
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(first.records, 0, "still processing");
    assert!(first.state.partitions[""].export.is_some());
    let second = f.run(first.state, NOW + 5 * MIN).await.unwrap();
    assert_eq!(ids(&second), ["v1", "v2"]);
    let part = &second.state.partitions[""];
    assert_eq!((part.export.is_none(), part.cursor), (true, Some(NOW)), "done: the next export starts where this one's window ended");
    assert_eq!(f.requests().iter().filter(|r| r.url.ends_with("/vulns/export")).count(), 1);
}

#[tokio::test]
async fn prometheus_samples_become_metrics_with_their_labels() {
    let f = fixture(&format!(
        "{}{}",
        "streams:\n  - id: metrics\n    name: Metrics\n    description: Scraped.\n    request: { url: '{{ config.base_url }}/metrics', format: prometheus }\n    filter: \"{{ record.type != 'histogram' }}\"\n    output:\n      metrics:\n        - { name: \"{{ 'streamnative.' + record.name }}\", value: '{{ record.value }}', tags_from: '{{ record.labels }}', tags: { org: '{{ connection.instance }}' } }\n",
        mock("    - { url: 'https://api.example.com/metrics', body: \"# TYPE pulsar_msg_rate_in gauge\\npulsar_msg_rate_in{cluster=\\\"shop\\\"} 12.5\\n# TYPE h histogram\\nh_bucket{le=\\\"1\\\"} 3\\n\" }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    let p = &run.mapped.points;
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].name.as_str(), p[0].value, p[0].time_ms), ("streamnative.pulsar_msg_rate_in", 12.5, NOW));
    assert_eq!(p[0].tags, [("cluster".to_string(), "shop".to_string()), ("org".to_string(), "example".to_string())]);
}

#[tokio::test]
async fn a_used_up_quota_stops_the_run_with_its_progress() {
    let pkg = package(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { p: '{{ page.cursor ?? 1 }}' } }\n    records: '{{ response.items }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n    incremental: { kind: time, record_time: '{{ record.time }}', lookback: 1h }\n"),
        mock(&format!(
            "    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '1' }}, body: {{ items: [{{ id: a, time: {t} }}], next: '2' }} }}\n    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '2' }}, body: {{ items: [{{ id: b, time: {t} }}], next: null }} }}",
            t = NOW - MIN
        ))
    ));
    let rt = Runtime::new(Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap())), false);
    let run = engine::run(&pkg.manifest, &pkg.manifest.streams[0], &conn(&pkg), StreamState::default(), NOW, &rt, Some(1)).await.unwrap();
    assert_eq!((run.records, run.requests), (1, 1));
    assert_eq!(run.warning.map(|w| w.kind), Some(IssueKind::Throttled));
    assert!(run.state.partitions[""].backlog.is_some());
}

#[tokio::test]
async fn error_rules_name_the_problem_in_the_vendors_words() {
    let f = fixture(&format!(
        "errors:\n  - {{ status: [403], when: \"{{{{ response.code == 'PLAN' }}}}\", kind: access_denied, message: 'The audit log needs the Enterprise plan' }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', status: 403, body: { code: PLAN } }")
    ));
    let issue = f.run(StreamState::default(), NOW).await.unwrap_err();
    assert_eq!((issue.kind, issue.message.as_str()), (IssueKind::AccessDenied, "The audit log needs the Enterprise plan"));
}

#[tokio::test]
async fn discovery_runs_first_and_is_cached() {
    let f = fixture(&format!(
        "discover:\n  - {{ name: whoami, request: {{ url: 'https://api.example.com/whoami' }}, value: '{{{{ response.apiHosts.dataRegion }}}}' }}\n{}{}",
        stream("    request: { url: '{{ discovery.whoami }}/siem/v1/events' }\n"),
        mock("    - { url: 'https://api.example.com/whoami', body: { apiHosts: { dataRegion: 'https://api-eu01.example.com' } } }\n    - { url: 'https://api-eu01.example.com/siem/v1/events', body: [{ id: a }] }")
    ));
    f.run(StreamState::default(), NOW).await.unwrap();
    let run = f.run(StreamState::default(), NOW + MIN).await.unwrap();
    assert_eq!(ids(&run), ["a"]);
    assert_eq!(f.requests().iter().filter(|r| r.url.contains("whoami")).count(), 1);
}

// ---------------------------------------------------------------- validation

#[test]
fn invalid_packages_name_every_problem() {
    let problems = invalid(
        "auth: { kind: bearer, token: '{{ config.token }}' }\nstreams:\n  - id: Events\n    name: E\n    description: E.\n    interval: 30s\n    request: { url: '{{ confg.base_url }}' }\n    incremental: { kind: new_records }\n    output:\n      logs: { message: '{{ secrets.token }}' }\n",
    );
    let text = problems.join("\n");
    for needle in [
        "auth.token: config.token is not a config field (token is a secret: secrets.token)",
        "streams[0].id: \"Events\" must be lowercase",
        "streams[0].interval: must be at least 1m",
        "streams[0].request.url: unknown variable \"confg\"",
        "streams[0].incremental: new_records needs the stream's record_id",
        "streams[0].output.logs.message: unknown variable \"secrets\"",
        "test: a package with streams needs a connection test",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(invalid("streams: []\nnope: 1").join("\n").contains("unknown field `nope`"));
}

// ---------------------------------------------------------------- downloads, JWTs, setup calls

#[tokio::test]
async fn index_entries_are_downloaded_and_their_csv_rows_mapped() {
    let f = fixture(&format!(
        "{}{}",
        "streams:\n  - id: event_log_files\n    name: Event log files\n    description: Salesforce-style index then download.\n    request: { url: '{{ config.base_url }}/query', query: { q: \"SELECT Id FROM EventLogFile\" } }\n    records: '{{ response.records }}'\n    expand:\n      request: { url: \"{{ config.base_url + parent.LogFile }}\", format: csv }\n    record_id: \"{{ parent.Id + ':' + record.REQUEST_ID }}\"\n    incremental: { kind: new_records }\n    output:\n      logs: { message: \"{{ parent.EventType + ' ' + record.URI }}\" }\n",
        mock("    - { url: 'https://api.example.com/query', body: { records: [{ Id: f1, EventType: API, LogFile: '/files/f1' }] } }\n    - { url: 'https://api.example.com/files/f1', body: \"REQUEST_ID,URI\\nr1,/services/data\\nr2,/services/apexrest\\n\" }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["API /services/data", "API /services/apexrest"]);
    let again = f.run(run.state, NOW + MIN).await.unwrap();
    assert_eq!(again.records, 0);
}

#[tokio::test]
async fn self_signed_jwts_carry_the_keys_fingerprint() {
    let pem = std::fs::read_to_string(format!("{}/tests/fixtures/test-rsa-pkcs8.pem", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let fingerprint = oc_connectors::auth::public_key_fingerprint(&pem).unwrap();
    assert!(fingerprint.starts_with("SHA256:") && fingerprint.len() == 51, "{fingerprint}");
    let yaml = format!(
        "{HEAD}config:\n  - {{ key: base_url, label: URL, type: url }}\n  - {{ key: private_key, label: Key, type: text, secret: true }}\n\
         auth:\n  kind: jwt\n  private_key: '{{{{ secrets.private_key }}}}'\n  claims: {{ iss: \"{{{{ 'SHOP.SVC.' + key.fingerprint }}}}\", sub: SHOP.SVC }}\n  headers: {{ Authorization: 'Bearer {{{{ auth.jwt }}}}', X-Snowflake-Authorization-Token-Type: KEYPAIR_JWT }}\n\
         {}mock:\n  config: {{ base_url: 'https://api.example.com' }}\n  secrets: {{ private_key: {} }}\n  now: '2026-11-10T12:00:00Z'\n  responses:\n    - {{ url: 'https://api.example.com/v1/events', body: [{{ id: a }}] }}\n{TEST}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        serde_json::to_string(&pem).unwrap()
    );
    let pkg = Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    engine::run(&pkg.manifest, &pkg.manifest.streams[0], &conn(&pkg), StreamState::default(), NOW, &rt, None).await.unwrap();
    let reqs = transport.requests.lock().unwrap();
    assert_eq!(reqs[0].header("x-snowflake-authorization-token-type"), Some("KEYPAIR_JWT"));
    let jwt = reqs[0].header("authorization").unwrap().strip_prefix("Bearer ").unwrap();
    let claims: Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1).unwrap()).unwrap()).unwrap();
    assert_eq!(claims["iss"], format!("SHOP.SVC.{fingerprint}"));
    assert_eq!(claims["exp"].as_i64(), Some(NOW / 1000 + 3600));
}

#[tokio::test]
async fn a_token_exchange_can_sign_its_login_with_a_jwt() {
    // GitHub Apps: the app's JWT buys an installation token.
    let pem = std::fs::read_to_string(format!("{}/tests/fixtures/test-rsa-pkcs1.pem", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let yaml = format!(
        "{HEAD}config:\n  - {{ key: base_url, label: URL, type: url }}\n  - {{ key: app_id, label: App id, type: string }}\n  - {{ key: private_key, label: Key, type: text, secret: true }}\n\
         auth:\n  kind: token_exchange\n  jwt: {{ private_key: '{{{{ secrets.private_key }}}}', claims: {{ iss: '{{{{ config.app_id }}}}' }}, lifetime: 9m }}\n  request: {{ method: POST, url: 'https://api.example.com/app/installations/7/access_tokens', headers: {{ Authorization: 'Bearer {{{{ auth.jwt }}}}' }} }}\n  capture: {{ token: '{{{{ response.token }}}}' }}\n  headers: {{ Authorization: 'token {{{{ auth.token }}}}' }}\n\
         {}mock:\n  config: {{ base_url: 'https://api.example.com', app_id: '1234' }}\n  secrets: {{ private_key: {} }}\n  now: '2026-11-10T12:00:00Z'\n  responses:\n    - {{ method: POST, url: 'https://api.example.com/app/installations/7/access_tokens', status: 201, body: {{ token: ghs_installation }} }}\n    - {{ url: 'https://api.example.com/v1/events', body: [{{ id: a }}] }}\n{TEST}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        serde_json::to_string(&pem).unwrap()
    );
    let pkg = Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    engine::run(&pkg.manifest, &pkg.manifest.streams[0], &conn(&pkg), StreamState::default(), NOW, &rt, None).await.unwrap();
    let reqs = transport.requests.lock().unwrap();
    assert!(reqs[0].header("authorization").unwrap().starts_with("Bearer ey"), "the app's JWT");
    assert_eq!(reqs[1].header("authorization"), Some("token ghs_installation"));
}

#[tokio::test]
async fn setup_calls_report_their_message_and_accept_their_ok_statuses() {
    let pkg = package(&format!(
        "on_connect:\n  - {{ request: {{ method: POST, url: 'https://api.example.com/subscriptions/start' }}, ok_status: [409], message: \"Subscribed ({{{{ status }}}})\" }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { method: POST, url: 'https://api.example.com/subscriptions/start', status: 409, body: { error: exists } }\n    - { url: 'https://api.example.com/v1/events', body: [] }")
    ));
    let rt = Runtime::new(Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap())), false);
    let message = engine::call(&pkg.manifest, &pkg.manifest.on_connect[0], &conn(&pkg), Default::default(), NOW, &rt).await.unwrap();
    assert_eq!(message, "Subscribed (409)");
}

// ---------------------------------------------------------------- framework additions (package batches 1-4)

#[tokio::test]
async fn a_newest_first_list_stops_at_the_first_record_older_than_the_window() {
    // The API ignores time: pages newest first. The second page reaches past the window: no third page.
    let page = |p: &str, items: Value, next: &str| format!("    - {{ url: 'https://api.example.com/v1/events', query: {{ p: '{p}' }}, body: {{ items: {items}, next: {next} }} }}");
    let responses = [
        page("1", json!([{ "id": "e1", "time": NOW - MIN }]), "'2'"),
        page("2", json!([{ "id": "e2", "time": NOW - 30 * MIN }, { "id": "old", "time": NOW - 2 * HOUR }]), "'3'"),
        page("3", json!([{ "id": "older", "time": NOW - 3 * HOUR }]), "null"),
    ]
    .join("\n");
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { p: '{{ page.cursor ?? 1 }}' } }\n    records: '{{ response.items }}'\n    record_id: '{{ record.id }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n    incremental: { kind: time, record_time: '{{ record.time }}', order: newest_first, lookback: 1h }\n"),
        mock(&responses)
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["e1", "e2"]);
    assert_eq!(f.requests().len(), 2, "the rest of the list is older");
    let part = &run.state.partitions[""];
    assert_eq!((part.cursor, part.backlog.is_none()), (Some(NOW), true), "the window is complete");
}

#[tokio::test]
async fn newest_first_new_records_stop_at_one_sent_before() {
    let page = |p: &str, ids: &[&str], next: &str| format!("    - {{ url: 'https://api.example.com/v1/alerts', query: {{ p: '{p}' }}, body: {{ items: {}, next: {next} }} }}", json!(ids.iter().map(|i| json!({ "id": i })).collect::<Vec<_>>()));
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/alerts', query: { p: '{{ page.cursor ?? 1 }}' } }\n    records: '{{ response.items }}'\n    record_id: '{{ record.id }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n    incremental: { kind: new_records, order: newest_first }\n"),
        mock(&[page("1", &["c", "b"], "'2'"), page("2", &["a"], "null")].join("\n"))
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["c", "b", "a"]);
    f.requests();
    let second = f.run(first.state, NOW + MIN).await.unwrap();
    assert_eq!(second.records, 0);
    assert_eq!(f.requests().len(), 1, "the first page had records sent before: no second page");
}

#[tokio::test]
async fn auth_is_chosen_by_a_config_field() {
    let body = format!(
        "  - {{ key: method, label: Method, type: select, default: token, options: [{{ value: token, label: Token }}, {{ value: oauth, label: OAuth }}] }}\n\
         auth:\n  kind: choice\n  field: method\n  options:\n    token: {{ kind: api_key, name: Authorization, value: 'SSWS {{{{ secrets.token }}}}' }}\n    oauth: {{ kind: bearer, token: 'oauth-{{{{ secrets.token }}}}' }}\n{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', body: [{ id: a }] }")
    );
    let pkg = Package::from_files([("connector.yaml".to_string(), format!("{HEAD}{CONFIG}{body}\n{TEST}"))].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    let mut c = conn(&pkg);
    engine::run(&pkg.manifest, &pkg.manifest.streams[0], &c, StreamState::default(), NOW, &rt, None).await.unwrap();
    c.config.insert("method".into(), json!("oauth"));
    engine::run(&pkg.manifest, &pkg.manifest.streams[0], &c, StreamState::default(), NOW, &rt, None).await.unwrap();
    let reqs = std::mem::take(&mut *transport.requests.lock().unwrap());
    assert_eq!((reqs[0].header("authorization"), reqs[1].header("authorization")), (Some("SSWS tok-SECRET-123"), Some("Bearer oauth-tok-SECRET-123")));
    let bad = invalid("auth: { kind: choice, field: nope, options: { a: { kind: none } } }\nstreams: []\n");
    assert!(bad.join("\n").contains("auth.field: nope is not a config field"), "{bad:?}");
}

#[tokio::test]
async fn lookups_can_read_the_item_and_be_turned_off() {
    let f = fixture(&format!(
        "{}{}",
        stream(
            "    for_each: \"{{ ['shop', 'billing'] }}\"\n    lookups:\n      - { name: owners, request: { url: \"{{ config.base_url + '/v1/projects/' + item + '/owners' }}\" }, key: '{{ record.id }}', value: '{{ record.name }}' }\n      - { name: teams, when: '{{ config.base_url == \"https://other.example.com\" }}', request: { url: '{{ config.base_url }}/v1/teams' }, key: '{{ record.slug }}' }\n    request: { url: \"{{ config.base_url + '/v1/projects/' + item + '/events' }}\" }\n"
        )
        .replace("'{{ record.id }}' }\n", "\"{{ item + ':' + lookups.owners[record.owner] + ':' + (lookups.teams | keys | len) }}\" }\n"),
        mock("    - { url: 'https://api.example.com/v1/projects/shop/owners', body: [{ id: u1, name: Jane }] }\n    - { url: 'https://api.example.com/v1/projects/billing/owners', body: [{ id: u1, name: Sam }] }\n    - { url: 'https://api.example.com/v1/projects/shop/events', body: [{ owner: u1 }] }\n    - { url: 'https://api.example.com/v1/projects/billing/events', body: [{ owner: u1 }] }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["shop:Jane:0", "billing:Sam:0"]);
    assert!(!f.requests().iter().any(|r| r.url.ends_with("/v1/teams")), "turned off by its config");
}

#[tokio::test]
async fn discovery_reads_every_page() {
    let f = fixture(&format!(
        "discover:\n  - {{ name: tenants, request: {{ url: 'https://api.example.com/tenants', query: {{ page: '{{{{ page.cursor ?? 1 }}}}' }} }}, records: '{{{{ response.data }}}}', pagination: {{ kind: cursor, next: '{{{{ response.next }}}}' }}, value: \"{{{{ records | pluck('id') }}}}\" }}\n{}{}",
        stream("    for_each: '{{ discovery.tenants }}'\n    request: { url: \"{{ config.base_url + '/v1/' + item + '/events' }}\" }\n"),
        mock("    - { url: 'https://api.example.com/tenants', query: { page: '1' }, body: { data: [{ id: t1 }], next: '2' } }\n    - { url: 'https://api.example.com/tenants', query: { page: '2' }, body: { data: [{ id: t2 }], next: null } }\n    - { url: 'https://api.example.com/v1/t1/events', body: [{ id: a }] }\n    - { url: 'https://api.example.com/v1/t2/events', body: [{ id: b }] }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a", "b"]);
}

#[tokio::test]
async fn index_entries_sent_before_are_not_downloaded_again() {
    let f = fixture(&format!(
        "{}{}",
        "streams:\n  - id: alerts\n    name: Alerts\n    description: Listed, then each one's detail.\n    request: { url: '{{ config.base_url }}/alerts' }\n    expand:\n      request: { url: \"{{ config.base_url + '/alerts/' + parent.id }}\" }\n    record_id: '{{ parent.id }}'\n    incremental: { kind: new_records }\n    output:\n      logs: { message: \"{{ parent.id + ' ' + record.severity }}\" }\n",
        mock("    - { url: 'https://api.example.com/alerts', body: [{ id: a1 }, { id: a2 }] }\n    - { url: 'https://api.example.com/alerts/a1', body: { severity: high } }\n    - { url: 'https://api.example.com/alerts/a2', body: { severity: low } }")
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["a1 high", "a2 low"]);
    f.requests();
    let second = f.run(first.state, NOW + MIN).await.unwrap();
    assert_eq!(second.records, 0);
    assert_eq!(f.requests().len(), 1, "only the list: the details were read before");
}

#[tokio::test]
async fn error_rules_can_read_a_successful_answer() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { method: POST, url: '{{ config.base_url }}/graphql', body: { query: '{ events { id } }' } }\n    records: '{{ response.data.events }}'\n    errors:\n      - { status: [200], when: '{{ response.errors | len > 0 }}', kind: access_denied, message: \"GraphQL: {{ response.errors[0].message }}\" }\n"),
        mock("    - { method: POST, url: 'https://api.example.com/graphql', body: { data: null, errors: [{ message: 'permission denied for events' }] } }")
    ));
    let issue = f.run(StreamState::default(), NOW).await.unwrap_err();
    assert_eq!((issue.kind, issue.message.as_str()), (IssueKind::AccessDenied, "GraphQL: permission denied for events"));
}

#[tokio::test]
async fn next_urls_may_change_the_method() {
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { method: POST, url: '{{ config.base_url }}/statements', body: { statement: 'SELECT 1' } }\n    records: '{{ response.rows }}'\n    pagination: { kind: next_url, next: '{{ response.next }}', method: GET }\n"),
        mock("    - { method: POST, url: 'https://api.example.com/statements', body: { rows: [{ id: a }], next: '/statements/s1/chunks/1' } }\n    - { url: 'https://api.example.com/statements/s1/chunks/1', body: { rows: [{ id: b }], next: null } }")
    ));
    let run = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&run), ["a", "b"]);
    let reqs = f.requests();
    assert_eq!((reqs[1].body.is_none(), reqs[1].header("content-type")), (true, None));
}

#[tokio::test]
async fn exports_dedupe_across_exports_and_keep_to_their_window() {
    let f = fixture(&format!(
        "{}{}",
        "streams:\n  - id: vulns\n    name: Vulnerabilities\n    description: Exported.\n    pagination:\n      kind: async_export\n      create: { method: POST, url: '{{ config.base_url }}/vulns/export' }\n      id: '{{ response.export_uuid }}'\n      status: { url: \"{{ config.base_url + '/vulns/export/' + export.id + '/status' }}\" }\n      ready: \"{{ response.status == 'FINISHED' }}\"\n      chunks: '{{ response.chunks_available }}'\n      chunk: { url: \"{{ config.base_url + '/vulns/export/' + export.id + '/chunks/' + chunk }}\" }\n    record_id: '{{ record.id }}'\n    incremental: { kind: time, record_time: '{{ record.seen }}', lookback: 1d }\n    output:\n      logs: { message: '{{ record.id }}' }\n",
        mock(&format!(
            "    - {{ method: POST, url: 'https://api.example.com/vulns/export', body: {{ export_uuid: x1 }} }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/status', body: {{ status: FINISHED, chunks_available: [1] }} }}\n    - {{ url: 'https://api.example.com/vulns/export/x1/chunks/1', body: [{{ id: v1, seen: {t} }}, {{ id: v0, seen: {old} }}] }}",
            t = NOW - HOUR,
            old = NOW - 3 * 86_400_000
        ))
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(ids(&first), ["v1"], "v0 is older than the export's window");
    let second = f.run(first.state, NOW + HOUR).await.unwrap();
    assert_eq!(second.records, 0, "the next export answers v1 again: sent once");
}

#[test]
fn yaml_merge_keys_are_applied() {
    let yaml = format!(
        "{HEAD}{CONFIG}x-defaults: &defaults\n  interval: 5m\n  request: {{ url: 'https://api.example.com/v1/events' }}\n  output:\n    logs: {{ message: '{{{{ record.id }}}}' }}\nstreams:\n  - <<: *defaults\n    id: events\n    name: Events\n    description: Events.\n{TEST}"
    );
    let problems = Package::from_files([("connector.yaml".to_string(), yaml.clone())].into(), Origin::Bundled).err().unwrap_or_default();
    // Top-level `x-` keys aren't manifest fields: an anchor lives in a stream instead.
    assert!(problems.iter().any(|p| p.contains("x-defaults")), "{problems:?}");
    let yaml = format!(
        "{HEAD}{CONFIG}streams:\n  - &events\n    id: events\n    name: Events\n    description: Events.\n    interval: 5m\n    request: {{ url: 'https://api.example.com/v1/events' }}\n    output:\n      logs: {{ message: '{{{{ record.id }}}}' }}\n  - <<: *events\n    id: audit\n    name: Audit\n{TEST}"
    );
    let pkg = Package::from_files([("connector.yaml".to_string(), yaml)].into(), Origin::Bundled).unwrap_or_else(|e| panic!("{e:#?}"));
    let audit = &pkg.manifest.streams[1];
    assert_eq!((audit.id.as_str(), audit.interval.as_deref(), audit.request.as_ref().map(|r| r.url.as_str())), ("audit", Some("5m"), Some("https://api.example.com/v1/events")));
}

#[tokio::test]
async fn the_most_specific_recorded_answer_wins_on_reruns_too() {
    // The general answer is listed first; a request with the cursor gets the specific one, run after run.
    let f = fixture(&format!(
        "{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events', query: { cursor: '{{ page.cursor }}' } }\n    records: '{{ response.items }}'\n    pagination: { kind: cursor, next: '{{ response.next }}' }\n"),
        mock("    - { url: 'https://api.example.com/v1/events', body: { items: [{ id: a }], next: c2 } }\n    - { url: 'https://api.example.com/v1/events', query: { cursor: c2 }, body: { items: [{ id: b }], next: null } }")
    ));
    for _ in 0..2 {
        assert_eq!(ids(&f.run(StreamState::default(), NOW).await.unwrap()), ["a", "b"]);
    }
}

/// Bucketed APIs revising settled buckets: a gauge per bucket (`time`), read again within `overlap`, with the value in
/// the record's id, sends a changed bucket again (at its own time) and an unchanged one never.
#[tokio::test]
async fn revised_buckets_are_sent_again_at_their_time() {
    let bucket = |t: i64, n: u32| json!({ "start": t, "count": n });
    let f = fixture(&format!(
        "streams:\n  - id: usage\n    name: Usage\n    description: Hourly buckets.\n    request: {{ url: '{{{{ config.base_url }}}}/v1/usage' }}\n    record_id: \"{{{{ record.start + ':' + record.count }}}}\"\n    incremental: {{ kind: time, record_time: '{{{{ record.start }}}}', lookback: 3h, overlap: 2h }}\n    output:\n      metrics:\n        - {{ name: example.requests, value: '{{{{ record.count }}}}', time: '{{{{ record.start }}}}' }}\n{}",
        mock(&format!(
            "    - {{ url: 'https://api.example.com/v1/usage', body: {} }}\n    - {{ url: 'https://api.example.com/v1/usage', body: {} }}",
            json!([bucket(NOW - 2 * HOUR, 10), bucket(NOW - HOUR, 4)]),
            json!([bucket(NOW - 2 * HOUR, 10), bucket(NOW - HOUR, 7)])
        ))
    ));
    let first = f.run(StreamState::default(), NOW).await.unwrap();
    assert_eq!(first.mapped.points.iter().map(|p| (p.time_ms, p.value)).collect::<Vec<_>>(), [(NOW - 2 * HOUR, 10.0), (NOW - HOUR, 4.0)]);
    let second = f.run(first.state, NOW + MIN).await.unwrap();
    assert_eq!(second.mapped.points.iter().map(|p| (p.time_ms, p.value)).collect::<Vec<_>>(), [(NOW - HOUR, 7.0)], "the late data of its bucket");
}

// ---------------------------------------------------------------- OAuth authorization code

const CODE_AUTH: &str = "auth: { kind: oauth2_authorization_code, authorize_url: 'https://login.example.com/oauth/authorize', token_url: 'https://login.example.com/oauth/token', scopes: [audit.read, offline_access], authorize_params: { prompt: consent } }\n";

#[tokio::test]
async fn authorization_code_connections_refresh_their_stored_token() {
    let pkg = package(&format!(
        "{CODE_AUTH}{}{}",
        stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"),
        mock("    - { method: POST, url: 'https://login.example.com/oauth/token', body: { access_token: at-1, refresh_token: rt-2, expires_in: 3600 } }\n    - { url: 'https://api.example.com/v1/events', body: [{ id: a }] }")
    ));
    let transport = Arc::new(MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap()));
    let rt = Runtime::new(transport.clone(), false);
    let mut c = conn(&pkg);
    let stream = &pkg.manifest.streams[0];
    let issue = engine::run(&pkg.manifest, stream, &c, StreamState::default(), NOW, &rt, None).await.unwrap_err();
    assert_eq!(issue.kind, IssueKind::Auth, "{}", issue.message);
    assert!(issue.message.contains("Connect"), "{}", issue.message);
    for (k, v) in [("oauth_refresh_token", "rt-1"), ("oauth_client_id", "ocapp"), ("oauth_client_secret", "ocapp-secret")] {
        c.secrets.insert(k.into(), json!(v));
    }
    let run = engine::run(&pkg.manifest, stream, &c, StreamState::default(), NOW, &rt, None).await.unwrap();
    assert_eq!(run.rotated, [("oauth_refresh_token".to_string(), "rt-2".to_string())], "a rotated refresh token is saved");
    let reqs = std::mem::take(&mut *transport.requests.lock().unwrap());
    assert_eq!(String::from_utf8(reqs[0].body.clone().unwrap()).unwrap(), "grant_type=refresh_token&refresh_token=rt-1");
    assert_eq!(reqs[0].header("authorization"), Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("ocapp:ocapp-secret")).as_str()));
    assert_eq!(reqs[1].header("authorization"), Some("Bearer at-1"));
    // A vendor without refresh tokens: the stored access token.
    let mut c = conn(&pkg);
    c.secrets.insert("oauth_access_token".into(), json!("long-lived"));
    engine::run(&pkg.manifest, stream, &c, StreamState::default(), NOW, &Runtime::new(transport.clone(), false), None).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().last().unwrap().header("authorization"), Some("Bearer long-lived"));
}

#[tokio::test]
async fn the_consent_page_and_the_code_exchange_carry_pkce() {
    use oc_connectors::auth;
    use oc_connectors::manifest::Auth;
    let pkg = package(&format!("{CODE_AUTH}{}{}", stream("    request: { url: '{{ config.base_url }}/v1/events' }\n"), mock("    - { method: POST, url: 'https://login.example.com/oauth/token', body: { access_token: at-1, refresh_token: rt-1 } }")));
    let Auth::Oauth2AuthorizationCode(code) = &pkg.manifest.auth else { panic!() };
    let mut scope = serde_json::Map::new();
    scope.insert("config".into(), json!({}));
    scope.insert("secrets".into(), json!({}));
    assert!(auth::authorize_url(code, &scope, "https://oc.example.com/cb", "st", None).unwrap_err().contains("OC_OAUTH_"), "no app: says how to register one");
    scope.insert("secrets".into(), json!({ "oauth_client_id": "ocapp", "oauth_client_secret": "s3" }));
    let (verifier, challenge) = auth::pkce();
    assert_eq!((verifier.len(), challenge.len()), (43, 43));
    assert_ne!(auth::pkce().0, verifier);
    let url = url::Url::parse(&auth::authorize_url(code, &scope, "https://oc.example.com/cb", "ocoa_state", Some(&challenge)).unwrap()).unwrap();
    let q: std::collections::BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/oauth/authorize");
    for (k, v) in [("response_type", "code"), ("client_id", "ocapp"), ("redirect_uri", "https://oc.example.com/cb"), ("state", "ocoa_state"), ("scope", "audit.read offline_access"), ("code_challenge", challenge.as_str()), ("code_challenge_method", "S256"), ("prompt", "consent")] {
        assert_eq!(q.get(k).map(String::as_str), Some(v), "{k}");
    }
    let transport = MockTransport::new(&pkg, pkg.manifest.mock.as_ref().unwrap());
    let answer = auth::exchange_code(code, &scope, "the-code", "https://oc.example.com/cb", Some(&verifier), &transport).await.unwrap();
    assert_eq!(answer["refresh_token"], "rt-1");
    let req = transport.requests.lock().unwrap()[0].clone();
    assert_eq!(String::from_utf8(req.body.unwrap()).unwrap(), format!("grant_type=authorization_code&code=the-code&redirect_uri=https%3A%2F%2Foc.example.com%2Fcb&code_verifier={verifier}"));
}
