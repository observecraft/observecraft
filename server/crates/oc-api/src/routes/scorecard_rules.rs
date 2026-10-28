//! Scorecard rules: what each built-in rule checks, how a service's outcomes add up to a level,
//! and which services a rule's scope covers. Pure functions over the facts that
//! [`super::scorecards`] gathers from telemetry, monitors, SLOs and service metadata.

use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::catalog::Faceted;

pub const DAY_MS: i64 = 86_400_000;
/// "Deployed in the past 3 months".
pub const DEPLOY_WINDOW_DAYS: i64 = 90;
pub const MAX_LEVEL: u8 = 3;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ScorecardOutcomeState {
    Pass,
    Fail,
    Skip,
}

impl ScorecardOutcomeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ScorecardOutcomeState::Pass => "pass",
            ScorecardOutcomeState::Fail => "fail",
            ScorecardOutcomeState::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pass" => Some(ScorecardOutcomeState::Pass),
            "fail" => Some(ScorecardOutcomeState::Fail),
            "skip" => Some(ScorecardOutcomeState::Skip),
            _ => None,
        }
    }
}

/// An outcome and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub state: ScorecardOutcomeState,
    pub reason: String,
}

fn pass(reason: impl Into<String>) -> Verdict {
    Verdict { state: ScorecardOutcomeState::Pass, reason: reason.into() }
}
fn fail(reason: impl Into<String>) -> Verdict {
    Verdict { state: ScorecardOutcomeState::Fail, reason: reason.into() }
}
fn skip(reason: impl Into<String>) -> Verdict {
    Verdict { state: ScorecardOutcomeState::Skip, reason: reason.into() }
}

/// The out-of-the-box rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum BuiltinRule {
    DeploymentTracking,
    LogsCorrelation,
    CodeReposDefined,
    ContactsDefined,
    DocsDefined,
    TeamDefined,
    DeployedRecently,
    MonitorsDefined,
    OnCallDefined,
    SlosDefined,
}

pub const OBSERVABILITY: &str = "Observability Best Practices";
pub const OWNERSHIP: &str = "Ownership & Documentation";
pub const PRODUCTION: &str = "Production Readiness";

/// The recommended scorecards, in display order, with their descriptions.
pub const BUILTIN_SCORECARDS: [(&str, &str); 3] = [
    (OBSERVABILITY, "Telemetry that makes a service easy to debug: versions on spans and logs tied to traces."),
    (OWNERSHIP, "Who owns a service, how to reach them, where its code and docs live."),
    (PRODUCTION, "What a service needs before it takes production traffic: monitors, SLOs, on-call and recent deploys."),
];

impl BuiltinRule {
    pub const ALL: [BuiltinRule; 10] = [
        BuiltinRule::DeploymentTracking,
        BuiltinRule::LogsCorrelation,
        BuiltinRule::CodeReposDefined,
        BuiltinRule::ContactsDefined,
        BuiltinRule::DocsDefined,
        BuiltinRule::TeamDefined,
        BuiltinRule::DeployedRecently,
        BuiltinRule::MonitorsDefined,
        BuiltinRule::OnCallDefined,
        BuiltinRule::SlosDefined,
    ];

