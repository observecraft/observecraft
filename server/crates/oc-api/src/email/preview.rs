//! Renders every kind of email to files for review:
//! `cargo test -p oc-api email::preview -- --ignored` writes `<name>.eml` (the message as sent) and
//! `<name>.html` (images inlined as `data:` URIs, for a browser) to `OC_EMAIL_PREVIEW_DIR`, else
//! `server/target/email-previews`. Demo data only.

use base64::Engine;

use super::templates::{self, tests::facts, tests::options, PageFacts, ReportFacts, ReportSeries, ReportWidget};
use super::{chart, Email};

const URL: &str = "https://observecraft.example.com";
const AT: i64 = 1_794_841_320_000;

fn series(rising: bool) -> Vec<(i64, f64)> {
    (0..=60)
        .map(|i| {
            let t = AT - 50 * 60_000 + i * 60_000;
            let wobble = ((i * 7919) % 13) as f64 / 13.0 * 0.5;
            let v = if rising { if i < 36 { 2.2 + wobble } else { 2.2 + (i - 36) as f64 * 0.22 + wobble } } else if i < 20 { 7.2 + wobble } else { (7.2 - (i - 20) as f64 * 0.3).max(1.8) + wobble };
            (t, v)
        })
        .collect()
}

fn write(dir: &std::path::Path, name: &str, e: &Email) {
    let msg = e.message("ObserveCraft <alerts@observecraft.example.com>".parse().unwrap(), "ana@example.com".parse().unwrap()).unwrap();
    std::fs::write(dir.join(format!("{name}.eml")), msg.formatted()).unwrap();
    let mut html = e.html.clone();
    for img in &e.images {
        html = html.replace(&format!("cid:{}", img.cid), &format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&img.png)));
    }
    std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
}

#[test]
#[ignore = "writes preview files; run with --ignored"]
fn write_previews() {
    let dir = std::env::var("OC_EMAIL_PREVIEW_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/email-previews"));
    std::fs::create_dir_all(&dir).unwrap();
    let o = options();
    let notified = vec!["oncall-checkout".to_string(), "slack-alerts-checkout".to_string()];

    let alert = facts("alert", &o, &notified);
    let graph = chart::render(&templates::monitor_chart("alert", Some(&o), series(true), AT - 50 * 60_000, AT + 10 * 60_000, AT));
    write(&dir, "monitor-triggered", &templates::monitor(URL, "[Triggered] High error rate on order-service on env:prod,service:order-service", &alert, graph));

    let mut ok = facts("ok", &o, &notified);
    ok.value = Some(2.1);
    let graph = chart::render(&templates::monitor_chart("ok", Some(&o), series(false), AT - 50 * 60_000, AT + 10 * 60_000, AT));
    write(&dir, "monitor-recovered", &templates::monitor(URL, "[Recovered] High error rate on order-service on env:prod,service:order-service", &ok, graph));

    let page = PageFacts {
        page_id: 1042,
        title: "[Triggered] High error rate on order-service on env:prod,service:order-service",
        description: "Error rate is up on order-service. Runbook: https://wiki.example.com/runbooks/order-service",
        urgency: "high",
        team: Some("checkout"),
        source: "monitor",
        group: Some("env:prod,service:order-service"),
        opened_at: AT,
        escalation: None,
        test: false,
        responder: "Ana Example",
        ack_url: Some(format!("{URL}/api/v1/public/on-call/actions?token=example-acknowledge")),
        resolve_url: Some(format!("{URL}/api/v1/public/on-call/actions?token=example-resolve")),
    };
    write(&dir, "on-call-page", &templates::on_call_page(URL, &page));
    let test = PageFacts { page_id: 0, title: "Test notification", description: "This is how ObserveCraft On-Call reaches you when you're paged.", team: None, source: "manual", group: None, test: true, ack_url: None, resolve_url: None, ..page };
    write(&dir, "on-call-test", &templates::on_call_page(URL, &test));

    write(&dir, "verification", &templates::verification(URL, "ana@example.com", &format!("{URL}/api/v1/auth/verify-email/ocev_example")));

    let s = |label: &'static str, current: &str, min: &str, max: &str| ReportSeries { label, current: current.into(), min: min.into(), max: max.into() };
    let report = ReportFacts {
        report_name: "Checkout weekly",
        dashboard_title: "Checkout overview",
        frame_label: "Past 1 Week",
        message: "Numbers for Monday's review.",
        link: &format!("{URL}/dashboards/abc-123?time=1w"),
        widgets: vec![
            ReportWidget { title: "Requests per second", series: vec![s("service:order-service", "412", "120", "980"), s("service:billing", "88", "31", "240")], note: None },
            ReportWidget { title: "Error rate", series: vec![s("service:order-service", "0.4", "0.1", "7.1")], note: None },
            ReportWidget { title: "Deploys", series: vec![], note: Some("See the dashboard".into()) },
        ],
    };
    write(&dir, "report", &templates::report(URL, "[Report] Checkout overview · Past 1 Week", &report));
    eprintln!("previews in {}", dir.display());
}
