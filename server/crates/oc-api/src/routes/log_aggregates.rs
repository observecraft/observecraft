//! Log aggregates maintained at ingest (ClickHouse migration 0012): error groups and the standard
//! facets' counts per minute.
//!
//! A read uses an aggregate for the whole minutes of its range from the org's first complete minute
//! in it on, and reads the logs for the rest (the partial minutes at the range's edges, logs older
//! than the aggregate), so its results are the same as a read of the logs alone. When the search or
//! the reader's restriction filters on something an aggregate doesn't keep (free text, other tags
//! and attributes, rehydrated logs), the read uses the logs only.

use oc_query::{build_any, build_where, Param, Schema, Term};

use crate::error::ApiError;
use crate::log_access::LogAccess;
use crate::routes::logs::{bind, scope, Scope, TimeRange};

const MINUTE_MS: i64 = 60_000;

/// A per-minute aggregate of `oc.logs`: its table and the search keys it keeps, with their columns.
pub(crate) struct Aggregate {
    pub table: &'static str,
    columns: &'static [(&'static str, &'static str)],
}

/// Error Tracking's error logs per issue key: the logs that are issues (`error_tracking::TRACKED_LOG_SQL`).
/// Every row is an error log, so `status` is constant.
pub(crate) const ERROR_GROUPS: Aggregate = Aggregate {
    table: "oc.error_issues_1m_read",
    columns: &[
        ("service", "service"),
        ("status", "'error'"),
        ("host", "host"),
        ("source", "source"),
        ("index", "index_name"),
        ("env", "env"),
        ("version", "version"),
        ("kube_namespace", "kube_namespace"),
    ],
};

/// Log counts of the standard facets (Log Explorer).
pub(crate) const LOG_FACETS: Aggregate = Aggregate {
    table: "oc.log_facets_1m_read",
    columns: &[
        ("service", "service"),
        ("status", "status"),
        ("host", "host"),
        ("source", "source"),
        ("index", "index_name"),
        ("env", "env"),
        ("kube_cluster_name", "kube_cluster_name"),
        ("region", "region"),
        ("kube_namespace", "kube_namespace"),
        ("kube_deployment", "kube_deployment"),
        ("container_name", "container_name"),
    ],
};

impl Schema for Aggregate {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        // Only called for keys the aggregate covers (see `covers`); anything else matches nothing.
        (self.column(key).unwrap_or("''").to_string(), None)
    }
    fn text_expr(&self) -> &'static str {
        "''"
    }
}

impl Aggregate {
    /// The column of a search key, if the aggregate keeps it.
    pub fn column(&self, key: &str) -> Option<&'static str> {
        self.columns.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
    }

    /// Whether the terms (but `skip_key`'s) filter only on keys the aggregate keeps.
    pub fn covers(&self, terms: &[Term], skip_key: Option<&str>) -> bool {
        terms.iter().all(|t| match t {
            Term::Text { .. } => false,
            Term::Tag { key, values, .. } => Some(key.as_str()) == skip_key || values.is_empty() || self.column(key).is_some(),
        })
    }
}

/// The whole minutes `[from, to)` (epoch ms) of `range` an aggregate answers for, given the start of
/// the org's first complete minute in it (`since`).
pub(crate) fn window(range: TimeRange, since: Option<i64>) -> Option<(i64, i64)> {
    let since = since?;
    let from = (range.from + MINUTE_MS - 1).div_euclid(MINUTE_MS) * MINUTE_MS;
    let from = from.max(since);
    let to = (range.to + 1).div_euclid(MINUTE_MS) * MINUTE_MS;
    (from < to).then_some((from, to))
}

/// How a read of logs in a range splits between an aggregate and the logs.
pub(crate) struct Plan {
    /// The aggregate's rows for the window, when it can answer the search and the restriction.
    pub agg: Option<Scope>,
    /// The logs the aggregate doesn't answer for (all of them without `agg`).
    pub logs: Scope,
}

