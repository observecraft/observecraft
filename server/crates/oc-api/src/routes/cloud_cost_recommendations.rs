//! Cloud Cost → Optimize → Recommendations: waste in the customers' AWS and Azure accounts, found from
//! the metrics the cloud crawlers already collect (`crate::aws`, `crate::azure`) by our own rules.
//!
//! - **Evidence.** Each resource kind ([`KINDS`]) has metric statistics ([`SPECS`]) read over the last
//!   [`LOOKBACK_DAYS`] days in daily steps through the metric engine (`metrics::evaluate_query_stepped`),
//!   grouped by the tags that name the resource (AWS: account, region and the CloudWatch dimension;
//!   Azure: subscription, resource group, region and name). Their tags come from the series
//!   (`otel.metrics_series`), so the account's and the resource's own tags can filter.
//! - **Rules** ([`recommend`]) are pure functions of a resource's statistics; their thresholds are the
//!   constants below. A resource seen on fewer than [`MIN_OBSERVED_DAYS`] days gets none.
//! - **Cost.** A resource's monthly cost comes from the imported bills (`otel.cloud_costs`, last 30 days,
//!   matched on `ResourceId`); without a bill line it is the list price of a typical size ([`Kind`]),
//!   marked as an estimate (the crawlers don't collect instance types or volume sizes).
//!
//! Instance types and volume types aren't collected, so rules that need them (gp2 → gp3, a specific
//! smaller instance type) aren't here; downsizing recommends the next smaller size of the same family.
//! Results are cached per org for [`CACHE_TTL`].
//!
//! Tenancy: the org comes from the session; ClickHouse queries filter `OrgId = {oc_org:String}` through
//! `AppState::ch(org)`, Postgres queries `org_id`.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use oc_query::{MetricQuery, Rollup, SpaceAgg};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::logs::TimeRange;
use super::metrics::TimeAggregator;
use crate::cloud::naming::glob;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// The window the rules look at.
pub const LOOKBACK_DAYS: i64 = 14;
/// A resource needs data on this many days before a rule judges it.
pub const MIN_OBSERVED_DAYS: f64 = 3.0;
/// Idle VM / instance: its highest CPU stayed under this (percent) …
pub const IDLE_CPU_MAX_PERCENT: f64 = 5.0;
/// … and its network traffic (in + out) under this per day.
pub const IDLE_NETWORK_BYTES_PER_DAY: f64 = 5e6;
/// Over-provisioned VM / instance: average CPU under this (percent) …
pub const RIGHTSIZE_CPU_AVG_PERCENT: f64 = 10.0;
/// … and its highest CPU under this: half the size would still have headroom.
pub const RIGHTSIZE_CPU_MAX_PERCENT: f64 = 40.0;
/// The next smaller size of a family costs about half.
pub const RIGHTSIZE_SAVINGS_SHARE: f64 = 0.5;
/// Unused database: no more connections than this, ever, in the window.
pub const UNUSED_DB_MAX_CONNECTIONS: f64 = 0.0;
/// Idle volume: fewer read + write operations than this per day.
pub const IDLE_VOLUME_OPS_PER_DAY: f64 = 1.0;
/// Idle Azure managed disk: its highest read and write rate stayed under this (operations per second).
pub const IDLE_DISK_MAX_OPS_PER_SEC: f64 = 0.01;
/// Unused NAT gateway, Azure load balancer or public IP: less traffic than this per day.
pub const UNUSED_NETWORK_BYTES_PER_DAY: f64 = 1e6;
/// Idle AWS load balancer: fewer requests (ALB, Classic) or new flows (NLB) than this per day.
pub const IDLE_LB_REQUESTS_PER_DAY: f64 = 100.0;
/// Days in an average month (monthly costs and savings).
pub const DAYS_PER_MONTH: f64 = 30.4;
/// Days of bills a resource's monthly cost is computed from.
const BILL_DAYS: i64 = 30;
const DAY_MS: i64 = 86_400_000;
/// How long an org's recommendations are reused (the crawlers publish every 10 minutes).
pub const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE: &str = "cost_recommendations";

// ================================================================ resource kinds and their metrics

/// A kind of resource the rules look at.
#[derive(Debug, PartialEq)]
pub struct Kind {
    pub id: &'static str,
    pub provider: &'static str,
    pub label: &'static str,
    /// AWS: the dimension tag naming the resource (`instanceid`). Azure resources are named by `name`.
    pub id_tag: &'static str,
    /// Azure: the resource type in its ARM id.
    pub arm_type: &'static str,
    /// The metric whose series carry the resources' tags.
    pub anchor: &'static str,
    /// Monthly on-demand list price (USD) of a typical size, for resources the bills don't have, and
    /// what it assumes. Public list prices of us-east-1 / East US, our own table.
    pub list_price: f64,
    pub price_basis: &'static str,
    /// How to act on its recommendations (downsizing has its own, [`RIGHTSIZE_ACTIONS`]).
    pub action: &'static str,
}

const fn aws(id: &'static str, label: &'static str, id_tag: &'static str, anchor: &'static str, list_price: f64, price_basis: &'static str, action: &'static str) -> Kind {
    Kind { id, provider: "aws", label, id_tag, arm_type: "", anchor, list_price, price_basis, action }
}

const fn azure(id: &'static str, label: &'static str, arm_type: &'static str, anchor: &'static str, list_price: f64, price_basis: &'static str, action: &'static str) -> Kind {
    Kind { id, provider: "azure", label, id_tag: "name", arm_type, anchor, list_price, price_basis, action }
}

const LB_ACTION: &str = "Check that no DNS record or client still points at its DNS name, then delete it and the target groups only it used.";

