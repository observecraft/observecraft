//! Each kind of email ObserveCraft sends, as a [`Frame`]: monitor transitions, On-Call pages and
//! their test, email verification, dashboard reports and the other notices (workflows, Assist,
//! surveys). The callers gather the facts; nothing here touches storage.

use super::frame::{colors, Button, ButtonStyle, Cell, Footer, Frame, Picture, Product, Row, Run, CONTENT_WIDTH};
use super::{chart, Email, Image};
use crate::routes::monitor_engine::{fmt_num, minutes_label, parse_group_key, Comparator, Detection, MonitorOptions};

pub(crate) const GRAPH_CID: &str = "graph@observecraft";

fn host(public_url: &str) -> String {
    url::Url::parse(public_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| public_url.to_string())
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `15:02 UTC · 16 Nov 2026`.
pub(crate) fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%H:%M UTC · %-d %b %Y").to_string()).unwrap_or_default()
}

fn notification_settings(public_url: &str) -> String {
    format!("{public_url}/account/profile?section=email")
}

// ================================================================ monitors

/// A monitor transition, as the notifier knows it.
pub(crate) struct MonitorFacts<'a> {
    pub monitor_id: i64,
    pub name: &'a str,
    /// metric | apm | log | synthetics
    pub kind: &'a str,
    pub priority: Option<i16>,
    /// `service:billing,env:prod`, or `*`.
    pub group: &'a str,
    /// alert | warn | nodata | ok
    pub to: &'a str,
    pub value: Option<f64>,
    pub message: &'a str,
    pub at: i64,
    pub test: bool,
    /// The full query (`avg(last_5m):… > 5`).
    pub query: &'a str,
    pub options: Option<&'a MonitorOptions>,
    /// The event's recipients (handles, without `@`).
    pub notified: &'a [String],
}

pub(crate) fn state_label(to: &str) -> &'static str {
    match to {
        "alert" => "Triggered",
        "warn" => "Warn",
        "nodata" => "No data",
        _ => "Recovered",
    }
}

fn state_color(to: &str) -> &'static str {
    match to {
        "alert" => colors::ALERT,
        "warn" => colors::WARN,
        "nodata" => colors::NO_DATA,
        _ => colors::OK,
    }
}

fn relation(c: Comparator) -> &'static str {
    match c {
        Comparator::Above => "above",
        Comparator::AboveOrEqual => "at or above",
        Comparator::Below => "below",
        Comparator::BelowOrEqual => "at or below",
    }
}

/// The one sentence under the title.
fn monitor_lead(f: &MonitorFacts) -> Vec<Run> {
    if f.test {
        return vec![Run::Plain(format!("This is a test of this monitor's {} notification: nothing has changed.", state_label(f.to).to_lowercase()))];
    }
    let what = if f.options.is_some_and(|o| o.detection == Detection::Change) { "The change" } else { "The value" };
    let window = f.options.map(|o| format!(" ({} over the last {})", format!("{:?}", o.time_aggregation).to_lowercase(), minutes_label(o.window_minutes))).unwrap_or_default();
    match (f.to, f.value, f.options) {
        ("nodata", _, Some(o)) => vec![Run::Plain(format!("No data has arrived for the last {}.", minutes_label(o.no_data_minutes)))],
        ("nodata", _, None) => vec![Run::Plain("No data has arrived.".into())],
        ("alert", Some(v), Some(o)) => vec![
            Run::Plain(format!("{what} is ")),
            Run::Strong(fmt_num(v)),
            Run::Plain(format!(", {} the alert threshold of {}{window}.", relation(o.comparator), fmt_num(o.thresholds.critical))),
        ],
        ("warn", Some(v), Some(o)) => vec![
            Run::Plain(format!("{what} is ")),
            Run::Strong(fmt_num(v)),
            Run::Plain(format!(", {} the warning threshold of {}{window}.", relation(o.comparator), fmt_num(o.thresholds.warning.unwrap_or(o.thresholds.critical)))),
        ],
        ("ok", Some(v), Some(o)) => vec![Run::Plain(format!("{what} is back to ")), Run::Strong(fmt_num(v)), Run::Plain(format!(", within the alert threshold of {}{window}.", fmt_num(o.thresholds.critical)))],
        ("ok", _, _) => vec![Run::Plain("The monitor is back to OK.".into())],
        (_, Some(v), _) => vec![Run::Plain(format!("{what} is ")), Run::Strong(fmt_num(v)), Run::Plain(".".into())],
        (to, None, _) => vec![Run::Plain(format!("The monitor is in {}.", state_label(to)))],
    }
}