/// Plans a read of the logs matching `terms` (but `skip_key`'s) in `range`, as `access` may read them,
/// that counts by the search keys `keys`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(org: &str, access: &LogAccess, agg: &Aggregate, range: TimeRange, terms: &[Term], skip_key: Option<&str>, since: Option<i64>, keys: &[&str]) -> Plan {
    let mut logs = scope(org, access, range, terms, skip_key);
    let restriction_covered = access.searches().is_none_or(|searches| searches.iter().all(|s| agg.covers(s, None)));
    let usable = logs.table == "logs_read" && agg.covers(terms, skip_key) && restriction_covered && keys.iter().all(|k| agg.column(k).is_some());
    let Some((from, to)) = window(range, since).filter(|_| usable) else {
        return Plan { agg: None, logs };
    };
    let f = build_where(agg, terms, skip_key);
    let mut params = vec![Param::Str(org.to_string()), Param::Int(from / 1000), Param::Int(to / 1000)];
    params.extend(f.params);
    let mut clause = format!("org_id = ? AND minute >= toDateTime(?) AND minute < toDateTime(?) AND ({})", f.clause);
    if let Some(searches) = access.searches() {
        let r = build_any(agg, searches);
        clause.push_str(&format!(" AND ({})", r.clause));
        params.extend(r.params);
    }
    logs.clause.push_str(" AND (timestamp < fromUnixTimestamp64Milli(?) OR timestamp >= fromUnixTimestamp64Milli(?))");
    logs.params.extend([Param::Int(from), Param::Int(to)]);
    Plan { agg: Some(Scope { clause, params, table: agg.table }), logs }
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct MinuteRow {
    t: i64,
}

/// The start of the org's first complete minute in the aggregate (its first minute may have begun
/// before the aggregate existed), or `None` while it has none.
/// `ch` is the org's client (`AppState::ch`).
pub(crate) async fn since(ch: &clickhouse::Client, org: &str, agg: &Aggregate) -> Result<Option<i64>, ApiError> {
    let first = bind(ch.query(&format!("SELECT toInt64(toUnixTimestamp(minute)) * 1000 AS t FROM {} WHERE org_id = ? ORDER BY minute LIMIT 1", agg.table)), &[Param::Str(org.to_string())])
        .fetch_optional::<MinuteRow>()
        .await?;
    Ok(first.map(|r| r.t + MINUTE_MS))
}

/// A read of the logs cut to the newest part of its range, when the whole range has too many logs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bound {
    /// The range read.
    pub range: TimeRange,
    /// The whole range's logs per log read (1 when the range is read whole).
    pub factor: f64,
}

impl Bound {
    pub fn approximate(&self) -> bool {
        self.factor != 1.0
    }

    /// A count of the logs read, scaled to the whole range.
    pub fn scale(&self, n: u64) -> u64 {
        (n as f64 * self.factor).round() as u64
    }
}

