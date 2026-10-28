//! Metric query syntax used by dashboard widgets (mirrors `web/src/lib/metricQuery.ts`):
//!
//! ```text
//! <space_agg>:<metric>{<filters>} [by {<tag>,<tag>}]
//! avg:system.cpu.utilization{env:prod,state:user,!host:i-0a*} by {host}
//! ```
//! Filters are ANDed; `*` matches everything; values may contain `*` wildcards. Template variables
//! (`$name`) are resolved by the client; any left unresolved match everything.

use crate::sql::{like_pattern, Param, SqlFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceAgg {
    Avg,
    Sum,
    Min,
    Max,
}

impl SpaceAgg {
    pub fn sql(self) -> &'static str {
        match self {
            SpaceAgg::Avg => "avg",
            SpaceAgg::Sum => "sum",
            SpaceAgg::Min => "min",
            SpaceAgg::Max => "max",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFilter {
    pub key: String,
    pub value: String,
    pub negate: bool,
}

/// How a series' points are combined within an interval (`.rollup(max)`); averaged when unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rollup {
    Avg,
    Sum,
    Min,
    Max,
    Count,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricQuery {
    pub agg: SpaceAgg,
    pub metric: String,
    pub filters: Vec<TagFilter>,
    pub by: Vec<String>,
    pub rollup: Option<Rollup>,
}

/// The query as text, in the syntax [`parse_metric_query`] reads (`avg:m{a:b,!c:d} by {x}.rollup(max)`).
impl std::fmt::Display for MetricQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let filters: Vec<String> = self.filters.iter().map(|t| format!("{}{}:{}", if t.negate { "!" } else { "" }, t.key, t.value)).collect();
        let filters = if filters.is_empty() { "*".to_string() } else { filters.join(",") };
        write!(f, "{}:{}{{{filters}}}", self.agg.sql(), self.metric)?;
        if !self.by.is_empty() {
            write!(f, " by {{{}}}", self.by.join(","))?;
        }
        if let Some(r) = self.rollup {
            let method = match r {
                Rollup::Avg => "avg",
                Rollup::Sum => "sum",
                Rollup::Min => "min",
                Rollup::Max => "max",
                Rollup::Count => "count",
            };
            write!(f, ".rollup({method})")?;
        }
        Ok(())
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
}

pub fn parse_metric_query(q: &str) -> Result<MetricQuery, String> {
    let q = q.trim();
    // `.rollup(method[, seconds])` after the query: the method here, the interval is the caller's.
    let (q, rollup) = match q.rfind(".rollup(") {
        Some(i) if q.ends_with(')') && q[..i].ends_with('}') => {
            let method = q[i + ".rollup(".len()..q.len() - 1].split(',').next().unwrap_or("").trim();
            let rollup = match method {
                "avg" => Rollup::Avg,
                "sum" => Rollup::Sum,
                "min" => Rollup::Min,
                "max" => Rollup::Max,
                "count" => Rollup::Count,
                other => return Err(format!("unknown rollup '{other}'")),
            };
            (&q[..i], Some(rollup))
        }
        _ => (q, None),
    };
    let (agg, rest) = q.split_once(':').ok_or("expected <agg>:<metric>{…}")?;
    let agg = match agg {
        "avg" => SpaceAgg::Avg,
        "sum" => SpaceAgg::Sum,
        "min" => SpaceAgg::Min,
        "max" => SpaceAgg::Max,
        other => return Err(format!("unknown aggregator '{other}'")),
    };
    let open = rest.find('{').ok_or("expected '{' after the metric name")?;
    let metric = &rest[..open];
    if !valid_name(metric) {
        return Err(format!("invalid metric name '{metric}'"));
    }
    let close = rest[open..].find('}').map(|i| open + i).ok_or("unclosed '{'")?;
    let mut filters = Vec::new();
    for raw in rest[open + 1..close].split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if raw == "*" || raw.starts_with('$') {
            continue;
        }
        let (negate, body) = match raw.strip_prefix('!') {
            Some(b) => (true, b),
            None => (false, raw),
        };
        let (key, value) = body.split_once(':').ok_or_else(|| format!("filter '{raw}' must be tag:value"))?;
        if !valid_name(key) || value.is_empty() {
            return Err(format!("invalid filter '{raw}'"));
        }
        filters.push(TagFilter { key: key.to_string(), value: value.to_string(), negate });
    }
    let tail = rest[close + 1..].trim();
    let mut by = Vec::new();
    if !tail.is_empty() {
        let inner = tail
            .strip_prefix("by")
            .map(str::trim)
            .and_then(|t| t.strip_prefix('{'))
            .and_then(|t| t.strip_suffix('}'))
            .ok_or("expected 'by {tag,…}' after the filters")?;
        for k in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !valid_name(k) {
                return Err(format!("invalid group-by tag '{k}'"));
            }
            by.push(k.to_string());
        }
    }
    Ok(MetricQuery { agg, metric: metric.to_string(), filters, by, rollup })
}

