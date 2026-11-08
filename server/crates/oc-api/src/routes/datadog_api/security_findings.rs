//! Datadog's security findings API for Cloud Security's account scans (`ListSecurityFindings`,
//! `SearchSecurityFindings`, `MuteSecurityFindings`): `GET /api/v2/security/findings`,
//! `POST /api/v2/security/findings/search` and `PATCH /api/v2/security/findings/mute`.
//!
//! - Findings are the scanner's misconfigurations (`crate::posture`, Postgres `posture_findings`), documents and
//!   `filter[query]` in [`crate::posture::datadog`]. Datadog's legacy `/api/v2/posture_management/findings` is
//!   deprecated upstream and not served.
//! - Paging: `page[limit]` (1–150, default 10), `meta.page.after` / `page[cursor]` an opaque offset;
//!   `sort` `@detection_changed_at` or `-@detection_changed_at` (the default, newest first).
//! - Mute: up to 100 findings, `is_muted` with a reason Datadog allows for that direction, an `expire_at` in the
//!   future; unknown ids are a 404. Muting needs the Standard role.
//! - Errors are JSON:API `{"errors": [{"status", "title", "detail"}]}`, as these endpoints answer.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use super::DdAuth;
use crate::auth::Access;
use crate::posture::{datadog, store};
use crate::state::AppState;

type FindingsResult = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

const MUTE_REASONS: [&str; 6] = ["PENDING_FIX", "FALSE_POSITIVE", "OTHER", "NO_FIX", "DUPLICATE", "RISK_ACCEPTED"];
const UNMUTE_REASONS: [&str; 4] = ["NO_PENDING_FIX", "HUMAN_ERROR", "NO_LONGER_ACCEPTED_RISK", "OTHER"];

fn error(status: StatusCode, title: &str, detail: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "errors": [{ "status": status.as_u16().to_string(), "title": title, "detail": detail }] })))
}

fn bad_request(detail: &str) -> (StatusCode, Json<Value>) {
    error(StatusCode::BAD_REQUEST, "Bad Request", detail)
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "security findings request failed");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error", "internal error")
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn cursor(offset: usize) -> String {
    B64.encode(format!("{{\"offset\":{offset}}}"))
}

fn offset(cursor: &str) -> Option<usize> {
    let v: Value = serde_json::from_slice(&B64.decode(cursor).ok()?).ok()?;
    v.get("offset")?.as_u64().map(|n| n as usize)
}

/// One page of the org's findings matching `query`.
async fn page(st: &AppState, org: &str, query: &str, cursor_in: Option<&str>, limit: Option<i64>, sort: Option<&str>) -> FindingsResult {
    let started = std::time::Instant::now();
    let limit = limit.unwrap_or(10);
    if !(1..=150).contains(&limit) {
        return Err(bad_request("page[limit] must be between 1 and 150"));
    }
    let ascending = match sort.unwrap_or("-@detection_changed_at") {
        "-@detection_changed_at" => false,
        "@detection_changed_at" => true,
        other => return Err(bad_request(&format!("invalid sort {other}: use @detection_changed_at or -@detection_changed_at"))),
    };
    let start = match cursor_in {
        Some(c) => offset(c).ok_or_else(|| bad_request("invalid page[cursor]"))?,
        None => 0,
    };
    let q = datadog::parse_query(if query.trim().is_empty() { "*" } else { query }).map_err(|e| bad_request(&format!("invalid filter[query]: {e}")))?;
    let mut rows: Vec<store::FindingRow> = store::findings(&st.postgres, org, false).await.map_err(db_err)?.into_iter().filter(|r| q.matches(r)).collect();
    if ascending {
        rows.reverse();
    }
    let data: Vec<Value> = rows.iter().skip(start).take(limit as usize).map(datadog::item).collect();
    let request_id = format!("{:032x}", rand::random::<u128>());
    let mut body = json!({ "data": data, "meta": { "elapsed": started.elapsed().as_millis() as u64, "request_id": request_id, "status": "done" } });
    if start + (limit as usize) < rows.len() {
        let after = cursor(start + limit as usize);
        body["meta"]["page"] = json!({ "after": after });
        let mut next = format!("/api/v2/security/findings?page%5Bcursor%5D={after}&page%5Blimit%5D={limit}");
        if query.trim() != "*" && !query.trim().is_empty() {
            next.push_str(&format!("&filter%5Bquery%5D={}", url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()));
        }
        body["links"] = json!({ "next": next });
    }
    Ok((StatusCode::OK, Json(body)))
}

