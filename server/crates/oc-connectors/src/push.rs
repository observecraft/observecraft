//! Push packages on the intake (docs/connectors.md#push-only-vendors-and-webhooks): what the intake needs of a package
//! ([`PushDef`], answered by the server's internal listener), the vendors' signature checks ([`verify`]) and a
//! webhook body as logs and events ([`webhook`]). No I/O: the intake fetches definitions and the org's signing
//! secrets, and publishes what this returns.

use std::collections::BTreeMap;

use hmac::{KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::expr;
use crate::manifest::{duration_ms, Manifest, Output, Push, PushKind, PushSignature, SignatureAlgorithm, Webhook};
use crate::output::{Item, Mapped, Mapper};
use crate::template::Template;

/// What the intake needs of a push package.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PushDef {
    pub id: String,
    /// The webhook's path segment (`/intake/webhook/<name>`): `webhook.name`, else the id.
    pub name: String,
    /// Logs' `ddsource` (`/api/v2/logs?ddsource=<source>`).
    pub source: String,
    #[serde(default)]
    pub push: Option<Push>,
    #[serde(default)]
    pub webhook: Option<Webhook>,
    #[serde(default)]
    pub tables: BTreeMap<String, Value>,
}

impl PushDef {
    /// The definition of a package that is pushed to (None for one that only polls).
    pub fn of(m: &Manifest) -> Option<PushDef> {
        if m.push.is_none() && m.webhook.is_none() {
            return None;
        }
        Some(PushDef {
            id: m.id.clone(),
            name: m.webhook.as_ref().and_then(|w| w.name.clone()).unwrap_or_else(|| m.id.clone()),
            source: m.source.clone().unwrap_or_else(|| m.id.clone()),
            push: m.push.clone(),
            webhook: m.webhook.clone(),
            tables: m.tables.clone(),
        })
    }

    pub fn signature(&self) -> Option<&PushSignature> {
        self.push.as_ref().and_then(|p| p.signature.as_ref())
    }

    /// Whether logs sent to `/api/v2/logs?ddsource=<source>` are this package's, signed.
    pub fn signs_logs(&self) -> bool {
        self.push.as_ref().is_some_and(|p| p.kind == PushKind::Logs && p.signature.is_some())
    }

    /// Whether `/intake/webhook/<name>` is this package's.
    pub fn takes_webhook(&self) -> bool {
        self.webhook.is_some()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time equality of two strings' bytes.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hmac_sha256(secret: &str, parts: &[&[u8]]) -> String {
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    for p in parts {
        mac.update(p);
    }
    hex(&mac.finalize().into_bytes())
}

fn hmac_sha1(secret: &str, body: &[u8]) -> String {
    let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(body);
    hex(&mac.finalize().into_bytes())
}

/// Why a pushed request was refused.
pub const MISSING: &str = "the request is not signed";
pub const INVALID: &str = "the request's signature does not match";
pub const EXPIRED: &str = "the request's signature is too old";

/// Checks a request's signature (`header`: the value of `sig.header`) against each of the connection secrets: any
/// one matching admits it (a secret being rotated, several connections).
pub fn verify(sig: &PushSignature, header: Option<&str>, body: &[u8], secrets: &[String], now_s: i64) -> Result<(), &'static str> {
    let raw = header.map(str::trim).filter(|h| !h.is_empty()).ok_or(MISSING)?;
    let value = match sig.prefix.as_deref() {
        Some(p) => raw.strip_prefix(p).or_else(|| (raw.len() >= p.len() && raw[..p.len()].eq_ignore_ascii_case(p)).then(|| &raw[p.len()..])).ok_or(INVALID)?,
        None => raw,
    }
    .trim();
    let lower = value.to_ascii_lowercase();
    // `k=v` pairs (Stripe's `t=…,v1=…`, PagerDuty's `v1=…,v1=…`).
    let pairs = || value.split(',').filter_map(|kv| kv.trim().split_once('=')).map(|(k, v)| (k.trim(), v.trim().to_ascii_lowercase()));
    let ok = match sig.algorithm {
        SignatureAlgorithm::HmacSha256 => secrets.iter().any(|s| same(&hmac_sha256(s, &[body]), &lower)),
        SignatureAlgorithm::HmacSha1 => secrets.iter().any(|s| same(&hmac_sha1(s, body), &lower)),
        SignatureAlgorithm::PagerdutyV3 => {
            let given: Vec<String> = pairs().filter(|(k, _)| *k == "v1").map(|(_, v)| v).collect();
            secrets.iter().any(|s| {
                let want = hmac_sha256(s, &[body]);
                given.iter().any(|g| same(&want, g))
            })
        }
        SignatureAlgorithm::Stripe => {
            let t: i64 = pairs().find(|(k, _)| *k == "t").and_then(|(_, v)| v.parse().ok()).ok_or(INVALID)?;
            let tolerance = sig.tolerance.as_deref().and_then(|d| duration_ms(d).ok()).unwrap_or(300_000) / 1000;
            let given: Vec<String> = pairs().filter(|(k, _)| *k == "v1").map(|(_, v)| v).collect();
            let ok = secrets.iter().any(|s| {
                let want = hmac_sha256(s, &[t.to_string().as_bytes(), b".", body]);
                given.iter().any(|g| same(&want, g))
            });
            if ok && (now_s - t).abs() > tolerance {
                return Err(EXPIRED);
            }
            ok
        }
        SignatureAlgorithm::Token => secrets.iter().any(|s| same(s.trim(), value)),
    };
    if ok {
        Ok(())
    } else {
        Err(INVALID)
    }
}

