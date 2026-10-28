//! Triage Inbox correlation with the org's settings (Event Management → Settings):
//! - **Correlators** group the warning and error events their query matches into one work item per
//!   distinct group-by values, as long as the events keep arriving within the correlator's time
//!   window of each other (a longer gap starts a new work item). The first matching correlator, in
//!   order, takes an event; the others keep the default grouping (one work item per affected
//!   object: the Agent's aggregation key, else source and title).
//! - **Maintenance windows** mark the events their query matches during the window; a work item
//!   made only of marked events is suppressed.
//!
//! Queries use the event search syntax (see `events`); [`matches`] evaluates it in memory with the
//! same semantics as the SQL conditions.

use std::collections::HashMap;

/// A warning or error event, as the inbox reads it.
#[derive(Clone, Debug, Default)]
pub(crate) struct RawEvent {
    /// Epoch milliseconds.
    pub ts: i64,
    pub title: String,
    pub text: String,
    /// error | warning (the alert type).
    pub status: String,
    pub priority: String,
    pub source: String,
    pub host: String,
    pub aggregation_key: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Correlator {
    pub id: String,
    pub name: String,
    pub query: String,
    pub group_by: Vec<String>,
    pub window_ms: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct MaintenanceWindow {
    pub name: String,
    pub query: String,
    pub from: i64,
    pub to: i64,
}

/// Whether the event matches the search (free text, `source:`, `host:`, `status:`, `priority:`,
/// `key:value` tags, each optionally negated with `-`), as `events::conditions` does in SQL.
pub(crate) fn matches(query: &str, e: &RawEvent) -> bool {
    let haystack = || format!("{} {}", e.title, e.text).to_lowercase();
    let mut text = Vec::new();
    for raw in query.split_whitespace() {
        let (neg, tok) = raw.strip_prefix('-').map_or((false, raw), |t| (true, t));
        let hit = match tok.split_once(':') {
            Some(("source", v)) => e.source == v,
            Some(("host", v)) => e.host == v,
            Some(("status" | "alert_type", v)) => e.status == v,
            Some(("priority", v)) => e.priority == v,
            Some((k, v)) if !k.is_empty() && !v.is_empty() => e.tags.iter().any(|t| t == tok),
            _ if neg => haystack().contains(&tok.to_lowercase()),
            _ => {
                text.push(tok);
                continue;
            }
        };
        if hit == neg {
            return false;
        }
    }
    text.is_empty() || haystack().contains(&text.join(" ").to_lowercase())
}

/// The event's value for a group-by key: `source`, `host`, `status`, `title`, or a tag's value.
pub(crate) fn group_value(e: &RawEvent, key: &str) -> String {
    match key {
        "source" => e.source.clone(),
        "host" => e.host.clone(),
        "status" => e.status.clone(),
        "title" => e.title.clone(),
        k => e.tags.iter().find_map(|t| t.strip_prefix(k).and_then(|r| r.strip_prefix(':'))).unwrap_or("").to_string(),
    }
}

/// The default grouping: the affected object (aggregation key), else source and title.
pub(crate) fn default_key(e: &RawEvent) -> String {
    if e.aggregation_key.is_empty() {
        format!("{}:{}", e.source, e.title)
    } else {
        e.aggregation_key.clone()
    }
}

/// Correlated events: indices into the events, oldest first.
#[derive(Debug, PartialEq)]
pub(crate) struct Correlated {
    /// Stable identity of the group (hashed into the work item id).
    pub key: String,
    /// The correlator that grouped them (None: the default grouping).
    pub correlator: Option<usize>,
    /// The correlator's group-by values.
    pub values: Vec<String>,
    pub events: Vec<usize>,
    /// The maintenance window covering every event, when one does.
    pub maintenance: Option<String>,
}

/// Groups `events` (any order) into work items.
pub(crate) fn correlate(events: &[RawEvent], correlators: &[Correlator], windows: &[MaintenanceWindow]) -> Vec<Correlated> {
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| events[i].ts);
    let mut out: Vec<Correlated> = Vec::new();
    // Group base key → index of its latest work item in `out`.
    let mut open: HashMap<String, usize> = HashMap::new();
    let mut in_maintenance: Vec<Option<String>> = Vec::new();
    for i in order {
        let e = &events[i];
        let window = windows.iter().find(|w| e.ts >= w.from && e.ts <= w.to && matches(&w.query, e)).map(|w| w.name.clone());
        let (base, correlator, values) = match correlators.iter().position(|c| matches(&c.query, e)) {
            Some(ci) => {
                let c = &correlators[ci];
                let values: Vec<String> = c.group_by.iter().map(|k| group_value(e, k)).collect();
                (format!("correlator:{}:{}", c.id, values.join("\u{1f}")), Some(ci), values)
            }
            None => (default_key(e), None, Vec::new()),
        };
        let current = open.get(&base).copied().filter(|&wi| match correlator {
            // A correlator's work item takes events within its window of the last one.
            Some(ci) => out[wi].events.last().is_some_and(|&last| e.ts - events[last].ts <= correlators[ci].window_ms),
            None => true,
        });
        let wi = match current {
            Some(wi) => wi,
            None => {
                // A new session of a correlator's group is its own work item.
                let key = if correlator.is_some() && open.contains_key(&base) { format!("{base}:{}", e.ts) } else { base.clone() };
                out.push(Correlated { key, correlator, values, events: Vec::new(), maintenance: None });
                in_maintenance.push(window.clone());
                open.insert(base, out.len() - 1);
                out.len() - 1
            }
        };
        out[wi].events.push(i);
        if window.is_none() {
            in_maintenance[wi] = None;
        } else if in_maintenance[wi].is_some() {
            in_maintenance[wi] = window;
        }
    }
    for (c, m) in out.iter_mut().zip(in_maintenance) {
        c.maintenance = m;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: i64, source: &str, title: &str, tags: &[&str]) -> RawEvent {
        RawEvent {
            ts,
            title: title.into(),
            text: "Back-off restarting failed container".into(),
            status: "warning".into(),
            priority: "normal".into(),
            source: source.into(),
            host: "i-0a1b2c3d4e5f60001".into(),
            aggregation_key: String::new(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn the_in_memory_matcher_follows_the_search_syntax() {
        let e = ev(0, "kubernetes", "Pod checkout-7d9 restarted", &["kube_namespace:shop", "env:prod"]);
        assert!(matches("", &e));
        assert!(matches("source:kubernetes kube_namespace:shop", &e));
        assert!(matches("-env:staging status:warning", &e));
        assert!(!matches("source:Kubernetes", &e), "exact values, like the SQL");
        assert!(!matches("-kube_namespace:shop", &e));
        assert!(matches("CHECKOUT-7d9 restarted", &e), "free text ignores case");
        assert!(matches("back-off", &e), "free text searches the text too");
        assert!(!matches("-restarted", &e));
        assert!(!matches("host:i-0a1b2c3d4e5f60002", &e));
    }

    #[test]
    fn without_correlators_events_group_per_affected_object() {
        let mut a = ev(0, "kubernetes", "Pod a", &[]);
        a.aggregation_key = "pod-a".into();
        let mut a2 = ev(MIN * 90, "kubernetes", "Pod a again", &[]);
        a2.aggregation_key = "pod-a".into();
        let b = ev(MIN, "nagios", "Disk full", &[]);
        let out = correlate(&[a, b, a2], &[], &[]);
        assert_eq!(out.iter().map(|c| (c.key.as_str(), c.events.clone())).collect::<Vec<_>>(), vec![("pod-a", vec![0, 2]), ("nagios:Disk full", vec![1])]);
    }

    #[test]
    fn a_correlator_groups_by_its_keys_within_its_window() {
        let c = Correlator { id: "c1".into(), name: "Shop pods".into(), query: "source:kubernetes".into(), group_by: vec!["kube_namespace".into()], window_ms: 10 * MIN };
        let events = vec![
            ev(0, "kubernetes", "Pod a", &["kube_namespace:shop"]),
            ev(5 * MIN, "kubernetes", "Pod b", &["kube_namespace:shop"]),
            ev(6 * MIN, "kubernetes", "Pod c", &["kube_namespace:billing"]),
            // 25 minutes after the last shop event: a new work item.
            ev(30 * MIN, "kubernetes", "Pod a", &["kube_namespace:shop"]),
            ev(31 * MIN, "nagios", "Disk full", &[]),
        ];
        let out = correlate(&events, &[c], &[]);
        let groups: Vec<(Option<usize>, Vec<String>, Vec<usize>)> = out.iter().map(|g| (g.correlator, g.values.clone(), g.events.clone())).collect();
        assert_eq!(
            groups,
            vec![(Some(0), vec!["shop".to_string()], vec![0, 1]), (Some(0), vec!["billing".to_string()], vec![2]), (Some(0), vec!["shop".to_string()], vec![3]), (None, vec![], vec![4])]
        );
        assert_ne!(out[0].key, out[2].key, "each session has its own identity");
        assert_eq!(out[2].key, format!("{}:{}", out[0].key, 30 * MIN));
    }

    #[test]
    fn the_first_matching_correlator_takes_the_event() {
        let first = Correlator { id: "a".into(), name: "Prod".into(), query: "env:prod".into(), group_by: vec![], window_ms: 60 * MIN };
        let second = Correlator { id: "b".into(), name: "Kubernetes".into(), query: "source:kubernetes".into(), group_by: vec!["source".into()], window_ms: 60 * MIN };
        let out = correlate(&[ev(0, "kubernetes", "x", &["env:prod"]), ev(MIN, "kubernetes", "y", &["env:qa"])], &[first, second], &[]);
        assert_eq!(out.iter().map(|c| c.correlator).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
        assert_eq!(out[1].values, vec!["kubernetes"]);
    }

    #[test]
    fn maintenance_windows_mark_work_items_whose_events_are_all_in_scope() {
        let w = MaintenanceWindow { name: "Shop upgrade".into(), query: "kube_namespace:shop".into(), from: 0, to: 10 * MIN };
        let mut a = ev(MIN, "kubernetes", "Pod a", &["kube_namespace:shop"]);
        a.aggregation_key = "pod-a".into();
        let mut a_late = ev(20 * MIN, "kubernetes", "Pod a", &["kube_namespace:shop"]);
        a_late.aggregation_key = "pod-a".into();
        let mut b = ev(2 * MIN, "kubernetes", "Pod b", &["kube_namespace:shop"]);
        b.aggregation_key = "pod-b".into();
        let other = ev(3 * MIN, "kubernetes", "Pod c", &["kube_namespace:billing"]);
        let out = correlate(&[a, a_late, b, other], &[], &[w]);
        let marks: Vec<(&str, Option<&str>)> = out.iter().map(|c| (c.key.as_str(), c.maintenance.as_deref())).collect();
        assert_eq!(marks, vec![("pod-a", None), ("pod-b", Some("Shop upgrade")), ("kubernetes:Pod c", None)], "pod-a kept reporting after the window");
    }
}
