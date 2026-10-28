//! Integration pipelines: the log pipelines of integrations-core (BSD-3-Clause; copyright and terms
//! in each `server/seeds/integrations/<integration>/LICENSE`), converted by
//! `scripts/import-integration-pipelines.mjs`. Organizations install them from Logs → Pipelines.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone)]
pub struct IntegrationPipeline {
    pub id: String,
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

const SEEDS: &[&str] = &[
    include_str!("../../../../seeds/integrations/coredns/pipeline.json"),
    include_str!("../../../../seeds/integrations/etcd/pipeline.json"),
    include_str!("../../../../seeds/integrations/kube_scheduler/pipeline.json"),
    include_str!("../../../../seeds/integrations/mysql/pipeline.json"),
    include_str!("../../../../seeds/integrations/nginx/pipeline.json"),
    include_str!("../../../../seeds/integrations/nginx_ingress_controller/pipeline.json"),
    include_str!("../../../../seeds/integrations/postgres/pipeline.json"),
    include_str!("../../../../seeds/integrations/redisdb/pipeline.json"),
];

/// Every integration pipeline, by name.
pub fn all() -> &'static [IntegrationPipeline] {
    static ALL: OnceLock<Vec<IntegrationPipeline>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<IntegrationPipeline> = SEEDS.iter().map(|s| serde_json::from_str(s).expect("integration pipeline seed")).collect();
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
    fn every_seed_is_listed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../seeds/integrations");
        let on_disk = std::fs::read_dir(dir).unwrap().filter(|e| e.as_ref().unwrap().path().join("pipeline.json").exists()).count();
        assert_eq!(on_disk, SEEDS.len());
        assert!(get("nginx").is_some());
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

    #[test]
    fn integration_pipelines_compile_and_parse_their_samples() {
        for p in all() {
            let (_, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
            assert!(errors.is_empty(), "{}: {errors:?}", p.id);
            let mut parsers = Vec::new();
            groks(&p.processors, &mut parsers);
            for g in parsers {
                let rules = &g["grok"];
                let grok = Grok::compile(rules["supportRules"].as_str().unwrap_or(""), rules["matchRules"].as_str().unwrap_or("")).unwrap();
                for sample in g["samples"].as_array().map(Vec::as_slice).unwrap_or_default() {
                    let sample = sample.as_str().unwrap();
                    assert!(grok.parse(sample, 1_792_933_200_000).is_some(), "{} / {}: sample doesn't parse: {sample}", p.id, g["name"]);
                }
            }
        }
    }
}
