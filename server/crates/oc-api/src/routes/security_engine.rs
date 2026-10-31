//! Cloud SIEM's detection engine: every minute, each active `log_detection` threshold rule counts
//! its queries' logs over its evaluation window, per group, and raises a signal for the groups whose
//! values meet a case (the first matching case, most severe first as the rule orders them).
//!
//! - A group already signalled within the rule's keep-alive (and the signal younger than its
//!   maximum duration) extends that signal: last seen, events matched since it started, entities,
//!   and a higher severity if a more severe case now matches. Otherwise a new signal starts.
//! - Suppressions ([`super::security_suppressions`]) whose rule query matches the rule drop the
//!   signals whose group or tags match their suppression query, and leave the logs matching their
//!   data exclusion query (one term) out of the counts. The rule's own `filters` do the same
//!   (`require` adds a term, `suppress` excludes one).
//! - Entities: the top client IPs, users, hosts, services and countries of the matching logs, with
//!   how many there are, stored on the signal (counts and values, never the log events: the signal
//!   panel reads samples with the reader's own log access).
//!
//! The engine reads the org's logs unrestricted (`LogAccess::org_background`), like the monitor
//! evaluator: it only derives counts and top values.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use oc_query::{parse_query, LogsSchema, Param, Schema, Term};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};

use super::logs::{bind, scope, TimeRange};
use super::security_rules::{load_rules, option_i64, query_names, severity_rank, Condition, RuleQuery, SecurityRule};
use super::security_suppressions::{active_suppressions, Suppression};
use crate::error::ApiError;
use crate::log_access::LogAccess;
use crate::state::AppState;

/// Groups evaluated per rule query and tick.
const MAX_GROUPS: u64 = 1000;
/// New or extended signals per rule and tick.
const MAX_SIGNALS: usize = 50;

/// The entity kinds of a signal and the log fields they come from.
pub const ENTITY_FIELDS: [(&str, &str); 6] = [
    ("ip", "@network.client.ip"),
    ("user", "@usr.email"),
    ("user_name", "@usr.name"),
    ("host", "host"),
    ("service", "service"),
    ("country", "@network.client.geoip.country.iso_code"),
];