pub const KINDS: [Kind; 12] = [
    aws("aws_ec2_instance", "EC2 Instance", "instanceid", "aws.ec2.cpuutilization.maximum", 70.08, "an m5.large on demand in us-east-1 ($0.096 an hour): instance types aren't collected",
        "Make sure nothing depends on it (a standby, a scheduled job's host), snapshot the EBS volumes you want to keep, then stop or terminate it (EC2 → Instances → Instance state)."),
    aws("aws_rds_instance", "RDS Instance", "dbinstanceidentifier", "aws.rds.database_connections.maximum", 49.64, "a db.t3.medium Single-AZ on demand in us-east-1 ($0.068 an hour), storage not included",
        "Take a final snapshot, then delete the instance. Stopping it only pauses the instance charge for 7 days, after which RDS starts it again."),
    aws("aws_ebs_volume", "EBS Volume", "volumeid", "aws.ebs.volume_read_ops.sum", 8.0, "100 GiB of gp3 in us-east-1 ($0.08 per GiB-month): volume sizes aren't collected",
        "Snapshot the volume if its data matters, detach it from its instance, then delete it."),
    aws("aws_nat_gateway", "AWS NAT Gateway", "natgatewayid", "aws.natgateway.bytes_out_to_destination.sum", 32.85, "$0.045 an hour in us-east-1, data processing not included",
        "Check that no route table still sends traffic to it, then delete it and release its Elastic IP address."),
    aws("aws_alb", "Application Load Balancer", "loadbalancer", "aws.applicationelb.request_count.sum", 16.43, "$0.0225 an hour in us-east-1, LCUs not included", LB_ACTION),
    aws("aws_nlb", "Network Load Balancer", "loadbalancer", "aws.networkelb.new_flow_count.sum", 16.43, "$0.0225 an hour in us-east-1, NLCUs not included", LB_ACTION),
    aws("aws_elb", "Classic Load Balancer", "loadbalancername", "aws.elb.request_count.sum", 18.25, "$0.025 an hour in us-east-1, data processed not included", LB_ACTION),
    azure("azure_vm", "Azure Virtual Machine", "Microsoft.Compute/virtualMachines", "azure.vm.percentage_cpu.max", 70.08, "a D2s v5 pay-as-you-go Linux VM in East US ($0.096 an hour): VM sizes aren't collected",
        "Make sure nothing depends on it, then stop (deallocate) it or delete it with its disks. A VM stopped from inside the OS is still allocated and billed."),
    azure("azure_disk", "Azure Managed Disk", "Microsoft.Compute/disks", "azure.compute_disks.composite_disk_read_operations_sec.max", 19.71, "a P10 Premium SSD (128 GiB) in East US: disk sizes aren't collected",
        "Snapshot the disk if its data matters, then delete it; a disk you must keep can move to Standard HDD."),
    azure("azure_nat_gateway", "Azure NAT Gateway", "Microsoft.Network/natGateways", "azure.network_natgateways.byte_count", 32.85, "$0.045 an hour in East US, data processed not included",
        "Detach it from its subnets, then delete it and the public IP addresses only it used."),
    azure("azure_load_balancer", "Azure Load Balancer", "Microsoft.Network/loadBalancers", "azure.network_loadbalancers.byte_count", 18.25, "a Standard load balancer with up to 5 rules ($0.025 an hour) in East US",
        "Check that nothing still sends traffic to its frontend IP, then delete it and the public IP addresses only it used."),
    azure("azure_public_ip", "Azure Public IP Address", "Microsoft.Network/publicIPAddresses", "azure.network_publicipaddresses.byte_count", 3.65, "a Standard static IPv4 address ($0.005 an hour)",
        "Check that no DNS record or allowlist still names the address, dissociate it, then delete it."),
];

/// How to downsize, per provider.
pub const RIGHTSIZE_ACTIONS: [(&str, &str); 2] = [
    ("aws", "Change it to the next smaller size of its family (stop it, then Actions → Instance settings → Change instance type, or update its launch template) and watch its CPU for a week."),
    ("azure", "Resize it to the next smaller size of its series (the VM's Size page; a running VM restarts) and watch its CPU for a week."),
];

/// A statistic of a kind's resources: a metric reduced per resource over the window.
#[derive(Debug)]
pub struct Spec {
    pub kind: &'static str,
    pub stat: &'static str,
    pub metric: &'static str,
    /// How the resource's series combine (load balancers have one per zone and target group too).
    pub space: SpaceAgg,
    /// How each day's points combine.
    pub rollup: Rollup,
    /// How the days combine.
    pub time: TimeAggregator,
}

const fn max(kind: &'static str, stat: &'static str, metric: &'static str) -> Spec {
    Spec { kind, stat, metric, space: SpaceAgg::Max, rollup: Rollup::Max, time: TimeAggregator::Max }
}

const fn avg(kind: &'static str, stat: &'static str, metric: &'static str) -> Spec {
    Spec { kind, stat, metric, space: SpaceAgg::Avg, rollup: Rollup::Avg, time: TimeAggregator::Avg }
}

/// The window's total (the largest of the resource's series: a load balancer's whole one).
const fn total(kind: &'static str, stat: &'static str, metric: &'static str) -> Spec {
    Spec { kind, stat, metric, space: SpaceAgg::Max, rollup: Rollup::Sum, time: TimeAggregator::Sum }
}

