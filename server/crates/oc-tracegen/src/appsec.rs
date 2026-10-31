//! App & API Protection's security traces, as Datadog's tracers with the In-App WAF report them on the
//! entry spans of api-gateway and storefront-api: `_dd.appsec.enabled`, the client IP and user agent,
//! response headers, `usr.id` on authenticated requests, login events
//! (`appsec.events.users.login.{success,failure}.track`) and, on attacks, `_dd.appsec.json` (the rules
//! that matched and what) and `appsec.blocked`. Mostly ordinary traffic, with a security scanner every
//! 30 minutes, SQL injection and path traversal attempts, a credential stuffing wave every 45 minutes,
//! and blocked requests from a denylisted IP. Addresses are documentation ranges; people `@example.com`.

use serde_json::{json, Value};

use crate::{attrs_json, Rng};

const CUSTOMERS: [(&str, &str); 6] = [("192.0.2.31", "RO"), ("192.0.2.77", "DE"), ("192.0.2.140", "NL"), ("192.0.2.201", "US"), ("192.0.2.18", "FR"), ("192.0.2.99", "GB")];
const ATTACKERS: [(&str, &str); 6] = [("203.0.113.7", "US"), ("203.0.113.41", "BR"), ("198.51.100.23", "VN"), ("198.51.100.71", "CA"), ("203.0.113.150", "IN"), ("198.51.100.140", "BE")];
const USERS: [&str; 5] = ["ana.popescu@example.com", "maya.chen@example.com", "dev.kumar@example.com", "lucia.torres@example.com", "sam.okafor@example.com"];
const UA_BROWSER: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";
const UA_SCANNER: &str = "Nuclei - Open-source project (github.com/projectdiscovery/nuclei)";
const UA_SCRIPT: &str = "python-requests/2.32.3";

/// (service, method, route, path)
const ROUTES: [(&str, &str, &str, &str); 7] = [
    ("api-gateway", "GET", "/api/products", "/api/products"),
    ("api-gateway", "GET", "/api/products/{id}", "/api/products/1042"),
    ("api-gateway", "POST", "/api/v1/login", "/api/v1/login"),
    ("api-gateway", "POST", "/api/orders/{id}/submit", "/api/orders/77/submit"),
    ("storefront-api", "GET", "/search", "/search"),
    ("storefront-api", "POST", "/api/cart/items", "/api/cart/items"),
    ("storefront-api", "GET", "/api/account", "/api/account"),
];

/// (type, rule id, rule name, address, value, scanner)
const ATTACKS: [(&str, &str, &str, &str, &str, bool); 6] = [
    ("security_scanner", "oc-913-001", "Security scanner detected", "server.request.headers.no_cookies", "nuclei", true),
    ("sql_injection", "oc-942-100", "SQL injection attempt", "server.request.query", "1' OR '1'='1", false),
    ("lfi", "oc-930-100", "Path traversal attempt", "server.request.uri.raw", "/../../etc/passwd", false),
    ("xss", "oc-941-110", "Cross-site scripting attempt", "server.request.query", "<script>alert(1)</script>", false),
    ("php_code_injection", "oc-933-130", "PHP code injection attempt", "server.request.body", "<?php system($_GET['c']); ?>", false),
    ("ssrf", "oc-934-100", "Server-side request forgery attempt", "server.request.query", "http://169.254.169.254/latest/meta-data/", false),
];

fn pick<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[rng.range(0, items.len() as u64) as usize]
}

fn hex(rng: &mut Rng, bytes: usize) -> String {
    (0..bytes).map(|_| format!("{:02x}", rng.range(0, 256))).collect()
}

struct Request<'a> {
    service: &'a str,
    method: &'a str,
    route: &'a str,
    path: &'a str,
    ip: &'a str,
    country: &'a str,
    ua: &'a str,
    status: u16,
    attrs: Vec<(String, Value)>,
}

fn span(rng: &mut Rng, now_ns: u64, r: Request) -> (String, Value) {
    let start = now_ns - rng.range(0, 1_000_000_000);
    let end = start + rng.range(2_000_000, 80_000_000);
    let mut attrs: Vec<(String, Value)> = vec![
        ("http.method".into(), json!(r.method)),
        ("http.route".into(), json!(r.route)),
        ("http.url".into(), json!(format!("https://shop.example.com{}", r.path))),
        ("http.status_code".into(), json!(r.status.to_string())),
        ("http.client_ip".into(), json!(r.ip)),
        ("http.useragent".into(), json!(r.ua)),
        ("http.host".into(), json!("shop.example.com")),
        ("network.client.geoip.country.iso_code".into(), json!(r.country)),
        ("_dd.appsec.enabled".into(), json!("1")),
        ("http.response.headers.content-type".into(), json!("application/json")),
    ];
    // storefront-api sets the security headers, api-gateway doesn't.
    if r.service == "storefront-api" {
        attrs.push(("http.response.headers.strict-transport-security".into(), json!("max-age=31536000; includeSubDomains")));
        attrs.push(("http.response.headers.x-frame-options".into(), json!("DENY")));
    }
    attrs.extend(r.attrs);
    let s = json!({
        "traceId": hex(rng, 16),
        "spanId": hex(rng, 8),
        "name": format!("{} {}", r.method, r.route),
        "kind": 2,
        "startTimeUnixNano": start.to_string(),
        "endTimeUnixNano": end.to_string(),
        "attributes": attrs_json(&attrs),
        "status": { "code": if r.status >= 500 { 2 } else { 1 } },
    });
    (r.service.to_string(), s)
}

