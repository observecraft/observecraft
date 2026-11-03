//! Assist Code: reading repositories and turning a proposed change into a pull request (GitHub) or merge
//! request (GitLab), through the org's existing source code connections (Developer Portal → Settings,
//! `portal_connectors`) and their SSRF-guarded client. Only GET and POST are used: GitHub's Git Data API
//! (tree, commit, ref, pull) and GitLab's commits API (one commit with actions on a new branch).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::routes::portal_sources::{send, Auth, HttpCall};
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgeKind {
    Github,
    Gitlab,
}

impl ForgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ForgeKind::Github => "github",
            ForgeKind::Gitlab => "gitlab",
        }
    }
}

/// One connection: the API's base URL and the org's token.
pub struct Forge {
    pub kind: ForgeKind,
    pub base: String,
    pub token: String,
}

/// A file of a proposed change, whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub content: String,
    /// It exists on the base branch (an update, not a new file).
    #[serde(default)]
    pub existed: bool,
}

/// `owner/name` (GitLab: `group/subgroup/name`).
pub fn check_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<&str> = repo.split('/').collect();
    let ok = parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
    if ok {
        Ok(())
    } else {
        Err(format!("{repo} isn't a repository name like owner/name"))
    }
}

/// A relative path inside the repository.
pub fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p.is_empty() || p == "..") || path.len() > 1000 {
        return Err(format!("{path} isn't a path inside the repository"));
    }
    Ok(())
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
}

impl Forge {
    fn auth(&self) -> Auth {
        match self.kind {
            ForgeKind::Github => Auth::Bearer(self.token.clone()),
            ForgeKind::Gitlab => Auth::GitlabToken(self.token.clone()),
        }
    }

    fn project(&self, repo: &str) -> String {
        format!("{}/api/v4/projects/{}", self.base, enc(repo))
    }

    pub fn repo_call(&self, repo: &str) -> HttpCall {
        match self.kind {
            ForgeKind::Github => HttpCall::get(format!("{}/repos/{repo}", self.base), self.auth()),
            ForgeKind::Gitlab => HttpCall::get(self.project(repo), self.auth()),
        }
    }

    pub fn tree_call(&self, repo: &str, reference: &str) -> HttpCall {
        match self.kind {
            ForgeKind::Github => HttpCall::get(format!("{}/repos/{repo}/git/trees/{}?recursive=1", self.base, enc(reference)), self.auth()),
            ForgeKind::Gitlab => HttpCall::get(format!("{}/repository/tree?{}", self.project(repo), query(&[("ref", reference), ("recursive", "true"), ("per_page", "100")])), self.auth()),
        }
    }

    pub fn file_call(&self, repo: &str, path: &str, reference: &str) -> HttpCall {
        match self.kind {
            ForgeKind::Github => HttpCall::get(format!("{}/repos/{repo}/contents/{path}?{}", self.base, query(&[("ref", reference)])), self.auth()),
            ForgeKind::Gitlab => HttpCall::get(format!("{}/repository/files/{}?{}", self.project(repo), enc(path), query(&[("ref", reference)])), self.auth()),
        }
    }

    pub fn search_call(&self, repo: &str, text: &str) -> HttpCall {
        match self.kind {
            ForgeKind::Github => HttpCall::get(format!("{}/search/code?{}", self.base, query(&[("q", &format!("{text} repo:{repo}"))])), self.auth()),
            ForgeKind::Gitlab => HttpCall::get(format!("{}/search?{}", self.project(repo), query(&[("scope", "blobs"), ("search", text)])), self.auth()),
        }
    }
}

