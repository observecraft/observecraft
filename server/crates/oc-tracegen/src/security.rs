//! Audit and access logs for Cloud SIEM's content packs, in the shapes those sources send them
//! (Datadog's log intake, `ddsource` per source): AWS CloudTrail, Okta, GitHub audit, Kubernetes
//! audit and nginx access logs. Mostly routine activity, with now and then what the default rules
//! look for: console brute force, MFA fatigue, a repository made public, an exec into a pod, bursts
//! of web attacks. All identities, accounts and addresses are fictional (documentation IP ranges,
//! account 123456789012, `@example.com` people).

use serde_json::{json, Value};

use crate::Rng;

const ACCOUNT: &str = "123456789012";
const PEOPLE: [&str; 6] = ["ana.popescu", "dev.kumar", "maya.chen", "lucia.torres", "arjun.patel", "sam.okafor"];
const OFFICE_IPS: [&str; 3] = ["192.0.2.10", "192.0.2.24", "192.0.2.57"];
const ATTACKER_IPS: [&str; 8] = ["203.0.113.7", "203.0.113.41", "203.0.113.88", "198.51.100.23", "198.51.100.71", "198.51.100.140", "203.0.113.150", "198.51.100.201"];
const COUNTRIES: [&str; 4] = ["RO", "DE", "NL", "US"];
const ATTACKER_COUNTRIES: [&str; 3] = ["CA", "BR", "VN"];
const REPOS: [&str; 4] = ["example-org/storefront", "example-org/order-service", "example-org/billing", "example-org/infra"];
const WEB_SERVICES: [&str; 2] = ["storefront-api", "api-gateway"];

fn pick<'a>(rng: &mut Rng, items: &[&'a str]) -> &'a str {
    items[rng.range(0, items.len() as u64) as usize]
}

fn email(person: &str) -> String {
    format!("{person}@example.com")
}

fn arn(person: &str) -> String {
    format!("arn:aws:iam::{ACCOUNT}:user/{person}")
}

fn base(source: &str, service: &str, ts: i64, message: String) -> Value {
    json!({ "ddsource": source, "service": service, "hostname": "", "message": message, "timestamp": ts, "ddtags": "env:prod" })
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Some(b)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    a
}

fn cloudtrail(ts: i64, event: &str, person: &str, ip: &str, outcome: &str, extra: Value) -> Value {
    let identity = if person == "root" {
        json!({ "type": "Root", "arn": format!("arn:aws:iam::{ACCOUNT}:root"), "accountId": ACCOUNT })
    } else {
        json!({ "type": "IAMUser", "arn": arn(person), "accountId": ACCOUNT, "userName": person })
    };
    merge(
        base("cloudtrail", "cloudtrail", ts, format!("{event} by {}", identity["arn"].as_str().unwrap_or_default())),
        merge(
            json!({
                "evt": { "name": event, "outcome": outcome },
                "userIdentity": identity,
                "network": { "client": { "ip": ip } },
                "awsRegion": "eu-central-1",
                "eventSource": if event == "ConsoleLogin" { "signin.amazonaws.com" } else { "iam.amazonaws.com" },
            }),
            extra,
        ),
    )
}

fn okta(ts: i64, event: &str, person: &str, ip: &str, outcome: &str, country: &str) -> Value {
    merge(
        base("okta", "okta", ts, format!("{event} {outcome}")),
        json!({
            "evt": { "name": event, "outcome": outcome },
            "usr": { "email": email(person), "name": person },
            "network": { "client": { "ip": ip, "geoip": { "country": { "iso_code": country } } } },
        }),
    )
}

fn github(ts: i64, event: &str, person: &str, repo: &str, extra: Value) -> Value {
    merge(base("github", "github", ts, format!("{event} on {repo} by {person}")), merge(json!({ "evt": { "name": event }, "usr": { "name": person, "email": email(person) }, "repo": repo }), extra))
}