pub async fn run(st: AppState) {
    let access = LogAccess::org_background();
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let mut conn = match st.postgres.acquire().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "security rules: postgres unavailable");
                continue;
            }
        };
        if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(7312)").fetch_one(&mut *conn).await.unwrap_or(false) {
            continue;
        }
        match sqlx::query_scalar::<_, String>("SELECT id FROM orgs WHERE status = 'active'").fetch_all(&st.postgres).await {
            Ok(orgs) => {
                for org in orgs {
                    if let Err(e) = evaluate_org(&st, &org, &access, chrono::Utc::now().timestamp_millis()).await {
                        tracing::warn!(error = %e, org, "security rules: evaluation failed");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "security rules: listing orgs failed"),
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock(7312)").execute(&mut *conn).await;
    }
}

/// Evaluates every active rule of an org once, at `now`; returns the signals raised or extended.
pub async fn evaluate_org(st: &AppState, org: &str, access: &LogAccess, now: i64) -> Result<usize, ApiError> {
    let rules = load_rules(st, org).await?;
    let suppressions = active_suppressions(st, org, now).await?;
    let mut n = 0;
    for rule in rules.iter().filter(|r| r.active && r.not_evaluated.is_none()) {
        match evaluate_rule(st, org, access, rule, &suppressions, now).await {
            Ok(k) => n += k,
            Err(e) => tracing::warn!(error = %e, org, rule = rule.id, "security rule evaluation failed"),
        }
    }
    Ok(n)
}

#[derive(clickhouse::Row, Deserialize)]
struct GroupRow {
    g: Vec<String>,
    n: u64,
}

/// The terms of a rule query, with the rule's filters and the suppressions' data exclusions.
fn query_terms(query: &str, rule: &SecurityRule, suppressions: &[&Suppression]) -> Vec<Term> {
    let mut terms = parse_query(query);
    // Workload Protection's rules query the Agent's runtime security events.
    if rule.kind == "workload_security" {
        terms.push(Term::Tag { key: "source".into(), values: vec![super::workload_protection::SOURCE.into()], negate: false });
    }
    for f in &rule.filters {
        let t = parse_query(&f.query);
        match f.action.as_str() {
            "require" => terms.extend(t),
            "suppress" => terms.extend(negated(t)),
            _ => {}
        }
    }
    for s in suppressions {
        terms.extend(negated(parse_query(&s.data_exclusion_query)));
    }
    terms
}

/// A one-term query, negated (a query of several terms can't be excluded term by term).
fn negated(terms: Vec<Term>) -> Option<Term> {
    if terms.len() != 1 {
        return None;
    }
    Some(match terms.into_iter().next().unwrap() {
        Term::Tag { key, values, negate } => Term::Tag { key, values, negate: !negate },
        Term::Text { text, negate } => Term::Text { text, negate: !negate },
    })
}

/// Per group (the group-by values), the query's value over `range`.
async fn group_values(st: &AppState, org: &str, access: &LogAccess, terms: &[Term], group_by: &[String], q: &RuleQuery, range: TimeRange) -> Result<Vec<(Vec<String>, f64)>, ApiError> {
    let (aggregation, distinct) = (q.aggregation.as_str(), &q.distinct_fields);
    let s = scope(org, access, range, terms, None);
    let mut params: Vec<Param> = Vec::new();
    let mut exprs = Vec::new();
    for k in group_by {
        let (e, p) = LogsSchema.field(k);
        exprs.push(format!("toString({e})"));
        params.extend(p);
    }
    let g = if exprs.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", exprs.join(", ")) };
    let value = if aggregation == "cardinality" && !distinct.is_empty() {
        let (e, p) = LogsSchema.field(&distinct[0]);
        params.extend(p);
        format!("uniqExact(toString({e}))")
    } else {
        "count()".to_string()
    };
    params.extend(s.params);
    let rows: Vec<GroupRow> = bind(
        st.ch(org).query(&format!("SELECT {g} AS g, toUInt64({value}) AS n FROM {} WHERE {} GROUP BY g ORDER BY n DESC LIMIT {MAX_GROUPS}", s.table, s.clause)),
        &params,
    )
    .fetch_all()
    .await?;
    Ok(rows.into_iter().map(|r| (r.g, r.n as f64)).collect())
}

pub async fn evaluate_rule(st: &AppState, org: &str, access: &LogAccess, rule: &SecurityRule, suppressions: &[Suppression], now: i64) -> Result<usize, ApiError> {
    let applicable: Vec<&Suppression> = suppressions.iter().filter(|s| s.applies_to(rule)).collect();
    let window = option_i64(&rule.options, "evaluationWindow", 300).max(60) * 1000;
    let range = TimeRange { from: now - window, to: now };
    let names = query_names(&rule.queries);
    let group_by: Vec<String> = rule.queries.first().map(|q| q.group_by_fields.clone()).unwrap_or_default();
    // Group key → query name → value.
    let mut groups: BTreeMap<Vec<String>, HashMap<String, f64>> = BTreeMap::new();
    for (q, name) in rule.queries.iter().zip(&names) {
        let terms = query_terms(&q.query, rule, &applicable);
        // Every query groups by the first query's fields, so their values line up.
        let values = if rule.kind == "application_security" && super::ai_guard::is_ai_guard_query(&q.query) {
            super::ai_guard::group_counts(st, org, &q.query, &group_by, range).await?
        } else if rule.kind == "application_security" {
            super::appsec::group_counts(st, org, &q.query, &group_by, range).await?
        } else {
            group_values(st, org, access, &terms, &group_by, q, range).await?
        };
        for (g, v) in values {
            groups.entry(g).or_default().insert(name.clone(), v);
        }
    }
    let cases: Vec<(usize, Condition)> = rule.cases.iter().enumerate().filter_map(|(i, c)| Condition::parse(&c.condition).ok().map(|cond| (i, cond))).collect();
    let mut raised = 0;
    for (g, values) in groups {
        if raised >= MAX_SIGNALS {
            break;
        }
        // A group with an empty value ("N/A") is not a group of the rule.
        if g.iter().any(|v| v.is_empty()) && !group_by.is_empty() {
            continue;
        }
        let Some((case_index, _)) = cases.iter().find(|(_, c)| c.eval(&values)) else { continue };
        let case = &rule.cases[*case_index];
        let group: BTreeMap<String, String> = group_by.iter().cloned().zip(g.iter().cloned()).collect();
        if applicable.iter().any(|s| s.suppresses(rule, &group, &case.status)) {
            continue;
        }
        upsert_signal(st, org, access, rule, &applicable, *case_index, &group, now, window).await?;
        raised += 1;
    }
    sqlx::query("UPDATE security_rules SET evaluated_at = now() WHERE org_id = $1 AND id = $2").bind(org).bind(&rule.id).execute(&st.postgres).await?;
    Ok(raised)
}

#[derive(clickhouse::Row, Deserialize)]
struct EntityRow {
    n: u64,
    t0: Vec<String>,
    t1: Vec<String>,
    t2: Vec<String>,
    t3: Vec<String>,
    t4: Vec<String>,
    t5: Vec<String>,
    u0: u64,
    u1: u64,
    u2: u64,
    u3: u64,
    u4: u64,
    u5: u64,
    first: i64,
}

/// The group's terms: `@field:"value"` per group-by field.
pub fn group_terms(group: &BTreeMap<String, String>) -> Vec<Term> {
    group.iter().map(|(k, v)| Term::Tag { key: k.clone(), values: vec![v.clone()], negate: false }).collect()
}

/// Matching events of the signal's queries and group over `range`, with the top entities.
async fn entities(st: &AppState, org: &str, access: &LogAccess, terms: &[Term], range: TimeRange) -> Result<(u64, Value, i64), ApiError> {
    let s = scope(org, access, range, terms, None);
    let mut params: Vec<Param> = Vec::new();
    let mut tops = Vec::new();
    let mut uniqs = Vec::new();
    for (i, (_, field)) in ENTITY_FIELDS.iter().enumerate() {
        let (e, p) = LogsSchema.field(field);
        // The expression appears twice: its map key is bound twice.
        tops.push(format!("topKIf(5)(toString({e}), toString({e}) != '') AS t{i}"));
        params.extend(p.clone());
        params.extend(p.clone());
        uniqs.push((i, e, p));
    }
    for (i, e, p) in uniqs {
        tops.push(format!("toUInt64(uniqExactIf(toString({e}), toString({e}) != '')) AS u{i}"));
        params.extend(p.clone());
        params.extend(p);
    }
    params.extend(s.params);
    let row: EntityRow = bind(
        st.ch(org).query(&format!(
            "SELECT count() AS n, {}, toInt64(toUnixTimestamp64Milli(min(timestamp))) AS first FROM {} WHERE {}",
            tops.join(", "),
            s.table,
            s.clause
        )),
        &params,
    )
    .fetch_one()
    .await?;
    let lists = [(&row.t0, row.u0), (&row.t1, row.u1), (&row.t2, row.u2), (&row.t3, row.u3), (&row.t4, row.u4), (&row.t5, row.u5)];
    let mut out = serde_json::Map::new();
    for ((kind, _), (top, total)) in ENTITY_FIELDS.iter().zip(lists) {
        if total > 0 {
            out.insert(kind.to_string(), json!({ "top": top, "count": total }));
        }
    }
    Ok((row.n, Value::Object(out), row.first))
}

fn new_signal_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    format!("AQAAA{}", (0..43).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect::<String>())
}

