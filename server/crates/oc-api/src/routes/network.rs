//! Cloud Network Monitoring: the connections the Datadog Agent's system-probe reports
//! (`otel.otel_network`), aggregated into client → server flows the way Datadog's Network Analytics
//! does: each side grouped by a tag (`service` by default), filtered with `client_*` / `server_*`
//! tags, with volume, throughput, TCP retransmits, latency (RTT), jitter (RTT variance) and
//! connections.
//!
//! Endpoint tags come from, in order:
//! - what the reporting Agent attaches to its own side (`local.tags`, `host.name`, the container);
//! - the Kubernetes inventory (`status.podIP` → pod, namespace, service; `spec.clusterIP` →
//!   `kube_service`);
//! - the host's Datadog metadata (host tags such as `env`, `availability-zone`);
//! - other Agents' reports: an IP some Agent reported as its local address takes that side's tags.
//!
//! What resolves to nothing groups under `N/A` (hidden unless asked for); public addresses are
//! external traffic. Each connection is oriented client → server from its direction: outgoing
//! (and local) ones are the client's view, with the server behind a NAT address resolved through
//! the translation; incoming ones count only when the client isn't monitored itself (otherwise
//! the client already reported the same connection).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;

use axum::extract::State;
use axum::Json;
use clickhouse::Row;
use oc_query::{glob, parse_query, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

fn default_group() -> String {
    "service".into()
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowRequest {
    pub range: TimeRange,
    /// `client_service:checkout server_env:prod -server_port:53`, plus free text matched against
    /// the client and server names.
    #[serde(default)]
    pub query: String,
    /// Tag the clients are grouped by: service, env, availability-zone, kube_namespace, pod_name,
    /// kube_service, host, container_name, short_image, ip, port, domain.
    #[serde(default = "default_group")]
    pub client_group_by: String,
    #[serde(default = "default_group")]
    pub server_group_by: String,
    /// Traffic to public addresses.
    #[serde(default = "yes")]
    pub show_external: bool,
    /// Traffic whose client or server has no value for the group-by tag (`N/A`).
    #[serde(default)]
    pub show_unresolved: bool,
    pub limit: usize,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    /// The client's value of the client group-by tag, or `N/A`.
    pub client: String,
    pub server: String,
    /// Tags every connection of the flow shares, per side (`key:value`).
    pub client_tags: Vec<String>,
    pub server_tags: Vec<String>,
    /// Bytes the client sent / received.
    pub bytes_sent: u64,
    pub bytes_received: u64,
    /// Bytes per second over the range.
    pub throughput_sent: f64,
    pub throughput_received: f64,
    pub retransmits: u64,
    /// Mean round-trip time and its variance (jitter), microseconds (TCP).
    pub rtt_us: Option<f64>,
    pub rtt_var_us: Option<f64>,
    /// TCP connections opened / closed in the range.
    pub established: u64,
    pub closed: u64,
    /// Whether the server is a public address.
    pub external: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkFacetValue {
    pub value: String,
    /// Connection reports of the matching traffic.
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkFacet {
    /// `client_service`, `server_port`, …
    pub key: String,
    pub values: Vec<NetworkFacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPoint {
    /// Bucket start, epoch ms.
    pub t: i64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub established: u64,
    pub closed: u64,
    pub retransmits: u64,
    pub rtt_us: Option<f64>,
    pub rtt_var_us: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPage {
    pub flows: Vec<Flow>,
    /// Flows before the limit.
    pub total: usize,
    pub total_bytes: u64,
    /// Whether any connection data exists (system-probe enabled somewhere).
    pub enabled: bool,
    pub step_ms: i64,
    /// Totals of the matching traffic over time (the summary graphs).
    pub series: Vec<NetworkPoint>,
    pub facets: Vec<NetworkFacet>,
}

#[derive(Row, Deserialize)]
struct ConnRow {
    t: i64,
    host: String,
    direction: String,
    local_ip: String,
    local_port: String,
    local_tags: String,
    container: String,
    remote_ip: String,
    remote_port: String,
    translated_ip: String,
    sent: u64,
    received: u64,
    retransmits: u64,
    rtt_sum: f64,
    rtt_var_sum: f64,
    rtt_n: u64,
    established: u64,
    closed: u64,
}

#[derive(Row, Deserialize)]
struct PodRow {
    ip: String,
    name: String,
    namespace: String,
    service: String,
    node: String,
    cluster: String,
}

#[derive(Row, Deserialize)]
struct SvcRow {
    ip: String,
    name: String,
    namespace: String,
}

type Tags = BTreeMap<String, String>;

/// Tags known for an address.
fn add(tags: &mut Tags, key: &str, value: &str) {
    if !value.is_empty() {
        tags.entry(key.to_string()).or_insert_with(|| value.to_string());
    }
}

/// `key:value,key:value` → tags.
fn parse_tags(s: &str, into: &mut Tags) {
    for t in s.split(',') {
        if let Some((k, v)) = t.trim().split_once(':') {
            add(into, k, v);
        }
    }
}

/// Public (internet) address: not private, loopback, link-local, CGNAT or unique-local.
pub fn is_public(ip: &str) -> bool {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v)) => {
            let o = v.octets();
            !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        Ok(IpAddr::V6(v)) => !(v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80),
        Err(_) => false,
    }
}

/// One connection, oriented and tagged.
struct Oriented<'a> {
    row: &'a ConnRow,
    client: Tags,
    server: Tags,
    sent: u64,
    received: u64,
    external: bool,
}

/// Whether a side's tags satisfy the `<side>_*` terms (and free text against `label`), skipping
/// terms on `skip`.
fn matches(terms: &[Term], o: &Oriented, labels: (&str, &str), skip: Option<&str>) -> bool {
    terms.iter().all(|t| match t {
        Term::Tag { key, values, negate } => {
            if Some(key.as_str()) == skip {
                return true;
            }
            let (tags, k) = match key.split_once('_') {
                Some(("client", k)) => (&o.client, k),
                Some(("server", k)) => (&o.server, k),
                // Neutral tags (both sides): either side matches.
                _ => {
                    let hit = [&o.client, &o.server].iter().any(|s| s.get(key.as_str()).is_some_and(|v| values.iter().any(|p| glob(p, v))));
                    return hit != *negate;
                }
            };
            let hit = tags.get(k).is_some_and(|v| values.iter().any(|p| glob(p, v)));
            hit != *negate
        }
        Term::Text { text, negate } => {
            let t = text.to_lowercase();
            let hit = labels.0.to_lowercase().contains(&t) || labels.1.to_lowercase().contains(&t);
            hit != *negate
        }
    })
}

const UNRESOLVED: &str = "N/A";
/// Facets of each side, as in Datadog's Client / Server panels.
const FACET_KEYS: [&str; 9] = ["service", "env", "availability-zone", "kube_namespace", "pod_name", "host", "ip", "port", "domain"];

/// Flows between the client and server groups in the range, biggest first, with the summary
/// series and the facets.
#[utoipa::path(post, path = "/api/v1/network/flows", operation_id = "network_flows", request_body = FlowRequest, responses((status = 200, body = FlowPage)), tag = "network")]
pub async fn flows(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<FlowRequest>) -> ApiResult<FlowPage> {
    req.range.validate()?;
    for g in [&req.client_group_by, &req.server_group_by] {
        if g.is_empty() || g.len() > 100 {
            return Err(ApiError::BadRequest("invalid group-by tag".into()));
        }
    }
    let span_ms = (req.range.to - req.range.from).max(1);
    // About 60 points, never under the Agent's 30-second report.
    let step_ms = ((span_ms / 60).max(30_000) / 30_000) * 30_000;
    let ch = st.ch(&org);
    let conns = ch
        .query(
            "SELECT toInt64(toUnixTimestamp(toStartOfInterval(Timestamp, toIntervalSecond(?)))) * 1000 AS t, \
             ResourceAttributes['host.name'] AS host, LogAttributes['direction'] AS direction, \
             LogAttributes['local.ip'] AS local_ip, if(LogAttributes['local.port_ephemeral'] = 'true', '', LogAttributes['local.port']) AS local_port, \
             LogAttributes['local.tags'] AS local_tags, LogAttributes['container.id'] AS container, \
             LogAttributes['remote.ip'] AS remote_ip, LogAttributes['remote.port'] AS remote_port, \
             LogAttributes['remote.translated_ip'] AS translated_ip, \
             sum(toUInt64OrZero(LogAttributes['bytes_sent'])) AS sent, sum(toUInt64OrZero(LogAttributes['bytes_received'])) AS received, \
             sum(toUInt64OrZero(LogAttributes['retransmits'])) AS retransmits, \
             sumIf(toFloat64OrZero(LogAttributes['rtt_us']), toFloat64OrZero(LogAttributes['rtt_us']) > 0) AS rtt_sum, \
             sumIf(toFloat64OrZero(LogAttributes['rtt_var_us']), toFloat64OrZero(LogAttributes['rtt_us']) > 0) AS rtt_var_sum, \
             countIf(toFloat64OrZero(LogAttributes['rtt_us']) > 0) AS rtt_n, \
             sum(toUInt64OrZero(LogAttributes['tcp_established'])) AS established, sum(toUInt64OrZero(LogAttributes['tcp_closed'])) AS closed \
             FROM otel.otel_network WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             GROUP BY t, host, direction, local_ip, local_port, local_tags, container, remote_ip, remote_port, translated_ip LIMIT 200000",
        )
        .bind(step_ms / 1000)
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_all::<ConnRow>();
    let pods = ch
        .query(
            "SELECT JSONExtractString(Body, 'status', 'podIP') AS ip, argMax(JSONExtractString(Body, 'metadata', 'name'), LastSeen) AS name, \
             argMax(JSONExtractString(Body, 'metadata', 'namespace'), LastSeen) AS namespace, \
             argMax(coalesce(nullIf(JSONExtractString(Body, 'metadata', 'labels', 'tags.datadoghq.com/service'), ''), \
                             nullIf(JSONExtractString(Body, 'metadata', 'labels', 'app.kubernetes.io/name'), ''), \
                             JSONExtractString(Body, 'metadata', 'labels', 'app')), LastSeen) AS service, \
             argMax(JSONExtractString(Body, 'spec', 'nodeName'), LastSeen) AS node, argMax(Cluster, LastSeen) AS cluster \
             FROM otel.k8s_resources WHERE OrgId = {oc_org:String} AND Kind = 'Pod' AND NOT JSONExtractBool(Body, 'spec', 'hostNetwork') GROUP BY ip HAVING ip != ''",
        )
        .fetch_all::<PodRow>();
    let services = ch
        .query(
            "SELECT JSONExtractString(Body, 'spec', 'clusterIP') AS ip, argMax(JSONExtractString(Body, 'metadata', 'name'), LastSeen) AS name, \
             argMax(JSONExtractString(Body, 'metadata', 'namespace'), LastSeen) AS namespace \
             FROM otel.k8s_resources WHERE OrgId = {oc_org:String} AND Kind = 'Service' GROUP BY ip HAVING ip NOT IN ('', 'None')",
        )
        .fetch_all::<SvcRow>();
    let host_tags = sqlx::query_as::<_, (String, Vec<String>)>("SELECT host, host_tags FROM datadog_hosts WHERE org_id = $1").bind(&org).fetch_all(&st.postgres);
    let (conns, pods, services, host_tags) =
        tokio::try_join!(async { conns.await.map_err(ApiError::from) }, async { pods.await.map_err(ApiError::from) }, async { services.await.map_err(ApiError::from) }, async {
            host_tags.await.map_err(ApiError::from)
        })?;

    let host_tags: HashMap<String, Vec<String>> = host_tags.into_iter().collect();
    let with_host = |tags: &mut Tags, host: &str| {
        add(tags, "host", host);
        for t in host_tags.get(host).into_iter().flatten() {
            if let Some((k, v)) = t.split_once(':') {
                add(tags, k, v);
            }
        }
    };
    // Address → tags.
    let mut by_ip: HashMap<String, Tags> = HashMap::new();
    for p in pods {
        let mut t = Tags::new();
        add(&mut t, "pod_name", &p.name);
        add(&mut t, "kube_namespace", &p.namespace);
        add(&mut t, "service", &p.service);
        add(&mut t, "kube_cluster_name", &p.cluster);
        with_host(&mut t, &p.node);
        by_ip.insert(p.ip, t);
    }
    for s in services {
        let mut t = Tags::new();
        add(&mut t, "kube_service", &s.name);
        add(&mut t, "service", &s.name);
        add(&mut t, "kube_namespace", &s.namespace);
        by_ip.entry(s.ip).or_insert(t);
    }
    // Each Agent's own addresses carry what it reported about them.
    let mut monitored: HashSet<&str> = HashSet::new();
    for c in &conns {
        if c.local_ip.starts_with("127.") || c.local_ip == "::1" {
            continue;
        }
        monitored.insert(&c.local_ip);
        let t = by_ip.entry(c.local_ip.clone()).or_default();
        parse_tags(&c.local_tags, t);
        // A pod runs on the reporting host too: it gets the host's tags it doesn't set itself.
        with_host(t, &c.host);
    }
    let resolve = |ip: &str, reporter: &str| -> Tags {
        if ip.starts_with("127.") || ip == "::1" {
            let mut t = Tags::new();
            with_host(&mut t, reporter);
            add(&mut t, "service", &format!("localhost ({reporter})"));
            return t;
        }
        by_ip.get(ip).cloned().unwrap_or_default()
    };

    let mut oriented: Vec<Oriented> = Vec::with_capacity(conns.len());
    for c in &conns {
        let (client_ip, server_ip, client_port, server_port, sent, received) = match c.direction.as_str() {
            "incoming" => {
                if monitored.contains(c.remote_ip.as_str()) || c.remote_ip.starts_with("127.") || by_ip.get(&c.remote_ip).is_some_and(|t| t.contains_key("pod_name")) {
                    continue;
                }
                (c.remote_ip.as_str(), c.local_ip.as_str(), "", c.local_port.as_str(), c.received, c.sent)
            }
            _ => {
                let server = if c.translated_ip.is_empty() { c.remote_ip.as_str() } else { c.translated_ip.as_str() };
                (c.local_ip.as_str(), server, c.local_port.as_str(), c.remote_port.as_str(), c.sent, c.received)
            }
        };
        let mut client = resolve(client_ip, &c.host);
        if c.direction != "incoming" {
            // The reporting side: its Agent's tags and container.
            parse_tags(&c.local_tags, &mut client);
            add(&mut client, "container_id", &c.container);
        }
        let mut server = resolve(server_ip, &c.host);
        add(&mut client, "ip", client_ip);
        add(&mut server, "ip", server_ip);
        add(&mut client, "port", client_port);
        add(&mut server, "port", server_port);
        if c.direction == "incoming" {
            parse_tags(&c.local_tags, &mut server);
        }
        // Behind a Kubernetes service: the service address is what the client asked for.
        if !c.translated_ip.is_empty() {
            if let Some(svc) = by_ip.get(&c.remote_ip).and_then(|t| t.get("kube_service")) {
                add(&mut server, "kube_service", svc);
            }
        }
        let external = is_public(server_ip) && !server.contains_key("service");
        oriented.push(Oriented { row: c, client, server, sent, received, external });
    }
    let enabled = !oriented.is_empty();

    let terms = parse_query(&req.query);
    let label = |tags: &Tags, key: &str| tags.get(key).cloned().unwrap_or_else(|| UNRESOLVED.to_string());
    let visible = |o: &Oriented| (req.show_external || !o.external) && (req.show_unresolved || (o.client.contains_key(&req.client_group_by) && o.server.contains_key(&req.server_group_by)));

    // Flows, series and facets of the visible traffic.
    struct Acc {
        flow: Flow,
        rtt_sum: f64,
        rtt_var_sum: f64,
        rtt_n: u64,
        client_common: Option<Tags>,
        server_common: Option<Tags>,
    }
    let common = |acc: &mut Option<Tags>, tags: &Tags| match acc {
        None => *acc = Some(tags.clone()),
        Some(c) => c.retain(|k, v| tags.get(k) == Some(v)),
    };
    let mut agg: HashMap<(String, String), Acc> = HashMap::new();
    let mut series: BTreeMap<i64, (NetworkPoint, f64, f64, u64)> = BTreeMap::new();
    for o in oriented.iter().filter(|o| visible(o)) {
        let (cl, sl) = (label(&o.client, &req.client_group_by), label(&o.server, &req.server_group_by));
        if !matches(&terms, o, (&cl, &sl), None) {
            continue;
        }
        let r = o.row;
        let a = agg.entry((cl.clone(), sl.clone())).or_insert_with(|| Acc {
            flow: Flow {
                client: cl,
                server: sl,
                client_tags: vec![],
                server_tags: vec![],
                bytes_sent: 0,
                bytes_received: 0,
                throughput_sent: 0.0,
                throughput_received: 0.0,
                retransmits: 0,
                rtt_us: None,
                rtt_var_us: None,
                established: 0,
                closed: 0,
                external: o.external,
            },
            rtt_sum: 0.0,
            rtt_var_sum: 0.0,
            rtt_n: 0,
            client_common: None,
            server_common: None,
        });
        a.flow.bytes_sent += o.sent;
        a.flow.bytes_received += o.received;
        a.flow.retransmits += r.retransmits;
        a.flow.established += r.established;
        a.flow.closed += r.closed;
        a.rtt_sum += r.rtt_sum;
        a.rtt_var_sum += r.rtt_var_sum;
        a.rtt_n += r.rtt_n;
        common(&mut a.client_common, &o.client);
        common(&mut a.server_common, &o.server);
        let p =
            series.entry(r.t).or_insert_with(|| (NetworkPoint { t: r.t, bytes_sent: 0, bytes_received: 0, established: 0, closed: 0, retransmits: 0, rtt_us: None, rtt_var_us: None }, 0.0, 0.0, 0));
        p.0.bytes_sent += o.sent;
        p.0.bytes_received += o.received;
        p.0.established += r.established;
        p.0.closed += r.closed;
        p.0.retransmits += r.retransmits;
        p.1 += r.rtt_sum;
        p.2 += r.rtt_var_sum;
        p.3 += r.rtt_n;
    }
    let secs = span_ms as f64 / 1000.0;
    let tags_of = |t: Option<Tags>| t.unwrap_or_default().into_iter().map(|(k, v)| format!("{k}:{v}")).collect();
    let mut flows: Vec<Flow> = agg
        .into_values()
        .map(|a| {
            let mut f = a.flow;
            f.rtt_us = (a.rtt_n > 0).then(|| a.rtt_sum / a.rtt_n as f64);
            f.rtt_var_us = (a.rtt_n > 0).then(|| a.rtt_var_sum / a.rtt_n as f64);
            f.throughput_sent = f.bytes_sent as f64 / secs;
            f.throughput_received = f.bytes_received as f64 / secs;
            f.client_tags = tags_of(a.client_common);
            f.server_tags = tags_of(a.server_common);
            f
        })
        .collect();
    flows.sort_by(|a, b| (b.bytes_sent + b.bytes_received).cmp(&(a.bytes_sent + a.bytes_received)).then(a.client.cmp(&b.client)).then(a.server.cmp(&b.server)));
    let total = flows.len();
    let total_bytes = flows.iter().map(|f| f.bytes_sent + f.bytes_received).sum();
    flows.truncate(req.limit.clamp(1, 1000));
    let series = series
        .into_values()
        .map(|(mut p, rtt, var, n)| {
            p.rtt_us = (n > 0).then(|| rtt / n as f64);
            p.rtt_var_us = (n > 0).then(|| var / n as f64);
            p
        })
        .collect();

    // Facets honour every filter except their own.
    let mut facets = Vec::new();
    for side in ["client", "server"] {
        for key in FACET_KEYS {
            let facet = format!("{side}_{key}");
            let mut values: HashMap<String, u64> = HashMap::new();
            for o in oriented.iter().filter(|o| visible(o)) {
                let (cl, sl) = (label(&o.client, &req.client_group_by), label(&o.server, &req.server_group_by));
                if !matches(&terms, o, (&cl, &sl), Some(&facet)) {
                    continue;
                }
                let tags = if side == "client" { &o.client } else { &o.server };
                if let Some(v) = tags.get(key) {
                    *values.entry(v.clone()).or_default() += 1;
                }
            }
            let mut values: Vec<NetworkFacetValue> = values.into_iter().map(|(value, count)| NetworkFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            values.truncate(100);
            facets.push(NetworkFacet { key: facet, values });
        }
    }
    Ok(Json(FlowPage { flows, total, total_bytes, enabled, step_ms, series, facets }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ConnRow {
        ConnRow {
            t: 0,
            host: "h".into(),
            direction: "outgoing".into(),
            local_ip: "10.0.0.1".into(),
            local_port: String::new(),
            local_tags: String::new(),
            container: String::new(),
            remote_ip: "10.0.0.2".into(),
            remote_port: "443".into(),
            translated_ip: String::new(),
            sent: 0,
            received: 0,
            retransmits: 0,
            rtt_sum: 0.0,
            rtt_var_sum: 0.0,
            rtt_n: 0,
            established: 0,
            closed: 0,
        }
    }

    #[test]
    fn public_addresses() {
        assert!(is_public("203.0.113.7"));
        assert!(!is_public("10.1.2.3"));
        assert!(!is_public("100.70.0.1"));
        assert!(!is_public("127.0.0.1"));
        assert!(!is_public("fd00::1"));
        assert!(is_public("2001:4860::8888"));
    }

    #[test]
    fn side_filters() {
        let r = row();
        let mut client = Tags::new();
        parse_tags("service:checkout,env:prod", &mut client);
        let mut server = Tags::new();
        parse_tags("service:billing,port:443", &mut server);
        let o = Oriented { row: &r, client, server, sent: 1, received: 1, external: false };
        let ok = |q: &str| matches(&parse_query(q), &o, ("checkout", "billing"), None);
        assert!(ok("client_service:checkout server_port:443"));
        assert!(!ok("client_service:billing"));
        assert!(ok("-server_service:orders"));
        assert!(ok("env:prod"));
        assert!(ok("server_service:bill*"));
        assert!(ok("bill"));
        assert!(matches(&parse_query("client_service:billing"), &o, ("checkout", "billing"), Some("client_service")));
    }
}
