//! Demo identity + incident data for development (`OC_SEED_DEMO=true`, the default outside
//! production). Installed once, when the org has no users besides the development user.
//! Every name is generated from generic first/last name lists with `@example.com` addresses.

use anyhow::Context;
use sqlx::PgPool;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn uuid(&mut self) -> String {
        let h = format!("{:016x}{:016x}", self.next(), self.next());
        format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
    }
}

const FIRST: &[&str] = &[
    "Jordan", "Taylor", "Morgan", "Casey", "Riley", "Avery", "Quinn", "Jamie", "Rowan", "Skyler", "Emerson", "Hayden", "Parker", "Reese", "Sawyer", "Dakota", "Elliot", "Finley",
    "Harper", "Kendall", "Logan", "Micah", "Noel", "Peyton", "Remy", "Sage", "Tatum", "Blair", "Cameron", "Drew", "Jesse", "Kai", "Lane", "Marley", "Oakley", "Robin",
];
const LAST: &[&str] = &[
    "Adams", "Baker", "Carter", "Diaz", "Evans", "Foster", "Garcia", "Hughes", "Ito", "Jensen", "Kim", "Lopez", "Meyer", "Nakamura", "Ortiz", "Patel", "Quinn", "Rossi", "Schmidt",
    "Turner", "Uddin", "Varga", "Walsh", "Xu", "Young", "Zimmer", "Novak", "Silva", "Kowalski", "Haddad", "Okafor", "Lindqvist",
];
const TITLES: &[&str] = &["Software Engineer", "Senior Software Engineer", "Site Reliability Engineer", "Engineering Manager", "Data Engineer", "QA Engineer", "Product Manager", "Support Engineer", "Security Engineer"];

/// (handle, name, description, custom role for its developers)
const TEAMS: &[(&str, &str, &str)] = &[
    ("platform", "Platform", "Shared infrastructure, CI/CD and the API gateway."),
    ("storefront", "Storefront", "Customer-facing web storefront and storefront-api."),
    ("checkout", "Checkout", "Cart, checkout and inventory reservation."),
    ("billing", "Billing", "Invoices, billing service and payment reconciliation."),
    ("payments", "Payments", "Payment providers and fraud checks."),
    ("catalog", "Catalog", "Product catalog, pricing and stock levels."),
    ("data-platform", "Data Platform", "Pipelines, warehouse and reporting."),
    ("mobile", "Mobile", "iOS and Android shop apps."),
    ("sre", "SRE", "Reliability, on-call tooling and incident process."),
    ("security", "Security", "Application and cloud security."),
    ("customer-support", "Customer Support", "Tier 2 support and support tooling."),
    ("notifications", "Notifications", "Email, push and the notification worker."),
    ("identity", "Identity", "Login, accounts and SSO."),
    ("search", "Search", "Product search and recommendations."),
];
const CUSTOM_ROLES: &[(&str, &str)] = &[
    ("Platform Engineers", "Manage collectors, pipelines and integrations."),
    ("Storefront Developers", "Edit storefront dashboards and monitors."),
    ("Checkout Developers", "Edit checkout dashboards and monitors."),
    ("Billing Developers", "Edit billing dashboards and monitors."),
    ("Data Analysts", "Read telemetry and export dashboards."),
    ("Support Read Access", "Read logs and traces without sensitive attributes."),
    ("Security Auditors", "Read audit trail and security signals."),
    ("Mobile Developers", "Edit RUM and mobile dashboards."),
];

