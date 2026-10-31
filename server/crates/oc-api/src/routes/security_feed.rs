//! Security → Research Feed: vulnerabilities worth knowing about, and whether they touch the org.
//!
//! - **Exploited vulnerabilities**: CISA's Known Exploited Vulnerabilities catalog (public domain),
//!   downloaded every six hours by [`run_kev_sync`] (`OC_KEV_URL`, off without internet) into the
//!   shared `security_kev` table.
//! - **Emerging vulnerabilities**: critical and high advisories published in the last 30 days for
//!   packages the org uses (Code Security's advisories).
//!
//! "Impacted" means a library of the org is affected (Code Security's matching), by CVE for the
//! catalog's entries.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::code_security::{self, CsFilter};
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const DAY: i64 = 86_400_000;
/// Catalog entries shown: added in the last 120 days.
const KEV_DAYS: i64 = 120;

pub async fn run_kev_sync(st: AppState) {
    let Some(url) = st.config.kev_url.clone() else {
        tracing::info!("the known exploited vulnerabilities catalog is off (OC_KEV_URL)");
        return;
    };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("ObserveCraft-Security").build().unwrap_or_default();
    let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
    loop {
        tick.tick().await;
        let Ok(mut conn) = st.postgres.acquire().await else { continue };
        if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(7311)").fetch_one(&mut *conn).await.unwrap_or(false) {
            continue;
        }
        match sync_kev(&st, &http, &url).await {
            Ok(n) => tracing::info!(entries = n, "known exploited vulnerabilities catalog updated"),
            Err(e) => tracing::warn!(error = %e, "known exploited vulnerabilities catalog download failed"),
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock(7311)").execute(&mut *conn).await;
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KevEntry {
    #[serde(rename = "cveID")]
    cve_id: String,
    #[serde(default)]
    vendor_project: String,
    #[serde(default)]
    product: String,
    #[serde(default)]
    vulnerability_name: String,
    date_added: String,
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    required_action: String,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    known_ransomware_campaign_use: String,
    #[serde(default)]
    cwes: Vec<String>,
}

/// Parses the catalog's JSON (`{vulnerabilities: [...]}`), skipping malformed entries.
fn parse_kev(doc: &Value) -> Vec<KevEntry> {
    doc.get("vulnerabilities").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()).unwrap_or_default()
}

async fn sync_kev(st: &AppState, http: &reqwest::Client, url: &str) -> anyhow::Result<usize> {
    let doc: Value = http.get(url).send().await?.error_for_status()?.json().await?;
    let entries = parse_kev(&doc);
    for e in &entries {
        if chrono::NaiveDate::parse_from_str(&e.date_added, "%Y-%m-%d").is_err() {
            continue;
        }
        let due = e.due_date.as_deref().filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
        sqlx::query(
            "INSERT INTO security_kev (cve_id, vendor, product, name, description, required_action, ransomware, cwes, date_added, due_date) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::date, $10::date) ON CONFLICT (cve_id) DO UPDATE SET vendor = $2, product = $3, name = $4, description = $5, \
             required_action = $6, ransomware = $7, cwes = $8, date_added = $9::date, due_date = $10::date, fetched_at = now()",
        )
        .bind(&e.cve_id)
        .bind(&e.vendor_project)
        .bind(&e.product)
        .bind(&e.vulnerability_name)
        .bind(&e.short_description)
        .bind(&e.required_action)
        .bind(e.known_ransomware_campaign_use.eq_ignore_ascii_case("known"))
        .bind(&e.cwes)
        .bind(&e.date_added)
        .bind(due)
        .execute(&st.postgres)
        .await?;
    }
    Ok(entries.len())
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: String,
    /// exploited_vulnerability | emerging_vulnerability
    pub article_type: String,
    pub title: String,
    pub summary: String,
    /// Epoch ms.
    pub published_at: i64,
    /// CISA KEV, OSV.dev, …
    pub source: String,
    pub url: String,
    pub cve: Option<String>,
    pub severity: Option<String>,
    /// Products the item concerns (Code Security, App & API Protection, Cloud SIEM).
    pub products: Vec<String>,
    pub vendor: String,
    pub product: String,
    pub ransomware: bool,
    pub required_action: String,
    pub due_date: Option<String>,
    /// Some library of the org is affected.
    pub impacted: bool,
    /// The services and repositories affected.
    pub impacted_resources: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct KevRow {
    cve_id: String,
    vendor: String,
    product: String,
    name: String,
    description: String,
    required_action: String,
    ransomware: bool,
    cwes: Vec<String>,
    added: i64,
    due_date: Option<String>,
}

/// Weaknesses an attacker reaches over HTTP (injection, traversal, deserialization, …).
const WEB_CWES: [&str; 8] = ["CWE-77", "CWE-78", "CWE-79", "CWE-89", "CWE-22", "CWE-94", "CWE-502", "CWE-918"];

#[utoipa::path(get, path = "/api/v1/security/feed", operation_id = "security_research_feed", responses((status = 200, body = Vec<FeedItem>)), tag = "security")]
pub async fn feed(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<FeedItem>> {
    Ok(Json(items(&st, &ctx.org_id).await?))
}

pub async fn items(st: &AppState, org: &str) -> Result<Vec<FeedItem>, ApiError> {
    // The org's affected libraries, by CVE and advisory.
    let vulns = code_security::list_library_vulnerabilities(State(st.clone()), OrgId(org.to_string()), Json(CsFilter { status: "all".into(), scope: "all".into(), limit: 5000, ..Default::default() }))
        .await?
        .0
        .vulnerabilities;
    let mut by_cve: HashMap<String, HashSet<String>> = HashMap::new();
    for v in &vulns {
        let resources: Vec<String> = v.services.iter().chain(v.repositories.iter()).cloned().collect();
        for id in v.cve.iter().chain(v.aliases.iter()).chain(std::iter::once(&v.advisory_id)) {
            by_cve.entry(id.clone()).or_default().extend(resources.iter().cloned());
        }
    }
    let rows: Vec<KevRow> = sqlx::query_as(
        "SELECT cve_id, vendor, product, name, description, required_action, ransomware, cwes, (extract(epoch FROM date_added) * 1000)::bigint AS added, \
         due_date::text AS due_date FROM security_kev WHERE date_added > now() - make_interval(days => $1) ORDER BY date_added DESC, cve_id LIMIT 500",
    )
    .bind(KEV_DAYS as i32)
    .fetch_all(&st.postgres)
    .await?;
    let mut out: Vec<FeedItem> = rows
        .into_iter()
        .map(|r| {
            let resources: Vec<String> = by_cve.get(&r.cve_id).map(|s| s.iter().cloned().collect()).unwrap_or_default();
            let mut products = vec!["Code Security".to_string()];
            if r.cwes.iter().any(|c| WEB_CWES.contains(&c.as_str())) {
                products.push("App & API Protection".into());
            }
            products.push("Cloud SIEM".into());
            FeedItem {
                id: r.cve_id.clone(),
                article_type: "exploited_vulnerability".into(),
                title: format!("{} ({})", r.name, r.cve_id),
                summary: r.description,
                published_at: r.added,
                source: "CISA Known Exploited Vulnerabilities".into(),
                url: format!("https://nvd.nist.gov/vuln/detail/{}", r.cve_id),
                cve: Some(r.cve_id),
                severity: None,
                products,
                vendor: r.vendor,
                product: r.product,
                ransomware: r.ransomware,
                required_action: r.required_action,
                due_date: r.due_date,
                impacted: !resources.is_empty(),
                impacted_resources: resources,
            }
        })
        .collect();
    let summary = code_security::summary(State(st.clone()), OrgId(org.to_string())).await?.0;
    let now = chrono::Utc::now().timestamp_millis();
    for a in summary.emerging.into_iter().filter(|a| matches!(a.severity.as_str(), "critical" | "high")) {
        if a.published.is_some_and(|p| p < now - 30 * DAY) {
            continue;
        }
        let resources: Vec<String> = by_cve.get(&a.id).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        let url = if a.id.starts_with("GHSA-") || a.id.starts_with("PYSEC-") { format!("https://osv.dev/vulnerability/{}", a.id) } else { String::new() };
        out.push(FeedItem {
            title: format!("{} ({})", a.summary, a.cve.clone().unwrap_or_else(|| a.id.clone())),
            id: a.id.clone(),
            article_type: "emerging_vulnerability".into(),
            summary: a.summary,
            published_at: a.published.unwrap_or(now),
            source: if url.is_empty() { "Code Security advisories".into() } else { "OSV.dev".into() },
            url,
            cve: a.cve,
            severity: Some(a.severity),
            products: vec!["Code Security".into()],
            vendor: String::new(),
            product: a.packages.join(", "),
            ransomware: false,
            required_action: String::new(),
            due_date: None,
            impacted: a.impacted || !resources.is_empty(),
            impacted_resources: resources,
        });
    }
    out.sort_by(|a, b| b.published_at.cmp(&a.published_at).then(a.id.cmp(&b.id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_entries_parse() {
        let doc = json!({"title": "CISA Catalog", "vulnerabilities": [
            {"cveID": "CVE-2026-0001", "vendorProject": "Example", "product": "Gateway", "vulnerabilityName": "Example Gateway Command Injection",
             "dateAdded": "2026-10-20", "shortDescription": "d", "requiredAction": "Apply updates.", "dueDate": "2026-11-10",
             "knownRansomwareCampaignUse": "Known", "notes": "", "cwes": ["CWE-78"]},
            {"cveID": 5}
        ]});
        let e = parse_kev(&doc);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].cve_id, "CVE-2026-0001");
        assert_eq!(e[0].cwes, ["CWE-78"]);
        assert!(e[0].known_ransomware_campaign_use.eq_ignore_ascii_case("known"));
    }
}