/// A file's text from the forge's JSON (base64 content).
pub fn decode_file(v: &Value) -> Result<String, String> {
    let raw: String = v["content"].as_str().ok_or("The file has no content (is it a directory?)")?.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD.decode(raw).map_err(|e| format!("Can't decode the file: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "The file isn't text".to_string())
}

/// The files of a tree listing under `prefix`.
pub fn tree_paths(kind: ForgeKind, v: &Value, prefix: &str) -> Vec<String> {
    let items = match kind {
        ForgeKind::Github => v["tree"].as_array(),
        ForgeKind::Gitlab => v.as_array(),
    };
    let prefix = prefix.trim_matches('/');
    items
        .into_iter()
        .flatten()
        .filter(|i| i["type"] == "blob")
        .filter_map(|i| i["path"].as_str())
        .filter(|p| prefix.is_empty() || p.starts_with(&format!("{prefix}/")))
        .map(str::to_string)
        .collect()
}

pub fn default_branch(_kind: ForgeKind, repo: &Value) -> Option<String> {
    repo["default_branch"].as_str().map(str::to_string)
}

/// `prefix` + a slug of the title + the run's first id segment.
pub fn branch_name(prefix: &str, title: &str, run: &str) -> String {
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(48).collect();
    format!("{prefix}{}-{}", slug.trim_end_matches('-'), run.split('-').next().unwrap_or(run))
}

pub fn gitlab_commit_body(branch: &str, base: &str, message: &str, files: &[FileChange]) -> Value {
    json!({
        "branch": branch,
        "start_branch": base,
        "commit_message": message,
        "actions": files.iter().map(|f| json!({"action": if f.existed { "update" } else { "create" }, "file_path": f.path, "content": f.content})).collect::<Vec<_>>(),
    })
}

pub fn github_tree_body(base_tree: &str, files: &[FileChange]) -> Value {
    json!({"base_tree": base_tree, "tree": files.iter().map(|f| json!({"path": f.path, "mode": "100644", "type": "blob", "content": f.content})).collect::<Vec<_>>()})
}

// ---------------------------------------------------------------- over the network

/// The org's connection for a repository added to Assist Code (its forge is recorded with it).
pub async fn forge_for(st: &AppState, org: &str, repo: &str) -> Result<Forge, String> {
    check_repo(repo)?;
    let row: Option<(String,)> = sqlx::query_as("SELECT provider FROM assist_code_repos WHERE org_id = $1 AND repo = $2").bind(org).bind(repo).fetch_optional(&st.postgres).await.map_err(|e| e.to_string())?;
    let Some((provider,)) = row else {
        return Err(format!("{repo} isn't added to Assist Code (Code → Settings → Repositories)"));
    };
    let kind = if provider == "gitlab" { ForgeKind::Gitlab } else { ForgeKind::Github };
    let provider_enum = if kind == ForgeKind::Gitlab { crate::routes::portal_sources::PortalProvider::Gitlab } else { crate::routes::portal_sources::PortalProvider::Github };
    let c = crate::routes::portal_home::stored_connector(st, org, provider_enum).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("No {} connection: add one in Developer Portal → Settings", kind.as_str()))?;
    Ok(Forge { kind, base: c.base_url.trim_end_matches('/').to_string(), token: c.secret })
}

async fn call(st: &AppState, c: &HttpCall) -> Result<Value, String> {
    send(c, st.config.portal_allow_private).await
}

pub async fn base_branch(st: &AppState, f: &Forge, repo: &str) -> Result<String, String> {
    let r = call(st, &f.repo_call(repo)).await?;
    default_branch(f.kind, &r).ok_or_else(|| "The repository has no default branch".into())
}

pub async fn list_files(st: &AppState, f: &Forge, repo: &str, prefix: &str, reference: Option<&str>) -> Result<Vec<String>, String> {
    let reference = match reference {
        Some(r) => r.to_string(),
        None => base_branch(st, f, repo).await?,
    };
    Ok(tree_paths(f.kind, &call(st, &f.tree_call(repo, &reference)).await?, prefix))
}

pub async fn read_file(st: &AppState, f: &Forge, repo: &str, path: &str, reference: Option<&str>) -> Result<String, String> {
    check_path(path)?;
    let reference = match reference {
        Some(r) => r.to_string(),
        None => base_branch(st, f, repo).await?,
    };
    decode_file(&call(st, &f.file_call(repo, path, &reference)).await?)
}

pub async fn search(st: &AppState, f: &Forge, repo: &str, text: &str) -> Result<Vec<Value>, String> {
    let v = call(st, &f.search_call(repo, text)).await?;
    Ok(match f.kind {
        ForgeKind::Github => v["items"].as_array().into_iter().flatten().take(30).map(|i| json!({"path": i["path"]})).collect(),
        ForgeKind::Gitlab => v.as_array().into_iter().flatten().take(30).map(|i| json!({"path": i["path"], "line": i["startline"], "excerpt": i["data"]})).collect(),
    })
}

/// A pull (merge) request opened from the change.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    pub number: i64,
    pub branch: String,
    pub draft: bool,
}

