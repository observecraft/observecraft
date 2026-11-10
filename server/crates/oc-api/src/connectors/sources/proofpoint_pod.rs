//! Proofpoint On-Demand's Log API (`native: proofpoint-on-demand`, integrations/connectors/proofpoint-on-demand): a
//! WebSocket per cluster and log type, `wss://logstream.proofpoint.com/v1/stream?cid=<cluster>&type=message|maillog
//! &sinceTime=<ISO 8601>`, the API key as a bearer token, one JSON event per text frame.
//!
//! A session opens the stream from the stored position (`token`: the last stored event's own `ts`; a new connection
//! starts [`LOOKBACK_MS`] back), maps every frame by the package's stream and commits a batch every [`FLUSH`] or
//! [`BATCH`] events, the position saved with it once Kafka acknowledged it. At the deadline (or when Proofpoint closes
//! the stream) the rest is committed and the socket closed; the next session resumes at the saved `ts`. `sinceTime`
//! includes that time, so its events come again: their ids (`guid`) are their logs', so they are stored once.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use oc_connectors::engine::{Connection, Issue, Tested};
use oc_connectors::manifest::{IssueKind, Manifest};
use oc_connectors::output::Mapped;
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};

use super::{failed, issue, Session, Source, Summary};

pub const NAME: &str = "proofpoint-on-demand";
/// A new connection's first `sinceTime`.
pub const LOOKBACK_MS: i64 = 3_600_000;
/// Events are committed at least this often while they arrive…
pub const FLUSH: Duration = Duration::from_secs(2);
/// …or every this many.
pub const BATCH: usize = 500;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The source: the Log API's base URL (an in-process server's, for tests).
pub struct Pod {
    base: String,
}

impl Pod {
    pub fn proofpoint() -> Pod {
        Pod { base: "wss://logstream.proofpoint.com:443".into() }
    }

    #[cfg(test)]
    pub fn at(base: &str) -> Pod {
        Pod { base: base.trim_end_matches('/').to_string() }
    }

    fn url(&self, cluster: &str, kind: &str, since: &str) -> String {
        let mut u = url::Url::parse(&format!("{}/v1/stream", self.base)).expect("the base URL is valid");
        u.query_pairs_mut().append_pair("cid", cluster).append_pair("type", kind).append_pair("sinceTime", since);
        u.to_string()
    }

    async fn connect(&self, conn: &Connection, kind: &str, since: &str) -> Result<Ws, Issue> {
        let cluster = conn.config.get("cluster_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| issue(IssueKind::Other, "The cluster ID is missing"))?;
        let key = conn.secrets.get("api_key").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| issue(IssueKind::Auth, "The API key is missing"))?;
        let mut req = self.url(cluster, kind, since).into_client_request().map_err(|e| issue(IssueKind::Other, e.to_string()))?;
        let bearer = format!("Bearer {key}").parse().map_err(|_| issue(IssueKind::Auth, "The API key has characters a header can't carry"))?;
        req.headers_mut().insert("authorization", bearer);
        let connect = tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(tokio_tungstenite::Connector::Rustls(tls())));
        match tokio::time::timeout(Duration::from_secs(30), connect).await {
            Err(_) => Err(issue(IssueKind::Other, "Proofpoint's log stream didn't answer within 30 s")),
            Ok(Ok((ws, _))) => Ok(ws),
            Ok(Err(tungstenite::Error::Http(r))) => {
                let status = r.status().as_u16();
                let body = r.body().as_ref().map(|b| String::from_utf8_lossy(b).chars().take(200).collect::<String>()).unwrap_or_default();
                Err(match status {
                    401 => issue(IssueKind::Auth, "Proofpoint refused the API key (HTTP 401)"),
                    403 => issue(IssueKind::AccessDenied, format!("The API key may not read cluster {cluster}'s {kind} log (HTTP 403)")),
                    429 => issue(IssueKind::Throttled, "Proofpoint refused another session for this cluster and log type (HTTP 429)"),
                    _ => issue(IssueKind::Other, format!("Proofpoint's log stream answered HTTP {status} {}", body.trim()).trim().to_string()),
                })
            }
            Ok(Err(e)) => Err(issue(IssueKind::Other, format!("connecting to Proofpoint's log stream failed: {e}"))),
        }
    }
}