#[allow(clippy::too_many_arguments)]
fn k8s(ts: i64, user: &str, verb: &str, resource: &str, subresource: &str, namespace: &str, code: u16, ip: &str) -> Value {
    merge(
        base("kubernetes.audit", "kube-apiserver", ts, format!("{verb} {resource}/{subresource} in {namespace} by {user}")),
        json!({
            "usr": { "name": user },
            "verb": verb,
            "objectRef": { "resource": resource, "subresource": subresource, "namespace": namespace },
            "http": { "status_code": code },
            "network": { "client": { "ip": ip } },
            "kube_cluster_name": "shop-prod",
        }),
    )
}

fn nginx(ts: i64, service: &str, ip: &str, path: &str, code: u16, attack: Option<&str>, country: &str) -> Value {
    let mut v = merge(
        base("nginx", service, ts, format!("{ip} - - \"GET {path} HTTP/1.1\" {code}")),
        json!({
            "http": { "method": "GET", "status_code": code, "url": path, "url_details": { "path": path.split('?').next().unwrap_or(path) } },
            "network": { "client": { "ip": ip, "geoip": { "country": { "iso_code": country } } } },
        }),
    );
    if let Some(kind) = attack {
        v["security"] = json!({ "attack": true, "attack_type": kind });
    }
    v
}