/// (title, services, teams, type, detection)
const INCIDENTS: &[(&str, &[&str], &[&str], &str, &str)] = &[
    ("[Checkout] Elevated 5xx on POST /orders/submit", &["order-service", "api-gateway"], &["checkout"], "Production Incident", "monitor"),
    ("[Billing] Invoice generation delayed for EU customers", &["billing"], &["billing"], "Production Incident", "customer"),
    ("[Catalog] Stock levels stale after UPDATE_STOCK_PKG timeouts", &["catalog-service"], &["catalog"], "EventBased-Alerts", "monitor"),
    ("[Storefront] p95 latency above SLO on storefront-api", &["storefront-api"], &["storefront"], "EventBased-Alerts", "monitor"),
    ("[Notifications] Order emails queued in order-events", &["notification-worker"], &["notifications"], "Production Incident", "employee"),
    ("[Platform] Collector disk buffer filling on shop-prod-eks", &[], &["platform"], "EventBased-Alerts", "monitor"),
    ("[Payments] Card provider returning intermittent timeouts", &["api-gateway"], &["payments"], "Production Incident", "automation"),
    ("[Identity] SSO login loop for some users", &[], &["identity"], "Production Incident", "customer"),
    ("[Search] Empty results for multi-word queries", &[], &["search"], "Production Incident", "employee"),
    ("[Inventory] Reservation conflicts during flash sale", &["inventory-service"], &["checkout"], "Production Incident", "monitor"),
    ("[Data Platform] Nightly export to warehouse failed", &[], &["data-platform"], "EventBased-Alerts", "automation"),
    ("[Workflow] Tasks stuck in /workflow/tasks/{id}/complete", &["workflow-engine"], &["platform"], "EventBased-Alerts", "monitor"),
    ("[Mobile] Crash on launch for app 5.12.0 on iOS 18", &[], &["mobile"], "Production Incident", "customer"),
    ("[SRE] Nightly database backup failed", &[], &["sre"], "EventBased-Alerts", "monitor"),
    ("[SRE] Elevated error budget burn on api-gateway", &["api-gateway"], &["sre", "platform"], "EventBased-Alerts", "monitor"),
    ("[Storefront] Images not loading from CDN in eu-central-1", &["storefront-api"], &["storefront"], "Production Incident", "customer"),
    ("[Billing] Duplicate invoices for annual plans", &["billing", "billing-watcher"], &["billing"], "Production Incident", "employee"),
    ("[Checkout] Redis cache evictions slowing customer lookups", &["order-service"], &["checkout"], "EventBased-Alerts", "monitor"),
    ("[Security] Unusual login attempts from new ASN", &[], &["security", "identity"], "Security Incident", "automation"),
    ("[Catalog] Pricing service returning stale prices", &["pricing-service"], &["catalog"], "Production Incident", "monitor"),
];
const NOTES: &[&str] = &[
    "Looking at the error spike — started right after the 3.2.1 rollout.",
    "Rolled back to the previous version; error rate is dropping.",
    "Customer support confirmed reports from several merchants.",
    "Increased the connection pool size as a mitigation.",
    "Root cause looks like an N+1 query introduced last week.",
    "Monitoring for 30 minutes before marking stable.",
    "Paging the owning team for a second pair of eyes.",
    "Dashboards linked in the channel; latency back under SLO.",
];

const DEV_USER: &str = "00000000-0000-0000-0000-000000000001";
const DAY: f64 = 86_400.0;

