//! Session Replay recordings of the demo shop's browser sessions, in the record format of Datadog's
//! browser SDK: a Meta record and a FullSnapshot of the page's DOM when a view opens, then incremental
//! snapshots (mouse moves, clicks, scrolls, the cart counter's text changing, inputs) while it stays
//! open. Each 5-second tick of a recorded session becomes one segment, posted as the SDK posts it:
//! multipart with a deflate-compressed `segment` part, to the intake's `/api/v2/replay`.
//!
//! The pages are simple fictional storefront pages whose call-to-action buttons sit at fixed
//! positions, so the clicks land on them in the player and in heatmaps.

use std::io::Write;

use serde_json::{json, Map, Value};

use crate::Rng;

/// Viewport of the recordings.
pub const WIDTH: i64 = 1280;
pub const HEIGHT: i64 = 800;

const CSS: &str = "body{margin:0;font-family:Helvetica,Arial,sans-serif;color:#1d2433;background:#f6f7f9}\
header{position:absolute;left:0;top:0;width:1280px;height:64px;background:#14324a;color:#fff}\
.logo{position:absolute;left:32px;top:18px;font-size:22px;font-weight:700}\
nav{position:absolute;left:260px;top:22px}nav a{margin-right:28px;color:#cfe3f2}\
.cart{position:absolute;left:1120px;top:20px;padding:4px 12px;border-radius:14px;background:#f2a93b;color:#14324a;font-weight:700}\
main{position:absolute;left:0;top:64px;width:1280px;height:680px}\
h1{position:absolute;left:64px;top:40px;margin:0;font-size:34px}\
.lead{position:absolute;left:64px;top:96px;width:620px;color:#5b6577}\
.cta{position:absolute;left:64px;top:170px;width:200px;height:46px;border:0;border-radius:6px;background:#2a7de1;color:#fff;font-size:16px}\
.card{position:absolute;top:280px;width:260px;height:300px;border-radius:8px;background:#fff;box-shadow:0 1px 3px rgba(0,0,0,.12)}\
.card .img{height:170px;border-radius:8px 8px 0 0;background:linear-gradient(135deg,#d8e6f3,#b9d3ea)}\
.card p{margin:12px 16px}.price{font-weight:700}\
input{position:absolute;left:64px;width:420px;height:36px;border:1px solid #c5cbd6;border-radius:4px;padding:0 10px}\
footer{position:absolute;left:0;top:744px;width:1280px;height:56px;background:#e8ebf0;color:#5b6577;text-align:center;line-height:56px}";

/// (title, lead, call to action, product cards, form fields).
const PAGES: &[(&str, &str, &str, usize, usize)] = &[
    ("Spring sale on running gear", "New season shoes and jackets, free returns for 60 days.", "Shop now", 4, 0),
    ("Running shoes", "128 products · sorted by popularity", "Filter: running shoes", 4, 0),
    ("Trail Runner 3", "Lightweight trail shoe with a grippy outsole. Sizes 38–47.", "Add to cart", 0, 0),
    ("Your cart", "2 items · free shipping over $50", "Proceed to checkout", 2, 0),
    ("Checkout", "Shipping and payment", "Place order", 0, 3),
];

struct Dom {
    next: i64,
}

impl Dom {
    fn id(&mut self) -> i64 {
        self.next += 1;
        self.next
    }
    fn el(&mut self, tag: &str, attrs: Value, children: Vec<Value>) -> Value {
        json!({"type": 2, "id": self.id(), "tagName": tag, "attributes": attrs, "childNodes": children})
    }
    fn text(&mut self, t: &str) -> Value {
        json!({"type": 3, "id": self.id(), "textContent": t})
    }
}

/// What a recorder needs to move on a page: its nodes' ids.
pub struct Recorder {
    page: usize,
    cart_text: i64,
    cta: i64,
    inputs: Vec<i64>,
    body: i64,
    mouse: (f64, f64),
    pub cart: u32,
}

