//! Integration pipelines: the log pipelines of integrations-core (BSD-3-Clause; copyright and terms
//! in each `server/seeds/integrations/<integration>/LICENSE`), converted by
//! `scripts/import-integration-pipelines.mjs`. Organizations install them from Logs → Pipelines, or with their
//! integration from Integrations (`routes::integration_installs`). Ours (MIT,
//! `server/seeds/pipelines/observecraft.json`) sit next to them: generic formats integrations-core has no pipeline
//! for, such as key-value (logfmt) lines.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone)]
pub struct IntegrationPipeline {
    pub id: String,
    /// The catalog integration it belongs to (`supply-chain-firewall`), installed with it; empty for ours.
    pub integration: String,
    pub name: String,
    pub filter: String,
    pub processors: Vec<Value>,
    pub source: Upstream,
}

/// Where an integration pipeline comes from.
#[derive(Deserialize, Clone)]
pub struct Upstream {
    pub repository: String,
    pub revision: String,
    pub path: String,
    pub license: String,
}

/// Every integration's pipelines (scripts/import-integration-pipelines.mjs --all).
const SEEDS: &str = include_str!("../../../../seeds/integrations/core/pipelines.json");
/// Our own pipelines, for generic formats (MIT).
const OWN: &str = include_str!("../../../../seeds/pipelines/observecraft.json");

/// Every integration pipeline, by name.
pub fn all() -> &'static [IntegrationPipeline] {
    static ALL: OnceLock<Vec<IntegrationPipeline>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<IntegrationPipeline> = serde_json::from_str(SEEDS).expect("integration pipelines seed");
        all.extend(serde_json::from_str::<Vec<IntegrationPipeline>>(OWN).expect("own pipelines seed"));
        all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        all
    })
}

