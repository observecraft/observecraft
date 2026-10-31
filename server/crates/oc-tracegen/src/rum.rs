//! Browser sessions of the shop's web front end (`storefront-web`), sent every 5 seconds the way
//! Datadog's browser SDK sends them: newline-delimited JSON events to the intake's `/api/v2/rum`
//! with the demo application's client token and `ddtags` in the query string, and the visitor's
//! User-Agent and address in the headers.
//!
//! A visitor browses home → product list → product → cart → checkout, a page every 10–40 s. Each
//! page is a view, sent again as it changes (`_dd.document_version`), with its loading time and Core
//! Web Vitals; clicks are actions (some of them rage clicks), API calls are resources, slow
//! renders long tasks. Checkout fails now and then with a JavaScript error. Some visitors are
//! signed in (`usr.*`); a few sessions come from synthetic tests.

use serde_json::{json, Value};

use crate::replay;
use crate::Rng;

/// The demo application of `oc-api`'s seed (`routes::rum::DEMO_APPLICATION_ID` / `DEMO_CLIENT_TOKEN`).
pub const APPLICATION_ID: &str = "5e1f0000-0000-4000-8000-0000000000a1";
pub const CLIENT_TOKEN: &str = "publocaldemoonlydonotuseinproduction";

const VERSIONS: &[&str] = &["2.14.1", "2.14.1", "2.14.1", "2.15.0-beta", "2.13.4"];

/// (view name, url path, typical loading time ms, API calls).
const PAGES: &[(&str, &str, f64, &[&str])] = &[
    ("/", "/", 620.0, &["/api/products/featured", "/api/cart"]),
    ("/products", "/products", 780.0, &["/api/products?limit=24", "/api/cart"]),
    ("/products/?", "/products/sku-", 900.0, &["/api/products/sku", "/api/inventory/sku", "/api/reviews"]),
    ("/cart", "/cart", 540.0, &["/api/cart", "/api/shipping/estimate"]),
    ("/checkout", "/checkout", 1180.0, &["/api/cart", "/api/payments/methods", "/api/orders"]),
];

/// (User-Agent, client address).
const VISITORS: &[(&str, &str)] = &[
    ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36", "198.51.100.23"),
    ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.2849.68", "198.51.100.71"),
    ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1", "203.0.113.45"),
    ("Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36", "203.0.113.88"),
    ("Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0", "198.51.100.140"),
    ("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15", "203.0.113.19"),
];

const USERS: &[(&str, &str)] = &[("Maya Chen", "maya.chen@example.com"), ("Jonas Weber", "jonas.weber@example.com"), ("Lucía Torres", "lucia.torres@example.com"), ("Arjun Patel", "arjun.patel@example.com")];

pub struct Session {
    id: String,
    visitor: usize,
    user: Option<usize>,
    synthetic: bool,
    version: &'static str,
    page: usize,
    view_id: String,
    view_started: u64,
    view_version: u64,
    next_page_at: u64,
    pages_left: usize,
    view_errors: u64,
    view_actions: u64,
    view_resources: u64,
    view_long_tasks: u64,
    vitals: (f64, f64, f64, f64),
    /// Session Replay (desktop sessions with a recording): the open view's recorder, its segments so far.
    recorder: Option<replay::Recorder>,
    segment_index: u64,
    /// Records to send with the next segment (a view's opening records).
    pending: Vec<Value>,
    cart: u32,
}

/// The sessions in progress.
#[derive(Default)]
pub struct Sessions(Vec<Session>);

fn new_session(rng: &mut Rng, now_ms: u64) -> Session {
    let mut s = Session {
        id: format!("{}-{}-{}-{}-{}", rng.hex(4), rng.hex(2), rng.hex(2), rng.hex(2), rng.hex(6)),
        visitor: rng.range(0, VISITORS.len() as u64) as usize,
        user: rng.chance(0.4).then(|| rng.range(0, USERS.len() as u64) as usize),
        synthetic: rng.chance(0.05),
        version: VERSIONS[rng.range(0, VERSIONS.len() as u64) as usize],
        page: 0,
        view_id: String::new(),
        view_started: now_ms,
        view_version: 0,
        next_page_at: now_ms,
        pages_left: rng.range(2, 9) as usize,
        view_errors: 0,
        view_actions: 0,
        view_resources: 0,
        view_long_tasks: 0,
        vitals: (0.0, 0.0, 0.0, 0.0),
        recorder: None,
        segment_index: 0,
        pending: vec![],
        cart: 0,
    };
    open_view(rng, &mut s, now_ms, 0);
    s
}