fn group_tags(group: &str) -> Vec<String> {
    if group.is_empty() || group == "*" {
        return Vec::new();
    }
    parse_group_key(group).into_iter().map(|(k, v)| if v.is_empty() { k } else { format!("{k}:{v}") }).collect()
}

/// The monitor email; `graph` is the PNG of the group's series when it could be drawn.
pub(crate) fn monitor(public_url: &str, subject: &str, f: &MonitorFacts, graph: Option<Vec<u8>>) -> Email {
    let link = if f.monitor_id > 0 { format!("{public_url}/monitors/{}", f.monitor_id) } else { format!("{public_url}/monitors") };
    let tags = group_tags(f.group);
    let mut rows = Vec::new();
    if !tags.is_empty() {
        rows.push(Row::Pair("Group".into(), Cell::Codes(tags.clone())));
    }
    if !f.query.trim().is_empty() {
        rows.push(Row::Pair("Query".into(), Cell::Codes(vec![f.query.trim().to_string()])));
    }
    if f.at > 0 {
        rows.push(Row::Pair("Since".into(), Cell::Text(when(f.at))));
    }
    if !f.notified.is_empty() {
        rows.push(Row::Pair("Notified".into(), Cell::Text(f.notified.iter().map(|r| format!("@{r}")).collect::<Vec<_>>().join(" · "))));
    }
    let mut buttons = vec![Button::new("View monitor", link.clone(), ButtonStyle::Primary)];
    if f.kind == "apm" {
        buttons.push(Button::new("Open traces", format!("{public_url}/apm/traces?query={}", enc(&tags.join(" "))), ButtonStyle::Ghost));
    }
    if f.monitor_id > 0 && f.to != "ok" {
        buttons.push(Button::new("Mute 1 h", format!("{link}?mute=60"), ButtonStyle::Ghost));
    }
    let state = state_label(f.to);
    let band = match (f.test, f.priority) {
        (true, _) => format!("Test · {state}"),
        (false, Some(p)) => format!("{state} · P{p}"),
        (false, None) => state.to_string(),
    };
    let picture = graph.as_ref().map(|_| Picture { cid: GRAPH_CID.into(), alt: format!("{}: the hour around the event", f.name), width: CONTENT_WIDTH, height: chart::HEIGHT });
    let frame = Frame {
        product: Product::Core,
        band_color: if f.test { colors::BRAND } else { state_color(f.to) },
        band,
        title: f.name.to_string(),
        lead: monitor_lead(f),
        message: Some(f.message.to_string()),
        picture,
        rows,
        buttons,
        note: None,
        footer: Footer { why: "You get this because the monitor's message mentions you.".into(), link_label: "Notification settings".into(), url: notification_settings(public_url) },
        host: host(public_url),
    };
    let mut email = Email::new(subject, &frame);
    if let Some(png) = graph {
        email.images.push(Image { cid: GRAPH_CID.into(), png });
    }
    email
}