#[derive(Deserialize, Default)]
pub struct ListParams {
    #[serde(rename = "filter[query]")]
    query: Option<String>,
    #[serde(rename = "page[cursor]")]
    cursor: Option<String>,
    #[serde(rename = "page[limit]")]
    limit: Option<i64>,
    sort: Option<String>,
}

#[utoipa::path(get, path = "/api/v2/security/findings", operation_id = "datadog_list_security_findings",
    params(("filter[query]" = Option<String>, Query), ("page[cursor]" = Option<String>, Query), ("page[limit]" = Option<i64>, Query), ("sort" = Option<String>, Query)),
    responses((status = 200, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(p): Query<ListParams>) -> FindingsResult {
    page(&st, &ctx.org_id, p.query.as_deref().unwrap_or("*"), p.cursor.as_deref(), p.limit, p.sort.as_deref()).await
}

#[utoipa::path(post, path = "/api/v2/security/findings/search", operation_id = "datadog_search_security_findings", request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn search(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Option<Json<Value>>) -> FindingsResult {
    let body = body.map(|Json(v)| v).unwrap_or(Value::Null);
    let Some(data) = body.get("data").filter(|d| d.is_object()) else {
        return Err(bad_request("the body is {\"data\": {\"attributes\": {\"filter\", \"page\", \"sort\"}}}"));
    };
    let attrs = data.get("attributes").cloned().unwrap_or(json!({}));
    let query = match attrs.get("filter") {
        None | Some(Value::Null) => "*".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(bad_request("data.attributes.filter must be a string")),
    };
    let limit = match attrs.pointer("/page/limit") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| bad_request("data.attributes.page.limit must be an integer"))?),
    };
    let cursor = attrs.pointer("/page/cursor").and_then(Value::as_str);
    let sort = attrs.get("sort").and_then(Value::as_str);
    page(&st, &ctx.org_id, &query, cursor, limit, sort).await
}

#[utoipa::path(patch, path = "/api/v2/security/findings/mute", operation_id = "datadog_mute_security_findings", request_body = Object,
    responses((status = 202, body = Object), (status = 400, body = Object), (status = 404, body = Object), (status = 422, body = Object)), tag = "datadog-api")]