/// Commits the files on a new branch from the default branch and opens a pull request.
#[allow(clippy::too_many_arguments)]
pub async fn open_pull_request(st: &AppState, f: &Forge, repo: &str, branch: &str, title: &str, body: &str, files: &[FileChange], draft: bool) -> Result<PullRequest, String> {
    let base = base_branch(st, f, repo).await?;
    match f.kind {
        ForgeKind::Github => {
            let r = format!("{}/repos/{repo}", f.base);
            let head = call(st, &HttpCall::get(format!("{r}/git/ref/heads/{}", enc(&base)), f.auth())).await?;
            let sha = head["object"]["sha"].as_str().ok_or("Can't read the default branch")?.to_string();
            let tree = call(st, &HttpCall::post(format!("{r}/git/trees"), f.auth(), github_tree_body(&sha, files))).await?;
            let commit = call(st, &HttpCall::post(format!("{r}/git/commits"), f.auth(), json!({"message": title, "tree": tree["sha"], "parents": [sha]}))).await?;
            call(st, &HttpCall::post(format!("{r}/git/refs"), f.auth(), json!({"ref": format!("refs/heads/{branch}"), "sha": commit["sha"]}))).await?;
            let pr = call(st, &HttpCall::post(format!("{r}/pulls"), f.auth(), json!({"title": title, "head": branch, "base": base, "body": body, "draft": draft}))).await?;
            Ok(PullRequest { url: pr["html_url"].as_str().unwrap_or_default().into(), number: pr["number"].as_i64().unwrap_or_default(), branch: branch.into(), draft })
        }
        ForgeKind::Gitlab => {
            let p = f.project(repo);
            call(st, &HttpCall::post(format!("{p}/repository/commits"), f.auth(), gitlab_commit_body(branch, &base, title, files))).await?;
            let title = if draft { format!("Draft: {title}") } else { title.to_string() };
            let mr = call(st, &HttpCall::post(format!("{p}/merge_requests"), f.auth(), json!({"source_branch": branch, "target_branch": base, "title": title, "description": body}))).await?;
            Ok(PullRequest { url: mr["web_url"].as_str().unwrap_or_default().into(), number: mr["iid"].as_i64().unwrap_or_default(), branch: branch.into(), draft })
        }
    }
}