fn attack_json(kind: &str, rule_id: &str, name: &str, address: &str, value: &str) -> String {
    let category = "attack_attempt";
    json!({ "triggers": [{ "rule": { "id": rule_id, "name": name, "tags": { "type": kind, "category": category } },
        "rule_matches": [{ "operator": "match_regex", "parameters": [{ "address": address, "key_path": ["q"], "value": value, "highlight": [value] }] }] }] })
    .to_string()
}

/// One second of requests, as OTLP resource spans.
pub fn tick(rng: &mut Rng, now_ns: u64, second: u64) -> Value {
    let mut spans: Vec<(String, Value)> = Vec::new();
    // Ordinary traffic, some of it authenticated.
    for _ in 0..rng.range(3, 8) {
        let (service, method, route, path) = pick(rng, &ROUTES);
        let (ip, country) = pick(rng, &CUSTOMERS);
        let mut attrs = Vec::new();
        if rng.chance(0.4) && route != "/api/v1/login" {
            attrs.push(("usr.id".into(), json!(pick(rng, &USERS))));
        }
        let mut status = 200;
        if route == "/api/v1/login" {
            let ok = rng.chance(0.85);
            status = if ok { 200 } else { 401 };
            let user = pick(rng, &USERS);
            attrs.push((format!("appsec.events.users.login.{}.track", if ok { "success" } else { "failure" }), json!("true")));
            attrs.push((if ok { "usr.id" } else { "appsec.events.users.login.failure.usr.id" }.into(), json!(user)));
        }
        spans.push(span(rng, now_ns, Request { service, method, route, path, ip, country, ua: UA_BROWSER, status, attrs }));
    }
    // A security scanner sweeps the gateway every 30 minutes; one-off attacks in between.
    let scan = (420..540).contains(&(second % 1800));
    let n = if scan { rng.range(3, 8) } else if rng.chance(0.08) { 1 } else { 0 };
    let attacker = ATTACKERS[((second / 1800) % ATTACKERS.len() as u64) as usize];
    for _ in 0..n {
        let (kind, rule, name, address, value, scanner) = if scan && rng.chance(0.7) { ATTACKS[0] } else { pick(rng, &ATTACKS[1..]) };
        let (service, method, route, path) = pick(rng, &ROUTES);
        let (ip, country) = if scan || rng.chance(0.5) { attacker } else { pick(rng, &ATTACKERS) };
        let blocked = ip == "203.0.113.41";
        let mut attrs: Vec<(String, Value)> = vec![("appsec.event".into(), json!("true")), ("_dd.appsec.json".into(), json!(attack_json(kind, rule, name, address, value))), ("actor.ip".into(), json!(ip))];
        if blocked {
            attrs.push(("appsec.blocked".into(), json!("true")));
        }
        let status = if blocked {
            403
        } else if kind == "sql_injection" && route == "/search" {
            500
        } else if scanner {
            404
        } else {
            400
        };
        spans.push(span(rng, now_ns, Request { service, method, route, path, ip, country, ua: if scanner { UA_SCANNER } else { UA_SCRIPT }, status, attrs }));
    }
    // Credential stuffing on the login route every 45 minutes.
    if (1200..1320).contains(&(second % 2700)) {
        for _ in 0..rng.range(1, 4) {
            let (ip, country) = ATTACKERS[(((second / 2700) + 2) % ATTACKERS.len() as u64) as usize];
            let attrs = vec![("appsec.events.users.login.failure.track".into(), json!("true")), ("appsec.events.users.login.failure.usr.id".into(), json!(pick(rng, &USERS)))];
            spans.push(span(rng, now_ns, Request { service: "api-gateway", method: "POST", route: "/api/v1/login", path: "/api/v1/login", ip, country, ua: UA_SCRIPT, status: 401, attrs }));
        }
    }
    let mut by: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    for (svc, s) in spans {
        by.entry(svc).or_default().push(s);
    }
    let resource_spans: Vec<Value> = by
        .into_iter()
        .map(|(svc, spans)| {
            let lang = if svc == "api-gateway" { "go" } else { "java" };
            json!({
                "resource": { "attributes": attrs_json(&[("service.name".into(), json!(svc)), ("deployment.environment".into(), json!("prod")), ("telemetry.sdk.language".into(), json!(lang))]) },
                "scopeSpans": [{ "scope": { "name": "oc-tracegen-appsec" }, "spans": spans }],
            })
        })
        .collect();
    json!({ "resourceSpans": resource_spans })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_stuffing_come_on_schedule() {
        let mut rng = Rng(11);
        let mut attacks = 0;
        let mut failures = 0;
        for second in 0..2700 {
            let v = tick(&mut rng, 1_793_000_000_000_000_000 + second * 1_000_000_000, second);
            let text = v.to_string();
            attacks += text.matches("_dd.appsec.json").count();
            failures += text.matches("login.failure.track").count();
        }
        assert!(attacks > 300, "{attacks}");
        assert!(failures > 100, "{failures}");
    }
}