/// The Meta, FullSnapshot and Focus records that open a view, and its recorder.
pub fn open_view(page: usize, url: &str, at: u64, cart_items: u32) -> (Vec<Value>, Recorder) {
    let page = page.min(PAGES.len() - 1);
    let (title, lead, cta_label, cards, fields) = PAGES[page];
    let mut d = Dom { next: 0 };
    let css = d.el("style", json!({"_cssText": CSS}), vec![]);
    let head_title = {
        let t = d.text("Example Shop");
        d.el("title", json!({}), vec![t])
    };
    let head = d.el("head", json!({}), vec![css, head_title]);
    let logo_text = d.text("Example Shop");
    let logo = d.el("a", json!({"class": "logo"}), vec![logo_text]);
    let mut links = vec![];
    for l in ["Products", "Deals", "Help"] {
        let t = d.text(l);
        links.push(d.el("a", json!({"href": "#"}), vec![t]));
    }
    let nav = d.el("nav", json!({}), links);
    let cart_label = d.text("Cart ");
    let cart_count = d.text(&cart_items.to_string());
    let cart_text = cart_count["id"].as_i64().unwrap_or_default();
    let cart = d.el("a", json!({"class": "cart"}), vec![cart_label, cart_count]);
    let header = d.el("header", json!({}), vec![logo, nav, cart]);
    let h1_text = d.text(title);
    let h1 = d.el("h1", json!({}), vec![h1_text]);
    let lead_text = d.text(lead);
    let lead = d.el("p", json!({"class": "lead"}), vec![lead_text]);
    let mut main_children = vec![h1, lead];
    let mut inputs = vec![];
    for (i, placeholder) in ["Full name", "Shipping address", "Card number"].iter().enumerate().take(fields) {
        let input = d.el("input", json!({"type": "text", "placeholder": placeholder, "style": format!("top:{}px", 170 + i * 56)}), vec![]);
        inputs.push(input["id"].as_i64().unwrap_or_default());
        main_children.push(input);
    }
    let cta_text = d.text(cta_label);
    let cta_top = if fields > 0 { 170 + fields * 56 + 10 } else { 170 };
    let cta = d.el("button", json!({"class": "cta", "style": format!("top:{cta_top}px")}), vec![cta_text]);
    let cta_id = cta["id"].as_i64().unwrap_or_default();
    main_children.push(cta);
    for c in 0..cards {
        let img = d.el("div", json!({"class": "img"}), vec![]);
        let name = d.text(["Trail Runner 3", "Road Pro 2", "Wind Shell", "Merino Tee"][c % 4]);
        let name_p = d.el("p", json!({}), vec![name]);
        let price = d.text(["$129", "$149", "$89", "$45"][c % 4]);
        let price_p = d.el("p", json!({"class": "price"}), vec![price]);
        main_children.push(d.el("div", json!({"class": "card", "style": format!("left:{}px", 64 + c * 290)}), vec![img, name_p, price_p]));
    }
    let main = d.el("main", json!({}), main_children);
    let footer_text = d.text("© Example Shop · shop.example.com");
    let footer = d.el("footer", json!({}), vec![footer_text]);
    let body = d.el("body", json!({}), vec![header, main, footer]);
    let body_id = body["id"].as_i64().unwrap_or_default();
    let html = d.el("html", json!({"lang": "en"}), vec![head, body]);
    let doctype = json!({"type": 1, "id": d.id(), "name": "html", "publicId": "", "systemId": ""});
    let document = json!({"type": 0, "id": 0, "childNodes": [doctype, html]});
    let records = vec![
        json!({"type": 4, "timestamp": at, "data": {"href": url, "width": WIDTH, "height": HEIGHT}}),
        json!({"type": 6, "timestamp": at, "data": {"has_focus": true}}),
        json!({"type": 2, "timestamp": at + 1, "data": {"node": document, "initialOffset": {"top": 0, "left": 0}}}),
    ];
    (records, Recorder { page, cart_text, cta: cta_id, inputs, body: body_id, mouse: (640.0, 400.0), cart: cart_items })
}

impl Recorder {
    /// The incremental records of [from, to): the visitor moves towards the page's button, maybe
    /// scrolls, types in the checkout form and clicks (adding to the cart on product pages).
    pub fn tick(&mut self, rng: &mut Rng, from: u64, to: u64, click: bool) -> Vec<Value> {
        let mut out = vec![];
        // Towards the button (whose center is 164, top + 23), not always on it.
        let (cx, cy) = (164.0 + (rng.f() - 0.5) * 120.0, cta_top(self.inputs.len()) + 23.0 + (rng.f() - 0.5) * 30.0);
        let steps = 6;
        let span = (to - from).max(steps);
        let mut positions = vec![];
        for i in 1..=steps {
            let k = i as f64 / steps as f64;
            let x = self.mouse.0 + (cx - self.mouse.0) * k + (rng.f() - 0.5) * 40.0;
            let y = self.mouse.1 + (cy - self.mouse.1) * k + (rng.f() - 0.5) * 40.0;
            positions.push(json!({"x": x.round(), "y": y.round(), "id": self.body, "timeOffset": -((span - span * i / steps) as i64)}));
        }
        self.mouse = (cx, cy);
        out.push(json!({"type": 3, "timestamp": from + span * 3 / 4, "data": {"source": 1, "positions": positions}}));
        if rng.chance(0.25) {
            out.push(json!({"type": 3, "timestamp": from + span / 3, "data": {"source": 3, "id": 0, "x": 0, "y": (rng.f() * 120.0).round()}}));
        }
        for (i, id) in self.inputs.iter().enumerate() {
            if rng.chance(0.5) {
                // Masked, as the SDK records inputs by default.
                out.push(json!({"type": 3, "timestamp": from + span * (i as u64 + 1) / 5, "data": {"source": 5, "id": id, "text": "*".repeat(6 + i * 4)}}));
            }
        }
        if click {
            let at = from + span * 4 / 5;
            for (kind, dt) in [(1, 0), (0, 80), (2, 81)] {
                out.push(json!({"type": 3, "timestamp": at + dt, "data": {"source": 2, "type": kind, "id": self.cta, "x": cx.round(), "y": cy.round()}}));
            }
            if self.page == 2 {
                self.cart += 1;
                out.push(json!({"type": 3, "timestamp": at + 200, "data": {"source": 0, "adds": [], "removes": [], "attributes": [], "texts": [{"id": self.cart_text, "value": self.cart.to_string()}]}}));
            }
        }
        out.sort_by_key(|r| r["timestamp"].as_u64().unwrap_or_default());
        out
    }