/// The graph's colours: the line in the state's, the thresholds in the alert's and warning's.
pub(crate) fn monitor_chart(to: &str, options: Option<&MonitorOptions>, points: Vec<(i64, f64)>, from: i64, until: i64, at: i64) -> chart::Chart {
    let mut thresholds = Vec::new();
    // A change monitor's thresholds apply to the change, not to the series drawn.
    if let Some(o) = options.filter(|o| o.detection == Detection::Threshold) {
        thresholds.push((o.thresholds.critical, colors::ALERT));
        if let Some(w) = o.thresholds.warning {
            thresholds.push((w, colors::WARN));
        }
    }
    chart::Chart { points, from, to: until, thresholds, color: state_color(to), event_at: Some(at) }
}

/// Message-IDs of monitor emails: one per monitor group transition (the event), so a recovery can
/// answer the alert that opened the episode.
pub(crate) fn monitor_message_id(monitor_id: i64, event_id: i64, domain: &str) -> String {
    format!("<monitor-{monitor_id}.event-{event_id}@{domain}>")
}

// ================================================================ On-Call

pub(crate) struct PageFacts<'a> {
    pub page_id: i64,
    pub title: &'a str,
    pub description: &'a str,
    /// high | low
    pub urgency: &'a str,
    pub team: Option<&'a str>,
    /// monitor | incident | manual | …
    pub source: &'a str,
    pub group: Option<&'a str>,
    pub opened_at: i64,
    /// "Not acknowledged: escalated to step 2".
    pub escalation: Option<String>,
    pub test: bool,
    pub responder: &'a str,
    /// The signed links (none for a test).
    pub ack_url: Option<String>,
    pub resolve_url: Option<String>,
}

/// `[Triggered] High error rate` → `High error rate`: the page's own subject tag replaces it.
fn without_tag(title: &str) -> &str {
    match title.strip_prefix('[').and_then(|t| t.split_once("] ")) {
        Some((tag, rest)) if !tag.contains(' ') || tag == "No data" => rest,
        _ => title,
    }
}

pub(crate) fn on_call_subject(title: &str) -> String {
    format!("[On-Call] {}", without_tag(title))
}