pub async fn install(pg: &PgPool, org: &str) -> anyhow::Result<()> {
    let (users,): (i64,) = sqlx::query_as("SELECT count(*) FROM users WHERE org_id = $1").bind(org).fetch_one(pg).await?;
    if users > 1 {
        return Ok(());
    }
    tracing::info!(org, "installing demo identity and incident data");
    let mut rng = Rng(0x0b5e_c0de_2026_1025);
    let mut tx = pg.begin().await?;

    // Roles.
    let mut role_ids = Vec::new();
    for (name, desc) in CUSTOM_ROLES {
        let id = format!("{org}-{}", name.to_lowercase().replace(' ', "-"));
        sqlx::query("INSERT INTO roles (id, org_id, name, description) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING").bind(&id).bind(org).bind(name).bind(desc).execute(&mut *tx).await?;
        role_ids.push(id);
    }

    // Users.
    let mut user_ids = vec![DEV_USER.to_string()];
    let mut seen = std::collections::HashSet::new();
    while user_ids.len() < 64 {
        let first = *rng.pick(FIRST);
        let last = *rng.pick(LAST);
        if !seen.insert((first, last)) {
            continue;
        }
        let id = rng.uuid();
        let status = match rng.f() {
            x if x < 0.86 => "active",
            x if x < 0.95 => "pending",
            _ => "deactivated",
        };
        let login: &[&str] = match rng.f() {
            x if x < 0.78 => &["saml"],
            x if x < 0.9 => &["google"],
            x if x < 0.95 => &["password", "google"],
            _ => &["password"],
        };
        let last_login = if status == "active" { Some(rng.f() * 30.0 * DAY) } else { None };
        sqlx::query(
            "INSERT INTO users (id, org_id, email, name, title, status, login_methods, mfa_enabled, timezone, last_login_at, created_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, now() - make_interval(secs => $10), now() - make_interval(days => $11))",
        )
        .bind(&id)
        .bind(org)
        .bind(format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase()))
        .bind(format!("{first} {last}"))
        .bind(*rng.pick(TITLES))
        .bind(status)
        .bind(login.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .bind(rng.f() < 0.35)
        .bind(*rng.pick(&["Europe/Berlin", "Europe/London", "America/New_York", "Asia/Jerusalem", "Europe/Warsaw"]))
        .bind(last_login.unwrap_or(0.0))
        .bind(30 + rng.below(700) as i32)
        .execute(&mut *tx)
        .await?;
        if last_login.is_none() {
            sqlx::query("UPDATE users SET last_login_at = NULL WHERE id = $1::uuid").bind(&id).execute(&mut *tx).await?;
        }
        // Every user gets a built-in role plus 0–3 custom roles.
        let base = if rng.f() < 0.08 { format!("{org}-admin") } else if rng.f() < 0.85 { format!("{org}-standard") } else { format!("{org}-readonly") };
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2) ON CONFLICT DO NOTHING").bind(&id).bind(&base).execute(&mut *tx).await?;
        for _ in 0..rng.below(4) {
            sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2) ON CONFLICT DO NOTHING").bind(&id).bind(rng.pick(&role_ids)).execute(&mut *tx).await?;
        }
        user_ids.push(id);
    }

    // Teams + members (the dev user is on Platform and SRE).
    for (handle, name, desc) in TEAMS {
        let via = *rng.pick(&["ui", "ui", "saml", "api", "terraform"]);
        let notifications: Vec<String> = if rng.f() < 0.6 { vec![format!("#{handle}-alerts")] } else { vec![] };
        sqlx::query(
            "INSERT INTO teams (org_id, handle, name, description, created_via, restricted, notifications) VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (org_id, handle) DO UPDATE SET description = EXCLUDED.description, created_via = EXCLUDED.created_via, notifications = EXCLUDED.notifications",
        )
        .bind(org)
        .bind(handle)
        .bind(name)
        .bind(desc)
        .bind(via)
        .bind(rng.f() < 0.7)
        .bind(&notifications)
        .execute(&mut *tx)
        .await?;
        let n = rng.below(18) as usize;
        for i in 0..n {
            let user = rng.pick(&user_ids[1..]).clone();
            sqlx::query("INSERT INTO team_members (org_id, handle, user_id, role) VALUES ($1, $2, $3::uuid, $4) ON CONFLICT DO NOTHING")
                .bind(org)
                .bind(handle)
                .bind(&user)
                .bind(if i == 0 { "admin" } else { "member" })
                .execute(&mut *tx)
                .await?;
        }
    }
    for handle in ["platform", "sre"] {
        sqlx::query("INSERT INTO team_members (org_id, handle, user_id, role) VALUES ($1, $2, $3::uuid, 'admin') ON CONFLICT DO NOTHING").bind(org).bind(handle).bind(DEV_USER).execute(&mut *tx).await?;
    }

    // SAML group mappings (Azure AD group object ids are random UUIDs here).
    const CLAIM: &str = "http://schemas.microsoft.com/ws/2008/06/identity/claims/groups";
    for (i, (handle, ..)) in TEAMS.iter().enumerate() {
        for _ in 0..(1 + (i % 3 == 0) as u32) {
            sqlx::query("INSERT INTO saml_mappings (org_id, kind, target, attribute_key, attribute_value, created_at) VALUES ($1, 'team', $2, $3, $4, now() - make_interval(days => $5))")
                .bind(org)
                .bind(handle)
                .bind(CLAIM)
                .bind(rng.uuid())
                .bind(5 + rng.below(60) as i32)
                .execute(&mut *tx)
                .await?;
        }
    }
    for role in role_ids.iter().chain([format!("{org}-admin"), format!("{org}-standard")].iter()) {
        sqlx::query("INSERT INTO saml_mappings (org_id, kind, target, attribute_key, attribute_value, created_at) VALUES ($1, 'role', $2, $3, $4, now() - make_interval(days => $5))")
            .bind(org)
            .bind(role)
            .bind(CLAIM)
            .bind(rng.uuid())
            .bind(5 + rng.below(90) as i32)
            .execute(&mut *tx)
            .await?;
    }

    // Incidents over the last 60 days: the newest few are still open.
    let n = INCIDENTS.len();
    for (i, (title, services, teams, kind, detection)) in INCIDENTS.iter().enumerate() {
        let age_days = (n - i) as f64 * 3.0 - rng.f() * 2.0;
        let status = match n - i {
            1..=3 => "active",
            4 => "stable",
            _ => "resolved",
        };
        let sev = *rng.pick(&["SEV-1", "SEV-2", "SEV-2", "SEV-3", "SEV-3", "SEV-3", "SEV-4"]);
        let commander = rng.pick(&user_ids).clone();
        let detect = age_days * DAY;
        let declare_after = 60.0 + rng.f() * 600.0;
        let ack_after = declare_after + 60.0 + rng.f() * 300.0;
        let resolve_after = declare_after + 1800.0 + rng.f() * 2.0 * DAY;
        let stable_after = resolve_after * 0.7;
        let row: (i64,) = sqlx::query_as(
            "INSERT INTO incidents (org_id, public_id, title, status, severity, incident_type, detection_method, commander_id, declared_by, customer_impact, summary, root_cause, services, teams, \
               detected_at, declared_at, acknowledged_at, stable_at, resolved_at, modified_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, $9, $10, $11, $12, $13, $14, \
               now() - make_interval(secs => $15), now() - make_interval(secs => $15 - $16), now() - make_interval(secs => $15 - $17), \
               CASE WHEN $4 IN ('stable', 'resolved') THEN now() - make_interval(secs => greatest($15 - $18, 0)) END, \
               CASE WHEN $4 = 'resolved' THEN now() - make_interval(secs => greatest($15 - $19, 0)) END, now()) RETURNING id",
        )
        .bind(org)
        .bind(80 + i as i32)
        .bind(title)
        .bind(status)
        .bind(sev)
        .bind(kind)
        .bind(detection)
        .bind(&commander)
        .bind(if *detection == "monitor" || *detection == "automation" { "ObserveCraft Incident Management" } else { "Alex Morgan" })
        .bind(rng.f() < 0.4)
        .bind(if rng.f() < 0.6 { format!("{title}. Impact is being assessed; see the timeline for updates.") } else { String::new() })
        .bind(if status == "resolved" && rng.f() < 0.7 { rng.pick(NOTES).to_string() } else { String::new() })
        .bind(services.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .bind(teams.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .bind(detect)
        .bind(declare_after)
        .bind(ack_after)
        .bind(stable_after)
        .bind(resolve_after)
        .fetch_one(&mut *tx)
        .await?;
        let id = row.0;
        let at = |secs: f64| format!("{}", detect - secs);
        let mut events: Vec<(&str, String, String, String)> = vec![("declared", "ObserveCraft Incident Management".into(), format!("Incident declared with severity {sev}"), at(declare_after))];
        for k in 0..(1 + rng.below(4)) {
            events.push(("note", format!("{} {}", rng.pick(FIRST), rng.pick(LAST)), rng.pick(NOTES).to_string(), at(ack_after + k as f64 * 900.0)));
        }
        events.push(("notification", "ObserveCraft Incident Management".into(), format!("Incident #{}: {title} — sent to #{}-alerts", 80 + i, teams.first().unwrap_or(&"sre")), at(declare_after + 5.0)));
        if status != "active" {
            events.push(("status", "Alex Morgan".into(), "Status changed from Active to Stable".into(), at(stable_after)));
        }
        if status == "resolved" {
            events.push(("status", "Alex Morgan".into(), "Status changed from Stable to Resolved".into(), at(resolve_after)));
        }
        for (kind, author, body, ago) in events {
            sqlx::query("INSERT INTO incident_events (incident_id, kind, author, body, created_at) VALUES ($1, $2, $3, $4, now() - make_interval(secs => greatest($5::float8, 0)))")
                .bind(id)
                .bind(kind)
                .bind(author)
                .bind(body)
                .bind(ago.parse::<f64>().unwrap_or(0.0))
                .execute(&mut *tx)
                .await?;
        }
        for _ in 0..rng.below(3) {
            let r = rng.pick(&user_ids).clone();
            sqlx::query("INSERT INTO incident_responders (incident_id, user_id) VALUES ($1, $2::uuid) ON CONFLICT DO NOTHING").bind(id).bind(&r).execute(&mut *tx).await?;
        }
        if status == "resolved" && rng.f() < 0.5 {
            sqlx::query("INSERT INTO incident_followups (incident_id, title, done) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(*rng.pick(&["Add a monitor for queue depth", "Write the post-incident review", "Load-test the checkout path before the next sale", "Add a runbook link to the alert"]))
                .bind(rng.f() < 0.5)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await.context("commit demo data")?;
    Ok(())
}

/// (name, type, query, options, message, priority, tags, teams, draft)
type MonitorSeed = (&'static str, &'static str, &'static str, &'static str, &'static str, Option<i16>, &'static [&'static str], &'static [&'static str], bool);

/// Options shorthand: (detection/time aggregation/window, comparator, critical, warning, recovery, notify no data).
const MONITORS: &[MonitorSeed] = &[
    (
        "[Storefront] High CPU on {{host.name}}",
        "metric",
        "avg:system.cpu.user{*} by {host}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":48,"warning":44,"criticalRecovery":46},"notifyNoData":false,"noDataMinutes":10}"#,
        "{{#is_alert}}CPU is at {{value}}% on {{host.name}}. @slack-storefront-alerts @pagerduty-storefront{{/is_alert}}\n{{#is_warning}}CPU is climbing on {{host.name}} ({{value}}%). @slack-storefront-alerts{{/is_warning}}\n{{#is_recovery}}CPU back to normal on {{host.name}}. @slack-storefront-alerts{{/is_recovery}}",
        Some(2),
        &["team:storefront", "env:prod", "service:storefront-api"],
        &["storefront"],
        false,
    ),
    (
        "[Platform] Load average is high on {{host.name}}",
        "metric",
        "avg:system.load.1{*} by {host}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":6.5,"warning":5.5},"notifyNoData":false,"noDataMinutes":10}"#,
        "Load is {{value}} on {{host.name}}. {{#is_alert}}@pagerduty-platform{{/is_alert}} @slack-platform-alerts",
        Some(3),
        &["team:platform", "env:prod"],
        &["platform"],
        false,
    ),
    (
        "[Platform] Disk space is running low on {{host.name}}",
        "metric",
        "max:system.disk.in_use{*} by {host}",
        r#"{"detection":"threshold","timeAggregation":"max","windowMinutes":15,"comparator":"above","thresholds":{"critical":80,"warning":75},"notifyNoData":true,"noDataMinutes":30}"#,
        "{{host.name}} is {{value}}% full. Clean up old logs or grow the volume. @sre-oncall@example.com @slack-platform-alerts",
        Some(2),
        &["team:platform", "env:prod"],
        &["platform", "sre"],
        false,
    ),
    (
        "[Platform] Memory usage above 90% on {{host.name}}",
        "metric",
        "avg:system.mem.pct_used{*} by {host}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":10,"comparator":"above","thresholds":{"critical":90,"warning":80},"notifyNoData":false,"noDataMinutes":10}"#,
        "Memory at {{value}}% on {{host.name}}. @slack-platform-alerts",
        Some(3),
        &["team:platform", "env:prod"],
        &["platform"],
        false,
    ),
    (
        "[Checkout] order-service error rate is high",
        "apm",
        "avg:trace.error_rate{service:order-service}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":4,"warning":3},"notifyNoData":true,"noDataMinutes":10}"#,
        "{{#is_alert}}order-service is failing {{value}}% of requests. Check recent deploys and the payment provider status. @pagerduty-checkout @slack-checkout-alerts{{/is_alert}}{{#is_warning}}order-service errors at {{value}}%. @slack-checkout-alerts{{/is_warning}}{{#is_recovery}}order-service error rate recovered. @slack-checkout-alerts{{/is_recovery}}",
        Some(1),
        &["team:checkout", "env:prod", "service:order-service"],
        &["checkout"],
        false,
    ),
    (
        "[Storefront] storefront-api p95 latency above 200ms",
        "apm",
        "avg:trace.latency.p95{service:storefront-api}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":10,"comparator":"above","thresholds":{"critical":200,"warning":150},"notifyNoData":true,"noDataMinutes":15}"#,
        "storefront-api p95 is {{value}} ms. @slack-storefront-alerts",
        Some(2),
        &["team:storefront", "env:prod", "service:storefront-api"],
        &["storefront"],
        false,
    ),
    (
        "[APM] p95 latency is high on {{service.name}}",
        "apm",
        "avg:trace.latency.p95{env:prod} by {service}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":180,"warning":120},"notifyNoData":false,"noDataMinutes":10}"#,
        "{{service.name}} p95 latency is {{value}} ms. @webhook-incident-bot @team-sre",
        Some(3),
        &["team:sre", "env:prod"],
        &["sre"],
        false,
    ),
    (
        "[Billing] billing error rate above 5%",
        "apm",
        "avg:trace.error_rate{service:billing}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":10,"comparator":"above","thresholds":{"critical":5},"notifyNoData":true,"noDataMinutes":15}"#,
        "billing is failing {{value}}% of requests.",
        Some(2),
        &["team:billing", "env:prod", "service:billing"],
        &["billing"],
        false,
    ),
    (
        "[Platform] api-gateway request volume dropped",
        "apm",
        "sum:trace.requests{service:api-gateway}",
        r#"{"detection":"change","timeAggregation":"avg","windowMinutes":5,"changeMode":"pct_change","changeMinutes":60,"comparator":"below","thresholds":{"critical":-50,"warning":-30},"notifyNoData":true,"noDataMinutes":10}"#,
        "api-gateway traffic changed by {{value}}% compared with an hour ago. @pagerduty-platform",
        Some(1),
        &["team:platform", "env:prod", "service:api-gateway"],
        &["platform"],
        false,
    ),
    (
        "[Notifications] notification-worker errors",
        "apm",
        "sum:trace.errors{service:notification-worker}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":1,"comparator":"above","thresholds":{"critical":0.06,"warning":0.05},"notifyNoData":false,"noDataMinutes":10}"#,
        "notification-worker is failing {{value}} jobs/s. @slack-notifications-alerts",
        None,
        &["team:notifications", "env:prod", "service:notification-worker"],
        &["notifications"],
        false,
    ),
    (
        "[Workflow] workflow-engine queue depth",
        "metric",
        "avg:workflow.queue.depth{env:prod}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":1000,"warning":500},"notifyNoData":true,"noDataMinutes":10}"#,
        "Workflow queue depth is {{value}}. @slack-platform-alerts",
        Some(4),
        &["team:platform", "env:prod", "service:workflow-engine"],
        &["platform"],
        false,
    ),
    (
        "[deprecated] api-gateway p99 latency",
        "apm",
        "avg:trace.latency.p99{service:api-gateway}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":5,"comparator":"above","thresholds":{"critical":150},"notifyNoData":false,"noDataMinutes":10}"#,
        "Superseded by the SLO-based alert. @slack-platform-alerts",
        Some(5),
        &["team:platform", "env:prod", "service:api-gateway"],
        &["platform"],
        false,
    ),
    (
        "[Inventory] inventory-service error budget burn",
        "apm",
        "avg:trace.error_rate{service:inventory-service}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":30,"comparator":"above","thresholds":{"critical":2,"warning":1.5},"notifyNoData":false,"noDataMinutes":10}"#,
        "inventory-service is burning its error budget ({{value}}%). @slack-checkout-alerts",
        Some(3),
        &["team:checkout", "env:prod", "service:inventory-service"],
        &["checkout"],
        true,
    ),
    (
        "[Workflow] workflow-engine has no traffic",
        "apm",
        "sum:trace.requests{service:workflow-engine}",
        r#"{"detection":"threshold","timeAggregation":"avg","windowMinutes":10,"comparator":"below","thresholds":{"critical":0.5},"notifyNoData":true,"noDataMinutes":10}"#,
        "workflow-engine stopped receiving work. @pagerduty-platform @team-platform",
        Some(2),
        &["team:platform", "env:prod", "service:workflow-engine"],
        &["platform"],
        false,
    ),
];