/// What happens in one second, as log events. `second` counts the seconds since the start, so the
/// incidents come back on a schedule.
pub fn tick(rng: &mut Rng, now_ms: i64, second: u64) -> Vec<Value> {
    let mut out = Vec::new();
    let ts = |rng: &mut Rng| now_ms - rng.range(0, 1000) as i64;
    // Routine: people signing in and working, with MFA.
    if rng.chance(0.3) {
        let p = pick(rng, &PEOPLE);
        let ip = pick(rng, &OFFICE_IPS);
        out.push(cloudtrail(ts(rng), "ConsoleLogin", p, ip, "success", json!({ "additionalEventData": { "MFAUsed": "Yes" } })));
    }
    if rng.chance(0.6) {
        let p = pick(rng, &PEOPLE);
        let ip = pick(rng, &OFFICE_IPS);
        let event = pick(rng, &["DescribeInstances", "GetObject", "PutObject", "AssumeRole", "ListBuckets", "CreateTags"]);
        out.push(cloudtrail(ts(rng), event, p, ip, "success", json!({})));
    }
    if rng.chance(0.4) {
        let p = pick(rng, &PEOPLE);
        let c = pick(rng, &COUNTRIES);
        let ip = pick(rng, &OFFICE_IPS);
        let event = pick(rng, &["user.session.start", "user.authentication.auth_via_mfa", "app.oauth2.as.token.grant"]);
        out.push(okta(ts(rng), event, p, ip, "SUCCESS", c));
    }
    if rng.chance(0.15) {
        let p = pick(rng, &PEOPLE);
        let r = pick(rng, &REPOS);
        let event = pick(rng, &["git.push", "pull_request.merge", "repo.clone", "workflows.completed_workflow_run"]);
        out.push(github(ts(rng), event, p, r, json!({})));
    }
    if rng.chance(0.5) {
        let ns = pick(rng, &["storefront", "payments", "catalog"]);
        out.push(k8s(ts(rng), "system:serviceaccount:argocd:argocd-application-controller", pick(rng, &["get", "list", "patch", "watch"]), "deployments", "", ns, 200, "10.0.4.8"));
    }
    for _ in 0..rng.range(2, 6) {
        let service = pick(rng, &WEB_SERVICES);
        let ip = pick(rng, &OFFICE_IPS);
        let path = pick(rng, &["/", "/products", "/products/sku-1042", "/cart", "/checkout", "/api/v1/orders"]);
        out.push(nginx(ts(rng), service, ip, path, 200, None, pick(rng, &COUNTRIES)));
    }
    // A scanner probes the web services now and then: the attack-attempts rule fires on bursts.
    let cycle = second % 1800;
    if (300..420).contains(&cycle) || rng.chance(0.02) {
        let ip = ATTACKER_IPS[((second / 1800) % ATTACKER_IPS.len() as u64) as usize];
        let n = if (300..420).contains(&cycle) { rng.range(2, 5) } else { 1 };
        for _ in 0..n {
            let (kind, path) = match rng.range(0, 4) {
                0 => ("sql_injection", "/products?id=1%27%20OR%20%271%27=%271"),
                1 => ("path_traversal", "/static/..%2f..%2fetc%2fpasswd"),
                2 => ("scanner", "/.env"),
                _ => ("xss", "/search?q=%3Cscript%3Ealert(1)%3C/script%3E"),
            };
            let other = pick(rng, &ATTACKER_IPS);
            let from = if rng.chance(0.7) { ip } else { other };
            out.push(nginx(ts(rng), pick(rng, &WEB_SERVICES), from, path, pick(rng, &["403", "404", "400"]).parse().unwrap_or(403), Some(kind), pick(rng, &ATTACKER_COUNTRIES)));
        }
    }
    // Console brute force against one IAM user, every 40 minutes.
    if (600..660).contains(&(second % 2400)) && rng.chance(0.6) {
        let ip = pick(rng, &ATTACKER_IPS);
        out.push(cloudtrail(ts(rng), "ConsoleLogin", "dev.kumar", ip, "failure", json!({ "errorMessage": "Failed authentication" })));
    }
    // MFA fatigue against one person, every 50 minutes.
    if (900..960).contains(&(second % 3000)) && rng.chance(0.4) {
        out.push(okta(ts(rng), "user.authentication.auth_via_mfa", "maya.chen", pick(rng, &ATTACKER_IPS), "FAILURE", pick(rng, &ATTACKER_COUNTRIES)));
    }
    // Rarer events: a console sign-in without MFA, root activity, an exec into a pod, a repository
    // made public, branch protection removed.
    match second % 3600 {
        1200 => out.push(cloudtrail(now_ms, "ConsoleLogin", "sam.okafor", pick(rng, &OFFICE_IPS), "success", json!({ "additionalEventData": { "MFAUsed": "No" } }))),
        1500 => out.push(k8s(now_ms, "lucia.torres@example.com", "create", "pods", "exec", "payments", 101, "192.0.2.24")),
        2100 if second % 7200 == 2100 => out.push(cloudtrail(now_ms, "GetAccountSummary", "root", "192.0.2.57", "success", json!({}))),
        2700 if second % 10800 == 2700 => out.push(github(now_ms, "repo.access", "arjun.patel", "example-org/infra", json!({ "visibility": "public" }))),
        3000 => out.push(github(now_ms, "protected_branch.destroy", "dev.kumar", "example-org/billing", json!({ "branch": "main" }))),
        _ => {}
    }
    out
}

/// Sends the events to the Datadog log intake.
pub async fn send(http: &reqwest::Client, url: &str, events: Vec<Value>) {
    if events.is_empty() {
        return;
    }
    match http.post(url).json(&events).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!(status = %r.status(), "security logs rejected"),
        Err(e) => tracing::warn!(error = %e, "security logs not sent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incidents_come_on_schedule() {
        let mut rng = Rng(7);
        let mut events = Vec::new();
        for second in 0..3600 {
            events.extend(tick(&mut rng, 1_793_000_000_000 + second as i64 * 1000, second));
        }
        let count = |f: &dyn Fn(&Value) -> bool| events.iter().filter(|e| f(e)).count();
        assert!(count(&|e| e["ddsource"] == "cloudtrail" && e["evt"]["outcome"] == "failure") > 20);
        assert!(count(&|e| e["ddsource"] == "okta" && e["evt"]["outcome"] == "FAILURE") > 10);
        assert!(count(&|e| e["security"]["attack"] == true) > 200);
        assert_eq!(count(&|e| e["objectRef"]["subresource"] == "exec"), 1);
        assert!(events.iter().all(|e| !e["message"].as_str().unwrap_or_default().is_empty()));
    }
}