    /// The record that closes a view.
    pub fn view_end(at: u64) -> Value {
        json!({"type": 7, "timestamp": at})
    }
}

/// The top of a page's call-to-action button in the viewport (below the form fields of checkout).
fn cta_top(fields: usize) -> f64 {
    64.0 + if fields > 0 { (170 + fields * 56 + 10) as f64 } else { 170.0 }
}

/// A segment as the SDK builds it.
pub fn segment(app: &str, session: &str, view: &str, index: u64, records: Vec<Value>) -> Value {
    let ts = |r: &Value| r["timestamp"].as_u64().unwrap_or_default();
    let start = records.iter().map(ts).min().unwrap_or_default();
    let end = records.iter().map(ts).max().unwrap_or_default();
    let mut m = Map::new();
    m.insert("application".into(), json!({"id": app}));
    m.insert("session".into(), json!({"id": session}));
    m.insert("view".into(), json!({"id": view}));
    m.insert("start".into(), json!(start));
    m.insert("end".into(), json!(end));
    m.insert("has_full_snapshot".into(), json!(records.iter().any(|r| r["type"] == 2)));
    m.insert("index_in_view".into(), json!(index));
    m.insert("records_count".into(), json!(records.len()));
    m.insert("source".into(), json!("browser"));
    m.insert("creation_reason".into(), json!(if index == 0 { "init" } else { "segment_duration_limit" }));
    m.insert("records".into(), Value::Array(records));
    Value::Object(m)
}

/// Posts segments like the SDK: multipart, the segment deflate-compressed.
pub async fn send(http: &reqwest::Client, url: &str, token: &str, segments: Vec<(&'static str, Value)>) {
    for (ua, seg) in segments {
        let raw = seg.to_string();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        if z.write_all(raw.as_bytes()).is_err() {
            continue;
        }
        let Ok(compressed) = z.finish() else { continue };
        let mut meta = seg.clone();
        if let Some(m) = meta.as_object_mut() {
            m.remove("records");
            m.insert("raw_segment_size".into(), json!(raw.len()));
            m.insert("compressed_segment_size".into(), json!(compressed.len()));
        }
        let Ok(part) = reqwest::multipart::Part::bytes(compressed).file_name("blob").mime_str("application/octet-stream") else { continue };
        let Ok(event) = reqwest::multipart::Part::text(meta.to_string()).file_name("blob").mime_str("application/json") else { continue };
        let form = reqwest::multipart::Form::new().part("segment", part).part("event", event);
        let r = http.post(url).query(&[("ddsource", "browser"), ("dd-api-key", token), ("dd-evp-origin", "browser")]).header(reqwest::header::USER_AGENT, ua).multipart(form).send().await;
        match r {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!(status = %r.status(), "replay segment rejected"),
            Err(e) => tracing::warn!(error = %e, "replay segment not sent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_opens_with_a_full_snapshot_and_moves_on_incrementally() {
        let (records, mut rec) = open_view(2, "https://shop.example.com/products/sku-1", 1000, 0);
        assert_eq!(records.iter().map(|r| r["type"].as_i64().unwrap()).collect::<Vec<_>>(), vec![4, 6, 2]);
        let mut rng = Rng(7);
        let inc = rec.tick(&mut rng, 1000, 6000, true);
        assert!(inc.iter().any(|r| r["data"]["source"] == 2 && r["data"]["type"] == 2), "a click");
        assert!(inc.iter().any(|r| r["data"]["source"] == 0), "the cart counter changes on a product page");
        let seg = segment("app", "s", "v", 0, [records, inc].concat());
        assert_eq!(seg["has_full_snapshot"], true);
        assert!(seg["start"].as_u64().unwrap() <= seg["end"].as_u64().unwrap());
    }
}
