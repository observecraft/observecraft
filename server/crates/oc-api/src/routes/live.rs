//! Live tail over Server-Sent Events. The Vector aggregator publishes every normalized log to
//! NATS (`oc.logs.live.<org>`); each SSE connection subscribes, filters with the search matcher,
//! samples down to a display cap, and pushes a batch every 250 ms. Only events the viewer's log
//! restriction queries allow are sent; the restriction is re-read every 10 s, so a change applies to
//! open connections too.

use std::collections::HashMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::{Stream, StreamExt};
use oc_query::{matches, parse_query, Fields};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utoipa::{IntoParams, ToSchema};

use super::logs::LogEvent;
use crate::auth::AuthContext;
use crate::log_access::{self, LogAccess};
use crate::state::AppState;

const TICK: Duration = Duration::from_millis(250);
const DISPLAY_CAP_PER_SEC: f64 = 40.0;
/// How often an open stream re-reads the viewer's log access.
const ACCESS_REFRESH: Duration = Duration::from_secs(10);

/// Shape published by the Vector aggregator (see deploy/vector/aggregator.yaml).
#[derive(Deserialize)]
struct LiveRecord {
    id: String,
    ts_ms: i64,
    status: String,
    service: String,
    host: String,
    source: String,
    message: String,
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    attr: HashMap<String, String>,
    #[serde(default)]
    attributes: String,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    span_id: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveTailBatch {
    /// Events to display (already sampled), oldest first.
    pub events: Vec<LogEvent>,
    /// Rate of events matching the query.
    pub matched_per_sec: f64,
    /// displayed / matched, 0..1.
    pub displayed_ratio: f64,
}

#[derive(Deserialize, IntoParams)]
pub struct LiveParams {
    /// Search query (same syntax as /logs/search).
    #[serde(default)]
    query: String,
}

/// Tiny xorshift PRNG for reservoir sampling (no crypto needs here).
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// Streams `batch` events (JSON `LiveTailBatch`) every 250 ms.
#[utoipa::path(get, path = "/api/v1/logs/live", params(LiveParams),
    responses((status = 200, description = "text/event-stream of `batch` events carrying LiveTailBatch JSON", content_type = "text/event-stream")),
    tag = "logs")]
pub async fn live(State(st): State<AppState>, ctx: AuthContext, mut access: LogAccess, Query(p): Query<LiveParams>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(8);
    let terms = parse_query(&p.query);
    let nats = st.nats.clone();
    let subject = format!("oc.logs.live.{}", ctx.org_id);

    tokio::spawn(async move {
        let mut sub = match nats.subscribe(subject.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, subject, "live tail subscribe failed");
                let _ = tx.send(Ok(Event::default().event("error").data("live tail unavailable"))).await;
                return;
            }
        };
        let cap = ((DISPLAY_CAP_PER_SEC * TICK.as_secs_f64()).ceil() as usize).max(1);
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut rng = Rng(seed | 1);
        let mut window: Vec<LogEvent> = Vec::with_capacity(cap);
        let mut matched = 0usize;
        let mut rate = 0.0f64;
        let mut tick = tokio::time::interval(TICK);
        let mut last = Instant::now();
        let mut access_read = Instant::now();

        loop {
            tokio::select! {
                msg = sub.next() => {
                    let Some(msg) = msg else { break };
                    let Ok(r) = serde_json::from_slice::<LiveRecord>(&msg.payload) else { continue };
                    let f = Fields { service: &r.service, status: &r.status, host: &r.host, source: &r.source, message: &r.message, tags: &r.tags, attr: &r.attr, index: None };
                    if !access.allows(&f) || !matches(&terms, &f) {
                        continue;
                    }
                    matched += 1;
                    let ev = LogEvent {
                        id: r.id,
                        timestamp: r.ts_ms,
                        status: r.status,
                        service: r.service,
                        host: r.host,
                        source: r.source,
                        message: r.message,
                        tags: r.tags,
                        attributes: serde_json::from_str(&r.attributes).unwrap_or(serde_json::Value::Null),
                        trace_id: super::logs::non_empty(r.trace_id),
                        span_id: super::logs::non_empty(r.span_id),
                    };
                    // Reservoir sampling keeps a uniform sample of this tick's matches.
                    if window.len() < cap {
                        window.push(ev);
                    } else {
                        let j = rng.below(matched);
                        if j < cap {
                            window[j] = ev;
                        }
                    }
                }
                _ = tick.tick() => {
                    if access_read.elapsed() >= ACCESS_REFRESH {
                        access_read = Instant::now();
                        match log_access::for_user(&st, &ctx).await {
                            Ok(a) => access = a,
                            // Can't tell what the viewer may see: stop rather than guess.
                            Err(e) => {
                                tracing::error!(error = %e, "live tail: reading log access failed");
                                break;
                            }
                        }
                    }
                    let secs = last.elapsed().as_secs_f64().max(0.001);
                    last = Instant::now();
                    rate = rate * 0.8 + (matched as f64 / secs) * 0.2;
                    let mut events = std::mem::take(&mut window);
                    events.sort_by_key(|e| e.timestamp);
                    let batch = LiveTailBatch {
                        displayed_ratio: if matched == 0 { 1.0 } else { events.len() as f64 / matched as f64 },
                        matched_per_sec: rate,
                        events,
                    };
                    matched = 0;
                    let data = serde_json::to_string(&batch).unwrap_or_default();
                    if tx.send(Ok(Event::default().event("batch").data(data))).await.is_err() {
                        break; // client went away
                    }
                }
            }
        }
        let _ = sub.unsubscribe().await;
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}