/// Whether the pull request was merged (`open`, `merged`, `closed`).
pub async fn pull_request_state(st: &AppState, f: &Forge, repo: &str, number: i64) -> Result<String, String> {
    match f.kind {
        ForgeKind::Github => {
            let pr = call(st, &HttpCall::get(format!("{}/repos/{repo}/pulls/{number}", f.base), f.auth())).await?;
            Ok(if pr["merged"] == true { "merged" } else { pr["state"].as_str().unwrap_or("open") }.into())
        }
        ForgeKind::Gitlab => {
            let mr = call(st, &HttpCall::get(format!("{}/merge_requests/{number}", f.project(repo)), f.auth())).await?;
            Ok(match mr["state"].as_str() {
                Some("merged") => "merged",
                Some("closed") => "closed",
                _ => "open",
            }
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gh() -> Forge {
        Forge { kind: ForgeKind::Github, base: "https://api.github.com".into(), token: "t".into() }
    }
    fn gl() -> Forge {
        Forge { kind: ForgeKind::Gitlab, base: "https://gitlab.example.com".into(), token: "t".into() }
    }

    #[test]
    fn repositories_are_owner_slash_name() {
        assert!(check_repo("example-org/storefront").is_ok());
        assert!(check_repo("example-org/platform/order-service").is_ok(), "GitLab subgroups");
        for bad in ["storefront", "example-org/../x", "a/b?c", "/a/b", "a//b", "a/b/"] {
            assert!(check_repo(bad).is_err(), "{bad}");
        }
        assert!(check_path("src/app.py").is_ok());
        assert!(check_path("../etc/passwd").is_err() && check_path("/abs").is_err());
    }

    #[test]
    fn reading_calls_for_each_forge() {
        assert_eq!(gh().tree_call("example-org/storefront", "main").url, "https://api.github.com/repos/example-org/storefront/git/trees/main?recursive=1");
        assert_eq!(gl().tree_call("example-org/storefront", "main").url, "https://gitlab.example.com/api/v4/projects/example-org%2Fstorefront/repository/tree?ref=main&recursive=true&per_page=100");
        assert_eq!(gh().file_call("example-org/storefront", "app/models/cart.rb", "main").url, "https://api.github.com/repos/example-org/storefront/contents/app/models/cart.rb?ref=main");
        assert_eq!(gl().file_call("example-org/storefront", "app/models/cart.rb", "main").url, "https://gitlab.example.com/api/v4/projects/example-org%2Fstorefront/repository/files/app%2Fmodels%2Fcart.rb?ref=main");
        assert_eq!(gh().search_call("example-org/storefront", "find_by session").url, "https://api.github.com/search/code?q=find_by+session+repo%3Aexample-org%2Fstorefront");
        assert_eq!(gl().search_call("example-org/storefront", "find_by").url, "https://gitlab.example.com/api/v4/projects/example-org%2Fstorefront/search?scope=blobs&search=find_by");
        assert_eq!(gh().repo_call("example-org/storefront").url, "https://api.github.com/repos/example-org/storefront");
        assert_eq!(gl().repo_call("example-org/storefront").url, "https://gitlab.example.com/api/v4/projects/example-org%2Fstorefront");
    }

    #[test]
    fn files_and_trees_are_decoded() {
        assert_eq!(decode_file(&json!({"content": "aGVsbG8K\nd29ybGQ=", "encoding": "base64"})).unwrap(), "hello\nworld");
        let gh_tree = json!({"tree": [{"path": "src", "type": "tree"}, {"path": "src/a.rs", "type": "blob"}, {"path": "README.md", "type": "blob"}], "truncated": false});
        assert_eq!(tree_paths(ForgeKind::Github, &gh_tree, "src"), vec!["src/a.rs"]);
        let gl_tree = json!([{"path": "src/a.rs", "type": "blob"}, {"path": "src", "type": "tree"}]);
        assert_eq!(tree_paths(ForgeKind::Gitlab, &gl_tree, ""), vec!["src/a.rs"]);
        assert_eq!(default_branch(ForgeKind::Github, &json!({"default_branch": "main"})), Some("main".into()));
    }

    #[test]
    fn a_change_becomes_a_commit_on_a_new_branch() {
        assert_eq!(branch_name("assist/", "Fix slow MongoDB query in checkout!", "8c6f0a1b-0000-4000-8000-000000000000"), "assist/fix-slow-mongodb-query-in-checkout-8c6f0a1b");
        let files = vec![FileChange { path: "app/cart.rb".into(), content: "new".into(), existed: true }, FileChange { path: "app/new.rb".into(), content: "x".into(), existed: false }];
        let body = gitlab_commit_body("assist/fix-1", "main", "Fix cart lookup", &files);
        assert_eq!(body["branch"], "assist/fix-1");
        assert_eq!(body["start_branch"], "main");
        assert_eq!(body["actions"][0], json!({"action": "update", "file_path": "app/cart.rb", "content": "new"}));
        assert_eq!(body["actions"][1]["action"], "create");
        let tree = github_tree_body("abc123", &files);
        assert_eq!(tree["base_tree"], "abc123");
        assert_eq!(tree["tree"][0], json!({"path": "app/cart.rb", "mode": "100644", "type": "blob", "content": "new"}));
    }
}