/// The signal's title: the rule's name, with the group's values when the rule extends its title,
/// and the case's name.
pub fn signal_title(rule: &SecurityRule, case_name: &str, group: &BTreeMap<String, String>) -> String {
    let mut parts = vec![rule.name.clone()];
    if rule.has_extended_title && !group.is_empty() {
        parts.push(group.values().cloned().collect::<Vec<_>>().join(", "));
    }
    if !case_name.is_empty() {
        parts.push(case_name.to_string());
    }
    parts.join(" - ")
}

#[allow(clippy::too_many_arguments)]
async fn upsert_signal(st: &AppState, org: &str, access: &LogAccess, rule: &SecurityRule, suppressions: &[&Suppression], case_index: usize, group: &BTreeMap<String, String>, now: i64, window: i64) -> Result<(), ApiError> {
    let case = &rule.cases[case_index];
    let group_key = serde_json::to_string(group).unwrap_or_default();
    let keep_alive = option_i64(&rule.options, "keepAlive", 3600);
    let max_duration = option_i64(&rule.options, "maxSignalDuration", 86400);
    let existing: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT id, severity, (extract(epoch FROM first_seen) * 1000)::bigint FROM security_signals \
         WHERE org_id = $1 AND rule_id = $2 AND group_key = $3 AND last_seen > to_timestamp($4 / 1000.0) - make_interval(secs => $5) \
         AND first_seen > to_timestamp($4 / 1000.0) - make_interval(secs => $6) ORDER BY last_seen DESC LIMIT 1",
    )
    .bind(org)
    .bind(&rule.id)
    .bind(&group_key)
    .bind(now)
    .bind(keep_alive as f64)
    .bind(max_duration as f64)
    .fetch_optional(&st.postgres)
    .await?;
    // Events and entities since the signal started (its first window).
    let started = existing.as_ref().map_or(now - window, |(_, _, first)| *first - window);
    // The first query's logs: what the signal is about.
    let mut terms = rule.queries.first().map(|q| query_terms(&q.query, rule, suppressions)).unwrap_or_default();
    terms.extend(group_terms(group));
    let first_query = rule.queries.first().map_or("", |q| q.query.as_str());
    let (events, ents, first) = if rule.kind == "application_security" && super::ai_guard::is_ai_guard_query(first_query) {
        super::ai_guard::entities(st, org, first_query, group, TimeRange { from: started, to: now }).await?
    } else if rule.kind == "application_security" {
        super::appsec::entities(st, org, rule.queries.first().map_or("", |q| q.query.as_str()), group, TimeRange { from: started, to: now }).await?
    } else {
        entities(st, org, access, &terms, TimeRange { from: started, to: now }).await?
    };
    let source = rule.queries.first().and_then(|q| parse_query(&q.query).into_iter().find_map(|t| match t {
        Term::Tag { key, values, negate: false } if key == "source" => values.into_iter().next(),
        _ => None,
    }));
    let services: Vec<String> = group.get("service").cloned().map(|s| vec![s]).unwrap_or_else(|| ents.get("service").and_then(|s| s["top"].as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(3).collect()).unwrap_or_default());
    match existing {
        Some((id, severity, _)) => {
            let severity = if severity_rank(&case.status) > severity_rank(&severity) { case.status.clone() } else { severity };
            sqlx::query("UPDATE security_signals SET last_seen = to_timestamp($3 / 1000.0), events_matched = $4, entities = $5, severity = $6, services = $7 WHERE org_id = $1 AND id = $2")
                .bind(org)
                .bind(&id)
                .bind(now)
                .bind(events as i64)
                .bind(&ents)
                .bind(&severity)
                .bind(&services)
                .execute(&st.postgres)
                .await?;
        }
        None => {
            let first_seen = if first > 0 { first } else { now };
            let queries: Vec<Value> = rule.queries.iter().zip(query_names(&rule.queries)).map(|(q, n)| json!({ "name": n, "query": q.query })).collect();
            let mut tags = rule.tags.clone();
            tags.extend(group.iter().filter(|(k, _)| !k.starts_with('@')).map(|(k, v)| format!("{k}:{v}")));
            sqlx::query(
                "INSERT INTO security_signals (org_id, id, rule_id, rule_name, rule_type, title, message, severity, case_name, condition, group_key, group_values, queries, \
                 source, services, env, tags, entities, events_matched, first_seen, last_seen) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, to_timestamp($20 / 1000.0), to_timestamp($21 / 1000.0))",
            )
            .bind(org)
            .bind(new_signal_id())
            .bind(&rule.id)
            .bind(&rule.name)
            .bind(&rule.kind)
            .bind(signal_title(rule, &case.name, group))
            .bind(&rule.message)
            .bind(&case.status)
            .bind(&case.name)
            .bind(&case.condition)
            .bind(&group_key)
            .bind(json!(group))
            .bind(json!(queries))
            .bind(source.unwrap_or_default())
            .bind(&services)
            .bind(group.get("env").cloned().unwrap_or_default())
            .bind(&tags)
            .bind(&ents)
            .bind(events as i64)
            .bind(first_seen)
            .bind(now)
            .execute(&st.postgres)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::security_rules::test_rule;

    fn rule(extended: bool) -> SecurityRule {
        SecurityRule { has_extended_title: extended, ..test_rule() }
    }

    #[test]
    fn titles_carry_the_group_when_extended() {
        let g: BTreeMap<String, String> = [("service".to_string(), "storefront-api".to_string())].into();
        assert_eq!(signal_title(&rule(true), "Condition 1", &g), "Attack attempts - storefront-api - Condition 1");
        assert_eq!(signal_title(&rule(false), "", &g), "Attack attempts");
    }

    #[test]
    fn filters_and_exclusions_become_terms() {
        let mut r = rule(false);
        r.filters = vec![
            crate::routes::security_rules::RuleFilter { action: "require".into(), query: "env:prod".into() },
            crate::routes::security_rules::RuleFilter { action: "suppress".into(), query: "@network.client.ip:10.0.0.1".into() },
        ];
        let terms = query_terms("source:nginx", &r, &[]);
        assert_eq!(terms.len(), 3);
        assert!(matches!(&terms[2], Term::Tag { negate: true, key, .. } if key == "@network.client.ip"));
        assert!(negated(parse_query("a:b c:d")).is_none());
    }

    /// The default Okta rule raises one signal for the user with many failed MFA challenges, extends
    /// it on the next run, and a suppression silences it. Live: skipped unless OC_TEST_CLICKHOUSE_URL
    /// and OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn rules_raise_extend_and_suppress_signals() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "siem").await;
        let org = ctx.org_id.clone();
        let now = chrono::Utc::now().timestamp_millis();
        st.ch(&org)
            .query(&format!(
                "INSERT INTO oc.logs (org_id, timestamp, status, service, host, source, message, tags, attr, retention_days) \
                 SELECT '{org}', fromUnixTimestamp64Milli(toInt64({now} - 120000 + number * 1000)), 'warn', 'okta', '', 'okta', 'MFA challenge failed', map('env', 'prod'), \
                        map('evt.name', 'user.authentication.auth_via_mfa', 'evt.outcome', 'FAILURE', 'usr.email', if(number < 30, 'ana@example.com', 'bob@example.com'), \
                            'network.client.ip', concat('198.51.100.', toString(number % 4))), 15 \
                 FROM numbers(32)"
            ))
            .execute()
            .await
            .expect("insert logs");
        let access = LogAccess::org_background();
        let okta = |signals: &[crate::routes::security_signals::SecuritySignal]| signals.iter().filter(|s| s.rule_id == "def-okt-001").cloned().collect::<Vec<_>>();
        let range = TimeRange { from: now - 3_600_000, to: now + 60_000 };
        let rules = load_rules(st, &org).await.unwrap();
        let rule = rules.iter().find(|r| r.id == "def-okt-001").unwrap();
        assert_eq!(evaluate_rule(st, &org, &access, rule, &[], now).await.expect("evaluate the Okta rule"), 1);
        evaluate_org(st, &org, &access, now).await.expect("evaluate");
        let signals = okta(&crate::routes::security_signals::signals_in(st, &org, range).await.unwrap());
        assert_eq!(signals.len(), 1, "one signal for ana, none for bob (2 failures)");
        let s = &signals[0];
        assert_eq!(s.severity, "high");
        assert_eq!(s.group_values.get("@usr.email").map(String::as_str), Some("ana@example.com"));
        assert_eq!(s.events_matched, 30);
        assert_eq!(s.entities["ip"]["count"], 4);
        assert_eq!(s.source, "okta");
        assert!(s.logs_query.contains("@usr.email:ana@example.com"), "{}", s.logs_query);
        // The next minute extends the same signal.
        evaluate_org(st, &org, &access, now + 60_000).await.expect("evaluate");
        let again = okta(&crate::routes::security_signals::signals_in(st, &org, range).await.unwrap());
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, s.id);
        // A suppression for ana drops new signals.
        sqlx::query("DELETE FROM security_signals WHERE org_id = $1").bind(&org).execute(&st.postgres).await.unwrap();
        let input = crate::routes::security_suppressions::SuppressionInput { name: "ana".into(), enabled: true, rule_query: "source:okta".into(), suppression_query: "@usr.email:ana@example.com".into(), ..Default::default() };
        crate::routes::security_suppressions::insert(st, &ctx, &input).await.unwrap();
        evaluate_org(st, &org, &access, now + 120_000).await.expect("evaluate");
        assert!(okta(&crate::routes::security_signals::signals_in(st, &org, range).await.unwrap()).is_empty());
        // The pack shows its logs.
        let packs = crate::routes::security_siem::pack_statuses(st, &org, &access, now).await.unwrap();
        let p = packs.iter().find(|p| p.id == "okta").unwrap();
        assert_eq!((p.status.as_str(), p.logs24h), ("active", 32));
        assert_eq!(packs.iter().find(|p| p.id == "github").unwrap().status, "available");
    }
}