pub(crate) fn on_call_page(public_url: &str, f: &PageFacts) -> Email {
    let page_url = format!("{public_url}/on-call/pages/{}", f.page_id);
    let profile_url = format!("{public_url}/on-call/profile");
    let urgency = if f.urgency == "low" { "Low urgency" } else { "High urgency" };
    if f.test {
        let frame = Frame {
            product: Product::OnCall,
            band_color: colors::BRAND,
            band: format!("Test · {urgency}"),
            title: f.title.to_string(),
            lead: vec![Run::Plain(f.description.trim().to_string())],
            message: None,
            picture: None,
            rows: vec![Row::Pair("Responder".into(), Cell::Text(f.responder.to_string())), Row::Pair("Sent".into(), Cell::Text(when(f.opened_at)))],
            buttons: vec![Button::new("Your On-Call profile", profile_url.clone(), ButtonStyle::Primary)],
            note: Some("A real page has Acknowledge and Resolve buttons that work without signing in.".into()),
            footer: Footer { why: "You get this because you asked for a test notification.".into(), link_label: "Your On-Call profile".into(), url: profile_url },
            host: host(public_url),
        };
        return Email::new(format!("[On-Call] {}", f.title), &frame);
    }
    let title = without_tag(f.title);
    let mut lead = vec![Run::Plain(match f.team {
        Some(t) => format!("You're paged for the @{t} team."),
        None => "You're paged as a responder.".into(),
    })];
    if let Some(e) = &f.escalation {
        lead.push(Run::Plain(" ".into()));
        lead.push(Run::Strong(format!("{e}.")));
    }
    let mut rows = vec![Row::Pair("Source".into(), Cell::Text(capitalized(f.source)))];
    if let Some(t) = f.team {
        rows.push(Row::Pair("Team".into(), Cell::Codes(vec![format!("@{t}")])));
    }
    let tags = f.group.map(group_tags).unwrap_or_default();
    if !tags.is_empty() {
        rows.push(Row::Pair("Group".into(), Cell::Codes(tags)));
    }
    if f.opened_at > 0 {
        rows.push(Row::Pair("Opened".into(), Cell::Text(when(f.opened_at))));
    }
    let mut buttons = Vec::new();
    if let Some(u) = &f.ack_url {
        buttons.push(Button::new("Acknowledge", u.clone(), ButtonStyle::Danger));
    }
    if let Some(u) = &f.resolve_url {
        buttons.push(Button::new("Resolve", u.clone(), ButtonStyle::Ghost));
    }
    buttons.push(Button::new("Open page", page_url, ButtonStyle::Ghost));
    let frame = Frame {
        product: Product::OnCall,
        band_color: colors::ON_CALL,
        band: format!("{} · {urgency}", if f.escalation.is_some() { "Escalated" } else { "You're on call" }),
        title: format!("Page #{} · {title}", f.page_id),
        lead,
        message: Some(f.description.to_string()),
        picture: None,
        rows,
        buttons,
        note: Some("Acknowledge and Resolve work without signing in: each link is for you and this page only, works once and expires in 24 hours.".into()),
        footer: Footer { why: "You get this because you're a responder of this page.".into(), link_label: "Your On-Call profile".into(), url: profile_url },
        host: host(public_url),
    };
    Email::new(on_call_subject(f.title), &frame)
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

// ================================================================ account

pub(crate) fn verification(public_url: &str, email: &str, link: &str) -> Email {
    let frame = Frame {
        product: Product::Core,
        band_color: colors::BRAND,
        band: "Verify your email".into(),
        title: "Confirm your email address".into(),
        lead: vec![Run::Plain("Open the link within a day to confirm that ".into()), Run::Strong(email.to_string()), Run::Plain(" is your address on ObserveCraft.".into())],
        message: None,
        picture: None,
        rows: Vec::new(),
        buttons: vec![Button::new("Verify email", link, ButtonStyle::Primary)],
        note: Some("If you didn't ask for it, ignore this email.".into()),
        footer: Footer { why: "You get this because this address was entered in ObserveCraft's Personal Settings.".into(), link_label: "Personal Settings".into(), url: format!("{public_url}/account/profile") },
        host: host(public_url),
    };
    Email::new("Verify your ObserveCraft email", &frame)
}

// ================================================================ reports

pub(crate) struct ReportSeries<'a> {
    pub label: &'a str,
    pub current: String,
    pub min: String,
    pub max: String,
}

pub(crate) struct ReportWidget<'a> {
    pub title: &'a str,
    pub series: Vec<ReportSeries<'a>>,
    /// Shown instead of the series (an error, "see the dashboard").
    pub note: Option<String>,
}

pub(crate) struct ReportFacts<'a> {
    pub report_name: &'a str,
    pub dashboard_title: &'a str,
    pub frame_label: &'a str,
    pub message: &'a str,
    pub link: &'a str,
    pub widgets: Vec<ReportWidget<'a>>,
}

pub(crate) fn report(public_url: &str, subject: &str, f: &ReportFacts) -> Email {
    let mut rows = Vec::new();
    for w in &f.widgets {
        rows.push(Row::Heading(w.title.to_string()));
        if let Some(n) = &w.note {
            rows.push(Row::Pair(String::new(), Cell::Text(n.clone())));
        }
        for s in &w.series {
            rows.push(Row::Pair(s.label.to_string(), Cell::Text(format!("{} now · min {} · max {}", s.current, s.min, s.max))));
        }
    }
    if f.widgets.is_empty() {
        rows.push(Row::Pair(String::new(), Cell::Text("This dashboard has no widgets.".into())));
    }
    let frame = Frame {
        product: Product::Core,
        band_color: colors::BRAND,
        band: format!("Report · {}", f.frame_label),
        title: f.report_name.to_string(),
        lead: vec![Run::Plain("The ".into()), Run::Strong(f.dashboard_title.to_string()), Run::Plain(format!(" dashboard over the {}.", f.frame_label.to_lowercase()))],
        message: Some(f.message.to_string()),
        picture: None,
        rows,
        buttons: vec![Button::new("Open dashboard", f.link, ButtonStyle::Primary)],
        note: None,
        footer: Footer { why: "You get this because you're a recipient of this report.".into(), link_label: "Your report subscriptions".into(), url: notification_settings(public_url) },
        host: host(public_url),
    };
    Email::new(subject, &frame)
}