    pub fn key(self) -> &'static str {
        match self {
            BuiltinRule::DeploymentTracking => "deployment_tracking",
            BuiltinRule::LogsCorrelation => "logs_correlation",
            BuiltinRule::CodeReposDefined => "code_repos_defined",
            BuiltinRule::ContactsDefined => "contacts_defined",
            BuiltinRule::DocsDefined => "docs_defined",
            BuiltinRule::TeamDefined => "team_defined",
            BuiltinRule::DeployedRecently => "deployed_recently",
            BuiltinRule::MonitorsDefined => "monitors_defined",
            BuiltinRule::OnCallDefined => "on_call_defined",
            BuiltinRule::SlosDefined => "slos_defined",
        }
    }

    pub fn from_key(key: &str) -> Option<BuiltinRule> {
        BuiltinRule::ALL.into_iter().find(|r| r.key() == key)
    }

    pub fn name(self) -> &'static str {
        match self {
            BuiltinRule::DeploymentTracking => "Deployment tracking is active",
            BuiltinRule::LogsCorrelation => "Logs correlation is active",
            BuiltinRule::CodeReposDefined => "Code repos defined",
            BuiltinRule::ContactsDefined => "Contacts defined",
            BuiltinRule::DocsDefined => "Docs defined",
            BuiltinRule::TeamDefined => "Team defined",
            BuiltinRule::DeployedRecently => "Deployed in the past 3 months",
            BuiltinRule::MonitorsDefined => "Monitors defined",
            BuiltinRule::OnCallDefined => "On-call defined",
            BuiltinRule::SlosDefined => "SLOs defined",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            BuiltinRule::DeploymentTracking => "The service's spans of the past 7 days carry service.version, so deployments can be compared.",
            BuiltinRule::LogsCorrelation => "The service's logs of the past 7 days carry trace ids, so logs and traces link to each other.",
            BuiltinRule::CodeReposDefined => "The service's metadata links to its code repository.",
            BuiltinRule::ContactsDefined => "The service's metadata lists at least one contact (email, chat channel or link).",
            BuiltinRule::DocsDefined => "The service's metadata links to its documentation or runbook.",
            BuiltinRule::TeamDefined => "The service's metadata names the team that owns it.",
            BuiltinRule::DeployedRecently => "A new version of the service was seen on spans, or a CI job deployed it, in the past 90 days.",
            BuiltinRule::MonitorsDefined => "At least one monitor's query or tags reference service:<name>.",
            BuiltinRule::OnCallDefined => "The owning team has an on-call schedule.",
            BuiltinRule::SlosDefined => "At least one SLO's tags or queries reference service:<name>.",
        }
    }

    pub fn level(self) -> u8 {
        match self {
            BuiltinRule::TeamDefined | BuiltinRule::MonitorsDefined | BuiltinRule::OnCallDefined => 1,
            BuiltinRule::DeploymentTracking | BuiltinRule::CodeReposDefined | BuiltinRule::ContactsDefined | BuiltinRule::DocsDefined | BuiltinRule::SlosDefined => 2,
            BuiltinRule::LogsCorrelation | BuiltinRule::DeployedRecently => 3,
        }
    }

    pub fn scorecard(self) -> &'static str {
        match self {
            BuiltinRule::DeploymentTracking | BuiltinRule::LogsCorrelation => OBSERVABILITY,
            BuiltinRule::CodeReposDefined | BuiltinRule::ContactsDefined | BuiltinRule::DocsDefined | BuiltinRule::TeamDefined => OWNERSHIP,
            BuiltinRule::DeployedRecently | BuiltinRule::MonitorsDefined | BuiltinRule::OnCallDefined | BuiltinRule::SlosDefined => PRODUCTION,
        }
    }
}

/// One version of a service seen on its entry spans of the deploy window.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct VersionSeen {
    /// '' for spans without service.version.
    pub version: String,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Spans of the past 7 days.
    pub recent_spans: u64,
}

/// What's known about one service.
#[derive(Clone, Debug, Default)]
pub struct ServiceFacts {
    pub team: String,
    pub contacts: usize,
    pub repo_url: String,
    pub docs_url: String,
    /// Versions on entry spans of the past 90 days (or as far as spans are kept).
    pub versions: Vec<VersionSeen>,
    /// Logs of the past 7 days, and those carrying a trace id.
    pub logs: u64,
    pub traced_logs: u64,
    /// Last CI deployment job of the service in the past 90 days (epoch ms).
    pub last_ci_deploy: Option<i64>,
    /// Names of the monitors and SLOs referencing the service.
    pub monitors: Vec<String>,
    pub slos: Vec<String>,
}

/// What's known about the org as a whole.
#[derive(Clone, Debug, Default)]
pub struct OrgFacts {
    pub now_ms: i64,
    /// Teams with an on-call schedule; `None` while On-Call isn't set up, with the reason shown.
    pub on_call_teams: Option<Vec<String>>,
    pub on_call_missing: String,
}