fn open_view(rng: &mut Rng, s: &mut Session, now_ms: u64, page: usize) {
    s.page = page;
    s.view_id = rng.hex(16);
    s.view_started = now_ms;
    s.view_version = 0;
    s.view_errors = 0;
    s.view_actions = 0;
    s.view_resources = 0;
    s.view_long_tasks = 0;
    s.next_page_at = now_ms + rng.range(10_000, 40_000);
    if s.records() {
        if s.recorder.is_some() {
            s.pending.push(replay::Recorder::view_end(now_ms));
            flush_into(s);
        }
        let (records, rec) = replay::open_view(page, &format!("https://shop.example.com{}", PAGES[page].1), now_ms, s.cart);
        s.pending = records;
        s.recorder = Some(rec);
        s.segment_index = 0;
    }
    let base = PAGES[page].2 * if VISITORS[s.visitor].0.contains("Mobile") { 1.4 } else { 1.0 };
    let loading = rng.latency_ns(base) as f64 / 1e6;
    s.vitals = (loading, loading * (0.7 + rng.f() * 0.25), 40.0 + rng.f() * if page == 4 { 380.0 } else { 160.0 }, rng.f() * if page == 2 { 0.22 } else { 0.06 });
}

/// Segments waiting to be sent (a closed view's last records).
static OUTBOX: std::sync::Mutex<Vec<(&'static str, Value)>> = std::sync::Mutex::new(Vec::new());

/// Queues the session's pending records as a segment of its current view.
fn flush_into(s: &mut Session) {
    if s.pending.is_empty() {
        return;
    }
    let seg = replay::segment(APPLICATION_ID, &s.id, &s.view_id, s.segment_index, std::mem::take(&mut s.pending));
    s.segment_index += 1;
    OUTBOX.lock().unwrap().push((VISITORS[s.visitor].0, seg));
}

impl Session {
    /// Recorded: desktop sessions of real users with replay on.
    fn records(&self) -> bool {
        !self.synthetic && self.visitor.is_multiple_of(2) && !VISITORS[self.visitor].0.contains("Mobile")
    }

    fn common(&self, now_ms: u64, kind: &str) -> Value {
        let (_, path, _, _) = PAGES[self.page];
        let path = if self.page == 2 { format!("{path}{}", &self.view_id[..6]) } else { path.to_string() };
        let mut e = json!({
            "type": kind, "date": now_ms, "service": "storefront-web", "version": self.version, "source": "browser",
            "application": { "id": APPLICATION_ID },
            "session": { "id": self.id, "type": if self.synthetic { "synthetics" } else { "user" }, "has_replay": self.records() },
            "view": { "id": self.view_id, "name": PAGES[self.page].0, "url": format!("https://shop.example.com{path}"), "url_path": path, "referrer": "" },
            "_dd": { "format_version": 2, "sdk_name": "rum" },
            "connectivity": { "status": "connected", "interfaces": ["wifi"] },
            "display": { "viewport": { "width": 1440, "height": 900 } },
        });
        if let Some(u) = self.user {
            e["usr"] = json!({ "id": format!("u-{}", 1000 + u), "name": USERS[u].0, "email": USERS[u].1 });
        }
        e
    }

    fn view_event(&mut self, now_ms: u64) -> Value {
        self.view_version += 1;
        let mut e = self.common(now_ms, "view");
        let ns = |ms: f64| (ms * 1e6) as u64;
        let (loading, lcp, inp, cls) = self.vitals;
        e["date"] = json!(self.view_started);
        e["view"]["loading_type"] = json!(if self.view_version == 1 && self.page == 0 { "initial_load" } else { "route_change" });
        e["view"]["loading_time"] = json!(ns(loading));
        e["view"]["largest_contentful_paint"] = json!(ns(lcp));
        e["view"]["first_contentful_paint"] = json!(ns(lcp * 0.6));
        e["view"]["interaction_to_next_paint"] = json!(ns(inp));
        e["view"]["cumulative_layout_shift"] = json!((cls * 1000.0).round() / 1000.0);
        e["view"]["time_spent"] = json!(ns((now_ms - self.view_started) as f64));
        e["view"]["is_active"] = json!(true);
        e["view"]["error"] = json!({ "count": self.view_errors });
        e["view"]["action"] = json!({ "count": self.view_actions });
        e["view"]["resource"] = json!({ "count": self.view_resources });
        e["view"]["long_task"] = json!({ "count": self.view_long_tasks });
        e["_dd"]["document_version"] = json!(self.view_version);
        e
    }
}

/// The events of the sessions over the last `step_ms` (with new sessions started and finished ones dropped).
pub fn tick(rng: &mut Rng, sessions: &mut Sessions, now_ms: u64, step_ms: u64) -> Vec<(usize, Vec<Value>)> {
    let starts = rng.range(1, 4);
    for _ in 0..starts {
        if sessions.0.len() < 40 {
            let start = now_ms - rng.range(0, step_ms);
            sessions.0.push(new_session(rng, start));
        }
    }
    let mut out = Vec::new();
    for s in sessions.0.iter_mut() {
        let mut events = Vec::new();
        if s.view_version == 0 {
            events.push(s.view_event(now_ms));
        }
        let page = PAGES[s.page];
        for api in page.3 {
            if rng.chance(0.35) {
                s.view_resources += 1;
                let failed = page.0 == "/checkout" && *api == "/api/orders" && rng.chance(0.15);
                let mut r = s.common(now_ms - rng.range(0, step_ms), "resource");
                r["resource"] = json!({ "id": rng.hex(8), "type": "fetch", "method": if *api == "/api/orders" { "POST" } else { "GET" },
                    "url": format!("https://api.example.com{api}"), "url_host": "api.example.com", "status_code": if failed { 502 } else { 200 },
                    "duration": rng.latency_ns(if failed { 900.0 } else { 120.0 }), "size": rng.range(400, 40_000) });
                events.push(r);
                if failed {
                    s.view_errors += 1;
                    let mut e = s.common(now_ms, "error");
                    e["error"] = json!({ "id": rng.hex(8), "source": "network", "type": "Error", "handling": "unhandled",
                        "message": format!("XHR error POST https://api.example.com{api}"), "resource": { "method": "POST", "status_code": 502, "url": format!("https://api.example.com{api}") } });
                    events.push(e);
                }
            }
        }
        let clicked = rng.chance(0.6);
        if let Some(rec) = s.recorder.as_mut() {
            let mut records = rec.tick(rng, now_ms.saturating_sub(step_ms), now_ms, clicked);
            s.cart = rec.cart;
            s.pending.append(&mut records);
            flush_into(s);
        }
        if clicked {
            s.view_actions += 1;
            let target = match page.0 {
                "/" => "Shop now",
                "/products" => "Filter: running shoes",
                "/products/?" => "Add to cart",
                "/cart" => "Proceed to checkout",
                _ => "Place order",
            };
            let mut a = s.common(now_ms - rng.range(0, step_ms), "action");
            let rage = page.0 == "/checkout" && rng.chance(0.2);
            a["action"] = json!({ "id": rng.hex(8), "type": "click", "target": { "name": target }, "loading_time": rng.latency_ns(80.0),
                "frustration": { "type": if rage { vec!["rage_click"] } else { vec![] } } });
            events.push(a);
        }
        if page.0 == "/checkout" && rng.chance(0.06) {
            s.view_errors += 1;
            let mut e = s.common(now_ms, "error");
            e["error"] = json!({ "id": rng.hex(8), "source": "source", "type": "TypeError", "handling": "unhandled",
                "message": "Cannot read properties of undefined (reading 'shippingAddress')",
                "stack": "TypeError: Cannot read properties of undefined (reading 'shippingAddress')\n    at submitOrder (https://shop.example.com/assets/checkout.4f2a1c.js:1:18234)" });
            events.push(e);
        }
        if rng.chance(if page.0 == "/products/?" { 0.3 } else { 0.08 }) {
            s.view_long_tasks += 1;
            let mut l = s.common(now_ms - rng.range(0, step_ms), "long_task");
            l["long_task"] = json!({ "id": rng.hex(8), "duration": rng.latency_ns(90.0).max(50_000_000), "entry_type": "long-animation-frame" });
            events.push(l);
        }
        events.push(s.view_event(now_ms));
        if now_ms >= s.next_page_at {
            if s.pages_left == 0 {
                s.pages_left = usize::MAX;
            } else {
                s.pages_left -= 1;
                let next = (s.page + 1).min(PAGES.len() - 1);
                let next = if next == s.page || rng.chance(0.25) { rng.range(0, 3) as usize } else { next };
                open_view(rng, s, now_ms, next);
                events.push(s.view_event(now_ms));
            }
        }
        out.push((s.visitor, events));
    }
    sessions.0.retain(|s| s.pages_left != usize::MAX);
    out
}

/// The replay segments made since the last call.
pub fn take_segments() -> Vec<(&'static str, Value)> {
    std::mem::take(&mut *OUTBOX.lock().unwrap())
}

/// Sends the events of each visitor as the SDK would.
pub async fn send(http: &reqwest::Client, url: &str, batches: Vec<(usize, Vec<Value>)>) {
    for (visitor, events) in batches {
        if events.is_empty() {
            continue;
        }
        let body: String = events.iter().map(|e| format!("{e}\n")).collect();
        let (ua, ip) = VISITORS[visitor];
        let r = http
            .post(url)
            .query(&[("ddsource", "browser"), ("dd-api-key", CLIENT_TOKEN), ("ddtags", "sdk_version:5.28.0,api:fetch,env:prod,service:storefront-web"), ("dd-evp-origin", "browser")])
            .header(reqwest::header::USER_AGENT, ua)
            .header("x-forwarded-for", ip)
            .header(reqwest::header::CONTENT_TYPE, "text/plain;charset=UTF-8")
            .body(body)
            .send()
            .await;
        match r {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!(status = %r.status(), "RUM events rejected"),
            Err(e) => tracing::warn!(error = %e, "RUM events not sent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_browse_and_end() {
        let mut rng = Rng(42);
        let mut sessions = Sessions::default();
        let mut now = 1_700_000_000_000u64;
        let mut kinds = std::collections::BTreeSet::new();
        for _ in 0..200 {
            for (_, events) in tick(&mut rng, &mut sessions, now, 5_000) {
                for e in events {
                    assert_eq!(e["application"]["id"], APPLICATION_ID);
                    kinds.insert(e["type"].as_str().unwrap().to_string());
                }
            }
            now += 5_000;
        }
        assert!(["view", "action", "resource", "error", "long_task"].iter().all(|k| kinds.contains(*k)), "{kinds:?}");
        assert!(sessions.0.len() <= 40);
    }
}
