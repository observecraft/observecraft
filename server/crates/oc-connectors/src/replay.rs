//! A package's recorded answers (`mock`) as a [`Transport`], and its expectations checked: every stream runs once
//! from an empty state at `mock.now` (and again with the state it left, for `rerun`), the test runs, and the counts
//! and golden files must match. `oc-connector test <dir>` and the server's tests over every bundled package run this.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::engine::{self, Connection, Runtime, StreamState};
use crate::http::{HttpRequest, HttpResponse, Transport};
use crate::manifest::{Mock, MockResponse};
use crate::package::Package;

/// Answers requests from a package's recorded answers.
pub struct MockTransport {
    responses: Vec<(MockResponse, Vec<u8>)>,
    used: Mutex<Vec<usize>>,
    /// Every request sent, in order.
    pub requests: Mutex<Vec<HttpRequest>>,
}

impl MockTransport {
    pub fn new(pkg: &Package, mock: &Mock) -> MockTransport {
        let responses: Vec<(MockResponse, Vec<u8>)> = mock
            .responses
            .iter()
            .map(|r| {
                let body = match (&r.file, &r.body) {
                    (Some(f), _) => pkg.file(f).unwrap_or_default().as_bytes().to_vec(),
                    (None, Some(Value::String(s))) => s.as_bytes().to_vec(),
                    (None, Some(v)) => serde_json::to_vec(v).unwrap_or_default(),
                    (None, None) => vec![],
                };
                (r.clone(), body)
            })
            .collect();
        let n = responses.len();
        MockTransport { responses, used: Mutex::new(vec![0; n]), requests: Mutex::default() }
    }

    fn matches(r: &MockResponse, req: &HttpRequest) -> bool {
        let Ok(url) = url::Url::parse(&req.url) else { return false };
        let bare = format!("{}://{}{}{}", url.scheme(), url.host_str().unwrap_or_default(), url.port().map(|p| format!(":{p}")).unwrap_or_default(), url.path());
        if r.method != req.method || bare.trim_end_matches('/') != r.url.trim_end_matches('/') {
            return false;
        }
        let query: BTreeMap<String, String> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        if !r.query.iter().all(|(k, v)| query.get(k) == Some(v)) {
            return false;
        }
        match &r.body_contains {
            Some(needle) => req.body.as_deref().is_some_and(|b| String::from_utf8_lossy(b).contains(needle.as_str())),
            None => true,
        }
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        self.requests.lock().unwrap().push(req.clone());
        let candidates: Vec<usize> = (0..self.responses.len()).filter(|i| Self::matches(&self.responses[*i].0, req)).collect();
        let mut used = self.used.lock().unwrap();
        // The most specific answer (most query and body conditions) first: unused ones in their order, else the
        // last of the most specific (a rerun gets the answers the first run got).
        let specific = |i: usize| self.responses[i].0.query.len() + usize::from(self.responses[i].0.body_contains.is_some());
        let unused = candidates.iter().copied().filter(|i| used[*i] == 0).max_by_key(|i| (specific(*i), std::cmp::Reverse(*i)));
        let Some(i) = unused.or_else(|| candidates.iter().copied().max_by_key(|i| (specific(*i), *i))) else {
            return Err(format!("no recorded answer for {:?} {}", req.method, req.url));
        };
        used[i] += 1;
        let (r, body) = &self.responses[i];
        Ok(HttpResponse { status: r.status, headers: r.headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.clone())).collect(), body: body.clone() })
    }
}

/// The connection a package's mock describes.
pub fn connection(mock: &Mock) -> Connection {
    Connection {
        id: "00000000-0000-4000-8000-000000000001".into(),
        name: "replay".into(),
        config: mock.config.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        secrets: mock.secrets.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect(),
    }
}

/// What a replay found: problems, and the golden files to rewrite (with `update`).
#[derive(Debug, Default)]
pub struct Report {
    pub problems: Vec<String>,
    pub updates: Vec<(String, String)>,
    /// One line per stream and the test.
    pub summary: Vec<String>,
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}