/// Rustls with the ring provider and the web PKI roots.
fn tls() -> Arc<rustls::ClientConfig> {
    static TLS: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    TLS.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    })
    .clone()
}

/// `sinceTime` for a time (UTC, microseconds, as Proofpoint writes `ts`).
pub fn since_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}

#[async_trait::async_trait]
impl Source for Pod {
    async fn test(&self, _m: &Manifest, conn: &Connection) -> Tested {
        let since = since_time(chrono::Utc::now().timestamp_millis() - 60_000);
        match self.connect(conn, "message", &since).await {
            Ok(mut ws) => {
                let _ = ws.close(None).await;
                let cluster = conn.config.get("cluster_id").and_then(Value::as_str).unwrap_or_default();
                Tested { ok: true, message: format!("Connected to cluster {cluster}'s message log stream"), records: None }
            }
            Err(i) => failed(i.message),
        }
    }

    async fn session(&self, mut s: Session<'_>) -> Result<Summary, Issue> {
        let since = s.state.token.as_ref().and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| since_time(chrono::Utc::now().timestamp_millis() - LOOKBACK_MS));
        let mut ws = self.connect(s.conn, &s.stream.id, &since).await?;
        let mut sum = Summary { until: s.state.cursor, ..Default::default() };
        let mut out = Mapped::default();
        let mut flushed = Instant::now();
        let mut bad = 0usize;
        let mut ended: Option<String> = None;
        while let Some(left) = s.deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            match tokio::time::timeout(left.min(FLUSH), ws.next()).await {
                Err(_) => {}
                Ok(None) => {
                    ended = Some("Proofpoint closed the stream".into());
                    break;
                }
                Ok(Some(Err(e))) => {
                    ended = Some(format!("the stream broke: {e}"));
                    break;
                }
                Ok(Some(Ok(msg))) => {
                    let text = match msg {
                        Message::Text(t) => t.as_str().to_string(),
                        Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                        Message::Close(frame) => {
                            ended = Some(frame.map(|f| f.reason.to_string()).filter(|r| !r.is_empty()).unwrap_or_else(|| "Proofpoint closed the stream".into()));
                            break;
                        }
                        _ => continue,
                    };
                    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                        match serde_json::from_str::<Value>(line) {
                            Ok(record @ Value::Object(_)) => {
                                sum.records += 1;
                                let ts = record.get("ts").and_then(Value::as_str).map(str::to_string);
                                if let Some((ms, ts)) = ts.as_ref().and_then(|t| Some((oc_connectors::expr::to_ms(&Value::String(t.clone()))?, t))) {
                                    if s.state.cursor.is_none_or(|c| ms >= c) {
                                        s.state.cursor = Some(ms);
                                        s.state.token = Some(Value::String(ts.clone()));
                                    }
                                }
                                s.records.map(record, None, &mut out);
                            }
                            _ => bad += 1,
                        }
                    }
                }
            }
            if out.logs.len() >= BATCH || (!out.logs.is_empty() && flushed.elapsed() >= FLUSH) {
                commit(&mut s, &mut out, &mut sum).await?;
                flushed = Instant::now();
            }
        }
        commit(&mut s, &mut out, &mut sum).await?;
        let _ = tokio::time::timeout(Duration::from_secs(5), ws.close(None)).await;
        // Proofpoint ending a session that read nothing is a problem (e.g. a session limit); after events, the next
        // session simply resumes.
        if let (Some(why), 0) = (&ended, sum.records) {
            return Err(issue(IssueKind::Other, format!("Proofpoint ended the {} stream: {why}", s.stream.id)));
        }
        if bad > 0 {
            sum.warning = Some(issue(IssueKind::Other, format!("{bad} frames that aren't JSON events were skipped")));
        }
        Ok(sum)
    }
}

