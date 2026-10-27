//! Synthetic host + process metrics for the hosts the services run on, sent as OTLP/HTTP JSON with
//! the same names and attributes as the OTel Collector `hostmetrics` receiver, so dashboards and
//! the Infrastructure views treat them exactly like a real host.

use serde_json::{json, Value};

use crate::{attrs_json, Rng};

pub struct Host {
    pub name: &'static str,
    instance_type: &'static str,
    pub(super) cpus: u32,
    pub(super) mem_gib: f64,
    region: &'static str,
    zone: &'static str,
    env: &'static str,
    team: &'static str,
    pub(super) cluster: Option<&'static str>,
    collector: &'static str,
    /// Typical CPU busy fraction; each host wanders around it.
    cpu_base: f64,
    procs: &'static [(&'static str, &'static str, f64, f64)], // (executable, owner, cpu share, rss GiB)
}

const JVM: &[(&str, &str, f64, f64)] = &[
    ("java", "app", 0.55, 2.4),
    ("observecraft-collector", "root", 0.06, 0.18),
    ("containerd", "root", 0.04, 0.12),
    ("kubelet", "root", 0.05, 0.09),
    ("sshd", "root", 0.0, 0.01),
];
const GO: &[(&str, &str, f64, f64)] = &[
    ("api-gateway", "app", 0.35, 0.4),
    ("order-service", "app", 0.3, 0.35),
    ("observecraft-collector", "root", 0.06, 0.18),
    ("containerd", "root", 0.04, 0.12),
    ("kubelet", "root", 0.05, 0.09),
];
const NODE: &[(&str, &str, f64, f64)] = &[
    ("node", "app", 0.6, 0.9),
    ("otelcol-contrib", "otel", 0.07, 0.2),
    ("dockerd", "root", 0.05, 0.15),
    ("cron", "root", 0.0, 0.01),
];
const DOTNET: &[(&str, &str, f64, f64)] = &[
    ("dotnet", "app", 0.45, 1.6),
    ("java", "app", 0.3, 2.1),
    ("observecraft-collector", "root", 0.06, 0.18),
    ("containerd", "root", 0.04, 0.12),
];
const PY: &[(&str, &str, f64, f64)] = &[
    ("python3", "app", 0.5, 0.7),
    ("observecraft-collector", "root", 0.05, 0.16),
    ("containerd", "root", 0.03, 0.1),
];

pub const HOSTS: &[Host] = &[
    Host {
        name: "i-0a1b2c3d4e5f60001",
        instance_type: "c6i.2xlarge",
        cpus: 8,
        mem_gib: 16.0,
        region: "eu-west-1",
        zone: "eu-west-1a",
        env: "prod",
        team: "platform",
        cluster: Some("shop-prod-eks"),
        collector: "observecraft",
        cpu_base: 0.42,
        procs: GO,
    },
    Host {
        name: "i-0a1b2c3d4e5f60002",
        instance_type: "m6i.2xlarge",
        cpus: 8,
        mem_gib: 32.0,
        region: "eu-west-1",
        zone: "eu-west-1b",
        env: "prod",
        team: "storefront",
        cluster: Some("shop-prod-eks"),
        collector: "observecraft",
        cpu_base: 0.58,
        procs: JVM,
    },
    Host {
        name: "i-0a1b2c3d4e5f60003",
        instance_type: "m6i.xlarge",
        cpus: 4,
        mem_gib: 16.0,
        region: "eu-west-1",
        zone: "eu-west-1c",
        env: "prod",
        team: "checkout",
        cluster: Some("shop-prod-eks"),
        collector: "observecraft",
        cpu_base: 0.31,
        procs: NODE,
    },
    Host {
        name: "i-0a1b2c3d4e5f60004",
        instance_type: "r6i.2xlarge",
        cpus: 8,
        mem_gib: 64.0,
        region: "eu-west-1",
        zone: "eu-west-1a",
        env: "prod",
        team: "billing",
        cluster: None,
        collector: "otel",
        cpu_base: 0.67,
        procs: DOTNET,
    },
    Host {
        name: "i-0a1b2c3d4e5f60005",
        instance_type: "t3.large",
        cpus: 2,
        mem_gib: 8.0,
        region: "eu-west-1",
        zone: "eu-west-1b",
        env: "prod",
        team: "platform",
        cluster: None,
        collector: "otel",
        cpu_base: 0.22,
        procs: PY,
    },
    Host {
        name: "i-0a1b2c3d4e5f60006",
        instance_type: "m6i.large",
        cpus: 2,
        mem_gib: 8.0,
        region: "eu-central-1",
        zone: "eu-central-1a",
        env: "staging",
        team: "storefront",
        cluster: Some("shop-staging-eks"),
        collector: "observecraft",
        cpu_base: 0.15,
        procs: NODE,
    },
];

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Slowly varying per-host state; cumulative counters only ever grow, like real ones.
pub struct HostState {
    busy: f64,
    mem_frac: f64,
    disk_frac: f64,
    net_tx: f64,
    net_rx: f64,
    start_ns: u64,
}