/// A webhook body (JSON) as logs and events, with the request's headers (lowercase names) readable as `headers`.
pub fn webhook(def: &PushDef, body: &Value, headers: &Map<String, Value>, now_ms: i64) -> Result<Mapped, String> {
    let w = def.webhook.as_ref().ok_or_else(|| format!("{} has no webhook", def.id))?;
    let mapper = Mapper::compile_for(&def.id, &def.source, &Output { logs: w.logs.clone(), events: w.events.clone(), ..Output::default() })?;
    let tpl = |t: &Option<String>| t.as_deref().map(Template::parse).transpose();
    let (records, record_id, filter) = (tpl(&w.records)?, tpl(&w.record_id)?, tpl(&w.filter)?);
    let mut scope = Map::new();
    scope.insert("body".into(), body.clone());
    scope.insert("headers".into(), Value::Object(headers.clone()));
    scope.insert("tables".into(), json!(def.tables));
    scope.insert("source".into(), json!(def.source));
    scope.insert("now".into(), json!(now_ms));
    let list = match records.map(|t| t.render(&scope)) {
        Some(Value::Array(a)) => a,
        Some(Value::Null) => vec![],
        Some(v) => vec![v],
        None => match body {
            Value::Array(a) => a.clone(),
            v => vec![v.clone()],
        },
    };
    let mut out = Mapped::default();
    for record in list {
        let mut s = scope.clone();
        s.insert("record".into(), record);
        if filter.as_ref().is_some_and(|f| !expr::truthy(&f.render(&s))) {
            continue;
        }
        let id = match &record_id {
            Some(t) => t.text(&s),
            None => serde_json::to_string(&s["record"]).unwrap_or_default(),
        };
        mapper.map(&Item { scope: &s, time_ms: None, id: &id, instance: "", now_ms }, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(algorithm: SignatureAlgorithm, prefix: Option<&str>) -> PushSignature {
        PushSignature { header: "x-signature".into(), algorithm, secret: "signing_secret".into(), prefix: prefix.map(str::to_string), tolerance: None }
    }

    #[test]
    fn github_and_launchdarkly_style_hmac_sha256() {
        // GitHub's documented example.
        let s = sig(SignatureAlgorithm::HmacSha256, Some("sha256="));
        let secrets = vec!["wrong".to_string(), "It's a Secret to Everybody".to_string()];
        let good = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert_eq!(verify(&s, Some(good), b"Hello, World!", &secrets, 0), Ok(()));
        assert_eq!(verify(&s, Some(good), b"Hello, World?", &secrets, 0), Err(INVALID));
        assert_eq!(verify(&s, Some(&good[7..]), b"Hello, World!", &secrets, 0), Err(INVALID), "the prefix is required");
        assert_eq!(verify(&s, None, b"Hello, World!", &secrets, 0), Err(MISSING));
        assert_eq!(verify(&s, Some(good), b"Hello, World!", &["wrong".to_string()], 0), Err(INVALID));
        // X-LD-Signature: the bare hex.
        let ld = sig(SignatureAlgorithm::HmacSha256, None);
        assert_eq!(verify(&ld, Some(&good[7..].to_uppercase()), b"Hello, World!", &secrets, 0), Ok(()), "hex in any case");
    }

    #[test]
    fn vercel_sha1() {
        let s = sig(SignatureAlgorithm::HmacSha1, None);
        let secrets = vec!["vercel-secret".to_string()];
        assert_eq!(verify(&s, Some("f62616bed9c901fa3e53616136188ecce2ba4312"), br#"[{"message":"hi"}]"#, &secrets, 0), Ok(()));
        assert_eq!(verify(&s, Some("f62616bed9c901fa3e53616136188ecce2ba4313"), br#"[{"message":"hi"}]"#, &secrets, 0), Err(INVALID));
    }

    #[test]
    fn stripe_timestamp_and_v1() {
        let s = sig(SignatureAlgorithm::Stripe, None);
        let secrets = vec!["whsec_test".to_string()];
        let body = br#"{"id":"evt_1"}"#;
        let header = "t=1794312000,v1=0000,v1=971fd22673504bbb8a9fa4674c20d55533b3df9d802dfe7b2a053c3bcf245f28,v0=abc";
        assert_eq!(verify(&s, Some(header), body, &secrets, 1_794_312_100), Ok(()));
        assert_eq!(verify(&s, Some(header), body, &secrets, 1_794_312_000 + 301), Err(EXPIRED), "replayed after 5 minutes");
        assert_eq!(verify(&s, Some("t=1794312001,v1=971fd22673504bbb8a9fa4674c20d55533b3df9d802dfe7b2a053c3bcf245f28"), body, &secrets, 1_794_312_000), Err(INVALID), "the timestamp is signed");
        assert_eq!(verify(&s, Some("v1=971fd22673504bbb8a9fa4674c20d55533b3df9d802dfe7b2a053c3bcf245f28"), body, &secrets, 1_794_312_000), Err(INVALID));
    }

    #[test]
    fn pagerduty_v3_any_of_several() {
        let s = sig(SignatureAlgorithm::PagerdutyV3, None);
        let secrets = vec!["pd-secret".to_string()];
        let body = br#"{"event":{}}"#;
        assert_eq!(verify(&s, Some("v1=deadbeef,v1=77fb999cdab1e010c5ae57458638a56696f56610cdadede6c7ceb94c77843e0b"), body, &secrets, 0), Ok(()));
        assert_eq!(verify(&s, Some("v1=deadbeef"), body, &secrets, 0), Err(INVALID));
    }

    #[test]
    fn shared_header_token() {
        let s = sig(SignatureAlgorithm::Token, Some("Bearer "));
        let secrets = vec!["tok-123".to_string()];
        assert_eq!(verify(&s, Some("Bearer tok-123"), b"", &secrets, 0), Ok(()));
        assert_eq!(verify(&s, Some("bearer tok-123"), b"", &secrets, 0), Ok(()));
        assert_eq!(verify(&s, Some("Bearer tok-124"), b"", &secrets, 0), Err(INVALID));
        assert_eq!(verify(&s, Some("Bearer tok-123"), b"", &[], 0), Err(INVALID), "no secret saved admits nothing");
    }

    #[test]
    fn webhook_bodies_become_logs_and_events() {
        let m: Manifest = serde_yaml_ng::from_str(
            "schema_version: 1\nid: example-billing\nname: Example Billing\npublisher: Example\nvendor: Example\ndescription: Billing events.\n\
             config:\n  - { key: signing_secret, label: Signing secret, type: string, secret: true }\n\
             push: { kind: webhook, instructions: Add a webhook, signature: { header: Stripe-Signature, algorithm: stripe, secret: signing_secret } }\n\
             webhook:\n  records: \"{{ body.events }}\"\n  record_id: \"{{ record.id }}\"\n  filter: \"{{ record.type != 'ping' }}\"\n\
             \x20 logs: { message: \"{{ record.type }}\", date: \"{{ record.created }}\", attributes: { evt: { name: \"{{ record.type }}\" }, delivery: \"{{ headers['x-delivery'] }}\" } }\n\
             \x20 events: { title: \"Billing: {{ record.type }}\", text: \"{{ record.id }}\", alert_type: \"{{ record.type == 'charge.failed' ? 'error' : 'info' }}\" }\n",
        )
        .unwrap();
        assert!(crate::validate::validate(&m, &Default::default()).is_empty(), "{:?}", crate::validate::validate(&m, &Default::default()));
        let def = PushDef::of(&m).unwrap();
        assert_eq!((def.name.as_str(), def.source.as_str(), def.takes_webhook(), def.signs_logs()), ("example-billing", "example-billing", true, false));
        let body = json!({ "events": [
            { "id": "evt_1", "type": "charge.failed", "created": 1794312000 },
            { "id": "evt_2", "type": "ping", "created": 1794312000 },
            { "id": "evt_3", "type": "invoice.paid", "created": 1794312060 }
        ] });
        let headers: Map<String, Value> = [("x-delivery".to_string(), json!("d-1"))].into_iter().collect();
        let out = webhook(&def, &body, &headers, 1_794_312_500_000).unwrap();
        assert_eq!(out.logs.len(), 2, "pings filtered out");
        assert_eq!(
            (&out.logs[0]["message"], &out.logs[0]["ddsource"], &out.logs[0]["evt"]["name"], &out.logs[0]["delivery"]),
            (&json!("charge.failed"), &json!("example-billing"), &json!("charge.failed"), &json!("d-1"))
        );
        let again = webhook(&def, &body, &headers, 1_794_312_900_000).unwrap();
        assert_eq!(out.logs[0]["_oc_ingest"], again.logs[0]["_oc_ingest"], "a retried delivery keeps its identity");
        assert_eq!((out.events[0].title.as_str(), out.events[0].alert_type.as_str(), out.events[1].alert_type.as_str()), ("Billing: charge.failed", "error", "info"));
        assert_eq!(out.events[1].time_ms, 1_794_312_500_000, "events without a date are the delivery's");
    }
}
