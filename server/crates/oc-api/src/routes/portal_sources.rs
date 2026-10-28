//! Developer Portal source connectors: the requests the server sends to GitHub, GitLab, Jira and
//! Linear for the home page's pull requests and tickets, and how their answers are mapped.
//!
//! - Requests are built and answers mapped by pure functions (tested below); [`send`] performs them.
//! - Base URLs are chosen by org admins only, and every request goes through the synthetic tests'
//!   SSRF guard ([`super::synthetics_net`]): the host is resolved once, refused when it is internal
//!   (unless the operator sets `OC_PORTAL_ALLOW_PRIVATE=true`), and connected to at the checked
//!   address. Redirects are not followed, answers are capped at [`MAX_RESPONSE_BYTES`] and every
//!   request times out after [`TIMEOUT_SECONDS`].
//! - Credentials go only into the request's authorization header; errors never echo them.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::synthetics_net::{check_addr, resolve};

pub const TIMEOUT_SECONDS: u64 = 10;
pub const MAX_RESPONSE_BYTES: usize = 2 << 20;
/// Items read per list (pull requests per search, tickets).
pub const PAGE_SIZE: usize = 25;
/// How far back "Recently merged" goes.
pub const MERGED_DAYS: i64 = 14;
const USER_AGENT: &str = "ObserveCraft-Developer-Portal/1.0";

// ================================================================ providers

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PortalProvider {
    Github,
    Gitlab,
    Jira,
    Linear,
}

impl PortalProvider {
    pub const ALL: [PortalProvider; 4] = [PortalProvider::Github, PortalProvider::Gitlab, PortalProvider::Jira, PortalProvider::Linear];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PortalProvider::Github => "github",
            PortalProvider::Gitlab => "gitlab",
            PortalProvider::Jira => "jira",
            PortalProvider::Linear => "linear",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PortalProvider::Github => "GitHub",
            PortalProvider::Gitlab => "GitLab",
            PortalProvider::Jira => "Jira",
            PortalProvider::Linear => "Linear",
        }
    }

    /// The API base URL used when admins leave it empty; Jira has none (every site has its own).
    pub fn default_base_url(self) -> &'static str {
        match self {
            PortalProvider::Github => "https://api.github.com",
            PortalProvider::Gitlab => "https://gitlab.com",
            PortalProvider::Jira => "",
            PortalProvider::Linear => "https://api.linear.app",
        }
    }

    /// Code hosts (pull requests) or trackers (tickets).
    pub fn is_code_host(self) -> bool {
        matches!(self, PortalProvider::Github | PortalProvider::Gitlab)
    }

    /// Jira and Linear identify members by email (defaulting to their ObserveCraft email), code
    /// hosts by username.
    pub fn identity_is_email(self) -> bool {
        !self.is_code_host()
    }
}

/// Validates the base URL an admin configured: https (http only when private targets are allowed),
/// no credentials, query or fragment; Linear's API host is fixed. A host given as an IP literal is
/// checked against the SSRF policy now; names are checked when requests are sent.
pub fn check_base_url(provider: PortalProvider, raw: &str, allow_private: bool) -> Result<String, String> {
    let raw = raw.trim();
    let raw = if raw.is_empty() { provider.default_base_url() } else { raw };
    if raw.is_empty() {
        return Err(format!("{} needs a site URL (https://your-site.atlassian.net)", provider.label()));
    }
    if raw.len() > 512 {
        return Err("The base URL is too long".into());
    }
    let url = url::Url::parse(raw).map_err(|e| format!("Invalid base URL: {e}"))?;
    match url.scheme() {
        "https" => {}
        "http" if allow_private => {}
        _ => return Err("The base URL must use https".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The base URL must not contain credentials".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The base URL must not have a query or fragment".into());
    }
    let host = url.host().ok_or("The base URL has no host")?;
    match host {
        url::Host::Ipv4(ip) => check_addr(ip.into(), allow_private)?,
        url::Host::Ipv6(ip) => check_addr(ip.into(), allow_private)?,
        url::Host::Domain(_) => {}
    }
    let out = url.as_str().trim_end_matches('/').to_string();
    if provider == PortalProvider::Linear && out != provider.default_base_url() && !allow_private {
        return Err("Linear's API URL cannot be changed".into());
    }
    Ok(out)
}

/// Validates a member's identity on a provider: a username for code hosts, an email otherwise.
/// Usernames go into search queries, so only the characters the providers allow are accepted.
pub fn check_identity(provider: PortalProvider, raw: &str) -> Result<String, String> {
    let s = raw.trim().trim_start_matches('@');
    let ok = match provider {
        PortalProvider::Github => (1..=39).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !s.starts_with('-'),
        PortalProvider::Gitlab => (1..=255).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
        PortalProvider::Jira | PortalProvider::Linear => is_email(s),
    };
    if !ok {
        return Err(match provider {
            PortalProvider::Github | PortalProvider::Gitlab => format!("{s} is not a valid {} username", provider.label()),
            _ => format!("{s} is not an email address"),
        });
    }
    Ok(s.to_string())
}

fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    !local.is_empty() && domain.contains('.') && s.len() <= 320 && !s.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\')
}

// ================================================================ API types

/// Which group of the home page a pull request belongs to.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PrGroup {
    NeedsReview,
    Returned,
    Approved,
    Waiting,
    Drafts,
    Merged,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortalPerson {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalPullRequest {
    pub group: PrGroup,
    pub title: String,
    /// `owner/repo` (GitHub) or the project path (GitLab).
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub author: PortalPerson,
    pub comments: u64,
    pub reviewers: Vec<PortalPerson>,
    /// Lines added and removed; GitLab's list API doesn't report them.
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalTicket {
    pub key: String,
    pub title: String,
    pub url: String,
    pub status: String,
    /// todo | in_progress | done
    pub status_category: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// The provider's priority name (`High`, `Urgent`, `SEV-2`); empty when none.
    pub priority: String,
    /// urgent | high | medium | low | none: the colour of the priority dot.
    pub priority_level: String,
}

// ================================================================ requests (pure)

#[derive(Clone, Debug, PartialEq)]
pub enum Auth {
    Bearer(String),
    Basic(String, String),
    /// Sent as-is in `Authorization` (Linear API keys).
    Raw(String),
    /// GitLab's `PRIVATE-TOKEN` header.
    GitlabToken(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpCall {
    pub post: bool,
    pub url: String,
    pub auth: Auth,
    pub body: Option<Value>,
}

impl HttpCall {
    fn get(url: String, auth: Auth) -> Self {
        HttpCall { post: false, url, auth, body: None }
    }

    fn post(url: String, auth: Auth, body: Value) -> Self {
        HttpCall { post: true, url, auth, body: Some(body) }
    }
}

fn query_string(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
}

/// `YYYY-MM-DD` of `now_ms` minus `days`.
pub fn date_days_before(now_ms: i64, days: i64) -> String {
    chrono::DateTime::from_timestamp_millis(now_ms - days * 86_400_000).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------- GitHub

pub fn github_user_call(base: &str, token: &str) -> HttpCall {
    HttpCall::get(format!("{base}/user"), Auth::Bearer(token.into()))
}

/// GitHub Enterprise serves REST under `/api/v3` and GraphQL under `/api/graphql`.
pub fn github_graphql_url(base: &str) -> String {
    match base.strip_suffix("/api/v3") {
        Some(root) => format!("{root}/api/graphql"),
        None => format!("{base}/graphql"),
    }
}

/// Searches for review requests, the member's open pull requests and the recently merged ones.
pub fn github_searches(login: &str, merged_since: &str) -> [String; 3] {
    [format!("is:pr is:open archived:false review-requested:{login}"), format!("is:pr is:open archived:false author:{login}"), format!("is:pr is:merged author:{login} merged:>={merged_since}")]
}

const GITHUB_PRS_QUERY: &str = "query($review: String!, $authored: String!, $merged: String!, $first: Int!) {
  review: search(query: $review, type: ISSUE, first: $first) { nodes { ...pr } }
  authored: search(query: $authored, type: ISSUE, first: $first) { nodes { ...pr } }
  merged: search(query: $merged, type: ISSUE, first: $first) { nodes { ...pr } }
}
fragment pr on PullRequest {
  number title url isDraft updatedAt additions deletions reviewDecision
  repository { nameWithOwner }
  author { login avatarUrl }
  comments { totalCount }
  reviewRequests(first: 10) { nodes { requestedReviewer { ... on User { login avatarUrl } ... on Team { name } } } }
  latestReviews(first: 10) { nodes { author { login avatarUrl } } }
}";

pub fn github_prs_call(base: &str, token: &str, login: &str, merged_since: &str) -> HttpCall {
    let [review, authored, merged] = github_searches(login, merged_since);
    HttpCall::post(
        github_graphql_url(base),
        Auth::Bearer(token.into()),
        json!({"query": GITHUB_PRS_QUERY, "variables": {"review": review, "authored": authored, "merged": merged, "first": PAGE_SIZE}}),
    )
}

fn text(v: &Value, ptr: &str) -> String {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn num(v: &Value, ptr: &str) -> Option<u64> {
    v.pointer(ptr).and_then(Value::as_u64)
}

/// Epoch ms of an ISO 8601 time (RFC 3339, or Jira's `+0000` offsets); 0 when unreadable.
pub fn parse_time(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z")).map(|d| d.timestamp_millis()).unwrap_or(0)
}

/// GraphQL answers carry failures in `errors` with a 200 status.
pub fn graphql_errors(v: &Value) -> Result<(), String> {
    match v.get("errors").and_then(Value::as_array) {
        Some(errors) if !errors.is_empty() => {
            let messages: Vec<String> = errors.iter().filter_map(|e| e.get("message").and_then(Value::as_str)).map(|m| m.chars().take(200).collect()).collect();
            Err(if messages.is_empty() { "The provider answered with an error".into() } else { messages.join("; ") })
        }
        _ => Ok(()),
    }
}

fn github_pr(node: &Value, group: PrGroup) -> Option<PortalPullRequest> {
    let number = num(node, "/number")?;
    let mut reviewers: Vec<PortalPerson> = Vec::new();
    let mut add = |login: String, avatar_url: String| {
        if !login.is_empty() && !reviewers.iter().any(|r| r.login == login) {
            reviewers.push(PortalPerson { login, avatar_url });
        }
    };
    for r in node.pointer("/reviewRequests/nodes").and_then(Value::as_array).into_iter().flatten() {
        let who = &r["requestedReviewer"];
        let login = who.get("login").or_else(|| who.get("name")).and_then(Value::as_str).unwrap_or_default();
        add(login.to_string(), text(who, "/avatarUrl"));
    }
    for r in node.pointer("/latestReviews/nodes").and_then(Value::as_array).into_iter().flatten() {
        add(text(r, "/author/login"), text(r, "/author/avatarUrl"));
    }
    Some(PortalPullRequest {
        group,
        title: text(node, "/title"),
        repo: text(node, "/repository/nameWithOwner"),
        number,
        url: text(node, "/url"),
        author: PortalPerson { login: text(node, "/author/login"), avatar_url: text(node, "/author/avatarUrl") },
        comments: num(node, "/comments/totalCount").unwrap_or(0),
        reviewers,
        additions: num(node, "/additions"),
        deletions: num(node, "/deletions"),
        updated_at: parse_time(&text(node, "/updatedAt")),
    })
}

/// Groups GitHub's three searches: review requests need the member's review; their own open pull
/// requests are drafts, returned (changes requested), approved or waiting for reviewers.
pub fn parse_github_prs(v: &Value) -> Result<Vec<PortalPullRequest>, String> {
    graphql_errors(v)?;
    let nodes = |name: &str| v.pointer(&format!("/data/{name}/nodes")).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out: Vec<PortalPullRequest> = nodes("review").iter().filter_map(|n| github_pr(n, PrGroup::NeedsReview)).collect();
    for n in nodes("authored") {
        let group = if n["isDraft"].as_bool().unwrap_or(false) {
            PrGroup::Drafts
        } else {
            match n["reviewDecision"].as_str() {
                Some("CHANGES_REQUESTED") => PrGroup::Returned,
                Some("APPROVED") => PrGroup::Approved,
                _ => PrGroup::Waiting,
            }
        };
        out.extend(github_pr(&n, group));
    }
    out.extend(nodes("merged").iter().filter_map(|n| github_pr(n, PrGroup::Merged)));
    Ok(sorted(out))
}

fn sorted(mut prs: Vec<PortalPullRequest>) -> Vec<PortalPullRequest> {
    prs.sort_by(|a, b| a.group.cmp(&b.group).then(b.updated_at.cmp(&a.updated_at)));
    prs.dedup_by(|a, b| a.group == b.group && a.url == b.url);
    prs
}

// ---------------------------------------------------------------- GitLab

pub fn gitlab_user_call(base: &str, token: &str) -> HttpCall {
    HttpCall::get(format!("{base}/api/v4/user"), Auth::GitlabToken(token.into()))
}

/// Merge requests waiting for the member's review, their own open ones and the recently merged ones.
pub fn gitlab_mr_calls(base: &str, token: &str, username: &str, merged_since: &str) -> [HttpCall; 3] {
    let per_page = PAGE_SIZE.to_string();
    let list = |pairs: &[(&str, &str)]| HttpCall::get(format!("{base}/api/v4/merge_requests?{}", query_string(pairs)), Auth::GitlabToken(token.into()));
    [
        list(&[("scope", "all"), ("state", "opened"), ("reviewer_username", username), ("order_by", "updated_at"), ("per_page", &per_page)]),
        list(&[("scope", "all"), ("state", "opened"), ("author_username", username), ("order_by", "updated_at"), ("per_page", &per_page)]),
        list(&[("scope", "all"), ("state", "merged"), ("author_username", username), ("updated_after", &format!("{merged_since}T00:00:00Z")), ("per_page", &per_page)]),
    ]
}

fn gitlab_mr(mr: &Value, group: PrGroup) -> Option<PortalPullRequest> {
    let number = num(mr, "/iid")?;
    let full = text(mr, "/references/full");
    let repo = full.rsplit_once('!').map(|(path, _)| path.to_string()).unwrap_or(full);
    let reviewers = mr["reviewers"].as_array().into_iter().flatten().map(|r| PortalPerson { login: text(r, "/username"), avatar_url: text(r, "/avatar_url") }).collect();
    Some(PortalPullRequest {
        group,
        title: text(mr, "/title"),
        repo,
        number,
        url: text(mr, "/web_url"),
        author: PortalPerson { login: text(mr, "/author/username"), avatar_url: text(mr, "/author/avatar_url") },
        comments: num(mr, "/user_notes_count").unwrap_or(0),
        reviewers,
        additions: None,
        deletions: None,
        updated_at: parse_time(&text(mr, "/updated_at")),
    })
}

/// Groups GitLab's three lists. Authored merge requests are drafts, returned (`requested_changes`),
/// approved (`mergeable`: every required approval given) or waiting.
pub fn parse_gitlab_mrs(review: &Value, authored: &Value, merged: &Value) -> Result<Vec<PortalPullRequest>, String> {
    let list = |v: &Value| v.as_array().cloned().ok_or_else(|| "GitLab answered with something other than a list of merge requests".to_string());
    let mut out: Vec<PortalPullRequest> = list(review)?.iter().filter_map(|m| gitlab_mr(m, PrGroup::NeedsReview)).collect();
    for m in list(authored)? {
        let draft = m["draft"].as_bool().or_else(|| m["work_in_progress"].as_bool()).unwrap_or(false);
        let group = if draft {
            PrGroup::Drafts
        } else {
            match m["detailed_merge_status"].as_str() {
                Some("requested_changes") => PrGroup::Returned,
                Some("mergeable") => PrGroup::Approved,
                _ => PrGroup::Waiting,
            }
        };
        out.extend(gitlab_mr(&m, group));
    }
    out.extend(list(merged)?.iter().filter_map(|m| gitlab_mr(m, PrGroup::Merged)));
    Ok(sorted(out))
}

// ---------------------------------------------------------------- Jira

pub fn jira_myself_call(base: &str, email: &str, token: &str) -> HttpCall {
    HttpCall::get(format!("{base}/rest/api/3/myself"), Auth::Basic(email.into(), token.into()))
}

pub fn jira_user_search_call(base: &str, email: &str, token: &str, member_email: &str) -> HttpCall {
    HttpCall::get(format!("{base}/rest/api/3/user/search?{}", query_string(&[("query", member_email)])), Auth::Basic(email.into(), token.into()))
}

/// The account id of the member among Jira's user search results: the one whose email matches
/// (Jira hides emails by privacy settings, so a single result counts too).
pub fn parse_jira_account(v: &Value, member_email: &str) -> Option<String> {
    let users = v.as_array()?;
    let id = |u: &Value| u.get("accountId").and_then(Value::as_str).map(str::to_string);
    users.iter().find(|u| u.get("emailAddress").and_then(Value::as_str).is_some_and(|e| e.eq_ignore_ascii_case(member_email))).and_then(id).or_else(|| match users.as_slice() {
        [only] => id(only),
        _ => None,
    })
}

/// The member's open work items. Account ids are opaque (`5b10ac8d82e05b22cc7d4ef5`,
/// `712020:…`); anything else is refused rather than quoted into JQL.
pub fn jira_jql(account_id: &str) -> Result<String, String> {
    if account_id.is_empty() || account_id.len() > 128 || !account_id.chars().all(|c| c.is_ascii_alphanumeric() || ":-_".contains(c)) {
        return Err("Jira returned an unexpected account id".into());
    }
    Ok(format!("assignee = \"{account_id}\" AND statusCategory != Done ORDER BY updated DESC"))
}

pub fn jira_search_call(base: &str, email: &str, token: &str, jql: &str) -> HttpCall {
    HttpCall::post(
        format!("{base}/rest/api/3/search/jql"),
        Auth::Basic(email.into(), token.into()),
        json!({"jql": jql, "maxResults": PAGE_SIZE, "fields": ["summary", "status", "created", "updated", "priority"]}),
    )
}

fn jira_priority_level(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "highest" | "blocker" | "critical" | "urgent" => "urgent",
        "high" | "major" => "high",
        "medium" => "medium",
        "low" | "lowest" | "minor" | "trivial" => "low",
        _ => "none",
    }
}

pub fn parse_jira_issues(v: &Value, site: &str) -> Result<Vec<PortalTicket>, String> {
    let issues = v.get("issues").and_then(Value::as_array).ok_or("Jira answered without issues")?;
    Ok(issues
        .iter()
        .filter_map(|i| {
            let key = i.get("key")?.as_str()?.to_string();
            let f = &i["fields"];
            let priority = text(f, "/priority/name");
            let category = match f.pointer("/status/statusCategory/key").and_then(Value::as_str) {
                Some("done") => "done",
                Some("indeterminate") => "in_progress",
                _ => "todo",
            };
            Some(PortalTicket {
                url: format!("{site}/browse/{key}"),
                key,
                title: text(f, "/summary"),
                status: text(f, "/status/name"),
                status_category: category.into(),
                created_at: parse_time(&text(f, "/created")),
                updated_at: parse_time(&text(f, "/updated")),
                priority_level: jira_priority_level(&priority).into(),
                priority,
            })
        })
        .collect())
}

// ---------------------------------------------------------------- Linear

pub fn linear_viewer_call(base: &str, key: &str) -> HttpCall {
    HttpCall::post(format!("{base}/graphql"), Auth::Raw(key.into()), json!({"query": "query { viewer { id name email } }"}))
}

const LINEAR_ISSUES_QUERY: &str = "query($email: String!, $first: Int!) {
  issues(first: $first, orderBy: updatedAt, filter: { assignee: { email: { eqIgnoreCase: $email } }, state: { type: { nin: [\"completed\", \"canceled\"] } } }) {
    nodes { identifier title url createdAt updatedAt priority priorityLabel state { name type } }
  }
}";

/// Issues assigned to the member (by email); the email is a GraphQL variable, never part of the query text.
pub fn linear_issues_call(base: &str, key: &str, member_email: &str) -> HttpCall {
    HttpCall::post(format!("{base}/graphql"), Auth::Raw(key.into()), json!({"query": LINEAR_ISSUES_QUERY, "variables": {"email": member_email, "first": PAGE_SIZE}}))
}

pub fn parse_linear_issues(v: &Value) -> Result<Vec<PortalTicket>, String> {
    graphql_errors(v)?;
    let nodes = v.pointer("/data/issues/nodes").and_then(Value::as_array).ok_or("Linear answered without issues")?;
    Ok(nodes
        .iter()
        .filter_map(|n| {
            let key = n.get("identifier")?.as_str()?.to_string();
            let level = match n["priority"].as_u64() {
                Some(1) => "urgent",
                Some(2) => "high",
                Some(3) => "medium",
                Some(4) => "low",
                _ => "none",
            };
            let category = match n.pointer("/state/type").and_then(Value::as_str) {
                Some("started") => "in_progress",
                Some("completed") | Some("canceled") => "done",
                _ => "todo",
            };
            Some(PortalTicket {
                key,
                title: text(n, "/title"),
                url: text(n, "/url"),
                status: text(n, "/state/name"),
                status_category: category.into(),
                created_at: parse_time(&text(n, "/createdAt")),
                updated_at: parse_time(&text(n, "/updatedAt")),
                priority: if level == "none" { String::new() } else { text(n, "/priorityLabel") },
                priority_level: level.into(),
            })
        })
        .collect())
}

// ================================================================ sending

fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let text = s.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = s.source();
    }
    out
}

/// The provider's own error message from a failed answer, shortened.
fn error_message(body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let message = v
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| v.get("errorMessages").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string))
        .or_else(|| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    message.chars().take(200).collect()
}

/// Sends `call` to its (admin-configured) host through the SSRF guard and returns the JSON answer.
pub async fn send(call: &HttpCall, allow_private: bool) -> Result<Value, String> {
    let url = url::Url::parse(&call.url).map_err(|e| format!("Invalid URL: {e}"))?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && allow_private)) {
        return Err("Only https URLs can be called".into());
    }
    let host = url.host_str().ok_or("The URL has no host")?.to_string();
    let port = url.port_or_known_default().ok_or("The URL has no port")?;
    let addr = resolve(&host, port, allow_private).await?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(TIMEOUT_SECONDS));
    // Connect to the checked address, never to a second lookup's answer.
    if matches!(url.host(), Some(url::Host::Domain(_))) {
        builder = builder.resolve(&host, addr);
    }
    let client = builder.build().map_err(|e| describe(&e))?;
    let mut rb = if call.post { client.post(url) } else { client.get(url) };
    rb = rb.header(reqwest::header::ACCEPT, "application/json");
    rb = match &call.auth {
        Auth::Bearer(t) => rb.bearer_auth(t),
        Auth::Basic(user, password) => rb.basic_auth(user, Some(password)),
        Auth::Raw(t) => rb.header(reqwest::header::AUTHORIZATION, t),
        Auth::GitlabToken(t) => rb.header("PRIVATE-TOKEN", t),
    };
    if let Some(body) = &call.body {
        rb = rb.json(body);
    }
    let mut resp = rb.send().await.map_err(|e| describe(&e))?;
    let status = resp.status();
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| describe(&e))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(format!("The answer is larger than {} MB", MAX_RESPONSE_BYTES >> 20));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail = error_message(&body);
        let what = match status.as_u16() {
            401 => "the credentials were refused".to_string(),
            403 => "access is forbidden".to_string(),
            404 => "the API was not found at this URL".to_string(),
            300..=399 => "the server redirected (check the base URL)".to_string(),
            s => format!("HTTP {s}"),
        };
        return Err(if detail.is_empty() { format!("The provider answered: {what}") } else { format!("The provider answered: {what} ({detail})") });
    }
    serde_json::from_slice(&body).map_err(|_| "The provider's answer is not JSON".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_default_and_are_checked() {
        assert_eq!(check_base_url(PortalProvider::Github, "", false).unwrap(), "https://api.github.com");
        assert_eq!(check_base_url(PortalProvider::Github, "https://ghe.example.com/api/v3/", false).unwrap(), "https://ghe.example.com/api/v3");
        assert_eq!(check_base_url(PortalProvider::Gitlab, " ", false).unwrap(), "https://gitlab.com");
        assert!(check_base_url(PortalProvider::Jira, "", false).unwrap_err().contains("site URL"));
        assert_eq!(check_base_url(PortalProvider::Jira, "https://example.atlassian.net", false).unwrap(), "https://example.atlassian.net");
        assert!(check_base_url(PortalProvider::Github, "http://ghe.example.com", false).unwrap_err().contains("https"));
        assert!(check_base_url(PortalProvider::Github, "ftp://ghe.example.com", true).is_err());
        assert!(check_base_url(PortalProvider::Github, "https://user:pw@ghe.example.com", false).unwrap_err().contains("credentials"));
        assert!(check_base_url(PortalProvider::Github, "https://ghe.example.com/?x=1", false).is_err());
        assert!(check_base_url(PortalProvider::Linear, "https://linear.example.com", false).unwrap_err().contains("cannot be changed"));
        assert_eq!(check_base_url(PortalProvider::Linear, "", false).unwrap(), "https://api.linear.app");
    }

    #[test]
    fn internal_hosts_need_the_operator_flag() {
        for internal in ["https://127.0.0.1", "https://10.0.0.5/api/v3", "https://169.254.169.254", "https://[::1]:8443", "https://[fd00::1]"] {
            assert!(check_base_url(PortalProvider::Github, internal, false).is_err(), "{internal}");
            assert!(check_base_url(PortalProvider::Github, internal, true).is_ok(), "{internal}");
        }
        assert_eq!(check_base_url(PortalProvider::Gitlab, "http://127.0.0.1:9000", true).unwrap(), "http://127.0.0.1:9000");
    }

    #[test]
    fn identities_are_validated() {
        assert_eq!(check_identity(PortalProvider::Github, " @jane-doe ").unwrap(), "jane-doe");
        assert!(check_identity(PortalProvider::Github, "jane doe").is_err());
        assert!(check_identity(PortalProvider::Github, "jane repo:other/x").is_err());
        assert!(check_identity(PortalProvider::Github, "-jane").is_err());
        assert_eq!(check_identity(PortalProvider::Gitlab, "jane.doe_2").unwrap(), "jane.doe_2");
        assert_eq!(check_identity(PortalProvider::Jira, "jane@example.com").unwrap(), "jane@example.com");
        assert!(check_identity(PortalProvider::Linear, "jane").is_err());
        assert!(check_identity(PortalProvider::Jira, "ja\"ne@example.com").is_err());
    }

    #[test]
    fn github_requests() {
        assert_eq!(github_graphql_url("https://api.github.com"), "https://api.github.com/graphql");
        assert_eq!(github_graphql_url("https://ghe.example.com/api/v3"), "https://ghe.example.com/api/graphql");
        let call = github_prs_call("https://api.github.com", "ghp_x", "jane-doe", "2026-10-14");
        assert!(call.post);
        assert_eq!(call.auth, Auth::Bearer("ghp_x".into()));
        let vars = &call.body.as_ref().unwrap()["variables"];
        assert_eq!(vars["review"], "is:pr is:open archived:false review-requested:jane-doe");
        assert_eq!(vars["authored"], "is:pr is:open archived:false author:jane-doe");
        assert_eq!(vars["merged"], "is:pr is:merged author:jane-doe merged:>=2026-10-14");
        assert_eq!(github_user_call("https://api.github.com", "t").url, "https://api.github.com/user");
        assert_eq!(date_days_before(1_793_145_600_000, 14), "2026-10-14");
    }

    fn gh_node(number: u64, draft: bool, decision: Option<&str>, updated: &str) -> Value {
        json!({
            "number": number, "title": format!("PR {number}"), "url": format!("https://github.com/example-org/storefront/pull/{number}"),
            "isDraft": draft, "updatedAt": updated, "additions": 12, "deletions": 3, "reviewDecision": decision,
            "repository": {"nameWithOwner": "example-org/storefront"}, "author": {"login": "jane-doe", "avatarUrl": "https://avatars.example.com/u/1"},
            "comments": {"totalCount": 2},
            "reviewRequests": {"nodes": [{"requestedReviewer": {"login": "sam-lee", "avatarUrl": "a"}}, {"requestedReviewer": {"name": "checkout-team"}}]},
            "latestReviews": {"nodes": [{"author": {"login": "sam-lee", "avatarUrl": "a"}}, {"author": {"login": "ana-ruiz", "avatarUrl": "b"}}]}
        })
    }

    #[test]
    fn github_pull_requests_are_grouped() {
        let answer = json!({"data": {
            "review": {"nodes": [gh_node(1, false, None, "2026-10-27T10:00:00Z")]},
            "authored": {"nodes": [
                gh_node(2, true, None, "2026-10-26T10:00:00Z"),
                gh_node(3, false, Some("CHANGES_REQUESTED"), "2026-10-26T09:00:00Z"),
                gh_node(4, false, Some("APPROVED"), "2026-10-25T10:00:00Z"),
                gh_node(5, false, Some("REVIEW_REQUIRED"), "2026-10-24T10:00:00Z"),
                gh_node(6, false, None, "2026-10-28T10:00:00Z"),
                {"title": "not a pull request"}
            ]},
            "merged": {"nodes": [gh_node(7, false, Some("APPROVED"), "2026-10-20T10:00:00Z")]}
        }});
        let prs = parse_github_prs(&answer).unwrap();
        let groups: Vec<(u64, PrGroup)> = prs.iter().map(|p| (p.number, p.group)).collect();
        assert_eq!(groups, vec![(1, PrGroup::NeedsReview), (3, PrGroup::Returned), (4, PrGroup::Approved), (6, PrGroup::Waiting), (5, PrGroup::Waiting), (2, PrGroup::Drafts), (7, PrGroup::Merged)]);
        let first = &prs[0];
        assert_eq!(first.repo, "example-org/storefront");
        assert_eq!((first.additions, first.deletions, first.comments), (Some(12), Some(3), 2));
        assert_eq!(first.reviewers.iter().map(|r| r.login.as_str()).collect::<Vec<_>>(), ["sam-lee", "checkout-team", "ana-ruiz"]);
        assert_eq!(first.updated_at, 1_793_095_200_000);
    }

    #[test]
    fn graphql_errors_are_reported() {
        let err = parse_github_prs(&json!({"errors": [{"message": "Bad credentials"}]})).unwrap_err();
        assert_eq!(err, "Bad credentials");
        assert!(parse_linear_issues(&json!({"errors": [{"message": "Authentication required"}]})).unwrap_err().contains("Authentication"));
    }

    #[test]
    fn gitlab_requests_and_grouping() {
        let calls = gitlab_mr_calls("https://gitlab.example.com", "glpat", "jane.doe", "2026-10-14");
        assert_eq!(calls[0].url, "https://gitlab.example.com/api/v4/merge_requests?scope=all&state=opened&reviewer_username=jane.doe&order_by=updated_at&per_page=25");
        assert_eq!(calls[1].url, "https://gitlab.example.com/api/v4/merge_requests?scope=all&state=opened&author_username=jane.doe&order_by=updated_at&per_page=25");
        assert!(calls[2].url.contains("state=merged") && calls[2].url.contains("updated_after=2026-10-14T00%3A00%3A00Z"));
        assert_eq!(calls[0].auth, Auth::GitlabToken("glpat".into()));
        let mr = |iid: u64, draft: bool, status: &str| {
            json!({"iid": iid, "title": format!("MR {iid}"), "web_url": format!("https://gitlab.example.com/example-org/order-service/-/merge_requests/{iid}"),
                   "draft": draft, "detailed_merge_status": status, "updated_at": "2026-10-27T10:00:00.000Z",
                   "references": {"full": "example-org/order-service!12"}, "author": {"username": "jane.doe", "avatar_url": "x"},
                   "user_notes_count": 4, "reviewers": [{"username": "sam.lee", "avatar_url": "y"}]})
        };
        let prs = parse_gitlab_mrs(
            &json!([mr(1, false, "not_approved")]),
            &json!([mr(2, true, "draft_status"), mr(3, false, "requested_changes"), mr(4, false, "mergeable"), mr(5, false, "not_approved")]),
            &json!([mr(6, false, "merged")]),
        )
        .unwrap();
        let groups: Vec<(u64, PrGroup)> = prs.iter().map(|p| (p.number, p.group)).collect();
        assert_eq!(groups, vec![(1, PrGroup::NeedsReview), (3, PrGroup::Returned), (4, PrGroup::Approved), (5, PrGroup::Waiting), (2, PrGroup::Drafts), (6, PrGroup::Merged)]);
        assert_eq!(prs[0].repo, "example-org/order-service");
        assert_eq!((prs[0].comments, prs[0].additions), (4, None));
        assert!(parse_gitlab_mrs(&json!({"message": "401 Unauthorized"}), &json!([]), &json!([])).is_err());
    }

    #[test]
    fn jira_requests_and_mapping() {
        let lookup = jira_user_search_call("https://example.atlassian.net", "bot@example.com", "tok", "jane+qa@example.com");
        assert_eq!(lookup.url, "https://example.atlassian.net/rest/api/3/user/search?query=jane%2Bqa%40example.com");
        assert_eq!(lookup.auth, Auth::Basic("bot@example.com".into(), "tok".into()));
        let users = json!([{"accountId": "a1", "emailAddress": "sam@example.com"}, {"accountId": "712020:b2", "emailAddress": "Jane@example.com"}]);
        assert_eq!(parse_jira_account(&users, "jane@example.com").as_deref(), Some("712020:b2"));
        assert_eq!(parse_jira_account(&json!([{"accountId": "c3"}]), "jane@example.com").as_deref(), Some("c3"));
        assert_eq!(parse_jira_account(&json!([{"accountId": "c3"}, {"accountId": "d4"}]), "jane@example.com"), None);
        assert_eq!(jira_jql("712020:b2").unwrap(), "assignee = \"712020:b2\" AND statusCategory != Done ORDER BY updated DESC");
        assert!(jira_jql("x\" OR project = SECRET").is_err());
        assert!(jira_jql("").is_err());
        let call = jira_search_call("https://example.atlassian.net", "bot@example.com", "tok", "assignee = \"a1\"");
        assert_eq!(call.url, "https://example.atlassian.net/rest/api/3/search/jql");
        assert_eq!(call.body.as_ref().unwrap()["jql"], "assignee = \"a1\"");
        let answer = json!({"issues": [{"key": "SHOP-142", "fields": {"summary": "Checkout button misaligned", "status": {"name": "In Progress", "statusCategory": {"key": "indeterminate"}},
            "created": "2026-10-20T10:15:30.000+0000", "updated": "2026-10-27T08:00:00.000+0200", "priority": {"name": "High"}}},
            {"key": "SHOP-7", "fields": {"summary": "Old", "status": {"name": "To Do", "statusCategory": {"key": "new"}}, "priority": null}}]});
        let t = parse_jira_issues(&answer, "https://example.atlassian.net").unwrap();
        assert_eq!(t[0].url, "https://example.atlassian.net/browse/SHOP-142");
        assert_eq!((t[0].status_category.as_str(), t[0].priority.as_str(), t[0].priority_level.as_str()), ("in_progress", "High", "high"));
        assert_eq!(t[0].created_at, 1_792_491_330_000);
        assert_eq!((t[1].status_category.as_str(), t[1].priority_level.as_str()), ("todo", "none"));
    }

    #[test]
    fn linear_requests_and_mapping() {
        let call = linear_issues_call("https://api.linear.app", "lin_api_x", "jane@example.com");
        assert_eq!(call.url, "https://api.linear.app/graphql");
        assert_eq!(call.auth, Auth::Raw("lin_api_x".into()));
        let body = call.body.unwrap();
        assert_eq!(body["variables"]["email"], "jane@example.com");
        assert!(!body["query"].as_str().unwrap().contains("jane@example.com"));
        let answer = json!({"data": {"issues": {"nodes": [
            {"identifier": "ENG-12", "title": "Retry payments", "url": "https://linear.app/example/issue/ENG-12", "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-02T00:00:00Z",
             "priority": 1, "priorityLabel": "Urgent", "state": {"name": "In Progress", "type": "started"}},
            {"identifier": "ENG-13", "title": "Docs", "url": "u", "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-02T00:00:00Z", "priority": 0, "priorityLabel": "No priority", "state": {"name": "Todo", "type": "unstarted"}}
        ]}}});
        let t = parse_linear_issues(&answer).unwrap();
        assert_eq!((t[0].key.as_str(), t[0].priority_level.as_str(), t[0].status_category.as_str()), ("ENG-12", "urgent", "in_progress"));
        assert_eq!((t[1].priority.as_str(), t[1].priority_level.as_str(), t[1].status_category.as_str()), ("", "none", "todo"));
    }

    /// A tiny HTTP server on loopback answering every request with `status` and `body`, recording the request head.
    async fn fake_server(status: u16, body: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16 * 1024];
            let n = sock.read(&mut buf).await.unwrap();
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            sock.write_all(resp.as_bytes()).await.unwrap();
        });
        (format!("http://{addr}"), rx)
    }

    #[tokio::test]
    async fn send_talks_to_allowed_hosts_only() {
        let (base, head) = fake_server(200, r#"{"login":"jane-doe"}"#).await;
        let v = send(&github_user_call(&base, "ghp_secret"), true).await.unwrap();
        assert_eq!(v["login"], "jane-doe");
        let head = head.await.unwrap().to_ascii_lowercase();
        assert!(head.starts_with("get /user "));
        assert!(head.contains("authorization: bearer ghp_secret"));

        let (base, _) = fake_server(200, "{}").await;
        let err = send(&github_user_call(&base, "t"), false).await.unwrap_err();
        assert!(err.contains("Only https") || err.contains("loopback"), "{err}");
        let err = send(&github_user_call(&base.replace("http://", "https://"), "t"), false).await.unwrap_err();
        assert!(err.contains("loopback"), "{err}");
    }

    #[tokio::test]
    async fn send_reports_provider_errors_without_the_credential() {
        let (base, _) = fake_server(401, r#"{"message":"Bad credentials"}"#).await;
        let err = send(&jira_myself_call(&base, "bot@example.com", "super-secret-token"), true).await.unwrap_err();
        assert_eq!(err, "The provider answered: the credentials were refused (Bad credentials)");
        assert!(!err.contains("super-secret-token"));
        let (base, _) = fake_server(302, "").await;
        assert!(send(&gitlab_user_call(&base, "t"), true).await.unwrap_err().contains("redirected"));
    }
}