/// Evaluates a built-in rule for one service.
pub fn evaluate(rule: BuiltinRule, facts: &ServiceFacts, org: &OrgFacts) -> Verdict {
    let defined = |value: &str, what: &str| {
        if value.trim().is_empty() {
            fail(format!("No {what} in the service's metadata"))
        } else {
            pass(format!("{}: {}", capitalize(what), value.trim()))
        }
    };
    let team = facts.team.trim();
    match rule {
        BuiltinRule::DeploymentTracking => deployment_tracking(facts),
        BuiltinRule::LogsCorrelation => match (facts.logs, facts.traced_logs) {
            (0, _) => fail("No logs in the past 7 days"),
            (_, 0) => fail("No log of the past 7 days carries a trace id"),
            (n, traced) => pass(format!("{}% of the logs of the past 7 days carry a trace id", (traced as f64 * 100.0 / n as f64).round())),
        },
        BuiltinRule::TeamDefined if team.is_empty() => fail("No team in the service's metadata"),
        BuiltinRule::TeamDefined => pass(format!("Owned by team {team}")),
        BuiltinRule::ContactsDefined => match facts.contacts {
            0 => fail("No contacts in the service's metadata"),
            1 => pass("1 contact"),
            n => pass(format!("{n} contacts")),
        },
        BuiltinRule::CodeReposDefined => defined(&facts.repo_url, "code repository"),
        BuiltinRule::DocsDefined => defined(&facts.docs_url, "documentation link"),
        BuiltinRule::DeployedRecently => deployed_recently(facts, org.now_ms),
        BuiltinRule::MonitorsDefined => references(&facts.monitors, "monitor", "monitors"),
        BuiltinRule::SlosDefined => references(&facts.slos, "SLO", "SLOs"),
        BuiltinRule::OnCallDefined => match &org.on_call_teams {
            None => skip(org.on_call_missing.clone()),
            Some(_) if team.is_empty() => fail("No owning team to page"),
            Some(teams) if teams.iter().any(|t| t.eq_ignore_ascii_case(team)) => pass(format!("Team {team} has an on-call schedule")),
            Some(_) => fail(format!("Team {team} has no on-call schedule")),
        },
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn references(names: &[String], one: &str, many: &str) -> Verdict {
    const SHOWN: usize = 3;
    if names.is_empty() {
        return fail(format!("No {one} references this service (service:<name> in its query or tags)"));
    }
    let mut list = names.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > SHOWN {
        list.push_str(&format!(" and {} more", names.len() - SHOWN));
    }
    pass(format!("{} {}: {list}", names.len(), if names.len() == 1 { one } else { many }))
}

/// The versioned version seen last.
fn latest_version<'a>(versions: impl Iterator<Item = &'a VersionSeen>) -> Option<&'a VersionSeen> {
    versions.filter(|v| !v.version.is_empty()).max_by_key(|v| v.last_seen)
}

fn deployment_tracking(facts: &ServiceFacts) -> Verdict {
    if !facts.versions.iter().any(|v| v.recent_spans > 0) {
        return fail("No spans in the past 7 days");
    }
    match latest_version(facts.versions.iter().filter(|v| v.recent_spans > 0)) {
        Some(v) => pass(format!("Spans carry service.version (latest {})", v.version)),
        None => fail("Spans of the past 7 days don't carry service.version"),
    }
}

fn days_ago(now: i64, t: i64) -> String {
    match (now - t).max(0) / DAY_MS {
        0 => "today".into(),
        1 => "1 day ago".into(),
        n => format!("{n} days ago"),
    }
}

fn deployed_recently(facts: &ServiceFacts, now: i64) -> Verdict {
    const NOTHING: &str = "No spans with service.version and no CI deployment in the past 90 days";
    let window_start = now - DEPLOY_WINDOW_DAYS * DAY_MS;
    if let Some(at) = facts.last_ci_deploy.filter(|t| *t >= window_start) {
        return pass(format!("Deployed by CI {}", days_ago(now, at)));
    }
    let Some(earliest) = facts.versions.iter().map(|v| v.first_seen).min() else {
        return fail(NOTHING);
    };
    // A version first seen well after the earliest span kept is a deploy; one there from the start
    // may have been deployed any time before.
    let new = facts.versions.iter().filter(|v| !v.version.is_empty() && v.first_seen > earliest + DAY_MS && v.first_seen >= window_start).max_by_key(|v| v.first_seen);
    if let Some(v) = new {
        return pass(format!("Version {} first seen {}", v.version, days_ago(now, v.first_seen)));
    }
    match latest_version(facts.versions.iter()) {
        None => fail(NOTHING),
        Some(v) if earliest <= window_start + DAY_MS => fail(format!("Version {} has been running for more than 90 days", v.version)),
        Some(v) => fail(format!("Only version {} seen since {}, as far back as spans are kept", v.version, days_ago(now, earliest))),
    }
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')
}