/// Short tag names → OTel resource attribute keys (both spellings are accepted).
pub fn resource_alias(key: &str) -> Option<&'static str> {
    Some(match key {
        "host" => "host.name",
        "env" => "deployment.environment",
        "version" => "service.version",
        "region" => "cloud.region",
        "zone" | "availability_zone" => "cloud.availability_zone",
        "cloud" => "cloud.provider",
        "account" => "cloud.account.id",
        "instance_type" => "host.type",
        "os" => "os.type",
        "cluster" | "kube_cluster_name" => "k8s.cluster.name",
        "kube_namespace" => "k8s.namespace.name",
        "pod_name" => "k8s.pod.name",
        "kube_deployment" => "k8s.deployment.name",
        "kube_daemon_set" => "k8s.daemonset.name",
        "kube_stateful_set" => "k8s.statefulset.name",
        "kube_replica_set" => "k8s.replicaset.name",
        "kube_job" => "k8s.job.name",
        "kube_cronjob" => "k8s.cronjob.name",
        "kube_node" => "k8s.node.name",
        "kube_container_name" => "k8s.container.name",
        "ecs_cluster" | "ecs_cluster_name" => "aws.ecs.cluster.name",
        "task_family" => "aws.ecs.task.family",
        "task_version" => "aws.ecs.task.revision",
        "task_arn" => "aws.ecs.task.arn",
        "task_launch_type" => "aws.ecs.launch_type",
        "container_name" => "container.name",
        "container_id" => "container.id",
        "image_name" => "container.image.name",
        "image_tag" => "container.image.tag",
        "process" => "process.executable.name",
        "collector" => "observecraft.collector",
        _ => return None,
    })
}

/// The short name shown for a resource attribute key, if it has one.
pub fn short_tag(resource_key: &str) -> Option<&'static str> {
    Some(match resource_key {
        "host.name" => "host",
        "deployment.environment" => "env",
        "service.version" => "version",
        "cloud.region" => "region",
        "cloud.availability_zone" => "zone",
        "cloud.provider" => "cloud",
        "cloud.account.id" => "account",
        "host.type" => "instance_type",
        "os.type" => "os",
        "k8s.cluster.name" => "cluster",
        "k8s.namespace.name" => "kube_namespace",
        "k8s.pod.name" => "pod_name",
        "k8s.deployment.name" => "kube_deployment",
        "k8s.daemonset.name" => "kube_daemon_set",
        "k8s.statefulset.name" => "kube_stateful_set",
        "k8s.replicaset.name" => "kube_replica_set",
        "k8s.job.name" => "kube_job",
        "k8s.cronjob.name" => "kube_cronjob",
        "k8s.node.name" => "kube_node",
        "k8s.container.name" => "kube_container_name",
        "aws.ecs.cluster.name" => "ecs_cluster",
        "aws.ecs.task.family" => "task_family",
        "aws.ecs.task.revision" => "task_version",
        "aws.ecs.launch_type" => "task_launch_type",
        "container.name" => "container_name",
        "container.id" => "container_id",
        "container.image.name" => "image_name",
        "container.image.tag" => "image_tag",
        "process.executable.name" => "process",
        "observecraft.collector" => "collector",
        _ => return None,
    })
}

/// SQL expression for `key` on the OTel metric tables, with its bound params (in text order).
/// Data-point attributes win over resource attributes of the same name.
pub fn metric_field(key: &str) -> (String, Vec<Param>) {
    match key {
        "service" => ("ServiceName".into(), vec![]),
        k => match resource_alias(k) {
            Some(r) => ("ResourceAttributes[?]".into(), vec![Param::Str(r.into())]),
            None => ("if(mapContains(Attributes, ?), Attributes[?], ResourceAttributes[?])".into(), vec![Param::Str(k.into()); 3]),
        },
    }
}

/// SQL expression for `key` on `otel.otel_traces` (span-derived `trace.*` metrics).
pub fn span_field(key: &str) -> (String, Vec<Param>) {
    match key {
        "service" => ("ServiceName".into(), vec![]),
        "resource" | "operation" => ("SpanName".into(), vec![]),
        k => match resource_alias(k) {
            Some(r) => ("ResourceAttributes[?]".into(), vec![Param::Str(r.into())]),
            None => ("if(mapContains(SpanAttributes, ?), SpanAttributes[?], ResourceAttributes[?])".into(), vec![Param::Str(k.into()); 3]),
        },
    }
}