/// Demo monitors with a day of replayed history, plus a few downtimes. Installed once, when the org
/// has no monitors, after the API state exists (history is computed from the stored telemetry).
pub async fn install_monitors(st: &crate::state::AppState, org: &str) -> anyhow::Result<()> {
    let pg = &st.postgres;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM monitors WHERE org_id = $1").bind(org).fetch_one(pg).await?;
    if n > 0 {
        return Ok(());
    }
    tracing::info!(org, "installing demo monitors");
    let creators: Vec<String> = sqlx::query_scalar("SELECT id::text FROM users WHERE org_id = $1 AND status = 'active' ORDER BY id LIMIT 6").bind(org).fetch_all(pg).await?;
    let mut rng = Rng(0x0b5e_0b5e_2026_1025);
    let mut ids = Vec::new();
    for (name, kind, query, options, message, priority, tags, teams, draft) in MONITORS {
        let def: serde_json::Value = serde_json::from_str(options).with_context(|| format!("monitor seed {name}"))?;
        let creator = (!creators.is_empty()).then(|| rng.pick(&creators).clone());
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO monitors (org_id, name, type, query, definition, status, message, priority, tags, teams, draft, created_by, created_at, modified_at) \
             VALUES ($1, $2, $3, $4, $5, 'nodata', $6, $7, $8, $9, $10, $11::uuid, now() - make_interval(days => $12::int), now() - make_interval(hours => $13::int)) RETURNING id",
        )
        .bind(org)
        .bind(name)
        .bind(kind)
        .bind(query)
        .bind(&def)
        .bind(message)
        .bind(priority)
        .bind(tags.iter().map(|t| t.to_string()).collect::<Vec<_>>())
        .bind(teams.iter().map(|t| t.to_string()).collect::<Vec<_>>())
        .bind(draft)
        .bind(creator)
        .bind((30 + rng.below(300)) as i32)
        .bind((1 + rng.below(400)) as i32)
        .fetch_one(pg)
        .await?;
        ids.push((id, *name));
    }
    let id_of = |prefix: &str| ids.iter().find(|(_, n)| n.starts_with(prefix)).map(|(id, _)| *id);
    let by = creators.first().cloned();
    // Muted indefinitely since long ago (shows up in the quality report).
    sqlx::query("INSERT INTO downtimes (org_id, monitor_id, scope, message, start_at, created_by, created_at) VALUES ($1, $2, '{}', 'Replaced by the SLO alert', now() - interval '45 days', $3::uuid, now() - interval '45 days')")
        .bind(org)
        .bind(id_of("[deprecated]"))
        .bind(&by)
        .execute(pg)
        .await?;
    // Nightly maintenance window for every platform monitor.
    sqlx::query(
        "INSERT INTO downtimes (org_id, monitor_tags, scope, message, start_at, end_at, recurrence, created_by) \
         VALUES ($1, '{team:platform}', '{}', 'Nightly maintenance 01:00-03:00 UTC', date_trunc('day', now()) + interval '1 hour', date_trunc('day', now()) + interval '3 hours', 'daily', $2::uuid)",
    )
    .bind(org)
    .bind(&by)
    .execute(pg)
    .await?;
    // Host patching right now: silences host 4 for every monitor.
    sqlx::query(
        "INSERT INTO downtimes (org_id, scope, message, start_at, end_at, created_by) \
         VALUES ($1, '{host:i-0a1b2c3d4e5f60004}', 'Kernel patching on i-0a1b2c3d4e5f60004', now() - interval '20 minutes', now() + interval '100 minutes', $2::uuid)",
    )
    .bind(org)
    .bind(creators.get(1).or(by.as_ref()))
    .execute(pg)
    .await?;
    // Scheduled for the weekend.
    sqlx::query(
        "INSERT INTO downtimes (org_id, monitor_id, scope, message, start_at, end_at, created_by) \
         VALUES ($1, $2, '{service:catalog-service}', 'Planned catalog reindex', date_trunc('week', now()) + interval '5 days 22 hours', date_trunc('week', now()) + interval '6 days 2 hours', $3::uuid)",
    )
    .bind(org)
    .bind(id_of("[APM] p95"))
    .bind(creators.get(2).or(by.as_ref()))
    .execute(pg)
    .await?;
    for (id, name) in &ids {
        if let Err(e) = crate::routes::monitors::backfill(st, *id).await {
            tracing::warn!(monitor = name, error = %e, "demo monitor history not replayed");
        }
    }
    Ok(())
}