/// Whether a monitor query, SLO query or tag mentions `service:<name>` (also quoted, or in a
/// `service:(a OR b)` list), not a longer name that starts the same.
pub fn references_service(text: &str, service: &str) -> bool {
    const KEY: &str = "service:";
    let text = text.to_lowercase();
    let service = service.to_lowercase();
    let mut from = 0;
    while let Some(found) = text[from..].find(KEY) {
        let at = from + found;
        from = at + KEY.len();
        if text[..at].chars().next_back().is_some_and(|c| name_char(c) || c == '@') {
            continue;
        }
        let rest = &text[from..];
        let values: Vec<&str> = if let Some(list) = rest.strip_prefix('(') {
            let list = &list[..list.find(')').unwrap_or(list.len())];
            list.split(|c: char| c.is_whitespace() || c == ',').filter(|v| !v.is_empty() && *v != "or").map(|v| v.trim_matches('"')).collect()
        } else if let Some(quoted) = rest.strip_prefix('"') {
            vec![&quoted[..quoted.find('"').unwrap_or(quoted.len())]]
        } else {
            vec![&rest[..rest.find(|c: char| !name_char(c)).unwrap_or(rest.len())]]
        };
        if values.contains(&service.as_str()) {
            return true;
        }
    }
    false
}

/// A service's level from its outcomes `(rule level, state)`: the highest level up to which no
/// rule fails (0 when a level-1 rule fails or nothing passes at all). Skipped rules don't count.
pub fn service_level(outcomes: &[(u8, ScorecardOutcomeState)]) -> u8 {
    if !outcomes.iter().any(|(_, s)| *s == ScorecardOutcomeState::Pass) {
        return 0;
    }
    (1..=MAX_LEVEL).take_while(|level| !outcomes.iter().any(|(l, s)| l == level && *s == ScorecardOutcomeState::Fail)).last().unwrap_or(0)
}

/// Share of passing rules among those that passed or failed, in percent (None: nothing evaluated).
pub fn score(outcomes: &[(u8, ScorecardOutcomeState)]) -> Option<f64> {
    let passed = outcomes.iter().filter(|(_, s)| *s == ScorecardOutcomeState::Pass).count();
    let failed = outcomes.iter().filter(|(_, s)| *s == ScorecardOutcomeState::Fail).count();
    (passed + failed > 0).then(|| passed as f64 * 100.0 / (passed + failed) as f64)
}

/// A service as a rule scope sees it.
pub struct ScopedService<'a> {
    pub service: &'a str,
    pub team: &'a str,
}

impl Faceted for ScopedService<'_> {
    fn facet_values(&self, key: &str) -> Vec<&str> {
        match key {
            "service" | "name" => vec![self.service],
            "team" => vec![self.team].into_iter().filter(|t| !t.is_empty()).collect(),
            "kind" => vec!["service"],
            _ => vec![],
        }
    }
    fn text(&self) -> String {
        self.service.to_string()
    }
}

/// A rule scope: a service search (`service:order-* team:checkout -service:billing`); empty for all.
pub struct Scope(Vec<Term>);

impl Scope {
    pub fn parse(scope: &str) -> Scope {
        Scope(parse_query(scope))
    }