pub async fn mute(State(st): State<AppState>, DdAuth(ctx): DdAuth, Json(body): Json<Value>) -> FindingsResult {
    super::users::require(&ctx, Access::Standard).map_err(|e| error(e.status, "Forbidden", e.errors.first().map(String::as_str).unwrap_or("Forbidden")))?;
    let m = body.pointer("/data/attributes/mute").ok_or_else(|| bad_request("data.attributes.mute is required"))?;
    let is_muted = m.get("is_muted").and_then(Value::as_bool).ok_or_else(|| bad_request("mute.is_muted is required"))?;
    let reason = m.get("reason").and_then(Value::as_str).ok_or_else(|| bad_request("mute.reason is required"))?;
    let allowed: &[&str] = if is_muted { &MUTE_REASONS } else { &UNMUTE_REASONS };
    if !allowed.contains(&reason) {
        let detail = format!("Invalid reason {reason} to {} findings: use one of {}", if is_muted { "mute" } else { "unmute" }, allowed.join(", "));
        return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid Request", &detail));
    }
    let description = m.get("description").and_then(Value::as_str).unwrap_or_default();
    let expire_at = match m.get("expire_at") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| bad_request("mute.expire_at must be a timestamp in milliseconds"))?),
    };
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    if is_muted && expire_at.is_some_and(|t| t <= now_ms) {
        return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid Request", "Invalid expiration date. Please provide an expiration date in the future"));
    }
    let refs = body.pointer("/data/relationships/findings/data").and_then(Value::as_array).ok_or_else(|| bad_request("data.relationships.findings.data is required"))?;
    if refs.is_empty() || refs.len() > 100 {
        return Err(bad_request("between 1 and 100 findings can be muted per request"));
    }
    let mut ids = vec![];
    for r in refs {
        let api_id = r.get("id").and_then(Value::as_str).unwrap_or_default();
        match datadog::row_id(api_id) {
            Some(id) => ids.push(id),
            None => return Err(error(StatusCode::NOT_FOUND, "Not Found", &format!("finding {api_id} not found"))),
        }
    }
    let found = store::existing(&st.postgres, &ctx.org_id, &ids).await.map_err(db_err)?;
    if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
        return Err(error(StatusCode::NOT_FOUND, "Not Found", &format!("finding {missing} not found")));
    }
    store::set_mute(&st.postgres, &ctx.org_id, &ids, is_muted.then_some((reason, description, expire_at)), &ctx.name).await.map_err(db_err)?;
    let detail = json!({ "reason": reason, "description": description, "expire_at": expire_at }).to_string();
    ctx.audit(&st, if is_muted { "security_finding.mute" } else { "security_finding.unmute" }, &ids.join(","), &detail).await;
    let id = format!("{:032x}", rand::random::<u128>());
    let id = format!("{}-{}-{}-{}-{}", &id[0..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32]);
    Ok((StatusCode::ACCEPTED, Json(json!({ "data": { "id": id, "type": "mute" } }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posture::ocsf::{Check, Evaluation, Results};
    use crate::routes::datadog_api::replay::{Feature, TestServer};

    /// Gives the server's org an AWS account and a stored scan with `n` failing high-severity findings.
    async fn seed(s: &TestServer, n: usize) {
        let pg = &s.st.postgres;
        sqlx::query("DELETE FROM aws_accounts WHERE org_id = $1").bind(&s.org).execute(pg).await.unwrap();
        let account: String = sqlx::query_scalar(
            "INSERT INTO aws_accounts (org_id, aws_account_id, role_name, external_id, resources_config) \
             VALUES ($1, '123456789012', 'ObserveCraftIntegrationRole', 'ext-0123', '{\"cloud_security_posture_management_collection\": true}') RETURNING id::text",
        )
        .bind(&s.org)
        .fetch_one(pg)
        .await
        .unwrap();
        let scan: String = sqlx::query_scalar("INSERT INTO posture_scans (org_id, provider, account_ref, account_id, status) VALUES ($1, 'aws', $2::uuid, '123456789012', 'running') RETURNING id::text")
            .bind(&s.org)
            .bind(&account)
            .fetch_one(pg)
            .await
            .unwrap();
        let mut results = Results { tool_version: "5.47.0".into(), ..Default::default() };
        let check = "s3_bucket_level_public_access_block";
        results.checks.insert(
            check.into(),
            Check {
                check_id: check.into(),
                title: "Ensure S3 buckets are not publicly accessible".into(),
                severity: "high".into(),
                service: "s3".into(),
                resource_type: "AwsS3Bucket".into(),
                description: "Checks whether buckets allow public access.".into(),
                risk: String::new(),
                remediation: "Turn on Block Public Access.".into(),
                reference_urls: vec![],
                compliance: [("CIS-3.0".to_string(), vec!["2.1.4".to_string()])].into(),
            },
        );
        for i in 0..n {
            results.evaluations.push(Evaluation {
                check_id: check.into(),
                pass: false,
                severity: "high".into(),
                detail: format!("S3 Bucket example-shop-assets-{i} is public."),
                resource_uid: format!("arn:aws:s3:::example-shop-assets-{i}"),
                resource_name: format!("example-shop-assets-{i}"),
                resource_type: "AwsS3Bucket".into(),
                region: "eu-central-1".into(),
                account_id: "123456789012".into(),
                tags: vec!["team:storefront".into()],
            });
        }
        let claimed = store::Claimed { id: scan, org_id: s.org.clone(), provider: "aws".into(), account_ref: account, account_id: "123456789012".into() };
        store::store_results(pg, &claimed, &results).await.unwrap();
    }

    #[tokio::test]
    async fn replays_datadogs_security_findings_scenarios() {
        let Some(f) = Feature::load("v2/security_monitoring") else { return };
        let Some(s) = TestServer::start("dd-findings-replay").await else { return };
        // More than a default page (10), as Datadog's recordings are.
        seed(&s, 12).await;
        let mut replayer = s.replayer();
        // Datadog's recordings are host vulnerabilities; ours are misconfigurations, so the finding document itself
        // isn't compared (its fields are, in `the_document_has_datadogs_fields`).
        replayer.ignore = vec!["data[].attributes.attributes.custom".into()];
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            let n = scenario.name.as_str();
            let ours = (n.starts_with("List security findings") || n.starts_with("Search security findings") || n.starts_with("Mute security findings") || n.starts_with("Unmute security findings"))
                // These mute findings by Datadog's own ids, which no org of ours has.
                && !n.ends_with("\"Accepted\" response");
            // The feature's own replay (security_monitoring.rs) leaves findings out, so their cassettes are read here.
            let Some(interactions) = (if ours { f.cassette_named(n) } else { None }) else { continue };
            let (_, p) = replayer.replay_interactions(f.sends(scenario), n, &interactions).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= 10, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn findings_are_listed_filtered_paged_and_muted() {
        let Some(s) = TestServer::start("dd-findings").await else { return };
        seed(&s, 3).await;
        let http = reqwest::Client::new();
        let call = |method: reqwest::Method, path: &str, body: Option<Value>| {
            let mut req = http.request(method, format!("{}{path}", s.base)).header("DD-API-KEY", &s.api_key).header("DD-APPLICATION-KEY", &s.app_key);
            if let Some(b) = body {
                req = req.json(&b);
            }
            async move {
                let res = req.send().await.unwrap();
                let status = res.status().as_u16();
                (status, res.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let (status, page1) = call(reqwest::Method::GET, "/api/v2/security/findings?page[limit]=2&filter[query]=@severity:high%20@status:open", None).await;
        assert_eq!(status, 200);
        assert_eq!(page1["data"].as_array().unwrap().len(), 2);
        let after = page1["meta"]["page"]["after"].as_str().unwrap().to_string();
        let (_, page2) = call(reqwest::Method::GET, &format!("/api/v2/security/findings?page[limit]=2&page[cursor]={after}"), None).await;
        assert_eq!(page2["data"].as_array().unwrap().len(), 1);
        assert!(page2["meta"].get("page").is_none());

        let id = page1["data"][0]["id"].as_str().unwrap().to_string();
        let mute = |is_muted: bool, reason: &str| json!({ "data": { "type": "mute", "attributes": { "mute": { "is_muted": is_muted, "reason": reason, "description": "Public assets" } }, "relationships": { "findings": { "data": [{ "id": id, "type": "findings" }] } } } });
        assert_eq!(call(reqwest::Method::PATCH, "/api/v2/security/findings/mute", Some(mute(true, "RISK_ACCEPTED"))).await.0, 202);
        let (_, muted) = call(reqwest::Method::POST, "/api/v2/security/findings/search", Some(json!({ "data": { "attributes": { "filter": "@status:muted" } } }))).await;
        assert_eq!(muted["data"].as_array().unwrap().len(), 1);
        assert_eq!(muted["data"][0]["attributes"]["attributes"]["custom"]["workflow"]["mute"]["reason"], "RISK_ACCEPTED");
        assert_eq!(call(reqwest::Method::PATCH, "/api/v2/security/findings/mute", Some(mute(false, "RISK_ACCEPTED"))).await.0, 422, "not a reason to unmute");
        assert_eq!(call(reqwest::Method::PATCH, "/api/v2/security/findings/mute", Some(mute(false, "NO_LONGER_ACCEPTED_RISK"))).await.0, 202);
        let (_, open) = call(reqwest::Method::GET, "/api/v2/security/findings?filter[query]=@status:muted", None).await;
        assert!(open["data"].as_array().unwrap().is_empty());
        assert_eq!(call(reqwest::Method::GET, "/api/v2/security/findings?filter[query]=@severity:high%20OR%20@status:open", None).await.0, 400);
    }

    #[test]
    fn cursors_round_trip() {
        assert_eq!(offset(&cursor(150)), Some(150));
        assert_eq!(offset("eyJhZnRlciI6IkF3QUFBWnNrS2JQ"), None);
    }

    #[test]
    fn reasons_follow_the_direction() {
        assert!(MUTE_REASONS.contains(&"RISK_ACCEPTED") && !UNMUTE_REASONS.contains(&"RISK_ACCEPTED"));
        assert!(UNMUTE_REASONS.contains(&"NO_LONGER_ACCEPTED_RISK"));
    }
}