/// ANDed filter clause for `filters`, using `field` to map keys onto columns.
pub fn metric_filter(filters: &[TagFilter], field: fn(&str) -> (String, Vec<Param>)) -> SqlFilter {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for f in filters {
        let (expr, key_params) = field(&f.key);
        params.extend(key_params);
        let cmp = if f.value.chars().all(|c| c == '*') {
            format!("notEmpty({expr})")
        } else if f.value.contains('*') {
            params.push(Param::Str(like_pattern(&f.value)));
            format!("{expr} ILIKE ?")
        } else {
            params.push(Param::Str(f.value.clone()));
            format!("{expr} = ?")
        };
        parts.push(if f.negate { format!("NOT ({cmp})") } else { format!("({cmp})") });
    }
    SqlFilter { clause: if parts.is_empty() { "1".into() } else { parts.join(" AND ") }, params }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_query() {
        let q = parse_metric_query("avg:system.cpu.utilization{env:prod, state:user,!host:i-0a*,$team} by {host,region}").unwrap();
        assert_eq!(q.agg, SpaceAgg::Avg);
        assert_eq!(q.metric, "system.cpu.utilization");
        assert_eq!(q.filters.len(), 3);
        assert!(q.filters[2].negate);
        assert_eq!(q.by, vec!["host", "region"]);
    }

    #[test]
    fn a_rollup_method_after_the_query() {
        let q = parse_metric_query("sum:kubernetes.containers.last_state.terminated{reason:oomkilled} by {pod_name}.rollup(max)").unwrap();
        assert_eq!((q.metric.as_str(), q.by.clone(), q.rollup), ("kubernetes.containers.last_state.terminated", vec!["pod_name".to_string()], Some(Rollup::Max)));
        // The interval argument is the caller's (it sets the points); avg is the default.
        assert_eq!(parse_metric_query("avg:a{*}.rollup(sum, 60)").unwrap().rollup, Some(Rollup::Sum));
        assert_eq!(parse_metric_query("avg:a{*}").unwrap().rollup, None);
        assert!(parse_metric_query("avg:a{*}.rollup(median)").is_err());
    }

    #[test]
    fn a_query_formats_back_to_text_that_parses_the_same() {
        for text in ["avg:system.cpu.utilization{env:prod,!host:i-0a*} by {host,region}", "sum:trace.requests{*}", "max:k.terminated{reason:oomkilled} by {pod_name}.rollup(max)"] {
            let q = parse_metric_query(text).unwrap();
            assert_eq!(q.to_string(), text);
            assert_eq!(parse_metric_query(&q.to_string()).unwrap(), q);
        }
    }

    #[test]
    fn star_and_no_group() {
        let q = parse_metric_query("sum:trace.requests{*}").unwrap();
        assert!(q.filters.is_empty() && q.by.is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_metric_query("median:x{*}").is_err());
        assert!(parse_metric_query("avg:x'y{*}").is_err());
        assert!(parse_metric_query("avg:x{a:b} group {c}").is_err());
        assert!(parse_metric_query("avg:x{a:b} by {c'}").is_err());
    }

    #[test]
    fn filters_bind_keys_and_values() {
        let q = parse_metric_query("avg:m{host:a,state:user,!env:qa*}").unwrap();
        let f = metric_filter(&q.filters, metric_field);
        assert_eq!(
            f.clause,
            "(ResourceAttributes[?] = ?) AND (if(mapContains(Attributes, ?), Attributes[?], ResourceAttributes[?]) = ?) AND NOT (ResourceAttributes[?] ILIKE ?)"
        );
        assert_eq!(f.params.len(), 8);
        assert_eq!(f.params[7], Param::Str("qa%".into()));
    }

    #[test]
    fn wildcard_tag_values_require_a_present_value() {
        for value in ["*", "**"] {
            let q = parse_metric_query(&format!("sum:m{{workload:{value},!env:*}}")).unwrap();
            let f = metric_filter(&q.filters, metric_field);
            assert_eq!(f.clause, "(notEmpty(if(mapContains(Attributes, ?), Attributes[?], ResourceAttributes[?]))) AND NOT (notEmpty(ResourceAttributes[?]))");
            assert_eq!(
                f.params,
                vec![
                    Param::Str("workload".into()),
                    Param::Str("workload".into()),
                    Param::Str("workload".into()),
                    Param::Str("deployment.environment".into())
                ]
            );
        }
    }
}