/// Replays `pkg`'s recorded answers (see the module documentation). `update`: golden files are rewritten instead of
/// compared.
pub async fn replay(pkg: &Package, update: bool) -> Report {
    let mut report = Report::default();
    let m = &pkg.manifest;
    let Some(mock) = &m.mock else {
        if !m.streams.is_empty() && m.native.is_none() {
            report.problems.push("mock: a package with streams needs recorded answers".into());
        }
        return report;
    };
    let Some(now) = crate::expr::to_ms(&Value::String(mock.now.clone())) else {
        report.problems.push("mock.now: not a time".into());
        return report;
    };
    let mut conn = connection(mock);
    conn.config = engine::effective_config(m, &conn.config);
    for stream in &m.streams {
        if m.native.is_some() && stream.request.is_none() {
            continue;
        }
        let transport = Arc::new(MockTransport::new(pkg, mock));
        let rt = Runtime::new(transport.clone(), false);
        let expect = mock.expect.get(&stream.id).cloned().unwrap_or_default();
        let at = format!("mock.expect.{}", stream.id);
        let run = match engine::run(m, stream, &conn, StreamState::default(), now, &rt, None).await {
            Ok(r) => r,
            Err(issue) => {
                report.problems.push(format!("{at}: the run failed: {}", issue.message));
                continue;
            }
        };
        if let Some(w) = &run.warning {
            report.problems.push(format!("{at}: the run warned: {}", w.message));
        }
        let mapped = &run.mapped;
        for (what, want, got) in [
            ("records", expect.records, run.records),
            ("logs", expect.logs, mapped.logs.len()),
            ("metrics", expect.metrics, mapped.points.len()),
            ("events", expect.events, mapped.events.len()),
            ("costs", expect.costs, mapped.costs.len()),
        ] {
            if let Some(want) = want {
                if want != got {
                    report.problems.push(format!("{at}.{what}: expected {want}, got {got}"));
                }
            }
        }
        if run.records == 0 && expect.records.is_none() {
            report.problems.push(format!("{at}: the stream read no records from the recorded answers"));
        }
        if let Some(path) = &expect.golden {
            let got = pretty(&json!({ "logs": mapped.logs, "metrics": mapped.points, "events": mapped.events, "costs": mapped.costs, "state": run.state }));
            if update {
                report.updates.push((path.clone(), got));
            } else {
                match pkg.file(path) {
                    None => report.problems.push(format!("{at}.golden: {path} is missing (OC_CONNECTOR_UPDATE=1 writes it)")),
                    Some(want) if want.trim_end() != got.trim_end() => report.problems.push(format!("{at}.golden: the output differs from {path} (OC_CONNECTOR_UPDATE=1 rewrites it)")),
                    Some(_) => {}
                }
            }
        }
        if let Some(want) = expect.rerun {
            let again = engine::run(m, stream, &conn, run.state.clone(), now + 60_000, &rt, None).await;
            match again {
                Ok(r) if r.records != want => report.problems.push(format!("{at}.rerun: expected {want}, got {}", r.records)),
                Ok(_) => {}
                Err(issue) => report.problems.push(format!("{at}.rerun: the run failed: {}", issue.message)),
            }
        }
        report.summary.push(format!("{}: {} records → {} logs, {} points, {} events, {} costs ({} requests)", stream.id, run.records, mapped.logs.len(), mapped.points.len(), mapped.events.len(), mapped.costs.len(), run.requests));
    }
    if m.test.is_some() {
        let rt = Runtime::new(Arc::new(MockTransport::new(pkg, mock)), false);
        let tested = engine::test(m, &conn, now, &rt).await;
        let expect = mock.expect.get("test").cloned().unwrap_or_default();
        if tested.ok != expect.ok.unwrap_or(true) {
            report.problems.push(format!("mock.expect.test: ok is {}: {}", tested.ok, tested.message));
        }
        if let Some(needle) = &expect.message {
            if !tested.message.contains(needle.as_str()) {
                report.problems.push(format!("mock.expect.test.message: {:?} doesn't contain {needle:?}", tested.message));
            }
        }
        report.summary.push(format!("test: {} — {}", if tested.ok { "ok" } else { "failed" }, tested.message));
    }
    report
}

/// A mock connection's configuration as JSON (the web's mock mode shows it).
pub fn mock_config(mock: &Mock) -> Map<String, Value> {
    mock.config.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}