// ================================================================ other notices

/// Workflow, Assist and survey notifications: a teal band, the text and one button.
pub(crate) struct NoticeFacts<'a> {
    /// The band: "Workflow", "Assist", "Survey".
    pub kind: &'a str,
    pub title: &'a str,
    pub text: &'a str,
    pub button: &'a str,
    pub link: &'a str,
    /// "a workflow notifies you".
    pub why: &'a str,
}

pub(crate) fn notice(public_url: &str, subject: &str, f: &NoticeFacts) -> Email {
    let frame = Frame {
        product: Product::Core,
        band_color: colors::BRAND,
        band: f.kind.to_string(),
        title: f.title.to_string(),
        lead: Vec::new(),
        message: Some(f.text.to_string()),
        picture: None,
        rows: Vec::new(),
        buttons: if f.link.is_empty() { Vec::new() } else { vec![Button::new(f.button, f.link, ButtonStyle::Primary)] },
        note: None,
        footer: Footer { why: format!("You get this because {}.", f.why), link_label: "Notification settings".into(), url: notification_settings(public_url) },
        host: host(public_url),
    };
    Email::new(subject, &frame)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::routes::metrics::TimeAggregator;
    use crate::routes::monitor_engine::Thresholds;

    pub(crate) fn options() -> MonitorOptions {
        MonitorOptions {
            detection: Detection::Threshold,
            time_aggregation: TimeAggregator::Avg,
            window_minutes: 5,
            change_mode: None,
            change_minutes: None,
            comparator: Comparator::Above,
            thresholds: Thresholds { critical: 5.0, warning: Some(3.0), critical_recovery: None, warning_recovery: None },
            notify_no_data: false,
            no_data_minutes: 10,
            missing_resolves: false,
            require_full_window: false,
        }
    }

    pub(crate) fn facts<'a>(to: &'a str, o: &'a MonitorOptions, notified: &'a [String]) -> MonitorFacts<'a> {
        MonitorFacts {
            monitor_id: 12,
            name: "High error rate on order-service",
            kind: "apm",
            priority: Some(2),
            group: "env:prod,service:order-service",
            to,
            value: Some(7.1),
            message: "Error rate is up on order-service. Runbook: https://wiki.example.com/runbooks/order-service @ana@example.com",
            at: 1_794_841_320_000,
            test: false,
            query: "avg(last_5m):sum:trace.http.request.errors{service:order-service} / sum:trace.http.request.hits{service:order-service} * 100 > 5",
            options: Some(o),
            notified,
        }
    }

    #[test]
    fn monitor_email_says_what_happened_once() {
        let o = options();
        let notified = vec!["ana@example.com".to_string(), "oncall-checkout".to_string()];
        let e = monitor("https://oc.example.com", "[Triggered] High error rate on order-service on env:prod,service:order-service", &facts("alert", &o, &notified), None);
        // The subject isn't repeated as the first line.
        assert!(!e.text.starts_with("[Triggered]"));
        assert!(e.text.contains("The value is 7.1, above the alert threshold of 5 (avg over the last 5m)."));
        assert!(e.text.contains("Group: env:prod service:order-service\n"));
        assert!(e.text.contains("Notified: @ana@example.com · @oncall-checkout\n"));
        assert!(e.text.contains("Since: 15:02 UTC · 16 Nov 2026\n"));
        assert!(e.text.contains("View monitor: https://oc.example.com/monitors/12\n"));
        assert!(e.text.contains("Open traces: https://oc.example.com/apm/traces?query=env%3Aprod+service%3Aorder-service\n"));
        assert!(e.text.contains("Mute 1 h: https://oc.example.com/monitors/12?mute=60\n"));
        assert!(e.html.contains("bgcolor=\"#b42318\""));
        assert!(e.html.contains("Triggered · P2"));
        // No graph: no picture and only the logo.
        assert!(!e.html.contains(GRAPH_CID));
        assert_eq!(e.images.len(), 1);
    }

    #[test]
    fn recoveries_are_green_without_mute_and_tests_are_teal() {
        let o = options();
        let mut f = facts("ok", &o, &[]);
        f.value = Some(1.2);
        let e = monitor("https://oc.example.com", "[Recovered] x", &f, None);
        assert!(e.html.contains("bgcolor=\"#12805c\""));
        assert!(!e.text.contains("Mute 1 h"));
        assert!(e.text.contains("The value is back to 1.2, within the alert threshold of 5"));
        f.test = true;
        f.to = "alert";
        let e = monitor("https://oc.example.com", "[Test] [Triggered] x", &f, None);
        assert!(e.html.contains("bgcolor=\"#0b5f5c\""));
        assert!(e.text.contains("TEST · TRIGGERED"));
        for (to, color) in [("warn", colors::WARN), ("nodata", colors::NO_DATA)] {
            let f = facts(to, &o, &[]);
            assert!(monitor("https://oc.example.com", "s", &f, None).html.contains(&format!("bgcolor=\"{color}\"")));
        }
    }

    #[test]
    fn the_graph_is_an_inline_image() {
        let o = options();
        let f = facts("alert", &o, &[]);
        let png = chart::render(&monitor_chart("alert", Some(&o), (0..60).map(|i| (i * 60_000, i as f64 / 8.0)).collect(), 0, 3_600_000, 3_000_000)).unwrap();
        let e = monitor("https://oc.example.com", "s", &f, Some(png));
        assert!(e.html.contains(&format!("src=\"cid:{GRAPH_CID}\"")));
        assert_eq!(e.images.iter().map(|i| i.cid.as_str()).collect::<Vec<_>>(), vec!["logo@observecraft", GRAPH_CID]);
    }

    #[test]
    fn on_call_subjects_drop_the_monitor_tag() {
        assert_eq!(on_call_subject("[Triggered] High error rate on service:billing"), "[On-Call] High error rate on service:billing");
        assert_eq!(on_call_subject("[No data] Queue depth"), "[On-Call] Queue depth");
        assert_eq!(on_call_subject("Checkout down"), "[On-Call] Checkout down");
        assert_eq!(on_call_subject("[draft notes] are kept"), "[On-Call] [draft notes] are kept");
    }

    #[test]
    fn on_call_pages_carry_the_signed_buttons() {
        let f = PageFacts {
            page_id: 1042,
            title: "[Triggered] High error rate on order-service",
            description: "order-service 5xx",
            urgency: "high",
            team: Some("checkout"),
            source: "monitor",
            group: Some("service:order-service"),
            opened_at: 1_794_841_320_000,
            escalation: None,
            test: false,
            responder: "Ana Example",
            ack_url: Some("https://oc.example.com/api/v1/public/on-call/actions/a".into()),
            resolve_url: Some("https://oc.example.com/api/v1/public/on-call/actions/r".into()),
        };
        let e = on_call_page("https://oc.example.com", &f);
        assert_eq!(e.subject, "[On-Call] High error rate on order-service");
        assert!(e.html.contains("ObserveCraft On-Call"));
        assert!(e.html.contains("bgcolor=\"#7a1a12\""));
        assert!(e.text.contains("Page #1042 · High error rate on order-service"));
        assert!(e.text.contains("Acknowledge: https://oc.example.com/api/v1/public/on-call/actions/a\n"));
        assert!(e.text.contains("Open page: https://oc.example.com/on-call/pages/1042\n"));
        assert!(e.text.contains("Your On-Call profile: https://oc.example.com/on-call/profile"));
    }
}