pub const SPECS: [Spec; 22] = [
    max("aws_ec2_instance", "cpu_max", "aws.ec2.cpuutilization.maximum"),
    avg("aws_ec2_instance", "cpu_avg", "aws.ec2.cpuutilization"),
    total("aws_ec2_instance", "net_in", "aws.ec2.network_in.sum"),
    total("aws_ec2_instance", "net_out", "aws.ec2.network_out.sum"),
    max("aws_rds_instance", "connections_max", "aws.rds.database_connections.maximum"),
    max("aws_rds_instance", "cpu_max", "aws.rds.cpuutilization.maximum"),
    total("aws_ebs_volume", "read_ops", "aws.ebs.volume_read_ops.sum"),
    total("aws_ebs_volume", "write_ops", "aws.ebs.volume_write_ops.sum"),
    total("aws_nat_gateway", "bytes_out", "aws.natgateway.bytes_out_to_destination.sum"),
    total("aws_nat_gateway", "bytes_in", "aws.natgateway.bytes_in_from_source.sum"),
    total("aws_alb", "requests", "aws.applicationelb.request_count.sum"),
    total("aws_nlb", "flows", "aws.networkelb.new_flow_count.sum"),
    total("aws_elb", "requests", "aws.elb.request_count.sum"),
    max("azure_vm", "cpu_max", "azure.vm.percentage_cpu.max"),
    avg("azure_vm", "cpu_avg", "azure.vm.percentage_cpu"),
    total("azure_vm", "net_in", "azure.vm.network_in_total"),
    total("azure_vm", "net_out", "azure.vm.network_out_total"),
    max("azure_disk", "read_ops_max", "azure.compute_disks.composite_disk_read_operations_sec.max"),
    max("azure_disk", "write_ops_max", "azure.compute_disks.composite_disk_write_operations_sec.max"),
    total("azure_nat_gateway", "bytes", "azure.network_natgateways.byte_count"),
    total("azure_load_balancer", "bytes", "azure.network_loadbalancers.byte_count"),
    total("azure_public_ip", "bytes", "azure.network_publicipaddresses.byte_count"),
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

// ================================================================ the rules

/// What a rule knows of one resource.
#[derive(Debug, Clone)]
pub struct ResourceStats {
    pub kind: &'static Kind,
    pub account: String,
    pub region: String,
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    /// Days with data in the window.
    pub observed_days: f64,
    pub stats: BTreeMap<&'static str, f64>,
    /// Its monthly cost in the bills, when they have it.
    pub bill_monthly: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct RecommendationEvidence {
    pub label: String,
    pub value: String,
}

/// One recommendation: a resource, what to do with it, why, and what it saves.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostRecommendation {
    /// `<resource type>:<resource id>`
    pub id: String,
    /// `aws` or `azure`
    pub provider: String,
    /// AWS account id or Azure subscription id.
    pub account: String,
    pub region: String,
    pub resource_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub resource_type_label: String,
    pub recommendation_type: String,
    pub recommendation_label: String,
    /// The rule's finding with its evidence values ("Max CPU 1.2% … over 14 days").
    pub reason: String,
    pub evidence: Vec<RecommendationEvidence>,
    /// How to act on it.
    pub action: String,
    /// Estimated monthly cost (USD).
    pub monthly_cost: f64,
    /// `bill` (the imported bills) or `list_price` (an estimate from a typical size's list price).
    pub cost_source: String,
    /// What a `list_price` cost assumes.
    pub cost_basis: String,
    pub monthly_savings: f64,
    pub observed_days: f64,
    /// `key:value`
    pub tags: Vec<String>,
}

/// The recommendation types, their labels.
pub const RECOMMENDATION_TYPES: [(&str, &str); 7] = [
    ("idle_compute", "Terminate idle instances"),
    ("rightsize_compute", "Downsize over-provisioned instances"),
    ("unused_database", "Delete unused databases"),
    ("idle_volume", "Delete idle volumes"),
    ("unused_nat_gateway", "Delete unused NAT gateways"),
    ("idle_load_balancer", "Delete idle load balancers"),
    ("unused_public_ip", "Release unused public IPs"),
];

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// `1.2`, `2`, `357.1`, `1,204` (no decimals from 1,000).
fn num(v: f64) -> String {
    if v.abs() >= 1000.0 {
        let s = format!("{:.0}", v.abs());
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        return if v < 0.0 { format!("-{out}") } else { out };
    }
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

fn pct(v: f64) -> String {
    format!("{}%", num(v))
}

/// Decimal units, as the cloud consoles show traffic.
fn bytes(v: f64) -> String {
    let units = [(1e12, "TB"), (1e9, "GB"), (1e6, "MB"), (1e3, "KB")];
    units.iter().find(|(size, _)| v >= *size).map_or_else(|| format!("{} B", num(v)), |(size, unit)| format!("{} {unit}", num(v / size)))
}

/// The recommendation for a resource, if a rule finds waste.
pub fn recommend(r: &ResourceStats) -> Option<CostRecommendation> {
    if r.observed_days < MIN_OBSERVED_DAYS {
        return None;
    }
    let days = r.observed_days;
    let get = |k: &str| r.stats.get(k).copied();
    // The sum of statistics, when at least one is there.
    let sum = |ks: &[&str]| ks.iter().filter_map(|k| get(k)).reduce(|a, b| a + b);
    let over = format!("over {} days", num(days.round()));
    let ev = |label: &str, value: String| RecommendationEvidence { label: label.into(), value };
    let (kind, share, reason, evidence) = match r.kind.id {
        "aws_ec2_instance" | "azure_vm" => {
            let cpu_max = get("cpu_max")?;
            let cpu_avg = get("cpu_avg");
            let net = sum(&["net_in", "net_out"]).map(|n| n / days);
            let mut evidence = vec![ev("Max CPU", pct(cpu_max))];
            evidence.extend(cpu_avg.map(|a| ev("Average CPU", pct(a))));
            evidence.extend(net.map(|n| ev("Network traffic a day", bytes(n))));
            if cpu_max < IDLE_CPU_MAX_PERCENT && net.is_none_or(|n| n < IDLE_NETWORK_BYTES_PER_DAY) {
                let reason = match net {
                    Some(n) => format!("Max CPU {} and {} of network traffic a day {over}", pct(cpu_max), bytes(n)),
                    None => format!("Max CPU {} {over}", pct(cpu_max)),
                };
                ("idle_compute", 1.0, reason, evidence)
            } else {
                let cpu_avg = cpu_avg?;
                if cpu_avg >= RIGHTSIZE_CPU_AVG_PERCENT || cpu_max >= RIGHTSIZE_CPU_MAX_PERCENT {
                    return None;
                }
                evidence.swap(0, 1);
                ("rightsize_compute", RIGHTSIZE_SAVINGS_SHARE, format!("Average CPU {} and max CPU {} {over}", pct(cpu_avg), pct(cpu_max)), evidence)
            }
        }
        "aws_rds_instance" => {
            let connections = get("connections_max")?;
            if connections > UNUSED_DB_MAX_CONNECTIONS {
                return None;
            }
            let mut evidence = vec![ev("Max connections", num(connections))];
            evidence.extend(get("cpu_max").map(|c| ev("Max CPU", pct(c))));
            ("unused_database", 1.0, format!("No connections {over}"), evidence)
        }
        "aws_ebs_volume" => {
            let ops = sum(&["read_ops", "write_ops"])? / days;
            if ops >= IDLE_VOLUME_OPS_PER_DAY {
                return None;
            }
            ("idle_volume", 1.0, format!("{} read and write operations a day {over}", num(ops)), vec![ev("Read and write operations a day", num(ops))])
        }
        "azure_disk" => {
            let peak = get("read_ops_max").into_iter().chain(get("write_ops_max")).reduce(f64::max)?;
            if peak >= IDLE_DISK_MAX_OPS_PER_SEC {
                return None;
            }
            ("idle_volume", 1.0, format!("At most {} read or write operations a second {over}", num(peak)), vec![ev("Max operations a second", num(peak))])
        }
        "aws_alb" | "aws_elb" | "aws_nlb" => {
            let (stat, what) = if r.kind.id == "aws_nlb" { ("flows", "new connections") } else { ("requests", "requests") };
            let per_day = get(stat)? / days;
            if per_day >= IDLE_LB_REQUESTS_PER_DAY {
                return None;
            }
            let label = format!("{}{} a day", what[..1].to_uppercase(), &what[1..]);
            ("idle_load_balancer", 1.0, format!("{} {what} a day {over}", num(per_day)), vec![ev(&label, num(per_day))])
        }
        "aws_nat_gateway" | "azure_nat_gateway" | "azure_load_balancer" | "azure_public_ip" => {
            let per_day = sum(&["bytes_out", "bytes_in", "bytes"])? / days;
            if per_day >= UNUSED_NETWORK_BYTES_PER_DAY {
                return None;
            }
            let kind = match r.kind.id {
                "azure_load_balancer" => "idle_load_balancer",
                "azure_public_ip" => "unused_public_ip",
                _ => "unused_nat_gateway",
            };
            (kind, 1.0, format!("{} of traffic a day {over}", bytes(per_day)), vec![ev("Traffic a day", bytes(per_day))])
        }
        _ => return None,
    };
    let (monthly_cost, cost_source, cost_basis) = match r.bill_monthly {
        Some(c) => (round2(c), "bill", ""),
        None => (r.kind.list_price, "list_price", r.kind.price_basis),
    };
    let action = if kind == "rightsize_compute" { RIGHTSIZE_ACTIONS.iter().find(|(p, _)| *p == r.kind.provider).map_or("", |(_, a)| a) } else { r.kind.action };
    Some(CostRecommendation {
        id: format!("{}:{}", r.kind.id, r.id),
        provider: r.kind.provider.into(),
        account: r.account.clone(),
        region: r.region.clone(),
        resource_id: r.id.clone(),
        resource_name: r.name.clone(),
        resource_type: r.kind.id.into(),
        resource_type_label: r.kind.label.into(),
        recommendation_type: kind.into(),
        recommendation_label: RECOMMENDATION_TYPES.iter().find(|(id, _)| *id == kind).map_or(kind, |(_, l)| l).into(),
        reason,
        evidence,
        action: action.into(),
        monthly_cost,
        cost_source: cost_source.into(),
        cost_basis: cost_basis.into(),
        monthly_savings: round2(monthly_cost * share),
        observed_days: days,
        tags: r.tags.clone(),
    })
}

// ================================================================ resources from metric groups

/// The tags a kind's metrics are grouped by.
pub fn group_by(kind: &Kind) -> Vec<String> {
    let keys: &[&str] = if kind.provider == "aws" { &["aws_account", "region", kind.id_tag] } else { &["subscription_id", "resource_group", "region", "name"] };
    keys.iter().map(|k| k.to_string()).collect()
}

/// (account, region, resource id) named by a group's (or a series') tags; None without a resource.
pub fn resource_key(kind: &Kind, tags: &BTreeMap<String, String>) -> Option<(String, String, String)> {
    let get = |k: &str| tags.get(k).map(String::as_str).filter(|v| !v.is_empty() && *v != "N/A");
    let region = get("region").unwrap_or_default().to_string();
    if kind.provider == "aws" {
        let id = get(kind.id_tag)?;
        return Some((get("aws_account").unwrap_or_default().to_string(), region, id.to_string()));
    }
    let (subscription, group, name) = (get("subscription_id")?, get("resource_group")?, get("name")?);
    Some((subscription.to_string(), region, format!("/subscriptions/{subscription}/resourceGroups/{group}/providers/{}/{name}", kind.arm_type)))
}

/// Tags that name the resource rather than describe it.
const STRUCTURAL_TAGS: [&str; 7] = ["aws_account", "region", "subscription_id", "resource_group", "name", "availabilityzone", "targetgroup"];

/// A resource's display name and tags from its series' attributes (structural tags left out).
pub fn name_and_tags(kind: &Kind, id: &str, attrs: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let name_key = if kind.provider == "aws" { "Name" } else { "name" };
    let name = attrs.get(name_key).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| id.to_string());
    let tags = attrs.iter().filter(|(k, _)| !STRUCTURAL_TAGS.contains(&k.as_str()) && k.as_str() != kind.id_tag).map(|(k, v)| format!("{k}:{v}")).collect();
    (name, tags)
}

// ================================================================ costs

/// The part of a resource id after its last `/` or `:` (`i-0a…`, an ARN's or ARM id's last segment).
fn last_segment(id: &str) -> &str {
    id.rsplit(['/', ':']).next().unwrap_or(id)
}

/// Monthly costs of the bills' resources, found by the resource ids the metrics name.
#[derive(Default)]
pub struct BillCosts {
    /// Last segment → (provider, lower-cased ResourceId, monthly cost).
    by_segment: HashMap<String, Vec<(String, String, f64)>>,
}

impl BillCosts {
    /// From (provider, ResourceId, cost over `days` days) rows.
    pub fn new(rows: Vec<(String, String, f64, u64)>) -> Self {
        let mut by_segment: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();
        for (provider, id, cost, days) in rows {
            let id = id.to_ascii_lowercase();
            by_segment.entry(last_segment(&id).to_string()).or_default().push((provider, id.clone(), cost / days.max(1) as f64 * DAYS_PER_MONTH));
        }
        BillCosts { by_segment }
    }

    /// A resource's monthly cost: bill lines whose ResourceId is its id, or an ARN ending in it.
    pub fn monthly(&self, provider: &str, id: &str) -> Option<f64> {
        let id = id.to_ascii_lowercase();
        let (slash, colon) = (format!("/{id}"), format!(":{id}"));
        self.by_segment
            .get(last_segment(&id))?
            .iter()
            .filter(|(p, full, _)| p == provider && (*full == id || full.ends_with(&slash) || full.ends_with(&colon)))
            .map(|(_, _, cost)| *cost)
            .reduce(|a, b| a + b)
    }
}

/// The last 30 days of the AWS and Azure bills, per resource.
pub fn bill_costs_sql() -> String {
    format!(
        "SELECT ProviderName AS provider, ResourceId AS resource, sum(EffectiveCost) AS cost, uniqExact(ChargeDate) AS days FROM {} FINAL \
         WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ProviderName IN ('aws', 'azure') AND ResourceId != '' AND ChargeDate >= today() - {BILL_DAYS} \
         GROUP BY provider, resource",
        super::cloud_cost::TABLE
    )
}

/// The attributes of the kinds' anchor series seen in the window.
pub fn series_tags_sql() -> String {
    let anchors: Vec<&str> = KINDS.iter().map(|k| k.anchor).collect();
    format!(
        "SELECT MetricName AS metric, any(Attributes) AS attrs FROM otel.metrics_series \
         WHERE OrgId = {{oc_org:String}} AND MetricName IN ({}) AND LastSeen >= now() - INTERVAL {LOOKBACK_DAYS} DAY \
         GROUP BY MetricName, SeriesId LIMIT 200000",
        super::metrics::name_list(&anchors)
    )
}

#[derive(clickhouse::Row, Deserialize)]
struct BillRow {
    provider: String,
    resource: String,
    cost: f64,
    days: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct SeriesTagsRow {
    metric: String,
    attrs: Vec<(String, String)>,
}

// ================================================================ finding them

fn reduce(points: &[(i64, f64)], agg: TimeAggregator) -> Option<f64> {
    let values = points.iter().map(|p| p.1);
    match agg {
        _ if points.is_empty() => None,
        TimeAggregator::Max => values.reduce(f64::max),
        TimeAggregator::Min => values.reduce(f64::min),
        TimeAggregator::Sum => Some(values.sum()),
        TimeAggregator::Avg => Some(values.sum::<f64>() / points.len() as f64),
        TimeAggregator::Last => points.last().map(|p| p.1),
    }
}

/// Every recommendation for the org's resources, from the last [`LOOKBACK_DAYS`] days ending `now_ms`.
pub async fn find(st: &AppState, org: &str, now_ms: i64) -> Result<Vec<CostRecommendation>, ApiError> {
    let range = TimeRange { from: now_ms - LOOKBACK_DAYS * DAY_MS, to: now_ms };
    let results = futures::future::join_all(SPECS.iter().map(|s| async move {
        let k = kind(s.kind).expect("a spec's kind");
        let q = MetricQuery { agg: s.space, metric: s.metric.into(), filters: vec![], by: group_by(k), rollup: Some(s.rollup) };
        (s, super::metrics::evaluate_query_stepped(st, org, &q, range, LOOKBACK_DAYS as u32).await)
    }))
    .await;
    let mut resources: BTreeMap<(&'static str, String), ResourceStats> = BTreeMap::new();
    for (s, result) in results {
        let (groups, step) = match result {
            Ok(r) => r,
            // A metric nobody sent yet (or a query the org's indexing rules refuse): no evidence.
            Err(ApiError::BadRequest(e)) => {
                tracing::debug!(org, metric = s.metric, "cost recommendations: {e}");
                continue;
            }
            Err(e) => return Err(e),
        };
        let k = kind(s.kind).expect("a spec's kind");
        for g in groups {
            let (Some((account, region, id)), Some(value)) = (resource_key(k, &g.tags), reduce(&g.points, s.time)) else { continue };
            let days = (g.points.len() as i64 * step) as f64 / DAY_MS as f64;
            let r = resources.entry((k.id, id.clone())).or_insert_with(|| ResourceStats { kind: k, account, region, name: id.clone(), id, tags: vec![], observed_days: 0.0, stats: BTreeMap::new(), bill_monthly: None });
            r.observed_days = r.observed_days.max(days.min(LOOKBACK_DAYS as f64));
            r.stats.insert(s.stat, value);
        }
    }
    if resources.is_empty() {
        return Ok(vec![]);
    }
    let ch = st.ch(org);
    let series = ch.query(&series_tags_sql()).fetch_all::<SeriesTagsRow>().await?;
    let mut attrs: HashMap<(&'static str, String), BTreeMap<String, String>> = HashMap::new();
    for row in series {
        let map: BTreeMap<String, String> = row.attrs.into_iter().collect();
        for k in KINDS.iter().filter(|k| k.anchor == row.metric) {
            if let Some((_, _, id)) = resource_key(k, &map) {
                attrs.entry((k.id, id)).or_default().extend(map.clone());
            }
        }
    }
    let bills = ch.query(&bill_costs_sql()).fetch_all::<BillRow>().await?;
    let costs = BillCosts::new(bills.into_iter().map(|b| (b.provider, b.resource, b.cost, b.days)).collect());
    Ok(resources
        .into_values()
        .filter_map(|mut r| {
            if let Some(a) = attrs.get(&(r.kind.id, r.id.clone())) {
                (r.name, r.tags) = name_and_tags(r.kind, &r.id, a);
            }
            r.bill_monthly = costs.monthly(r.kind.provider, &r.id);
            recommend(&r)
        })
        .collect())
}

// ================================================================ filters and totals

#[derive(Default, Debug)]
pub struct RecommendationFilters {
    pub provider: Option<String>,
    pub resource_type: Option<String>,
    pub recommendation_type: Option<String>,
    /// `key:value` (with `*` wildcards); a recommendation needs all of them.
    pub tags: Vec<String>,
}

impl RecommendationFilters {
    /// The account and region filter like the tags `account:` and `region:`.
    fn matches(&self, r: &CostRecommendation) -> bool {
        let is = |f: &Option<String>, v: &str| f.as_deref().is_none_or(|f| f == v);
        let (account, region) = (format!("account:{}", r.account), format!("region:{}", r.region));
        is(&self.provider, &r.provider)
            && is(&self.resource_type, &r.resource_type)
            && is(&self.recommendation_type, &r.recommendation_type)
            && self.tags.iter().all(|t| r.tags.iter().chain([&account, &region]).any(|x| glob(t, x)))
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
pub struct ConnectedAccounts {
    pub aws: i64,
    pub azure: i64,
}

/// A recommendation type's (or resource type's) share.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationGroup {
    pub id: String,
    pub label: String,
    pub count: usize,
    pub monthly_cost: f64,
    pub monthly_savings: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationsResult {
    /// Largest savings first.
    pub recommendations: Vec<CostRecommendation>,
    pub count: usize,
    pub monthly_cost: f64,
    pub monthly_savings: f64,
    pub annual_savings: f64,
    /// Per recommendation type, largest savings first.
    pub by_type: Vec<RecommendationGroup>,
    /// The filters' choices: every resource and recommendation type found (before filtering).
    pub resource_types: Vec<RecommendationGroup>,
    pub recommendation_types: Vec<RecommendationGroup>,
    /// The connected AWS accounts and Azure app registrations.
    pub accounts: ConnectedAccounts,
    pub lookback_days: i64,
}

/// Recommendations grouped by `key` (id, label), largest savings first.
fn groups<'a>(recs: impl Iterator<Item = &'a CostRecommendation>, key: impl Fn(&'a CostRecommendation) -> (&'a str, &'a str)) -> Vec<RecommendationGroup> {
    let mut by: BTreeMap<&str, RecommendationGroup> = BTreeMap::new();
    for r in recs {
        let (id, label) = key(r);
        let g = by.entry(id).or_insert_with(|| RecommendationGroup { id: id.into(), label: label.into(), count: 0, monthly_cost: 0.0, monthly_savings: 0.0 });
        g.count += 1;
        g.monthly_cost += r.monthly_cost;
        g.monthly_savings += r.monthly_savings;
    }
    let mut out: Vec<RecommendationGroup> = by.into_values().map(|g| RecommendationGroup { monthly_cost: round2(g.monthly_cost), monthly_savings: round2(g.monthly_savings), ..g }).collect();
    out.sort_by(|a, b| b.monthly_savings.total_cmp(&a.monthly_savings).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The filtered recommendations with their totals.
pub fn summarize(all: Vec<CostRecommendation>, f: &RecommendationFilters, accounts: ConnectedAccounts) -> RecommendationsResult {
    let resource_types = groups(all.iter(), |r| (&r.resource_type, &r.resource_type_label));
    let recommendation_types = groups(all.iter(), |r| (&r.recommendation_type, &r.recommendation_label));
    let mut recommendations: Vec<CostRecommendation> = all.into_iter().filter(|r| f.matches(r)).collect();
    recommendations.sort_by(|a, b| b.monthly_savings.total_cmp(&a.monthly_savings).then_with(|| a.id.cmp(&b.id)));
    let monthly_savings: f64 = recommendations.iter().map(|r| r.monthly_savings).sum();
    RecommendationsResult {
        count: recommendations.len(),
        monthly_cost: round2(recommendations.iter().map(|r| r.monthly_cost).sum()),
        monthly_savings: round2(monthly_savings),
        annual_savings: round2(monthly_savings * 12.0),
        by_type: groups(recommendations.iter(), |r| (&r.recommendation_type, &r.recommendation_label)),
        recommendations,
        resource_types,
        recommendation_types,
        accounts,
        lookback_days: LOOKBACK_DAYS,
    }
}

// ================================================================ the endpoint

#[derive(Deserialize, IntoParams, Default)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct RecommendationsQuery {
    /// `aws` or `azure`
    pub provider: Option<String>,
    pub resource_type: Option<String>,
    pub recommendation_type: Option<String>,
    /// Comma-separated `key:value` tags (`*` wildcards); a recommendation needs all of them.
    pub tags: Option<String>,
}

impl RecommendationsQuery {
    pub fn filters(&self) -> RecommendationFilters {
        let given = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        RecommendationFilters {
            provider: given(&self.provider),
            resource_type: given(&self.resource_type),
            recommendation_type: given(&self.recommendation_type),
            tags: self.tags.as_deref().unwrap_or_default().split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
        }
    }
}

/// Cost-saving recommendations for the org's AWS and Azure resources, with their totals.
#[utoipa::path(get, path = "/api/v1/cloud-cost/recommendations", operation_id = "cloud_cost_recommendations", params(RecommendationsQuery),
    responses((status = 200, body = RecommendationsResult)), tag = "cloud-cost")]
pub async fn recommendations(State(st): State<AppState>, OrgId(org): OrgId, Query(q): Query<RecommendationsQuery>) -> ApiResult<RecommendationsResult> {
    let accounts = connected_accounts(&st, &org).await?;
    let all = if accounts.aws + accounts.azure == 0 {
        vec![]
    } else {
        let found = st.cache.get_or_load(CACHE, &org, "", CACHE_TTL, || find(&st, &org, super::metrics::now_ms())).await?;
        std::sync::Arc::unwrap_or_clone(found)
    };
    Ok(Json(summarize(all, &q.filters(), accounts)))
}

/// The org's connected AWS accounts and Azure app registrations.
pub async fn connected_accounts(st: &AppState, org: &str) -> Result<ConnectedAccounts, ApiError> {
    let (aws, azure): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM aws_accounts WHERE org_id = $1), (SELECT count(*) FROM azure_accounts WHERE org_id = $1)").bind(org).fetch_one(&st.postgres).await?;
    Ok(ConnectedAccounts { aws, azure })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(kind_id: &str, days: f64, values: &[(&'static str, f64)]) -> ResourceStats {
        let k = kind(kind_id).unwrap_or_else(|| panic!("kind {kind_id}"));
        ResourceStats {
            kind: k,
            account: "123456789012".into(),
            region: "us-east-1".into(),
            id: "i-0a1b2c3d4e5f60001".into(),
            name: "storefront-api-1".into(),
            tags: vec!["env:prod".into(), "team:storefront".into()],
            observed_days: days,
            stats: values.iter().cloned().collect(),
            bill_monthly: None,
        }
    }

    const MB: f64 = 1e6;

    #[test]
    fn every_kind_has_its_specs_and_the_specs_name_metrics_the_crawlers_publish() {
        use crate::aws::naming::metric_name as aws;
        use crate::azure::naming::metric_name as azure;
        let published = [
            format!("{}.maximum", aws("AWS/EC2", "CPUUtilization")),
            aws("AWS/EC2", "CPUUtilization"),
            format!("{}.sum", aws("AWS/EC2", "NetworkIn")),
            format!("{}.sum", aws("AWS/EC2", "NetworkOut")),
            format!("{}.maximum", aws("AWS/RDS", "DatabaseConnections")),
            format!("{}.maximum", aws("AWS/RDS", "CPUUtilization")),
            format!("{}.sum", aws("AWS/EBS", "VolumeReadOps")),
            format!("{}.sum", aws("AWS/EBS", "VolumeWriteOps")),
            format!("{}.sum", aws("AWS/NATGateway", "BytesOutToDestination")),
            format!("{}.sum", aws("AWS/NATGateway", "BytesInFromSource")),
            format!("{}.sum", aws("AWS/ApplicationELB", "RequestCount")),
            format!("{}.sum", aws("AWS/NetworkELB", "NewFlowCount")),
            format!("{}.sum", aws("AWS/ELB", "RequestCount")),
            format!("{}.max", azure("Microsoft.Compute/virtualMachines", "Percentage CPU")),
            azure("Microsoft.Compute/virtualMachines", "Percentage CPU"),
            azure("Microsoft.Compute/virtualMachines", "Network In Total"),
            azure("Microsoft.Compute/virtualMachines", "Network Out Total"),
            format!("{}.max", azure("Microsoft.Compute/disks", "Composite Disk Read Operations/sec")),
            format!("{}.max", azure("Microsoft.Compute/disks", "Composite Disk Write Operations/sec")),
            azure("Microsoft.Network/natGateways", "ByteCount"),
            azure("Microsoft.Network/loadBalancers", "ByteCount"),
            azure("Microsoft.Network/publicIPAddresses", "ByteCount"),
        ];
        for s in &SPECS {
            assert!(published.iter().any(|p| p == s.metric), "{} isn't a metric the crawlers publish", s.metric);
            assert!(kind(s.kind).is_some(), "{}", s.kind);
        }
        for k in &KINDS {
            assert!(SPECS.iter().any(|s| s.kind == k.id && s.metric == k.anchor), "{}'s anchor", k.id);
            assert!(k.list_price > 0.0 && !k.price_basis.is_empty() && !k.action.is_empty(), "{}", k.id);
        }
        assert_eq!(KINDS.len(), 12);
    }

    #[test]
    fn an_idle_instance_is_terminated_and_saves_its_whole_list_price() {
        let r = recommend(&stats("aws_ec2_instance", 14.0, &[("cpu_max", 1.2), ("cpu_avg", 0.4), ("net_in", 3.0 * MB), ("net_out", 2.0 * MB)])).expect("idle");
        assert_eq!((r.recommendation_type.as_str(), r.resource_type.as_str(), r.provider.as_str()), ("idle_compute", "aws_ec2_instance", "aws"));
        assert_eq!(r.reason, "Max CPU 1.2% and 357.1 KB of network traffic a day over 14 days");
        assert_eq!((r.monthly_cost, r.monthly_savings, r.cost_source.as_str()), (70.08, 70.08, "list_price"));
        assert!(!r.cost_basis.is_empty());
        assert_eq!(r.id, "aws_ec2_instance:i-0a1b2c3d4e5f60001");
        assert_eq!(r.evidence[0], RecommendationEvidence { label: "Max CPU".into(), value: "1.2%".into() });
        assert_eq!(r.tags, ["env:prod", "team:storefront"]);
    }

    #[test]
    fn a_busy_instance_gets_nothing_and_a_quiet_one_is_downsized_at_half_its_billed_cost() {
        assert_eq!(recommend(&stats("aws_ec2_instance", 14.0, &[("cpu_max", 92.0), ("cpu_avg", 55.0)])), None);
        // Low CPU but real traffic: not idle; low enough to halve.
        let mut s = stats("aws_ec2_instance", 14.0, &[("cpu_max", 3.0), ("cpu_avg", 2.0), ("net_in", 900.0 * MB * 14.0)]);
        s.bill_monthly = Some(140.0);
        let r = recommend(&s).expect("rightsize");
        assert_eq!((r.recommendation_type.as_str(), r.monthly_cost, r.monthly_savings, r.cost_source.as_str()), ("rightsize_compute", 140.0, 70.0, "bill"));
        assert_eq!(r.reason, "Average CPU 2% and max CPU 3% over 14 days");
        // Peaks too high to halve.
        assert_eq!(recommend(&stats("aws_ec2_instance", 14.0, &[("cpu_max", 65.0), ("cpu_avg", 4.0)])), None);
    }

    #[test]
    fn resources_seen_on_too_few_days_get_nothing() {
        assert_eq!(recommend(&stats("aws_ec2_instance", 2.0, &[("cpu_max", 0.5), ("cpu_avg", 0.1)])), None);
        assert!(recommend(&stats("aws_ec2_instance", MIN_OBSERVED_DAYS, &[("cpu_max", 0.5), ("cpu_avg", 0.1)])).is_some());
    }

    #[test]
    fn databases_without_connections_are_unused() {
        let r = recommend(&stats("aws_rds_instance", 14.0, &[("connections_max", 0.0), ("cpu_max", 4.0)])).expect("unused");
        assert_eq!((r.recommendation_type.as_str(), r.reason.as_str()), ("unused_database", "No connections over 14 days"));
        assert_eq!(recommend(&stats("aws_rds_instance", 14.0, &[("connections_max", 3.0)])), None);
    }

    #[test]
    fn volumes_nat_gateways_and_load_balancers_without_traffic_are_idle() {
        let r = recommend(&stats("aws_ebs_volume", 10.0, &[("read_ops", 0.0), ("write_ops", 4.0)])).expect("idle volume");
        assert_eq!((r.recommendation_type.as_str(), r.reason.as_str()), ("idle_volume", "0.4 read and write operations a day over 10 days"));
        assert_eq!(recommend(&stats("aws_ebs_volume", 10.0, &[("read_ops", 5000.0), ("write_ops", 90.0)])), None);

        let r = recommend(&stats("aws_nat_gateway", 14.0, &[("bytes_out", 1.4 * MB), ("bytes_in", 0.0)])).expect("unused nat");
        assert_eq!((r.recommendation_type.as_str(), r.monthly_cost), ("unused_nat_gateway", 32.85));
        assert_eq!(recommend(&stats("aws_nat_gateway", 14.0, &[("bytes_out", 40e9)])), None);

        let r = recommend(&stats("aws_alb", 14.0, &[("requests", 28.0)])).expect("idle alb");
        assert_eq!((r.recommendation_type.as_str(), r.reason.as_str()), ("idle_load_balancer", "2 requests a day over 14 days"));
        assert_eq!(recommend(&stats("aws_alb", 14.0, &[("requests", 1e6)])), None);
        assert!(recommend(&stats("aws_nlb", 14.0, &[("flows", 0.0)])).is_some());
        assert!(recommend(&stats("aws_elb", 14.0, &[("requests", 0.0)])).is_some());
    }

    #[test]
    fn azure_vms_disks_and_network_resources_follow_the_same_rules() {
        let r = recommend(&stats("azure_vm", 14.0, &[("cpu_max", 2.0), ("cpu_avg", 1.0), ("net_in", 0.0), ("net_out", 0.0)])).expect("idle vm");
        assert_eq!((r.recommendation_type.as_str(), r.provider.as_str()), ("idle_compute", "azure"));
        assert_eq!(recommend(&stats("azure_vm", 14.0, &[("cpu_max", 35.0), ("cpu_avg", 6.0)])).map(|r| r.recommendation_type), Some("rightsize_compute".to_string()));
        let r = recommend(&stats("azure_disk", 14.0, &[("read_ops_max", 0.0), ("write_ops_max", 0.0)])).expect("idle disk");
        assert_eq!(r.recommendation_type, "idle_volume");
        assert_eq!(recommend(&stats("azure_disk", 14.0, &[("read_ops_max", 12.0), ("write_ops_max", 0.0)])), None);
        assert_eq!(recommend(&stats("azure_nat_gateway", 14.0, &[("bytes", 0.0)])).map(|r| r.recommendation_type), Some("unused_nat_gateway".to_string()));
        assert_eq!(recommend(&stats("azure_load_balancer", 14.0, &[("bytes", 0.0)])).map(|r| r.recommendation_type), Some("idle_load_balancer".to_string()));
        assert_eq!(recommend(&stats("azure_public_ip", 14.0, &[("bytes", 0.0)])).map(|r| r.recommendation_type), Some("unused_public_ip".to_string()));
        assert_eq!(recommend(&stats("azure_public_ip", 14.0, &[("bytes", 9e9)])), None);
        // A statistic that isn't there: no judgement.
        assert_eq!(recommend(&stats("azure_public_ip", 14.0, &[])), None);
    }

    fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn groups_name_aws_resources_by_their_dimension_and_azure_ones_by_their_arm_id() {
        let ec2 = kind("aws_ec2_instance").unwrap();
        assert_eq!(group_by(ec2), ["aws_account", "region", "instanceid"]);
        let key = resource_key(ec2, &tags(&[("aws_account", "123456789012"), ("region", "us-east-1"), ("instanceid", "i-0a1b2c3d4e5f60001")]));
        assert_eq!(key, Some(("123456789012".into(), "us-east-1".into(), "i-0a1b2c3d4e5f60001".into())));
        // The per-type aggregate series CloudWatch also publishes name no instance.
        assert_eq!(resource_key(ec2, &tags(&[("aws_account", "123456789012"), ("region", "us-east-1"), ("instanceid", "N/A")])), None);
        assert_eq!(resource_key(ec2, &tags(&[("aws_account", "123456789012"), ("instancetype", "m5.large")])), None);

        let vm = kind("azure_vm").unwrap();
        assert_eq!(group_by(vm), ["subscription_id", "resource_group", "region", "name"]);
        let key = resource_key(vm, &tags(&[("subscription_id", "00000000-0000-0000-0000-000000000001"), ("resource_group", "shop-prod"), ("region", "eastus"), ("name", "catalog-vm-1")]));
        assert_eq!(key, Some(("00000000-0000-0000-0000-000000000001".into(), "eastus".into(), "/subscriptions/00000000-0000-0000-0000-000000000001/resourceGroups/shop-prod/providers/Microsoft.Compute/virtualMachines/catalog-vm-1".into())));
    }

    #[test]
    fn names_come_from_the_name_tag_and_structural_tags_are_left_out() {
        let ec2 = kind("aws_ec2_instance").unwrap();
        let attrs = tags(&[("aws_account", "123456789012"), ("region", "us-east-1"), ("instanceid", "i-0a1b2c3d4e5f60001"), ("Name", "storefront-api-1"), ("env", "prod")]);
        assert_eq!(name_and_tags(ec2, "i-0a1b2c3d4e5f60001", &attrs), ("storefront-api-1".to_string(), vec!["Name:storefront-api-1".to_string(), "env:prod".to_string()]));
        assert_eq!(name_and_tags(ec2, "i-0a1b2c3d4e5f60002", &tags(&[("instanceid", "i-0a1b2c3d4e5f60002")])).0, "i-0a1b2c3d4e5f60002");
        let disk = kind("azure_disk").unwrap();
        let attrs = tags(&[("subscription_id", "s"), ("resource_group", "shop-prod"), ("region", "eastus"), ("name", "catalog-data-1"), ("team", "catalog")]);
        assert_eq!(name_and_tags(disk, "/subscriptions/s/…", &attrs), ("catalog-data-1".to_string(), vec!["team:catalog".to_string()]));
    }

    #[test]
    fn bill_lines_match_resource_ids_and_arns_ending_in_them() {
        let costs = BillCosts::new(vec![
            ("aws".into(), "i-0a1b2c3d4e5f60001".into(), 300.0, 30),
            ("aws".into(), "arn:aws:ec2:us-east-1:123456789012:natgateway/nat-0a1b2c3d4e5f60001".into(), 15.0, 10),
            ("aws".into(), "arn:aws:elasticloadbalancing:us-east-1:123456789012:loadbalancer/app/storefront/50dc6c495c0c9188".into(), 16.0, 30),
            ("aws".into(), "arn:aws:rds:us-east-1:123456789012:db:orders-db".into(), 60.0, 30),
            ("azure".into(), "/subscriptions/S/resourcegroups/SHOP-PROD/providers/microsoft.compute/virtualmachines/catalog-vm-1".into(), 90.0, 30),
        ]);
        assert_eq!(costs.monthly("aws", "i-0a1b2c3d4e5f60001"), Some(300.0 / 30.0 * DAYS_PER_MONTH));
        assert_eq!(costs.monthly("aws", "nat-0a1b2c3d4e5f60001"), Some(15.0 / 10.0 * DAYS_PER_MONTH));
        assert_eq!(costs.monthly("aws", "app/storefront/50dc6c495c0c9188"), Some(16.0 / 30.0 * DAYS_PER_MONTH));
        assert_eq!(costs.monthly("aws", "orders-db"), Some(60.0 / 30.0 * DAYS_PER_MONTH));
        assert_eq!(costs.monthly("azure", "/subscriptions/s/resourceGroups/shop-prod/providers/Microsoft.Compute/virtualMachines/catalog-vm-1"), Some(90.0 / 30.0 * DAYS_PER_MONTH));
        assert_eq!(costs.monthly("aws", "i-0a1b2c3d4e5f60009"), None);
        assert_eq!(costs.monthly("azure", "i-0a1b2c3d4e5f60001"), None);
        // `db:orders-db` must not match `db:shop-orders-db`.
        assert_eq!(costs.monthly("aws", "shop-orders-db"), None);
    }

    #[test]
    fn the_queries_read_only_the_orgs_rows_and_take_no_input() {
        for sql in [bill_costs_sql(), series_tags_sql()] {
            assert!(sql.contains("OrgId = {oc_org:String}"), "{sql}");
            assert!(!sql.contains('?'), "no bound input: {sql}");
        }
        assert!(bill_costs_sql().contains("otel.cloud_costs FINAL"));
        assert!(series_tags_sql().contains("otel.metrics_series"));
        for k in &KINDS {
            assert!(series_tags_sql().contains(&format!("'{}'", k.anchor)), "{}", k.anchor);
        }
    }

    fn rec(kind_id: &str, id: &str, values: &[(&'static str, f64)], tags: &[&str]) -> CostRecommendation {
        let mut s = stats(kind_id, 14.0, values);
        s.id = id.into();
        s.tags = tags.iter().map(|t| t.to_string()).collect();
        recommend(&s).expect("a recommendation")
    }

    #[test]
    fn filters_and_totals() {
        let all = vec![
            rec("aws_ec2_instance", "i-0a1b2c3d4e5f60001", &[("cpu_max", 1.0), ("cpu_avg", 0.5)], &["env:prod", "team:storefront"]),
            rec("aws_ec2_instance", "i-0a1b2c3d4e5f60002", &[("cpu_max", 20.0), ("cpu_avg", 5.0)], &["env:staging"]),
            rec("aws_nat_gateway", "nat-0a1b2c3d4e5f60001", &[("bytes_out", 0.0)], &["env:prod"]),
            rec("azure_public_ip", "/subscriptions/s/resourceGroups/shop/providers/Microsoft.Network/publicIPAddresses/ip-1", &[("bytes", 0.0)], &[]),
        ];
        let accounts = ConnectedAccounts { aws: 3, azure: 1 };
        let r = summarize(all.clone(), &RecommendationFilters::default(), accounts.clone());
        assert_eq!(r.count, 4);
        assert_eq!(r.recommendations.iter().map(|x| x.monthly_savings).collect::<Vec<_>>(), [70.08, 35.04, 32.85, 3.65]);
        assert!((r.monthly_savings - 141.62).abs() < 1e-9 && (r.annual_savings - 141.62 * 12.0).abs() < 1e-6);
        assert_eq!(r.by_type.iter().map(|g| (g.id.as_str(), g.count)).collect::<Vec<_>>(), [("idle_compute", 1), ("rightsize_compute", 1), ("unused_nat_gateway", 1), ("unused_public_ip", 1)]);
        assert_eq!(r.by_type[0].label, "Terminate idle instances");
        assert_eq!((r.accounts.clone(), r.lookback_days), (accounts.clone(), LOOKBACK_DAYS));

        let only = |f: RecommendationFilters| summarize(all.clone(), &f, accounts.clone());
        let azure = only(RecommendationFilters { provider: Some("azure".into()), ..Default::default() });
        assert_eq!((azure.count, azure.monthly_savings), (1, 3.65));
        // The filters' choices stay those of everything found.
        assert_eq!(azure.resource_types.len(), 3);
        assert_eq!(azure.recommendation_types.len(), 4);
        assert_eq!(only(RecommendationFilters { resource_type: Some("aws_ec2_instance".into()), ..Default::default() }).count, 2);
        assert_eq!(only(RecommendationFilters { recommendation_type: Some("rightsize_compute".into()), ..Default::default() }).count, 1);
        assert_eq!(only(RecommendationFilters { tags: vec!["env:prod".into()], ..Default::default() }).count, 2);
        assert_eq!(only(RecommendationFilters { tags: vec!["env:prod".into(), "team:store*".into()], ..Default::default() }).count, 1);
        // Account and region filter like tags.
        assert_eq!(only(RecommendationFilters { tags: vec!["region:us-east-1".into(), "account:123456789012".into()], ..Default::default() }).count, 4);
    }

    #[test]
    fn query_parameters_become_filters() {
        let q = RecommendationsQuery { provider: Some("aws".into()), resource_type: Some("".into()), recommendation_type: None, tags: Some(" env:prod, team:storefront ,".into()) };
        let f = q.filters();
        assert_eq!((f.provider.as_deref(), f.resource_type, f.recommendation_type), (Some("aws"), None, None));
        assert_eq!(f.tags, ["env:prod", "team:storefront"]);
    }

    async fn aws_account(st: &AppState, org: &str) {
        let attrs = serde_json::json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "wJalrXUtnFEMI" }, "aws_regions": { "include_only": ["us-east-1"] } });
        crate::routes::datadog_api::aws_integration::create_row(st, org, attrs.as_object().unwrap()).await.unwrap();
    }

    #[tokio::test]
    async fn the_route_finds_an_orgs_idle_instance_and_another_org_sees_none_of_it() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "cost-recs").await;
        let other = crate::test_support::new_org(st, "cost-recs-other").await;
        let get = |org: &str, q: RecommendationsQuery| {
            let (st, org) = (st.clone(), org.to_string());
            async move { recommendations(State(st), OrgId(org), Query(q)).await.unwrap().0 }
        };
        // Nothing connected: nothing to look at.
        let none = get(&org, RecommendationsQuery::default()).await;
        assert_eq!((none.count, none.accounts.clone()), (0, ConnectedAccounts::default()));

        aws_account(st, &org).await;
        aws_account(st, &other).await;
        // Six days of an instance that does nothing, as the crawler publishes it.
        for (metric, value) in [("aws.ec2.cpuutilization.maximum", 1.5), ("aws.ec2.cpuutilization", 0.5), ("aws.ec2.network_in.sum", 1000.0)] {
            st.ch(&org)
                .query(
                    "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
                     SELECT map('oc.org_id', ?, 'host.name', 'i-0a1b2c3d4e5f60001', 'cloud.provider', 'aws'), ?, \
                            map('aws_account', '123456789012', 'region', 'us-east-1', 'instanceid', 'i-0a1b2c3d4e5f60001', 'Name', 'storefront-api-1', 'env', 'prod'), \
                            now64(9) - toIntervalHour(arrayJoin(range(0, 144, 6))), ?",
                )
                .bind(&org)
                .bind(metric)
                .bind(value)
                .execute()
                .await
                .expect("insert points");
        }
        let got = get(&org, RecommendationsQuery { tags: Some("env:prod".into()), ..Default::default() }).await;
        assert_eq!((got.count, got.accounts.aws), (1, 1), "{got:?}");
        let r = &got.recommendations[0];
        assert_eq!((r.resource_id.as_str(), r.resource_name.as_str(), r.recommendation_type.as_str(), r.cost_source.as_str()), ("i-0a1b2c3d4e5f60001", "storefront-api-1", "idle_compute", "list_price"));
        assert!(r.observed_days >= MIN_OBSERVED_DAYS && r.tags.contains(&"env:prod".to_string()), "{r:?}");
        assert_eq!(get(&org, RecommendationsQuery { tags: Some("env:staging".into()), ..Default::default() }).await.count, 0);

        let theirs = get(&other, RecommendationsQuery::default()).await;
        assert_eq!((theirs.count, theirs.accounts.aws), (0, 1));
    }
}
