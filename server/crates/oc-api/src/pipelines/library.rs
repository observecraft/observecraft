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
    fn gitlab_audit_events_get_categories_users_and_statuses() {
        use crate::gitlab_audit::{fixtures, logs::log_of};
        use serde_json::json;
        assert_eq!(get("gitlab-audit-events").unwrap().integration, "gitlab-audit-events", "installed with the integration");
        let attrs = |event: Value| {
            let mut log = log_of(&event, "gitlab.example.com");
            for k in ["date", "message", "status", "service", "ddsource", "host", "_oc_ingest"] {
                log.as_object_mut().unwrap().remove(k);
            }
            log
        };
        let run = |event: Value| with_attrs("gitlab-audit-events", "gitlab-audit-events", attrs(event));
        let category = |name: &str| run(fixtures::event(1, name, "2026-11-06T10:00:00Z", json!({}))).get("gitlab.category");
        for (name, want) in [
            ("personal_access_token_created", "token"),
            ("group_deploy_token_revoked", "token"),
            ("merge_request_approval_operation", "merge_request"),
            ("member_created", "membership"),
            ("user_authorized_oauth_application", "authorization"),
            ("user_blocked", "user_management"),
            ("ci_variable_updated", "ci_cd"),
            ("protected_branch_allow_force_push_updated", "repository"),
            ("project_visibility_level_updated", "project"),
            ("group_created", "group"),
            ("audit_events_streaming_headers_create", "compliance"),
            ("something_new", "other"),
        ] {
            assert_eq!(category(name), Some(json!(want)), "{name}");
        }
        let failed = run(fixtures::failed_login(2, "2026-11-06T10:00:00Z", "root"));
        assert_eq!((failed.get("gitlab.category"), failed.get("usr.name"), failed.status.as_str()), (Some(json!("authentication")), Some(json!("root")), "error"));
        let locked = run(fixtures::event(3, "user_access_locked", "2026-11-06T10:00:00Z", json!({})));
        assert_eq!((locked.get("gitlab.category"), locked.status.as_str()), (Some(json!("authentication")), "warn"));
        assert_eq!(run(fixtures::changed(4, "2026-11-06T10:00:00Z")).status, "info");
        // An event as GitLab's API serializes it, shipped another way (a streaming destination, a log shipper): remapped.
        let raw = with_attrs("gitlab-audit-events", "gitlab-audit-events", fixtures::event(5, "personal_access_token_created", "2026-11-06T10:00:00.250Z", json!({})));
        assert_eq!((raw.get("evt.name"), raw.get("usr.id"), raw.get("usr.name"), raw.get("usr.email")), (Some(json!("personal_access_token_created")), Some(json!(42)), Some(json!("Jordan Patel")), Some(json!("jordan.patel@example.com"))));
        assert_eq!((raw.get("network.client.ip"), raw.get("gitlab.entity_path"), raw.get("gitlab.category")), (Some(json!("203.0.113.24")), Some(json!("example-org/storefront-api")), Some(json!("token"))));
        assert_eq!(raw.timestamp_ms, Some(1_793_959_200_250), "GitLab's time");
    }

    /// The Entra ID poller's logs (`crate::entra`) read as the Event Hub records integrations-core's
    /// `azure.activedirectory_tests.yaml` (revision in `pipelines.json`) expects: the same `evt.*`, `usr.*`, client IP,
    /// message, date and title. Its OCSF sub-pipelines are left out of the library (no schema processor here), and its
    /// user-agent and GeoIP parsers are skipped by the engine.
    #[test]
    fn entra_id_logs_read_as_the_event_hubs() {
        use crate::entra::{api::Stream, fixtures, logs::log_of};
        use serde_json::json;
        assert_eq!(get("azure_active_directory").unwrap().integration, "azure-active-directory", "installed with the integration");
        let run = |stream: Stream, event: Value| agent_log("azure_active_directory", log_of(stream, &event, fixtures::TENANT));
        let ok = run(Stream::SignIns, fixtures::sign_in("b95c8eca", "2026-11-06T10:00:00.1234567Z", "jordan.patel@example.com", 0));
        assert_eq!((ok.get("evt.category"), ok.get("evt.name"), ok.get("evt.outcome")), (Some(json!("SignInLogs")), Some(json!("Sign-in activity")), Some(json!("success"))));
        assert_eq!((ok.get("usr.id"), ok.get("usr.email"), ok.get("usr.name")), (Some(json!("jordan.patel@example.com")), Some(json!("jordan.patel@example.com")), Some(json!("Jordan Patel"))));
        assert_eq!((ok.get("network.client.ip"), ok.get("duration").and_then(|d| d.as_f64())), (Some(json!("203.0.113.24")), Some(0.0)));
        assert_eq!((ok.source.as_str(), ok.service.as_str(), ok.status.as_str()), ("azure.activedirectory", "azure", "info"));
        assert_eq!(ok.timestamp_ms, Some(1_793_959_200_123), "the sign-in's time");
        assert_eq!(ok.get("title"), Some(json!("'Sign-in activity' resulted in success initiated by jordan.patel@example.com ")));
        assert_eq!(ok.message, "jordan.patel@example.com signed in to Azure Portal");
        let failed = run(Stream::SignIns, fixtures::sign_in("c95c8eca", "2026-11-06T10:00:00Z", "jordan.patel@example.com", 50126));
        assert_eq!(failed.get("evt.outcome"), Some(json!("failure")));
        assert_eq!(failed.message, "Invalid username or password or Invalid on-premise username or password.", "the failure reason, as `resultDescription`");
        let role = run(Stream::Audits, fixtures::role_assigned("Directory_5d3c_1", "2026-11-06T10:00:00Z", "Global Administrator"));
        assert_eq!((role.get("evt.category"), role.get("evt.name"), role.get("evt.outcome")), (Some(json!("AuditLogs")), Some(json!("Add member to role")), Some(json!("success"))));
        assert_eq!((role.get("usr.id"), role.get("usr.name"), role.get("network.client.ip")), (Some(json!("sam.lee@example.com")), Some(json!("sam.lee@example.com")), Some(json!("198.51.100.7"))));
        assert_eq!(role.get("title"), Some(json!("'Add member to role' resulted in success initiated by sam.lee@example.com ")));
        assert_eq!(role.timestamp_ms, Some(1_793_959_200_000));
        let risk = run(Stream::Risk, fixtures::risk("r-1", "2026-11-06T10:00:00Z", "high"));
        assert_eq!((risk.get("evt.category"), risk.get("network.client.ip"), risk.timestamp_ms), (Some(json!("UserRiskEvents")), Some(json!("198.51.100.200")), Some(1_793_959_200_000)));
    }

    /// A log as the Datadog Agent forwards it to the intake (`/api/v2/logs`), normalized as the log ingester
    /// does, then run through integration pipeline `id` as the indexer does.
    fn agent_log(id: &str, envelope: Value) -> super::super::Log {
        let mut fields = envelope.as_object().unwrap().clone();
        fields.insert("source_type".into(), "kafka".into());
        fields.insert("_oc_kafka_headers".into(), serde_json::json!({"x-oc-org": "example-org"}));
        let mut row = crate::log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: None }, chrono::Utc::now()).unwrap();
        let p = get(id).unwrap();
        let (pipeline, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
        assert!(errors.is_empty(), "{errors:?}");
        let mut log = super::super::Log::default();
        (log.message, log.status, log.service, log.host, log.source) = (row.message, row.status, row.service, row.host, row.source);
        log.tags = row.tags.into_iter().collect();
        log.attrs = std::mem::take(&mut row.attrs);
        assert!(pipeline.apply(&mut log, 1_792_933_200_000), "{id} matches {}", log.source);
        log
    }

    /// Supply Chain Firewall's run logs (`scfw configure`: its Datadog Agent TCP listener or its JSON Lines file
    /// tailed by the Agent, `source:scfw`), as integrations-core's `supply-chain-firewall_tests.yaml` expects:
    /// the fields its dashboard reads (`@usr.name`, `@evt.outcome`, `@warning`, `@install_targets.name`).
    #[test]
    fn supply_chain_firewall_runs() {
        use serde_json::json;
        assert_eq!(get("supply_chain_firewall").unwrap().integration, "supply-chain-firewall", "installed with the integration");
        let line = json!({
            "msg": "Command 'pip install urllib3==2.0.2' was allowed", "ecosystem": "PyPI", "created": 1755017456.590666, "source": "scfw",
            "env": "dev", "version": "3.0.0", "log_version": 1, "install_targets": [{"ecosystem": "PyPI", "name": "urllib3", "version": "2.0.2"}],
            "executable": "/home/devuser/.venv/bin/python", "hostname": "test10-10-1-10", "package_manager": "pip", "service": "scfw",
            "warning": true, "verified": true, "action": "ALLOW", "username": "root", "cwd": "/home/devuser/project"
        });
        let envelope = json!({"message": line.to_string(), "status": "info", "hostname": "dev-laptop-01", "service": "scfw", "ddsource": "scfw", "ddtags": "filename:scfw.log"});
        let log = agent_log("supply_chain_firewall", envelope);
        assert_eq!((log.message.as_str(), log.service.as_str(), log.source.as_str(), log.status.as_str()), ("Command 'pip install urllib3==2.0.2' was allowed", "scfw", "scfw", "info"));
        assert_eq!((log.get("usr.name"), log.get("evt.outcome"), log.get("warning")), (Some(json!("root")), Some(json!("ALLOW")), Some(json!(true))));
        assert_eq!((log.get("package_manager"), log.get("ecosystem"), log.host.as_str()), (Some(json!("pip")), Some(json!("PyPI")), "test10-10-1-10"), "the host scfw ran on");
        assert_eq!(log.get("hostname"), Some(json!("test10-10-1-10")), "kept as `@hostname`, as Datadog does");
        assert_eq!((log.get("install_targets"), log.get("version"), log.get("env")), (Some(json!([{"ecosystem": "PyPI", "name": "urllib3", "version": "2.0.2"}])), Some(json!("3.0.0")), Some(json!("dev"))));
        assert_eq!((log.get("username"), log.get("action")), (None, None), "remapped, not kept");
        assert_eq!(log.timestamp_ms, Some(1_755_017_456_590), "the run's time");
        // Before v3: `warned` and `targets`.
        let old = json!({"msg": "Command 'npm install left-pad' was blocked", "created": 1755017456.5, "action": "BLOCK", "warned": false, "username": "dev", "package_manager": "npm", "targets": ["left-pad-1.3.0"]});
        let log = agent_log("supply_chain_firewall", json!({"message": old.to_string(), "hostname": "dev-laptop-01", "service": "scfw", "ddsource": "scfw"}));
        assert_eq!((log.get("evt.outcome"), log.get("warning"), log.get("warned")), (Some(json!("BLOCK")), Some(json!(false)), None));
    }

    /// GuardDog's scan results (the Agent check's logs) and IDE Shepherd's OCSF events (its Agent TCP listener), as
    /// integrations-core's `guarddog_tests.yaml` and `ide-shepherd_tests.yaml` expect.
    #[test]
    fn guarddog_scans_and_ide_shepherd_events() {
        use serde_json::json;
        let scan = json!({"enrichment_details": {"package_ecosystem": "pypi"}, "log": {"result": {"path": "/tmp/scan/urllib3", "issues": 0, "errors": {"repository_integrity_mismatch": "failed to run rule"}}, "dependency": "urllib3", "version": "1.24.1"}});
        let log = agent_log("guarddog", json!({"message": scan.to_string(), "timestamp": 1758799611000_i64, "hostname": "ci-runner-01", "service": "guarddog", "ddsource": "guarddog"}));
        assert_eq!((log.get("has_errors"), log.get("log.dependency")), (Some(json!("yes")), Some(json!("urllib3"))), "an object attribute exists (`@log.result.errors:*`)");
        let clean = json!({"log": {"result": {"issues": 0, "results": {}}, "dependency": "requests"}});
        assert_eq!(agent_log("guarddog", json!({"message": clean.to_string(), "service": "guarddog", "ddsource": "guarddog"})).get("has_errors"), None);
        let event = json!({"isAlert": false, "category_uid": 2, "class_uid": 2007, "activity_id": 2, "severity_id": 1, "status_id": 1, "type_uid": 200702, "time": 1769418941951_i64,
            "metadata": {"product": {"name": "IDE Shepherd"}, "environment": {"ide_flavor": "cursor"}}, "application": {"name": "rust-lang.rust-analyzer", "version": "0.3.2769"}});
        let log = agent_log("ide_shepherd", json!({"message": event.to_string(), "hostname": "dev-laptop-01", "service": "ide-shepherd-telemetry", "ddsource": "ide-shepherd"}));
        assert_eq!((log.get("ocsf.application.name"), log.get("ocsf.metadata.environment.ide_flavor"), log.get("ocsf.is_alert")), (Some(json!("rust-lang.rust-analyzer")), Some(json!("cursor")), Some(json!(false))));
        assert_eq!((log.get("ide_shepherd.activity_name"), log.get("ide_shepherd.class_name"), log.timestamp_ms), (Some(json!("Update")), Some(json!("Application Security Posture Finding")), Some(1_769_418_941_951)));
    }

    /// The leaves of an expected result's attributes, by dotted path.
    fn leaves(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
        match v.as_object() {
            Some(o) if !o.is_empty() => {
                for (k, v) in o {
                    leaves(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, v, out);
                }
            }
            _ => out.push((prefix.to_string(), v.clone())),
        }
    }

    /// Equal as Datadog's test results compare them: numbers by value (`12` and `12.0`).
    fn same(a: &Value, b: &Value) -> bool {
        match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-6 * x.abs().max(1.0),
            _ => match (a, b) {
                (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y)),
                _ => a == b,
            },
        }
    }

    /// Runs every sample of integrations-core's own tests of an integration pipeline (`assets/logs/<name>_tests.yaml`,
    /// BSD-3-Clause, vendored in `tests/fixtures/pipelines` with its LICENSE) through ours, a raw line of `source`
    /// as the Agent tails it, and lists how the result differs from the one Datadog's engine gives: the attributes,
    /// the message, the status (ours: error, warn, info, debug) and the date (when the sample's has a year).
    fn upstream_differences(id: &str, source: &str, yaml: &str) -> Vec<String> {
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
        let doc: Value = serde_json::to_value(doc).unwrap();
        let mut out = Vec::new();
        let tests = doc["tests"].as_array().unwrap();
        assert!(tests.len() >= 3, "{id}: {} samples", tests.len());
        for (i, t) in tests.iter().enumerate() {
            let sample = t["sample"].as_str().map(str::to_string).unwrap_or_else(|| t["sample"].to_string());
            let log = run(id, source, &sample);
            let r = &t["result"];
            let mut expected = Vec::new();
            leaves("", &r["custom"], &mut expected);
            // Datadog's harness reads a date without a year in 1970; ours, in the current year.
            let yearless = r["timestamp"].as_i64().filter(|ms| *ms < 946_684_800_000);
            for (path, want) in expected.into_iter().filter(|(_, v)| yearless.is_none() || v.as_i64() != yearless) {
                let got = log.get(&path).unwrap_or(Value::Null);
                if !same(&got, &want) {
                    out.push(format!("#{i} @{path}: {got} instead of {want}"));
                }
            }
            if let Some(want) = r["message"].as_str() {
                if log.message != want {
                    out.push(format!("#{i} message: {:?} instead of {want:?}", log.message));
                }
            }
            if let Some(want) = r["status"].as_str() {
                if log.status != super::super::processors::status_of(&Value::from(want)) {
                    out.push(format!("#{i} status: {} instead of {want}", log.status));
                }
            }
            if let Some(want) = r["timestamp"].as_i64().filter(|ms| *ms > 946_684_800_000) {
                if log.timestamp_ms != Some(want) {
                    out.push(format!("#{i} timestamp: {:?} instead of {want}", log.timestamp_ms));
                }
            }
        }
        out
    }

    #[test]
    fn integration_pipelines_give_integrations_cores_results() {
        for (id, source, yaml) in [
            ("postgres", "postgresql", include_str!("../../tests/fixtures/pipelines/postgresql_tests.yaml")),
            ("pgbouncer", "pgbouncer", include_str!("../../tests/fixtures/pipelines/pgbouncer_tests.yaml")),
            ("redisdb", "redis", include_str!("../../tests/fixtures/pipelines/redis_tests.yaml")),
            ("clickhouse", "clickhouse", include_str!("../../tests/fixtures/pipelines/clickhouse_tests.yaml")),
            // Broker logs (log4j's default, recommended and Kafka's layouts), Confluent Platform's components with a
            // stack trace, and Strimzi's operators and the Kafka, ZooKeeper and Connect pods it runs.
            ("kafka", "kafka", include_str!("../../tests/fixtures/pipelines/kafka_tests.yaml")),
            ("confluent_platform", "confluent_platform", include_str!("../../tests/fixtures/pipelines/confluent_platform_tests.yaml")),
            ("strimzi", "strimzi", include_str!("../../tests/fixtures/pipelines/strimzi_tests.yaml")),
        ] {
            assert_eq!(upstream_differences(id, source, yaml), Vec::<String>::new(), "{id}");
        }
    }

    #[test]
    fn google_workspace_activities_get_categories_and_the_pollers_statuses() {
        use crate::google_workspace::{fixtures, logs::logs_of};
        use serde_json::json;
        assert_eq!(get("google-workspace").unwrap().integration, "google-workspace", "installed with the integration");
        let run = |activity: Value| {
            let mut log = logs_of(&activity).remove(0);
            let (status, service) = (log["status"].as_str().unwrap().to_string(), log["service"].as_str().unwrap().to_string());
            for k in ["date", "message", "status", "service", "ddsource", "_oc_ingest"] {
                log.as_object_mut().unwrap().remove(k);
            }
            let p = get("google-workspace").unwrap();
            let (pipeline, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
            assert!(errors.is_empty(), "{errors:?}");
            let mut out = super::super::Log::from_row(&json!({"status": "info", "service": service, "source": "gsuite", "message": "event", "attributes": log.to_string()}));
            assert!(pipeline.apply(&mut out, 1_792_933_200_000));
            assert_eq!(out.status, status, "the pipeline's status is the poller's");
            out
        };
        let at = "2026-11-06T10:00:00Z";
        for (activity, category) in [
            (fixtures::login_success("1", at), "authentication"),
            (fixtures::login_failure("2", at, "sam.lee@example.com"), "authentication"),
            (fixtures::suspicious_login("3", at), "authentication"),
            (fixtures::role_assigned("4", at), "administration"),
            (fixtures::shared_publicly("5", at, "people_with_link"), "data_access"),
            (fixtures::token_authorized("6", at), "authorization"),
            (fixtures::activity("groups_enterprise", "7", at, "admin@example.com", "moderator_action", "add_member", &[]), "groups"),
            (fixtures::activity("mobile", "8", at, "sam.lee@example.com", "device_updates", "DEVICE_COMPROMISED_EVENT", &[]), "devices"),
            (fixtures::activity("user_accounts", "9", at, "sam.lee@example.com", "2sv_change", "2sv_disable", &[]), "authentication"),
            (fixtures::activity("rules", "10", at, "admin@example.com", "rule_trigger_type", "rule_match", &[]), "cloud"),
        ] {
            let name = activity["events"][0]["name"].clone();
            assert_eq!(run(activity).get("google_workspace.category"), Some(json!(category)), "{name}");
        }
        // An activity as the Reports API answers it, shipped another way (a log shipper): remapped.
        let raw = with_attrs("google-workspace", "gsuite", fixtures::login_failure("11", "2026-11-06T10:00:00.250Z", "sam.lee@example.com"));
        assert_eq!((raw.get("usr.email"), raw.get("usr.id"), raw.get("network.client.ip")), (Some(json!("sam.lee@example.com")), Some(json!("104857600000000000042")), Some(json!("198.51.100.7"))));
        assert_eq!((raw.get("network.client.geoip.country.iso_code"), raw.service.as_str(), raw.timestamp_ms), (Some(json!("NL")), "login", Some(1_793_959_200_250)));
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
        // Redis marks severities with a symbol: '#' is a warning. Its pipeline parses the text into `@message` but has no
        // message remapper: the message stays the line, as integrations-core's redis_tests.yaml expects.
        let line = "1:M 25 Oct 2026 12:59:58.000 # WARNING overcommit_memory is set to 0";
        let log = run("redisdb", "redis", line);
        assert_eq!((log.status.as_str(), log.message.as_str(), log.get("message")), ("warn", line, Some(serde_json::json!("WARNING overcommit_memory is set to 0"))));
        let log = run("redisdb", "redis", "1:M 25 Oct 2026 12:59:58.000 * Ready to accept connections");
        assert_eq!(log.status, "info");
        let log = run("etcd", "etcd", "2026-10-25 12:59:58.123456 E | rafthttp: failed to dial peer");
        assert_eq!((log.status.as_str(), log.message.as_str()), ("error", "failed to dial peer"), "its message remapper takes the parsed `message`");
        assert_eq!((log.get("package"), log.attrs.get("message")), (Some(serde_json::json!("rafthttp")), None));
    }

    /// The pipelines the mock offers too: every sample of theirs parses.
    const CURATED: [&str; 10] = ["clickhouse", "coredns", "etcd", "kube_scheduler", "mysql", "nginx", "nginx_ingress_controller", "pgbouncer", "postgres", "redisdb"];

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