    pub fn covers(&self, service: &str, team: &str) -> bool {
        ScopedService { service, team }.matches(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ScorecardOutcomeState::{Fail, Pass, Skip};
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn org() -> OrgFacts {
        OrgFacts { now_ms: NOW, on_call_teams: None, on_call_missing: "On-Call is not configured".into() }
    }

    fn version(v: &str, first_days_ago: i64, recent: u64) -> VersionSeen {
        VersionSeen { version: v.into(), first_seen: NOW - first_days_ago * DAY_MS, last_seen: NOW - DAY_MS / 24, recent_spans: recent }
    }

    fn eval(rule: BuiltinRule, f: &ServiceFacts) -> Verdict {
        evaluate(rule, f, &org())
    }

    #[test]
    fn builtin_keys_round_trip_and_match_the_recommended_levels() {
        for r in BuiltinRule::ALL {
            assert_eq!(BuiltinRule::from_key(r.key()), Some(r));
            assert!(BUILTIN_SCORECARDS.iter().any(|(n, _)| *n == r.scorecard()));
        }
        assert_eq!(BuiltinRule::from_key("nope"), None);
        assert_eq!(BuiltinRule::LogsCorrelation.level(), 3);
        assert_eq!(BuiltinRule::TeamDefined.level(), 1);
        assert_eq!(BuiltinRule::SlosDefined.level(), 2);
    }

    #[test]
    fn deployment_tracking_needs_versions_on_recent_spans() {
        let mut f = ServiceFacts::default();
        let v = eval(BuiltinRule::DeploymentTracking, &f);
        assert_eq!(v.state, Fail);
        assert!(v.reason.contains("No spans"), "{}", v.reason);

        f.versions = vec![version("", 20, 500)];
        let v = eval(BuiltinRule::DeploymentTracking, &f);
        assert_eq!(v.state, Fail);
        assert!(v.reason.contains("service.version"), "{}", v.reason);

        // Old versioned spans don't count: only the past 7 days.
        f.versions = vec![version("", 20, 500), version("1.2.0", 30, 0)];
        assert_eq!(eval(BuiltinRule::DeploymentTracking, &f).state, Fail);

        f.versions = vec![version("", 20, 500), version("1.3.0", 3, 120)];
        let v = eval(BuiltinRule::DeploymentTracking, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.contains("1.3.0"), "{}", v.reason);
    }

    #[test]
    fn logs_correlation_needs_trace_ids_on_logs() {
        let mut f = ServiceFacts::default();
        let v = eval(BuiltinRule::LogsCorrelation, &f);
        assert_eq!(v.state, Fail);
        assert!(v.reason.contains("No logs"), "{}", v.reason);
        f.logs = 400;
        let v = eval(BuiltinRule::LogsCorrelation, &f);
        assert_eq!(v.state, Fail);
        assert!(!v.reason.contains("400") && v.reason.contains("trace id"), "{}", v.reason);
        f.traced_logs = 100;
        let v = eval(BuiltinRule::LogsCorrelation, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.contains("25%"), "{}", v.reason);
    }

    #[test]
    fn metadata_rules_read_the_service_metadata() {
        let mut f = ServiceFacts::default();
        for r in [BuiltinRule::TeamDefined, BuiltinRule::ContactsDefined, BuiltinRule::CodeReposDefined, BuiltinRule::DocsDefined] {
            assert_eq!(eval(r, &f).state, Fail, "{r:?}");
        }
        f.team = "checkout".into();
        f.contacts = 2;
        f.repo_url = "https://git.example.com/example-org/order-service".into();
        f.docs_url = "  ".into();
        let team = eval(BuiltinRule::TeamDefined, &f);
        assert_eq!(team.state, Pass);
        assert!(team.reason.contains("checkout"));
        assert_eq!(eval(BuiltinRule::ContactsDefined, &f).state, Pass);
        assert_eq!(eval(BuiltinRule::CodeReposDefined, &f).state, Pass);
        // Blank is not defined.
        assert_eq!(eval(BuiltinRule::DocsDefined, &f).state, Fail);
    }

    #[test]
    fn monitors_and_slos_pass_when_one_references_the_service() {
        let mut f = ServiceFacts::default();
        let v = eval(BuiltinRule::MonitorsDefined, &f);
        assert_eq!(v.state, Fail);
        assert!(v.reason.contains("service:"), "{}", v.reason);
        assert_eq!(eval(BuiltinRule::SlosDefined, &f).state, Fail);
        f.monitors = vec!["[Checkout] error rate".into(), "p95 latency".into()];
        f.slos = vec!["Checkout availability".into()];
        let v = eval(BuiltinRule::MonitorsDefined, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.starts_with("2 monitors"), "{}", v.reason);
        let v = eval(BuiltinRule::SlosDefined, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.starts_with("1 SLO"), "{}", v.reason);
    }

    #[test]
    fn deployed_recently_wants_a_version_change_or_ci_deploy_in_90_days() {
        let mut f = ServiceFacts::default();
        let v = eval(BuiltinRule::DeployedRecently, &f);
        assert_eq!(v.state, Fail);

        // One version running since before the window opened: no deploy in 90 days.
        f.versions = vec![version("1.0.0", 90, 10)];
        let v = eval(BuiltinRule::DeployedRecently, &f);
        assert_eq!(v.state, Fail);
        assert!(v.reason.contains("1.0.0"), "{}", v.reason);

        // Spans kept for 15 days and one version throughout: when it was deployed is unknown.
        f.versions = vec![version("1.0.0", 15, 10)];
        assert_eq!(eval(BuiltinRule::DeployedRecently, &f).state, Fail);

        // A new version appeared after the first data: a deploy.
        f.versions = vec![version("1.0.0", 15, 0), version("1.1.0", 4, 10)];
        let v = eval(BuiltinRule::DeployedRecently, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.contains("1.1.0") && v.reason.contains("4 days ago"), "{}", v.reason);

        // Unversioned spans before the version showed up don't make it a deploy.
        f.versions = vec![version("", 15, 0), version("2.0.0", 15, 10)];
        assert_eq!(eval(BuiltinRule::DeployedRecently, &f).state, Fail);

        // A CI deployment counts too.
        f.versions = vec![];
        f.last_ci_deploy = Some(NOW - 10 * DAY_MS);
        let v = eval(BuiltinRule::DeployedRecently, &f);
        assert_eq!(v.state, Pass);
        assert!(v.reason.contains("CI"), "{}", v.reason);
    }

    #[test]
    fn on_call_skips_until_on_call_exists() {
        let mut f = ServiceFacts { team: "checkout".into(), ..Default::default() };
        let v = eval(BuiltinRule::OnCallDefined, &f);
        assert_eq!(v, Verdict { state: Skip, reason: "On-Call is not configured".into() });

        let with_on_call = OrgFacts { on_call_teams: Some(vec!["checkout".into()]), ..org() };
        assert_eq!(evaluate(BuiltinRule::OnCallDefined, &f, &with_on_call).state, Pass);
        f.team = "billing".into();
        assert_eq!(evaluate(BuiltinRule::OnCallDefined, &f, &with_on_call).state, Fail);
        f.team = String::new();
        assert_eq!(evaluate(BuiltinRule::OnCallDefined, &f, &with_on_call).state, Fail);
    }

    #[test]
    fn service_references_match_whole_names_only() {
        assert!(references_service("avg:trace.error_rate{service:order-service}", "order-service"));
        assert!(references_service("avg:trace.error_rate{env:prod,service:order-service} > 5", "order-service"));
        assert!(references_service("service:order-service", "order-service"));
        assert!(references_service("logs(\"service:billing status:error\") by {host}", "billing"));
        assert!(references_service("service:\"billing\"", "billing"));
        assert!(references_service("service:(billing OR order-service)", "order-service"));
        assert!(references_service("SERVICE:Billing", "billing"));
        assert!(!references_service("avg:trace.error_rate{service:order-service-v2}", "order-service"));
        assert!(!references_service("service:billing-watcher", "billing"));
        assert!(!references_service("myservice:billing", "billing"));
        assert!(!references_service("avg:billing.invoices{*}", "billing"));
        assert!(!references_service("", "billing"));
    }

    #[test]
    fn levels_stop_at_the_first_failing_level() {
        assert_eq!(service_level(&[]), 0);
        assert_eq!(service_level(&[(1, Skip), (2, Skip)]), 0);
        assert_eq!(service_level(&[(1, Fail), (2, Pass), (3, Pass)]), 0);
        assert_eq!(service_level(&[(1, Pass), (2, Fail), (3, Pass)]), 1);
        assert_eq!(service_level(&[(1, Pass), (1, Skip), (2, Pass), (3, Fail)]), 2);
        assert_eq!(service_level(&[(1, Pass), (2, Pass), (3, Pass)]), 3);
        // A level without rules doesn't hold a service back.
        assert_eq!(service_level(&[(1, Pass), (3, Pass)]), 3);
        assert_eq!(service_level(&[(2, Pass)]), 3);
    }

    #[test]
    fn scores_ignore_skipped_rules() {
        assert_eq!(score(&[]), None);
        assert_eq!(score(&[(1, Skip)]), None);
        assert_eq!(score(&[(1, Pass), (2, Fail), (2, Skip), (3, Pass), (3, Pass)]), Some(75.0));
    }

    #[test]
    fn scopes_filter_services_by_name_and_team() {
        assert!(Scope::parse("").covers("billing", ""));
        let s = Scope::parse("service:order-* team:checkout");
        assert!(s.covers("order-service", "checkout"));
        assert!(!s.covers("order-service", "billing"));
        assert!(!s.covers("billing", "checkout"));
        let s = Scope::parse("-service:billing");
        assert!(s.covers("order-service", ""));
        assert!(!s.covers("billing", ""));
        assert!(Scope::parse("kind:service").covers("billing", ""));
        assert!(Scope::parse("order").covers("order-service", ""));
    }
}