pub fn initial_state(rng: &mut Rng, now_ns: u64) -> Vec<HostState> {
    HOSTS
        .iter()
        .map(|h| HostState {
            busy: h.cpu_base,
            mem_frac: 0.45 + rng.f() * 0.3,
            disk_frac: 0.3 + rng.f() * 0.5,
            net_tx: rng.f() * 1e10,
            net_rx: rng.f() * 1e10,
            // Booted between 2 and 40 days ago.
            start_ns: now_ns - (2 + rng.range(0, 38)) * 86_400_000_000_000,
        })
        .collect()
}

fn walk(rng: &mut Rng, v: f64, target: f64, step: f64, lo: f64, hi: f64) -> f64 {
    (v + (target - v) * 0.1 + (rng.f() - 0.5) * step).clamp(lo, hi)
}

fn point(t: &str, v: f64, attrs: &[(String, Value)]) -> Value {
    json!({ "timeUnixNano": t, "asDouble": v, "attributes": attrs_json(attrs) })
}

fn gauge(name: &str, unit: &str, desc: &str, points: Vec<Value>) -> Value {
    json!({ "name": name, "unit": unit, "description": desc, "gauge": { "dataPoints": points } })
}

fn sum(
    name: &str,
    unit: &str,
    desc: &str,
    monotonic: bool,
    start: &str,
    mut points: Vec<Value>,
) -> Value {
    for p in &mut points {
        p["startTimeUnixNano"] = json!(start);
    }
    // aggregationTemporality 2 = cumulative.
    json!({ "name": name, "unit": unit, "description": desc, "sum": { "aggregationTemporality": 2, "isMonotonic": monotonic, "dataPoints": points } })
}

fn kv(k: &str, v: impl Into<Value>) -> (String, Value) {
    (k.to_string(), v.into())
}

fn host_resource(h: &Host) -> Vec<(String, Value)> {
    let mut r = vec![
        kv("host.name", h.name),
        kv("host.id", h.name),
        kv("host.type", h.instance_type),
        kv("host.arch", "amd64"),
        kv("os.type", "linux"),
        kv("os.description", "Ubuntu 22.04.4 LTS"),
        kv("cloud.provider", "aws"),
        kv("cloud.platform", "aws_ec2"),
        kv("cloud.region", h.region),
        kv("cloud.availability_zone", h.zone),
        kv("cloud.account.id", "123456789012"),
        kv("deployment.environment", h.env),
        kv("team", h.team),
        kv("host.cpu.count", h.cpus as i64),
        kv("observecraft.collector", h.collector),
        kv(
            "observecraft.collector.version",
            if h.collector == "otel" {
                "otelcol-contrib 0.163.2"
            } else {
                "observecraft-collector 1.4.2"
            },
        ),
    ];
    if let Some(c) = h.cluster {
        r.push(kv("k8s.cluster.name", c));
    }
    r
}