/// Publishes what was mapped, with the position it reaches.
async fn commit(s: &mut Session<'_>, out: &mut Mapped, sum: &mut Summary) -> Result<(), Issue> {
    let logs = std::mem::take(&mut out.logs);
    if logs.is_empty() {
        return Ok(());
    }
    let n = logs.len();
    let state = s.state.clone();
    s.out.commit(logs, Some(&state)).await.map_err(|e| issue(IssueKind::Other, e))?;
    sum.written += n;
    sum.until = s.state.cursor;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::SinkExt;
    use serde_json::json;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    use super::super::testing::{records, Recorder};
    use super::*;

    const MESSAGES: &str = include_str!("../../../../../../integrations/connectors/proofpoint-on-demand/fixtures/message.ndjson");
    const MAILLOG: &str = include_str!("../../../../../../integrations/connectors/proofpoint-on-demand/fixtures/maillog.ndjson");

    /// A Log API in process: checks the bearer token, records each session's query, sends `frames` then waits (or
    /// closes when `close` is set).
    struct Fake {
        base: String,
        queries: Arc<Mutex<Vec<String>>>,
    }

    async fn fake(frames: Vec<String>, close: bool) -> Fake {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("ws://{}", listener.local_addr().unwrap());
        let queries = Arc::new(Mutex::new(vec![]));
        let seen = queries.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (frames, seen) = (frames.clone(), seen.clone());
                tokio::spawn(async move {
                    #[allow(clippy::result_large_err)] // tungstenite's callback type
                    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
                        seen.lock().unwrap().push(req.uri().query().unwrap_or_default().to_string());
                        if req.headers().get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer pod-test-key-0001") {
                            let mut r = ErrorResponse::new(Some("Unauthorized".into()));
                            *r.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                            return Err(r);
                        }
                        Ok(resp)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tcp, check).await else { return };
                    for f in frames {
                        if ws.send(Message::text(f)).await.is_err() {
                            return;
                        }
                    }
                    if close {
                        let _ = ws.close(None).await;
                    }
                    while let Some(Ok(_)) = ws.next().await {}
                });
            }
        });
        Fake { base, queries }
    }

    fn conn(stream: &str) -> (&'static Manifest, &'static oc_connectors::manifest::Stream, Connection, oc_connectors::engine::Records) {
        let (m, s, mut c, r) = records("proofpoint-on-demand", stream, json!({ "cluster_id": "example_hosted" }));
        c.secrets = json!({ "api_key": "pod-test-key-0001" }).as_object().unwrap().clone();
        (m, s, c, r)
    }

    async fn session(pod: &Pod, stream: &str, state: oc_connectors::engine::Partition, out: &mut Recorder, millis: u64) -> Result<Summary, Issue> {
        let (_, s, c, records) = conn(stream);
        pod.session(Session { stream: s, conn: &c, records, state, deadline: Instant::now() + Duration::from_millis(millis), out }).await
    }

    fn query(q: &str) -> std::collections::BTreeMap<String, String> {
        url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
    }

    /// Frames become logs for the tile's pipeline; the position (the last event's own ts) is saved with each commit
    /// and the next session resumes there; events read again keep their ids.
    #[tokio::test]
    async fn frames_become_logs_and_the_next_session_resumes_at_the_last_stored_event() {
        let f = fake(MESSAGES.lines().map(str::to_string).collect(), false).await;
        let pod = Pod::at(&f.base);
        let mut out = Recorder::default();
        let sum = session(&pod, "message", Default::default(), &mut out, 300).await.unwrap();
        assert_eq!((sum.records, sum.written, sum.warning), (3, 3, None));
        let q = query(&f.queries.lock().unwrap()[0]);
        assert_eq!((q["cid"].as_str(), q["type"].as_str()), ("example_hosted", "message"));
        let first_since = oc_connectors::expr::to_ms(&json!(q["sinceTime"])).unwrap();
        assert!((chrono::Utc::now().timestamp_millis() - LOOKBACK_MS - first_since).abs() < 10_000, "a new connection starts an hour back");
        let log = &out.logs[0];
        assert_eq!((log["ddsource"].as_str(), log["service"].as_str(), log["host"].as_str()), (Some("proofpoint-on-demand"), Some("message"), Some("example_hosted")));
        assert_eq!(log["message"], "Message accept: Your invoice for order 10421 (billing@example.com → jane.doe@example.com)");
        assert_eq!(log["connection"]["ip"], "203.0.113.25", "the pipeline maps connection.ip");
        assert_eq!(log["filter"]["routeDirection"], "inbound");
        assert_eq!(out.logs[1]["status"], "warning", "rejected");
        let state = out.state.clone().unwrap();
        assert_eq!(state.token, Some(json!("2026-11-10T11:58:05.900000+00:00")));
        assert_eq!(sum.until, Some(1_794_311_885_900));
        assert!(!serde_json::to_string(&out.logs).unwrap().contains("pod-test-key-0001"));

        let mut again = Recorder::default();
        session(&pod, "message", state, &mut again, 300).await.unwrap();
        assert_eq!(query(&f.queries.lock().unwrap()[1])["sinceTime"], "2026-11-10T11:58:05.900000+00:00");
        let ids = |r: &Recorder| r.logs.iter().map(|l| l["_oc_ingest"]["id"].clone()).collect::<Vec<_>>();
        assert_eq!(ids(&out), ids(&again), "replayed events are stored once");
    }

    /// Kafka down: the position isn't saved, so the next session reads the same events again.
    #[tokio::test]
    async fn a_failed_commit_saves_no_position() {
        let f = fake(MESSAGES.lines().map(str::to_string).collect(), false).await;
        let mut down = Recorder { fail_after: Some(0), ..Default::default() };
        let err = session(&Pod::at(&f.base), "message", Default::default(), &mut down, 300).await.unwrap_err();
        assert!(err.message.contains("Kafka"), "{err:?}");
        assert_eq!(down.state, None);
    }

    /// Mail logs, a closed stream, a bad frame, a refused key, and the test.
    #[tokio::test]
    async fn maillogs_closes_bad_frames_and_refused_keys() {
        let f = fake(vec![MAILLOG.trim().to_string(), "not json".into()], true).await;
        let pod = Pod::at(&f.base);
        let mut out = Recorder::default();
        let sum = session(&pod, "maillog", Default::default(), &mut out, 2_000).await.unwrap();
        assert_eq!((sum.records, out.logs.len()), (1, 1), "the session ends when Proofpoint closes it");
        assert!(sum.warning.unwrap().message.contains("1 frames"));
        assert_eq!(query(&f.queries.lock().unwrap()[0])["type"], "maillog");
        let log = &out.logs[0];
        assert_eq!((log["service"].as_str(), log["sm"]["stat"].as_str(), log.get("data")), (Some("maillog"), Some("Sent"), None));
        assert!(log["message"].as_str().unwrap().contains("sendmail[23410]"));

        let empty = fake(vec![], true).await;
        let err = session(&Pod::at(&empty.base), "message", Default::default(), &mut Recorder::default(), 2_000).await.unwrap_err();
        assert!(err.message.contains("ended the message stream"), "{err:?}");

        let (m, _, c, _) = conn("message");
        let t = pod.test(m, &c).await;
        assert_eq!((t.ok, t.message.as_str()), (true, "Connected to cluster example_hosted's message log stream"));
        let mut wrong = c.clone();
        wrong.secrets.insert("api_key".into(), json!("pod-wrong-key"));
        let t = pod.test(m, &wrong).await;
        assert_eq!((t.ok, t.message.as_str()), (false, "Proofpoint refused the API key (HTTP 401)"));
    }

    #[test]
    fn since_time_is_proofpoints_format() {
        assert_eq!(since_time(1_794_311_885_900), "2026-11-10T11:58:05.900000+00:00");
    }
}