/// The newest minutes of `range` holding at most about `max_rows` of the org's logs, from the org's
/// logs per minute (`minutes`, any order) and where the aggregate starts (`since`). The whole range
/// when it holds fewer, or when logs before the aggregate may make it more.
pub(crate) fn bound(range: TimeRange, since: Option<i64>, minutes: &[(i64, u64)], max_rows: u64) -> Bound {
    let whole = Bound { range, factor: 1.0 };
    let Some(since) = since else { return whole };
    let mut newest_first: Vec<(i64, u64)> = minutes.iter().copied().filter(|(t, _)| *t >= since && *t <= range.to).collect();
    newest_first.sort_by(|a, b| b.0.cmp(&a.0));
    let total: u64 = newest_first.iter().map(|(_, n)| n).sum();
    let mut read = 0u64;
    for (t, n) in newest_first {
        read += n;
        if read >= max_rows && t > range.from {
            return Bound { range: TimeRange { from: t, to: range.to }, factor: total as f64 / read as f64 };
        }
    }
    whole
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct MinuteCount {
    t: i64,
    n: u64,
}

/// [`bound`] for the org's logs in `range` (`ch`: the org's client).
pub(crate) async fn bound_logs(ch: &clickhouse::Client, org: &str, range: TimeRange, since: Option<i64>, max_rows: u64) -> Result<Bound, ApiError> {
    if since.is_none() {
        return Ok(Bound { range, factor: 1.0 });
    }
    let rows = bind(
        ch.query(&format!(
            "SELECT toInt64(toUnixTimestamp(minute)) * 1000 AS t, sum(n) AS n FROM {} WHERE org_id = ? AND minute >= toStartOfMinute(fromUnixTimestamp64Milli(?)) \
             AND minute <= fromUnixTimestamp64Milli(?) GROUP BY t",
            LOG_FACETS.table
        )),
        &[Param::Str(org.to_string()), Param::Int(range.from), Param::Int(range.to)],
    )
    .fetch_all::<MinuteCount>()
    .await?;
    let minutes: Vec<(i64, u64)> = rows.into_iter().map(|r| (r.t, r.n)).collect();
    Ok(bound(range, since, &minutes, max_rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_query::parse_query;

    const T0: i64 = 1_793_095_200_000; // 2026-10-27 10:00:00 UTC

    #[test]
    fn the_window_is_the_whole_minutes_from_the_aggregates_start() {
        let range = |from: i64, to: i64| TimeRange { from, to };
        // Partial minutes at both ends are left to the logs; `to` is inclusive.
        assert_eq!(window(range(T0 + 5_000, T0 + 10 * MINUTE_MS + 30_000), Some(T0)), Some((T0 + MINUTE_MS, T0 + 10 * MINUTE_MS)));
        assert_eq!(window(range(T0, T0 + 10 * MINUTE_MS - 1), Some(T0)), Some((T0, T0 + 10 * MINUTE_MS)));
        // Logs before the aggregate's first complete minute come from the logs.
        assert_eq!(window(range(T0, T0 + 10 * MINUTE_MS), Some(T0 + 3 * MINUTE_MS)), Some((T0 + 3 * MINUTE_MS, T0 + 10 * MINUTE_MS)));
        // No whole minute, or no aggregate yet: logs only.
        assert_eq!(window(range(T0 + 5_000, T0 + 50_000), Some(T0)), None);
        assert_eq!(window(range(T0, T0 + 10 * MINUTE_MS), None), None);
        assert_eq!(window(range(T0, T0 + 10 * MINUTE_MS), Some(T0 + 20 * MINUTE_MS)), None);
    }

    #[test]
    fn an_aggregate_covers_searches_on_the_keys_it_keeps() {
        assert!(LOG_FACETS.covers(&parse_query("service:(billing OR order-*) -env:qa status:error index:main"), None));
        assert!(LOG_FACETS.covers(&[], None));
        // Free text, other tags and attributes need the logs.
        assert!(!LOG_FACETS.covers(&parse_query("service:billing timeout"), None));
        assert!(!LOG_FACETS.covers(&parse_query("pod_name:billing-7d9"), None));
        assert!(!LOG_FACETS.covers(&parse_query("@http.status_code:500"), None));
        // A facet's own filter is dropped.
        assert!(LOG_FACETS.covers(&parse_query("service:billing pod_name:billing-7d9"), Some("pod_name")));
        assert!(ERROR_GROUPS.covers(&parse_query("version:1.4.2 kube_namespace:payments"), None));
        assert!(!ERROR_GROUPS.covers(&parse_query("container_name:billing"), None));
    }

    fn restricted_to(query: &str) -> LogAccess {
        let rule = crate::log_access::Restriction { terms: parse_query(query), roles: ["r-finance".to_string()].into(), teams: Default::default() };
        LogAccess::resolve(&[rule], false, &["r-finance".to_string()], &[])
    }

    #[test]
    fn a_plan_reads_whole_minutes_from_the_aggregate_and_the_rest_from_the_logs() {
        let range = TimeRange { from: T0 + 5_000, to: T0 + 10 * MINUTE_MS + 30_000 };
        let all = LogAccess::org_background();
        let p = plan("acme", &all, &LOG_FACETS, range, &parse_query("service:billing"), None, Some(T0), &[]);
        let agg = p.agg.expect("the aggregate answers");
        assert_eq!(agg.table, "oc.log_facets_1m_read");
        assert_eq!(agg.clause, "org_id = ? AND minute >= toDateTime(?) AND minute < toDateTime(?) AND ((service = ?))");
        assert_eq!(agg.params, vec![Param::Str("acme".into()), Param::Int((T0 + MINUTE_MS) / 1000), Param::Int((T0 + 10 * MINUTE_MS) / 1000), Param::Str("billing".into())]);
        // The logs outside the window, with the same search.
        assert!(p.logs.clause.starts_with("org_id = ? AND timestamp >= "), "{}", p.logs.clause);
        assert!(p.logs.clause.ends_with(" AND (timestamp < fromUnixTimestamp64Milli(?) OR timestamp >= fromUnixTimestamp64Milli(?))"), "{}", p.logs.clause);
        assert_eq!(&p.logs.params[p.logs.params.len() - 2..], &[Param::Int(T0 + MINUTE_MS), Param::Int(T0 + 10 * MINUTE_MS)]);

        // Without a window, or with a search the aggregate can't answer: the logs, unchanged.
        let whole = scope("acme", &all, range, &parse_query("service:billing timeout"), None);
        for p in [
            plan("acme", &all, &LOG_FACETS, range, &parse_query("service:billing timeout"), None, Some(T0), &[]),
            plan("acme", &all, &LOG_FACETS, range, &parse_query("service:billing timeout"), None, None, &[]),
        ] {
            assert!(p.agg.is_none());
            assert_eq!((p.logs.clause, p.logs.params), (whole.clause.clone(), whole.params.clone()));
        }
        // Counting by a key the aggregate doesn't keep: the logs.
        assert!(plan("acme", &all, &LOG_FACETS, range, &[], None, Some(T0), &["pod_name"]).agg.is_none());
        assert!(plan("acme", &all, &LOG_FACETS, range, &[], None, Some(T0), &["service", "kube_namespace"]).agg.is_some());
        // Rehydrated logs aren't aggregated.
        assert!(plan("acme", &all, &LOG_FACETS, range, &parse_query("index:rehydrated"), None, Some(T0), &[]).agg.is_none());
    }

    #[test]
    fn a_plan_applies_the_readers_restriction_to_the_aggregate_or_reads_the_logs() {
        let range = TimeRange { from: T0, to: T0 + 10 * MINUTE_MS };
        let p = plan("acme", &restricted_to("env:prod kube_namespace:payments"), &LOG_FACETS, range, &[], None, Some(T0), &[]);
        let agg = p.agg.expect("the restriction filters on kept keys");
        assert!(agg.clause.ends_with(" AND (((env = ?) AND (kube_namespace = ?)))"), "{}", agg.clause);
        assert_eq!(&agg.params[agg.params.len() - 2..], &[Param::Str("prod".into()), Param::Str("payments".into())]);
        // The facet's own filter is dropped, never the restriction's.
        let p = plan("acme", &restricted_to("service:billing"), &LOG_FACETS, range, &parse_query("service:checkout"), Some("service"), Some(T0), &[]);
        assert_eq!(p.agg.unwrap().params.last(), Some(&Param::Str("billing".into())));
        // A restriction the aggregate can't express: the logs only, restricted.
        let p = plan("acme", &restricted_to("pod_name:billing-*"), &LOG_FACETS, range, &[], None, Some(T0), &[]);
        assert!(p.agg.is_none());
        assert!(p.logs.clause.ends_with(" AND (((tags[?] ILIKE ?)))"), "{}", p.logs.clause);
        let p = plan("acme", &restricted_to("\"card declined\""), &ERROR_GROUPS, range, &[], None, Some(T0), &[]);
        assert!(p.agg.is_none());
        // Nothing assigned: the aggregate matches nothing either.
        let none = LogAccess::resolve(&[crate::log_access::Restriction { terms: parse_query("service:billing"), roles: Default::default(), teams: Default::default() }], false, &[], &[]);
        assert!(plan("acme", &none, &LOG_FACETS, range, &[], None, Some(T0), &[]).agg.unwrap().clause.ends_with(" AND (0)"));
    }

    #[test]
    fn the_error_aggregate_holds_only_errors() {
        let range = TimeRange { from: T0, to: T0 + 10 * MINUTE_MS };
        let p = plan("acme", &LogAccess::org_background(), &ERROR_GROUPS, range, &parse_query("-status:warn"), None, Some(T0), &[]);
        assert!(p.agg.unwrap().clause.ends_with("AND (NOT ('error' = ?))"));
    }

    #[test]
    fn a_large_range_is_read_from_its_newest_minutes() {
        let range = TimeRange { from: T0, to: T0 + 60 * MINUTE_MS - 1 };
        let minutes: Vec<(i64, u64)> = (0..60).map(|m| (T0 + m * MINUTE_MS, 1_000)).collect();
        // 60k logs, 10k at most: the newest 10 minutes, counts × 6.
        let b = bound(range, Some(T0), &minutes, 10_000);
        assert_eq!(b, Bound { range: TimeRange { from: T0 + 50 * MINUTE_MS, to: range.to }, factor: 6.0 });
        assert!(b.approximate());
        assert_eq!(b.scale(7), 42);
        // Few enough logs: the whole range, exact.
        let b = bound(range, Some(T0), &minutes, 100_000);
        assert_eq!((b.range.from, b.factor, b.approximate()), (T0, 1.0, false));
        // Logs before the aggregate's start are unknown: the whole range unless the known ones are too many.
        assert!(!bound(range, Some(T0 + 30 * MINUTE_MS), &minutes, 40_000).approximate());
        assert_eq!(bound(range, Some(T0 + 30 * MINUTE_MS), &minutes, 10_000).range.from, T0 + 50 * MINUTE_MS);
        assert!(!bound(range, None, &minutes, 10).approximate());
    }
}
