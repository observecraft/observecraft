//! Ingestion controls (Metrics → Settings → Rules, Postgres `metric_rules`): drop matching metrics,
//! or strip tags from them, before the Datadog series, sketches and service checks are converted
//! and stored. OTLP metrics are forwarded unchanged.
//!
//! Rules are loaded per org and reloaded every [`TTL`], so a change in the UI applies within
//! seconds; a failed load keeps the cached rules (an org not loaded yet then has none).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oc_query::glob;
use sqlx::PgPool;

use crate::datadog::{pb, Series};

const TTL: Duration = Duration::from_secs(30);
/// After a failed load, how long until another request tries again.
const RETRY: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Drop,
    /// Tag keys to remove (`host` clears the series' host).
    Strip(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// Metric names, `*` for any characters.
    pub pattern: String,
    pub action: Action,
}

pub type Rules = Arc<Vec<Rule>>;

/// Per-org ingestion controls.
#[derive(Default)]
pub struct MetricRules {
    orgs: Mutex<HashMap<String, (Rules, Instant)>>,
}

impl MetricRules {
    /// The org's enabled ingestion controls (cached).
    pub async fn rules(&self, pg: &PgPool, org: &str) -> Rules {
        let cached = match self.orgs.lock().unwrap().get(org) {
            Some((r, refresh_at)) if *refresh_at > Instant::now() => return r.clone(),
            Some((r, _)) => r.clone(),
            None => Rules::default(),
        };
        let loaded = sqlx::query_as::<_, (String, String, Vec<String>)>("SELECT metric_pattern, action, tags FROM metric_rules WHERE org_id = $1 AND kind = 'ingestion' AND enabled")
            .bind(org)
            .fetch_all(pg)
            .await;
        let (rules, ttl) = match loaded {
            Ok(rows) => (Arc::new(rows.into_iter().map(|(pattern, action, tags)| Rule { pattern, action: if action == "drop" { Action::Drop } else { Action::Strip(tags) } }).collect()), TTL),
            Err(e) => {
                tracing::warn!(error = %e, org, "loading metric ingestion rules failed");
                (cached, RETRY)
            }
        };
        self.orgs.lock().unwrap().insert(org.to_string(), (rules.clone(), Instant::now() + ttl));
        rules
    }
}

/// Whether `metric` is dropped, and the tag keys stripped from it.
fn decide<'a>(metric: &str, rules: &'a [Rule]) -> (bool, Vec<&'a str>) {
    let mut strip = Vec::new();
    for r in rules.iter().filter(|r| glob(&r.pattern, metric)) {
        match &r.action {
            Action::Drop => return (true, Vec::new()),
            Action::Strip(keys) => strip.extend(keys.iter().map(String::as_str)),
        }
    }
    (false, strip)
}

fn strip_tags(tags: &mut Vec<String>, keys: &[&str]) {
    tags.retain(|t| !keys.contains(&t.split_once(':').map_or(t.as_str(), |(k, _)| k)));
}

/// Applies the rules to series; returns how many were dropped.
pub fn apply(series: &mut Vec<Series>, rules: &[Rule]) -> usize {
    if rules.is_empty() {
        return 0;
    }
    let before = series.len();
    series.retain_mut(|s| {
        let (drop, strip) = decide(&s.metric, rules);
        if !strip.is_empty() {
            strip_tags(&mut s.tags, &strip);
            if strip.contains(&"host") {
                s.host.clear();
            }
        }
        !drop
    });
    before - series.len()
}

/// Applies the rules to distribution sketches.
pub fn apply_sketches(sketches: &mut Vec<pb::Sketch>, rules: &[Rule]) {
    if rules.is_empty() {
        return;
    }
    sketches.retain_mut(|s| {
        let (drop, strip) = decide(&s.metric, rules);
        if !strip.is_empty() {
            strip_tags(&mut s.tags, &strip);
            if strip.contains(&"host") {
                s.host.clear();
            }
        }
        !drop
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datadog::Kind;

    fn series(metric: &str, tags: &[&str]) -> Series {
        Series {
            metric: metric.into(),
            kind: Kind::Gauge,
            unit: String::new(),
            host: "i-0a1b2c3d4e5f60001".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            interval: 0,
            points: vec![(1, 1.0)],
        }
    }

    fn strip(pattern: &str, keys: &[&str]) -> Rule {
        Rule { pattern: pattern.into(), action: Action::Strip(keys.iter().map(|k| k.to_string()).collect()) }
    }

    #[test]
    fn drop_rules_remove_matching_metrics() {
        let rules = vec![Rule { pattern: "debug.*".into(), action: Action::Drop }];
        let mut batch = vec![series("debug.queue.depth", &[]), series("system.load.1", &[]), series("DEBUG.x", &[])];
        assert_eq!(apply(&mut batch, &rules), 2, "patterns ignore case");
        assert_eq!(batch.iter().map(|s| s.metric.as_str()).collect::<Vec<_>>(), vec!["system.load.1"]);
    }

    #[test]
    fn strip_rules_remove_tags_by_key() {
        let rules = vec![strip("kubernetes.*", &["pod_name", "container_id"]), strip("kubernetes.cpu.*", &["host"])];
        let mut batch = vec![series("kubernetes.cpu.usage.total", &["pod_name:checkout-7d9", "kube_namespace:shop", "container_id:abc", "pod_name"]), series("system.load.1", &["pod_name:x"])];
        assert_eq!(apply(&mut batch, &rules), 0);
        assert_eq!(batch[0].tags, vec!["kube_namespace:shop"]);
        assert_eq!(batch[0].host, "", "stripping host clears the series' host");
        assert_eq!(batch[1].tags, vec!["pod_name:x"], "other metrics keep their tags");
        assert_eq!(batch[1].host, "i-0a1b2c3d4e5f60001");
    }

    #[test]
    fn a_drop_wins_over_stripping_and_no_rules_change_nothing() {
        let rules = vec![strip("a.*", &["x"]), Rule { pattern: "a.b".into(), action: Action::Drop }];
        let mut batch = vec![series("a.b", &["x:1"]), series("a.c", &["x:1", "y:2"])];
        apply(&mut batch, &rules);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].tags, vec!["y:2"]);
        let mut untouched = vec![series("a.b", &["x:1"])];
        assert_eq!(apply(&mut untouched, &[]), 0);
        assert_eq!(untouched[0].tags, vec!["x:1"]);
    }

    #[test]
    fn sketches_follow_the_same_rules() {
        let rules = vec![Rule { pattern: "debug.*".into(), action: Action::Drop }, strip("http.*", &["url"])];
        let sketch = |m: &str, tags: &[&str]| pb::Sketch { metric: m.into(), host: "h".into(), tags: tags.iter().map(|t| t.to_string()).collect(), dogsketches: vec![] };
        let mut batch = vec![sketch("debug.latency", &[]), sketch("http.latency", &["url:/cart", "env:prod"])];
        apply_sketches(&mut batch, &rules);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].tags, vec!["env:prod"]);
    }
}