/// One OTLP metrics export for every host at `now_ns`.
pub fn metrics_payload(
    rng: &mut Rng,
    state: &mut [HostState],
    now_ns: u64,
    interval_s: f64,
) -> Value {
    let t = now_ns.to_string();
    let mut resource_metrics = Vec::new();
    for (h, s) in HOSTS.iter().zip(state.iter_mut()) {
        // Daily-ish swell plus noise, occasional spikes.
        let spike = if rng.chance(0.01) { 0.35 } else { 0.0 };
        s.busy = walk(rng, s.busy, h.cpu_base + spike, 0.08, 0.02, 0.98);
        s.mem_frac = walk(rng, s.mem_frac, 0.6, 0.01, 0.2, 0.95);
        s.disk_frac = (s.disk_frac + rng.f() * 0.00002).min(0.97);
        let net_rate = 2e5 + s.busy * 4e6 * h.cpus as f64;
        s.net_tx += net_rate * interval_s * (0.8 + rng.f() * 0.4);
        s.net_rx += net_rate * 1.3 * interval_s * (0.8 + rng.f() * 0.4);

        let user = s.busy * 0.72;
        let system = s.busy * 0.22;
        let iowait = s.busy * 0.06;
        let idle = (1.0 - user - system - iowait).max(0.0);
        let mem_total = h.mem_gib * GIB;
        let used = mem_total * s.mem_frac;
        let cached = mem_total * 0.18;
        let free = (mem_total - used - cached).max(0.0);
        let load1 = s.busy * h.cpus as f64 * (0.9 + rng.f() * 0.3);
        let boot = s.start_ns.to_string();

        let state_pts = |vals: &[(&str, f64)]| {
            vals.iter()
                .map(|(st, v)| point(&t, *v, &[kv("state", *st)]))
                .collect::<Vec<_>>()
        };
        let metrics = vec![
            gauge(
                "system.cpu.utilization",
                "1",
                "Fraction of CPU time spent in each state",
                state_pts(&[
                    ("user", user),
                    ("system", system),
                    ("wait", iowait),
                    ("idle", idle),
                ]),
            ),
            gauge(
                "system.cpu.load_average.1m",
                "{thread}",
                "Load average over 1 minute",
                vec![point(&t, load1, &[])],
            ),
            gauge(
                "system.cpu.load_average.5m",
                "{thread}",
                "Load average over 5 minutes",
                vec![point(&t, load1 * 0.95, &[])],
            ),
            gauge(
                "system.cpu.load_average.15m",
                "{thread}",
                "Load average over 15 minutes",
                vec![point(&t, load1 * 0.9, &[])],
            ),
            sum(
                "system.memory.usage",
                "By",
                "Bytes of memory in use",
                false,
                &boot,
                state_pts(&[("used", used), ("cached", cached), ("free", free)]),
            ),
            gauge(
                "system.memory.utilization",
                "1",
                "Fraction of memory in each state",
                state_pts(&[
                    ("used", used / mem_total),
                    ("cached", cached / mem_total),
                    ("free", free / mem_total),
                ]),
            ),
            gauge(
                "system.filesystem.utilization",
                "1",
                "Fraction of filesystem bytes used",
                vec![point(
                    &t,
                    s.disk_frac,
                    &[
                        kv("device", "/dev/nvme0n1p1"),
                        kv("mountpoint", "/"),
                        kv("type", "ext4"),
                    ],
                )],
            ),
            sum(
                "system.network.io",
                "By",
                "Bytes transmitted and received",
                true,
                &boot,
                vec![
                    point(
                        &t,
                        s.net_tx.round(),
                        &[kv("device", "eth0"), kv("direction", "transmit")],
                    ),
                    point(
                        &t,
                        s.net_rx.round(),
                        &[kv("device", "eth0"), kv("direction", "receive")],
                    ),
                ],
            ),
            gauge(
                "system.uptime",
                "s",
                "Seconds since boot",
                vec![point(&t, (now_ns - s.start_ns) as f64 / 1e9, &[])],
            ),
        ];
        resource_metrics.push(json!({
            "resource": { "attributes": attrs_json(&host_resource(h)) },
            "scopeMetrics": [{ "scope": { "name": "oc-tracegen/hostmetrics" }, "metrics": metrics }],
        }));

        // Processes: one resource per process, as the hostmetrics `process` scraper reports them.
        for (i, (exe, owner, share, rss)) in h.procs.iter().enumerate() {
            let mut res = host_resource(h);
            res.push(kv("process.pid", 1000 + i as i64 * 137));
            res.push(kv("process.executable.name", *exe));
            res.push(kv("process.command", format!("/usr/bin/{exe}")));
            res.push(kv("process.owner", *owner));
            let cpu = (s.busy * share * (0.85 + rng.f() * 0.3)).min(1.0);
            let rss_b = rss * GIB * (0.95 + rng.f() * 0.1);
            resource_metrics.push(json!({
                "resource": { "attributes": attrs_json(&res) },
                "scopeMetrics": [{ "scope": { "name": "oc-tracegen/hostmetrics/process" }, "metrics": [
                    gauge("process.cpu.utilization", "1", "Fraction of host CPU used by the process", vec![point(&t, cpu, &[])]),
                    sum("process.memory.usage", "By", "Resident set size", false, &boot, vec![point(&t, rss_b.round(), &[])]),
                ] }],
            }));
        }
    }
    json!({ "resourceMetrics": resource_metrics })
}