pub fn get(id: &str) -> Option<&'static IntegrationPipeline> {
    all().iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::super::grok::Grok;
    use super::super::Pipeline;
    use super::*;

    /// The grok parsers of a processor list, nested pipelines included.
    fn groks(processors: &[Value], out: &mut Vec<Value>) {
        for p in processors {
            match p["type"].as_str() {
                Some("grok-parser") => out.push(p.clone()),
                Some("pipeline") => groks(p["processors"].as_array().map(Vec::as_slice).unwrap_or_default(), out),
                _ => {}
            }
        }
    }

    #[test]
    fn logfmt_lines_become_attributes_status_date_and_message() {
        let p = get("logfmt").unwrap();
        let (_, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
        assert!(errors.is_empty(), "{errors:?}");
        let log = run(
            "logfmt",
            "go",
            r#"time=2026-11-08T10:15:42.120Z level=warn msg="finished call" grpc.component=server grpc.method=PlaceOrder grpc.time_ms=12.4 peer.address=10.0.0.5:51234"#,
        );
        assert_eq!(log.message, "finished call");
        assert_eq!(log.status, "warn");
        assert_eq!(log.timestamp_ms, Some(1_794_132_942_120));
        assert_eq!(log.get("grpc.component"), Some(serde_json::json!("server")));
        assert_eq!(log.get("grpc.method"), Some(serde_json::json!("PlaceOrder")));
        assert_eq!(log.get("peer.address"), Some(serde_json::json!("10.0.0.5:51234")));
        // Other lines are left as they are.
        for line in ["GET /cart 200", "error: retry=3 failed", "connection refused"] {
            let (pipeline, _) = Pipeline::compile(&p.name, &p.filter, &p.processors);
            let mut log = super::super::Log::from_row(&serde_json::json!({"status": "info", "service": "storefront-api", "source": "go", "message": line, "attributes": "{}"}));
            pipeline.apply(&mut log, 1_792_933_200_000);
            assert_eq!((log.message.as_str(), log.status.as_str(), log.attrs.len()), (line, "info", 0), "{line}");
        }
    }

    /// A log of `source` with these attributes.
    fn with_attrs(id: &str, source: &str, attrs: serde_json::Value) -> super::super::Log {
        let p = get(id).unwrap();
        let (pipeline, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
        assert!(errors.is_empty(), "{errors:?}");
        let mut log = super::super::Log::from_row(&serde_json::json!({"status": "info", "service": source, "source": source, "message": "event", "attributes": attrs.to_string()}));
        assert!(pipeline.apply(&mut log, 1_792_933_200_000), "{id} matches source:{source}");
        log
    }

    #[test]
    fn authentik_events_get_categories_users_and_statuses() {
        use crate::authentik::{fixtures, logs::log_of};
        use serde_json::json;
        assert_eq!(get("authentik").unwrap().integration, "authentik", "installed with the integration");
        // The poller's logs: categorized.
        let attrs = |event: Value| {
            let mut log = log_of(&event, "portal.example.com");
            for k in ["date", "message", "status", "service", "ddsource", "host", "_oc_ingest"] {
                log.as_object_mut().unwrap().remove(k);
            }
            log
        };
        let login = with_attrs("authentik", "authentik", attrs(fixtures::login("1", "2026-11-06T10:00:00Z")));
        assert_eq!((login.get("authentik.category"), login.status.as_str()), (Some(json!("authentication")), "info"));
        let device = fixtures::event("2", "model_created", "2026-11-06T10:00:00Z", json!({ "model": { "app": "authentik_stages_authenticator_totp", "model_name": "totpdevice", "name": "phone" } }));
        assert_eq!(with_attrs("authentik", "authentik", attrs(device)).get("authentik.category"), Some(json!("mfa")));
        let group = fixtures::event("3", "model_updated", "2026-11-06T10:00:00Z", json!({ "model": { "app": "authentik_core", "model_name": "group", "name": "admins" } }));
        assert_eq!(with_attrs("authentik", "authentik", attrs(group)).get("authentik.category"), Some(json!("configuration_change")));
        let mut failed = fixtures::event("4", "login_failed", "2026-11-06T10:00:00Z", json!({ "username": "admin", "stage": { "name": "default-authentication-password" } }));
        failed["user"] = json!({ "pk": 1, "username": "AnonymousUser" });
        let failed = with_attrs("authentik", "authentik", attrs(failed));
        assert_eq!((failed.get("authentik.category"), failed.get("usr.name"), failed.status.as_str()), (Some(json!("authentication")), Some(json!("admin")), "error"), "the user who tried");
        let impersonation = with_attrs("authentik", "authentik", attrs(fixtures::event("5", "impersonation_started", "2026-11-06T10:00:00Z", json!({}))));
        assert_eq!((impersonation.get("authentik.category"), impersonation.status.as_str()), (Some(json!("impersonation")), "warn"));
        assert_eq!(with_attrs("authentik", "authentik", attrs(fixtures::event("6", "custom_thing", "2026-11-06T10:00:00Z", json!({})))).get("authentik.category"), Some(json!("other")));
        // An event as authentik's API serializes it, shipped another way (a notification webhook): remapped.
        let raw = with_attrs("authentik", "authentik", fixtures::event("7", "suspicious_request", "2026-11-06T10:00:00.250Z", json!({ "geo": { "country": "RO", "city": "Bucharest" }, "message": "too many attempts" })));
        assert_eq!((raw.get("evt.name"), raw.get("usr.name"), raw.get("usr.email")), (Some(json!("suspicious_request")), Some(json!("jordan.patel")), Some(json!("jordan.patel@example.com"))));
        assert_eq!((raw.get("network.client.ip"), raw.get("network.client.geoip.country.iso_code"), raw.get("network.client.geoip.city.name")), (Some(json!("203.0.113.24")), Some(json!("RO")), Some(json!("Bucharest"))));
        assert_eq!((raw.get("authentik.category"), raw.status.as_str(), raw.get("authentik.context.message")), (Some(json!("suspicious")), "error", Some(json!("too many attempts"))));
        assert_eq!(raw.timestamp_ms, Some(1_793_959_200_250), "authentik's time");
    }

    #[test]
    fn fleetdm_activities_get_categories_users_and_statuses() {
        use crate::fleetdm::{fixtures, logs::log_of};
        use serde_json::json;
        assert_eq!(get("fleetdm").unwrap().integration, "fleetdm", "installed with the integration");
        let attrs = |activity: Value| {
            let mut log = log_of(&activity, "fleet.example.com");
            for k in ["date", "message", "status", "service", "ddsource", "host", "_oc_ingest"] {
                log.as_object_mut().unwrap().remove(k);
            }
            log
        };
        let run = |activity: Value| with_attrs("fleetdm", "fleetdm", attrs(activity));
        let policy = run(fixtures::created_policy(1, "2026-11-06T10:00:00Z"));
        assert_eq!((policy.get("fleetdm.category"), policy.status.as_str()), (Some(json!("policy")), "info"));
        let wiped = run(fixtures::activity(2, "wiped_host", "2026-11-06T10:00:00Z", json!({ "host_id": 3, "host_display_name": "desktop-sam" })));
        assert_eq!((wiped.get("fleetdm.category"), wiped.status.as_str()), (Some(json!("host_action")), "warn"));
        let install = run(fixtures::activity(3, "installed_software", "2026-11-06T10:00:00Z", json!({ "software_title": "zoom.us.app", "status": "failed_install" })));
        assert_eq!((install.get("fleetdm.category"), install.status.as_str()), (Some(json!("software")), "error"));
        let profile = run(fixtures::activity(4, "created_macos_profile", "2026-11-06T10:00:00Z", json!({ "profile_name": "Wi-Fi" })));
        assert_eq!(profile.get("fleetdm.category"), Some(json!("mdm")));
        let mut failed = fixtures::activity(5, "user_failed_login", "2026-11-06T10:00:00Z", json!({ "email": "admin@example.com", "public_ip": "198.51.100.7" }));
        failed["actor_email"] = Value::Null;
        let failed = run(failed);
        assert_eq!((failed.get("fleetdm.category"), failed.get("usr.email"), failed.status.as_str()), (Some(json!("authentication")), Some(json!("admin@example.com")), "error"));
        assert_eq!(run(fixtures::activity(6, "something_new", "2026-11-06T10:00:00Z", json!({}))).get("fleetdm.category"), Some(json!("other")));
        // An activity as Fleet's audit log destination writes it: remapped.
        let raw = with_attrs("fleetdm", "fleetdm", fixtures::activity(7, "user_logged_in", "2026-11-06T10:00:00.000Z", json!({ "public_ip": "203.0.113.24" })));
        assert_eq!((raw.get("evt.name"), raw.get("usr.name"), raw.get("usr.email"), raw.get("network.client.ip")), (Some(json!("user_logged_in")), Some(json!("Jordan Patel")), Some(json!("jordan.patel@example.com")), Some(json!("203.0.113.24"))));
        assert_eq!((raw.get("fleetdm.category"), raw.timestamp_ms), (Some(json!("authentication")), Some(1_793_959_200_000)));
    }

    #[test]
    fn every_integrations_pipelines_are_listed() {
        assert!(all().len() >= 240, "{} pipelines", all().len());
        for id in ["nginx", "postgres", "mysql", "redisdb", "cisco_asa", "mongo", "symantec_endpoint_protection"] {
            assert!(get(id).is_some(), "{id}");
        }
    }

    /// Runs an integration pipeline on a log of its source.
    fn run(id: &str, source: &str, message: &str) -> super::super::Log {
        let p = get(id).unwrap();
        let (pipeline, _) = Pipeline::compile(&p.name, &p.filter, &p.processors);
        let mut log = super::super::Log::from_row(&serde_json::json!({"status": "info", "service": "storefront-api", "source": source, "message": message, "attributes": "{}"}));
        assert!(pipeline.apply(&mut log, 1_792_933_200_000), "{id} matches source:{source}");
        log
    }

    #[test]
    fn nginx_access_and_error_logs() {
        let log = run("nginx", "nginx", r#"10.0.0.12 - - [25/Oct/2026:12:59:58 +0000] "GET /cart?id=7 HTTP/1.1" 503 612 "-" "curl/8.4.0""#);
        assert_eq!(log.get("http.status_code"), Some(serde_json::json!(503)));
        assert_eq!(log.get("http.method"), Some(serde_json::json!("GET")));
        assert_eq!(log.get("network.client.ip"), Some(serde_json::json!("10.0.0.12")));
        assert_eq!(log.get("http.url_details.path"), Some(serde_json::json!("/cart")));
        assert_eq!(log.get("http.status_category"), Some(serde_json::json!("error")));
        assert_eq!((log.status.as_str(), log.timestamp_ms), ("error", Some(1_792_933_198_000)));
        let log =
            run("nginx", "nginx", r#"2026/10/25 12:59:58 [warn] 7#7: *1 upstream timed out, client: 10.0.0.12, server: shop.example.com, request: "GET /cart HTTP/1.1", host: "shop.example.com""#);
        assert_eq!(log.status, "warn");
        assert_eq!(log.get("network.client.ip"), Some(serde_json::json!("10.0.0.12")));
        assert_eq!(log.get("client"), None);
        assert_eq!(log.get("http.url"), Some(serde_json::json!("/cart")));
    }

    /// As integrations-core's `forescout_tests.yaml` expects (Datadog's engine): the stray `|` in its rule is ignored.
    #[test]
    fn forescout_syslog_events() {
        let log = run("forescout", "forescout", "<165>Sep 22 11:44:01 ddsfs forescout-syslog[2895]: Port bite. Source: 10.10.10.10. Destination: 20.20.20.20:139");
        assert_eq!(log.get("appliance"), Some(serde_json::json!("ddsfs")));
        assert_eq!(log.get("identifier"), Some(serde_json::json!("forescout-syslog")));
        assert_eq!(log.get("event_type_identifier"), Some(serde_json::json!("Port bite")));
        assert_eq!(log.get("evt.name"), Some(serde_json::json!("bite-events")));
        assert_eq!(log.get("network.destination.ip"), Some(serde_json::json!("20.20.20.20")));
        assert_eq!(log.get("network.destination.port"), Some(serde_json::json!(139)));
    }

    /// As integrations-core's `mac-audit-logs_tests.yaml` expects: the audit record's XML as attributes.
    #[test]
    fn mac_audit_records() {
        let xml = r#"<record version="11" event="AUE_CONNECT" modifier="0" time="Tue Feb 25 12:11:44 2025" msec=" + 922 msec" ><argument arg-num="1" value="0x3" desc="fd" /><socket-inet type="2" port="15234" addr="10.10.10.10" /><subject audit-uid="it" uid="root" gid="wheel" ruid="root" rgid="wheel" pid="72739" sid="100003" tid="50331650 0.0.0.0" /><return errval="success" retval="0" /><identity signer-type="1" signing-id="com.apple.nc" signing-id-truncated="no" team-id="" team-id-truncated="no" cdhash="0x023e2436839a72573bd0007c9173f298d9f094f8" /></record>"#;
        let log = run("mac_audit_logs", "mac-audit-logs", xml);
        assert_eq!(log.get("record.event"), Some(serde_json::json!("AUE_CONNECT")));
        assert_eq!(log.get("record.argument"), Some(serde_json::json!({"arg-num": "1", "desc": "fd", "value": "0x3"})));
        assert_eq!(log.get("record.return.errval"), Some(serde_json::json!("success")));
        assert_eq!(log.get("usr.name"), Some(serde_json::json!("root")));
        assert_eq!(log.get("network.destination.ip"), Some(serde_json::json!("10.10.10.10")));
        assert_eq!(log.get("event-category"), Some(serde_json::json!("Network")));
        assert_eq!(log.get("result"), Some(serde_json::json!("success")));
    }

    #[test]
    fn redis_and_etcd_statuses() {
        // Redis marks severities with a symbol: '#' is a warning.
        let log = run("redisdb", "redis", "1:M 25 Oct 2026 12:59:58.000 # WARNING overcommit_memory is set to 0");
        assert_eq!((log.status.as_str(), log.message.as_str()), ("warn", "WARNING overcommit_memory is set to 0"));
        let log = run("redisdb", "redis", "1:M 25 Oct 2026 12:59:58.000 * Ready to accept connections");
        assert_eq!(log.status, "info");
        let log = run("etcd", "etcd", "2026-10-25 12:59:58.123456 E | rafthttp: failed to dial peer");
        assert_eq!((log.status.as_str(), log.message.as_str()), ("error", "failed to dial peer"));
        assert_eq!(log.get("package"), Some(serde_json::json!("rafthttp")));
    }

    /// The pipelines the mock offers too: every sample of theirs parses.
    const CURATED: [&str; 8] = ["coredns", "etcd", "kube_scheduler", "mysql", "nginx", "nginx_ingress_controller", "postgres", "redisdb"];

    #[test]
    fn integration_pipelines_compile_and_parse_their_samples() {
        let (mut parsers_total, mut parsers_ok) = (0, 0);
        for p in all() {
            let (_, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
            assert!(errors.is_empty(), "{}: {errors:?}", p.id);
            let mut parsers = Vec::new();
            groks(&p.processors, &mut parsers);
            for g in parsers {
                let rules = &g["grok"];
                let grok = Grok::compile(rules["supportRules"].as_str().unwrap_or(""), rules["matchRules"].as_str().unwrap_or("")).unwrap();
                let samples = g["samples"].as_array().map(Vec::as_slice).unwrap_or_default();
                let failing: Vec<&str> = samples.iter().filter_map(Value::as_str).filter(|s| grok.parse(s, 1_792_933_200_000).is_none()).collect();
                if CURATED.contains(&p.id.as_str()) {
                    assert!(failing.is_empty(), "{} / {}: samples don't parse: {failing:?}", p.id, g["name"]);
                }
                if !samples.is_empty() {
                    parsers_total += 1;
                    parsers_ok += usize::from(failing.is_empty());
                }
            }
        }
        // The rest are upstream samples that don't match their own rules (typos, multi-line logs, negative examples).
        assert!(parsers_ok * 100 >= parsers_total * 95, "{parsers_ok} of {parsers_total} parsers parse their samples");
    }
}
