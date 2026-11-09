//! AI Testing (Synthetic Monitoring → AI Testing): an Assist run of kind `testing` explores a site from a
//! starting URL in an oc-browser session and proposes browser tests for its user journeys, grouped into suites for
//! review. The model drives the browser with refs from page snapshots; each action comes back as the test step that
//! repeats it (with locators oc-browser computed), so proposed tests are made of recorded steps, not steps the model
//! wrote. Every proposal is run once as a sample before it is stored. Credentials come from an agent profile whose
//! entries name global variables: the model only sees `{{ NAME }}`, values are filled in by oc-browser as it types.

use std::collections::HashMap;
use std::sync::Mutex;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::assist::agent::{self, Loop, Model, Outcome, Tools};
use crate::assist::llm::{Msg, ToolSpec, Usage};
use crate::assist::runs::{Conclusion, Kind, RunTools};
use crate::auth::{AuthContext, Writer};
use crate::browser::client::{ActionResult, Browser, BrowserClient, OpenSession, RunRequest, VideoMode};
use crate::browser::steps::{self, Step};
use crate::error::{ApiError, ApiResult};
use crate::routes::assist_runs::{self, Done, RunStep, RunSummary};
use crate::routes::synthetics_exec::SyntheticRunDetail;
use crate::routes::synthetics_settings as vars;
use crate::state::AppState;

/// Proposals per job, at most.
const MAX_PROPOSALS: usize = 20;
/// Model turns per job, at most (exploring takes many actions).
const MAX_TURNS: usize = 80;
const USAGES: [&str; 5] = ["username", "email", "password", "otp", "other"];
const MAX_PROFILE_VARIABLES: usize = 10;
/// Accepted tests run every 15 minutes (editable like any test).
const ACCEPTED_FREQUENCY: i32 = 900;
/// The monitor message of accepted tests (the editor's default).
const ACCEPTED_MESSAGE: &str = "{{#is_alert}}The test is failing.{{/is_alert}}\n{{#is_recovery}}The test passes again.{{/is_recovery}}";
const MAX_TEST_STEPS: usize = 40;
/// A snapshot sent to the model, at most (characters).
const MAX_SNAPSHOT: usize = 6000;

/// What an agent profile lends a job: a credential's use and the global variable holding it.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileVariable {
    /// username | email | password | otp | other
    pub usage: String,
    /// The global variable (Synthetics → Settings → Global Variables).
    pub variable: String,
}

/// A job's parameters, as stored in its run's subject.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub start_url: String,
    #[serde(default = "yes")]
    pub restrict_domain: bool,
    #[serde(default)]
    pub device_id: String,
    /// What to test, in the user's words (empty: explore and find the key journeys).
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub profile_id: Option<String>,
}

fn yes() -> bool {
    true
}

impl Job {
    /// The hosts the page may navigate to: the start URL's (without `www.`) and its subdomains, when restricted.
    pub fn allowed_hosts(&self) -> Option<Vec<String>> {
        if !self.restrict_domain {
            return None;
        }
        let host = url::Url::parse(&self.start_url).ok()?.host_str()?.to_ascii_lowercase();
        Some(vec![host.strip_prefix("www.").unwrap_or(&host).to_string()])
    }
}

/// A proposed browser test, before it is stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub suite: String,
    pub name: String,
    pub description: String,
    pub start_url: String,
    pub device_id: String,
    pub steps: Vec<Step>,
    /// The sample run (screenshots aren't kept).
    pub verification: Option<SyntheticRunDetail>,
}

/// One action the model took that a test can repeat.
#[derive(Clone, Debug)]
struct Recorded {
    url_before: String,
    step: Step,
}

/// The first message of a job's run: where to start, what to cover, the profile's variables and the existing tests.
pub fn opening(job: &Job, profile: &[AgentProfileVariable], existing: &[String]) -> String {
    let goal = match job.prompt.trim() {
        "" => "Explore the site and find its key user journeys (navigation, search, product or content pages, cart, sign-up and \
contact forms filled in without submitting them), then propose browser tests for the ones the existing tests don't cover."
            .to_string(),
        p => p.to_string(),
    };
    let mut s = format!("Starting URL: {}\nGoal: {goal}\n", job.start_url);
    match job.allowed_hosts() {
        Some(hosts) => s.push_str(&format!("The browser stays on {} and its subdomains.\n", hosts.join(", "))),
        None => s.push_str("The browser may follow links to other sites (sign-in providers, for example).\n"),
    }
    if profile.is_empty() {
        s.push_str("You browse as an anonymous visitor: don't try to sign in or create accounts.\n");
    } else {
        s.push_str("You browse with an agent profile. Type its credentials with these placeholders (the values are filled in as you type; you never see them):\n");
        for v in profile {
            s.push_str(&format!("- {}: {{{{ {} }}}}\n", v.usage, v.variable));
        }
    }
    if existing.is_empty() {
        s.push_str("The organization has no browser tests yet.\n");
    } else {
        s.push_str("Existing browser tests (don't propose these journeys again):\n");
        for t in existing.iter().take(50) {
            s.push_str(&format!("- {t}\n"));
        }
    }
    s.push_str("\nStart with browser_snapshot.");
    s
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec { name: name.into(), description: description.into(), schema: json!({"type": "object", "properties": properties, "required": required}) }
}

const REF: &str = "The element's ref from the latest snapshot, such as e12";

/// The tools of an AI Testing run.
pub struct TestingTools<B: Browser> {
    pub browser: B,
    pub job: Job,
    /// The profile's variables with their values (filled in by oc-browser as it types).
    pub variables: HashMap<String, String>,
    /// Every global variable, for sample runs, and the secure values to hide.
    pub all_values: HashMap<String, String>,
    pub secrets: Vec<String>,
    session: tokio::sync::Mutex<Option<String>>,
    actions: Mutex<Vec<Recorded>>,
    proposals: Mutex<Vec<Proposal>>,
}

impl<B: Browser> TestingTools<B> {
    pub fn new(browser: B, job: Job, variables: HashMap<String, String>, all_values: HashMap<String, String>, secrets: Vec<String>) -> Self {
        TestingTools { browser, job, variables, all_values, secrets, session: tokio::sync::Mutex::new(None), actions: Mutex::new(Vec::new()), proposals: Mutex::new(Vec::new()) }
    }

    pub fn proposals(&self) -> Vec<Proposal> {
        self.proposals.lock().unwrap().clone()
    }

    /// Ends the browser session, if one was opened.
    pub async fn finish(&self) {
        if let Some(id) = self.session.lock().await.take() {
            self.browser.end_session(&id).await;
        }
    }
}

impl<B: Browser> TestingTools<B> {
    fn mask(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in self.secrets.iter().chain(self.variables.values()) {
            if secret.len() >= 3 {
                out = out.replace(secret.as_str(), "******");
            }
        }
        out
    }

    /// The open session, opened at the starting URL on first use.
    async fn session(&self) -> Result<(String, Option<ActionResult>), String> {
        let mut s = self.session.lock().await;
        if let Some(id) = s.as_ref() {
            return Ok((id.clone(), None));
        }
        let opened = self.browser.open_session(&OpenSession { start_url: &self.job.start_url, device_id: &self.job.device_id, allowed_hosts: self.job.allowed_hosts() }).await?;
        let id = opened.id.clone().ok_or("oc-browser opened no session")?;
        *s = Some(id.clone());
        Ok((id, Some(opened)))
    }

    /// One action in the browser; a step it records gets the next action number.
    async fn act(&self, action: Value) -> Result<String, String> {
        let (id, _) = self.session().await?;
        if let Some(text) = action["text"].as_str().filter(|_| action["type"] == "type") {
            let unknown: Vec<String> = crate::routes::synthetics_settings::placeholders(text).into_iter().filter(|n| !self.variables.contains_key(n)).collect();
            if !unknown.is_empty() {
                return Err(format!("Unknown variables: {} (use the agent profile's placeholders)", unknown.join(", ")));
            }
        }
        let r = self.browser.act(&id, &action, &self.variables).await?;
        let snapshot: String = r.snapshot.chars().take(MAX_SNAPSHOT).collect();
        let requests = if r.requests.is_empty() {
            String::new()
        } else {
            let calls: Vec<String> = r.requests.iter().take(10).map(|q| format!("{} {} → {}", q["method"].as_str().unwrap_or("GET"), q["url"].as_str().unwrap_or_default(), q["status"])).collect();
            format!("\nAPI calls: {}", calls.join("; "))
        };
        if !r.ok {
            return Err(self.mask(&format!("The action failed: {}\n\n{snapshot}{requests}", r.error.unwrap_or_default())));
        }
        let head = match r.step {
            Some(step) => {
                let mut actions = self.actions.lock().unwrap();
                actions.push(Recorded { url_before: r.url_before.clone(), step: step.clone() });
                format!("Action #{} recorded: {}", actions.len(), step.name)
            }
            None => "Done.".to_string(),
        };
        Ok(self.mask(&format!("{head}\n\n{snapshot}{requests}")))
    }

    async fn propose(&self, input: &Value) -> Result<String, String> {
        if self.proposals.lock().unwrap().len() >= MAX_PROPOSALS {
            return Err(format!("A job proposes at most {MAX_PROPOSALS} tests; conclude now"));
        }
        let text = |k: &str| input[k].as_str().map(str::trim).unwrap_or_default().to_string();
        let (suite, name) = (text("suite"), text("name"));
        if suite.is_empty() || name.is_empty() {
            return Err("Name the test and its suite (the journey it belongs to)".into());
        }
        let ids: Vec<usize> = input["action_ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).map(|n| n as usize).collect()).unwrap_or_default();
        if ids.is_empty() || ids.len() > MAX_TEST_STEPS {
            return Err(format!("List the 1–{MAX_TEST_STEPS} recorded actions the test repeats, in order"));
        }
        let recorded = {
            let actions = self.actions.lock().unwrap();
            ids.iter().map(|&n| actions.get(n.wrapping_sub(1)).cloned().ok_or(format!("There is no recorded action #{n}"))).collect::<Result<Vec<_>, _>>()?
        };
        // The test starts where its first action happened; a leading navigation becomes the starting URL.
        let mut steps: Vec<Step> = recorded.iter().map(|r| r.step.clone()).collect();
        let start_url = match steps.first() {
            Some(s) if s.r#type == steps::StepType::GoToUrl => {
                let url = s.params.text();
                steps.remove(0);
                url
            }
            _ => recorded[0].url_before.clone(),
        };
        steps::check_steps(&mut steps)?;
        let run = self
            .browser
            .run_test(&RunRequest { start_url: &start_url, steps: &steps, variables: &self.all_values, secrets: &self.secrets, device_id: &self.job.device_id, video: VideoMode::OnFailure })
            .await
            .map(|r| crate::routes::synthetics_browser::split(r).0);
        let (verification, outcome) = match run {
            Ok(v) if v.passed => (Some(v), "Its sample run passed.".to_string()),
            Ok(v) => {
                let why = v.error.clone().unwrap_or_default();
                (Some(v), format!("Its sample run failed: {why}. Fix it (more stable actions, a wait for the page) and propose it again, or leave it for review as it is."))
            }
            Err(e) => (None, format!("It couldn't be run as a sample: {e}")),
        };
        let mut proposals = self.proposals.lock().unwrap();
        proposals.push(Proposal { suite, name: name.clone(), description: text("description"), start_url, device_id: self.job.device_id.clone(), steps, verification });
        Ok(self.mask(&format!("Proposed \"{name}\" ({} of {MAX_PROPOSALS}). {outcome}", proposals.len())))
    }
}

impl<B: Browser> Tools for TestingTools<B> {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![
            spec("browser_snapshot", "The current page: URL, title, headings, its interactive elements with refs ([e12]) and its text.", json!({}), &[]),
            spec("browser_goto", "Open a URL in the browser (recorded as a navigation step).", json!({"url": {"type": "string"}}), &["url"]),
            spec("browser_click", "Click an element (recorded).", json!({"ref": {"type": "string", "description": REF}}), &["ref"]),
            spec(
                "browser_type",
                "Clear a text field and type into it (recorded). Type credentials only with the agent profile's {{ PLACEHOLDERS }}; for anything else use example values (qa@example.com).",
                json!({"ref": {"type": "string", "description": REF}, "text": {"type": "string"}}),
                &["ref", "text"],
            ),
            spec("browser_select", "Choose an option of a select by its label or value (recorded).", json!({"ref": {"type": "string", "description": REF}, "value": {"type": "string"}}), &["ref", "value"]),
            spec("browser_press", "Press a key: Enter, Tab, Escape, ArrowDown… (recorded).", json!({"key": {"type": "string"}}), &["key"]),
            spec("browser_back", "Go back to the previous page (not recorded).", json!({}), &[]),
            spec("browser_scroll", "Scroll the page to see more of it (not recorded).", json!({"direction": {"type": "string", "enum": ["down", "up"]}}), &[]),
            spec(
                "browser_check",
                "Check something on the page now; when it holds it is recorded as an assertion step. Tests need assertions that prove the journey worked.",
                json!({
                    "check": {"type": "string", "enum": ["page_contains", "url_contains", "element_visible", "element_text"]},
                    "text": {"type": "string", "description": "The text the page, URL or element must contain"},
                    "ref": {"type": "string", "description": "For element checks: the element's ref"},
                }),
                &["check"],
            ),
            spec(
                "propose_test",
                "Propose a browser test made of recorded actions (their numbers, in order), with at least one check. It is run once as a sample, then waits for the user's review in its suite.",
                json!({
                    "suite": {"type": "string", "description": "The user journey it belongs to (Checkout, Search, Sign-in…)"},
                    "name": {"type": "string"},
                    "description": {"type": "string", "description": "What it proves, in one sentence"},
                    "action_ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &["suite", "name", "action_ids"],
            ),
        ]
    }

    fn writes(&self, _name: &str) -> bool {
        false
    }

    async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
        let s = |k: &str| input[k].as_str().unwrap_or_default().to_string();
        match name {
            "browser_snapshot" => {
                if let (_, Some(opened)) = self.session().await? {
                    return Ok(self.mask(&opened.snapshot.chars().take(MAX_SNAPSHOT).collect::<String>()));
                }
                self.act(json!({"type": "snapshot"})).await
            }
            "browser_goto" => self.act(json!({"type": "goto", "url": s("url")})).await,
            "browser_click" => self.act(json!({"type": "click", "ref": s("ref")})).await,
            "browser_type" => self.act(json!({"type": "type", "ref": s("ref"), "text": s("text")})).await,
            "browser_select" => self.act(json!({"type": "select", "ref": s("ref"), "value": s("value")})).await,
            "browser_press" => self.act(json!({"type": "press", "key": s("key")})).await,
            "browser_back" => self.act(json!({"type": "back"})).await,
            "browser_scroll" => self.act(json!({"type": "scroll", "direction": input["direction"].as_str().unwrap_or("down")})).await,
            "browser_check" => {
                let check = match s("check").as_str() {
                    "page_contains" => "pageContains",
                    "url_contains" => "urlContains",
                    "element_visible" => "elementVisible",
                    "element_text" => "elementText",
                    other => return Err(format!("Unknown check {other}")),
                };
                self.act(json!({"type": "assert", "assert": check, "text": s("text"), "ref": s("ref")})).await
            }
            "propose_test" => self.propose(input).await,
            other => Err(format!("Unknown tool {other}")),
        }
    }
}

// ================================================================ jobs

/// A new AI Testing job.
#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingJobInput {
    pub start_url: String,
    /// Keep the browser on the starting URL's domain and its subdomains (default true).
    #[serde(default = "yes")]
    pub restrict_domain: bool,
    /// chrome.laptop_large (default), chrome.tablet or chrome.mobile_small.
    #[serde(default)]
    pub device_id: String,
    /// What to test (empty: explore the site and find its key journeys).
    #[serde(default)]
    pub prompt: String,
    /// The agent profile lending credentials; none: an anonymous visitor.
    #[serde(default)]
    pub profile_id: Option<String>,
}

/// Checks a job and turns it into the run's subject and title.
pub fn check_job(input: AiTestingJobInput) -> Result<(Job, String), String> {
    let start_url = input.start_url.trim().to_string();
    if start_url.chars().count() > 2048 {
        return Err("The starting URL is at most 2048 characters".into());
    }
    steps::check_url(&start_url).map_err(|_| "Enter the site's http or https starting URL, such as https://staging.shop.example.com".to_string())?;
    if !vars::placeholders(&start_url).is_empty() {
        return Err("The starting URL can't use variables".into());
    }
    let device_id = match input.device_id.trim() {
        "" => crate::routes::synthetics::BROWSER_DEVICES[0].to_string(),
        d if crate::routes::synthetics::BROWSER_DEVICES.contains(&d) => d.to_string(),
        d => return Err(format!("Unknown device {d}")),
    };
    let prompt = input.prompt.trim().to_string();
    if prompt.chars().count() > 4000 {
        return Err("Describe what to test in at most 4000 characters".into());
    }
    let host = url::Url::parse(&start_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let title = match prompt.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
        Some(line) => line.chars().take(120).collect(),
        None => format!("Explore {host}"),
    };
    Ok((Job { start_url, restrict_domain: input.restrict_domain, device_id, prompt, profile_id: input.profile_id.filter(|p| !p.trim().is_empty()) }, title))
}

/// Starts a job: an Assist run of kind `testing`, run by the worker as the member who started it.
#[utoipa::path(post, path = "/api/v1/synthetics/ai-testing/jobs", operation_id = "ai_testing_start_job", request_body = AiTestingJobInput,
    responses((status = 200, body = RunSummary), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn start_job(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<AiTestingJobInput>) -> ApiResult<RunSummary> {
    if st.config.browser.is_none() {
        return Err(ApiError::BadRequest("AI Testing needs oc-browser, which this server doesn't use (set OC_BROWSER_URL and OC_BROWSER_TOKEN)".into()));
    }
    let (job, title) = check_job(input).map_err(ApiError::BadRequest)?;
    if let Some(p) = &job.profile_id {
        load_profile(&st, &ctx.org_id, p).await?;
    }
    if crate::routes::assist_provider::resolve(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::BadRequest("Assist has no model: an admin sets one in Organization Settings → Assist AI".into()));
    }
    assist_runs::within_limits(&st, &ctx.org_id).await?;
    let subject = serde_json::to_value(&job).map_err(|e| ApiError::Internal(e.into()))?;
    let run = assist_runs::insert_run(&st, &ctx.org_id, Kind::Testing, &title, "manual", &subject, &ctx.user_id, &ctx.name).await?;
    ctx.audit(&st, "synthetics.ai_testing_started", &run.id, &format!("{title} ({})", job.start_url)).await;
    Ok(Json(run))
}

/// The org's browser tests in one line each, so the agent doesn't propose them again.
async fn existing_tests(st: &AppState, org: &str) -> Result<Vec<String>, String> {
    let rows: Vec<(String, Value, i64)> = sqlx::query_as("SELECT name, request, jsonb_array_length(steps)::bigint FROM synthetic_tests WHERE org_id = $1 AND type = 'browser' ORDER BY name LIMIT 50")
        .bind(org)
        .fetch_all(&st.postgres)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(name, request, n)| format!("{name}: {} ({n} steps)", request["url"].as_str().unwrap_or_default())).collect())
}

/// Runs a claimed job with oc-browser (assist_runs' worker calls it for runs of kind `testing`).
#[allow(clippy::too_many_arguments)]
pub async fn run_job<M: Model + Sync>(st: &AppState, org: &str, id: &str, subject: &Value, model: &M, system: &str, steps: &Arc<std::sync::Mutex<Vec<RunStep>>>, msgs: &mut Vec<Msg>, usage: &mut Usage) -> Result<Done, String> {
    let settings = st.config.browser.clone().ok_or("AI Testing needs oc-browser, which this server doesn't use (OC_BROWSER_URL)")?;
    run_job_with(st, org, id, subject, model, BrowserClient::new(settings), system, steps, msgs, usage).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_job_with<M: Model + Sync, B: Browser>(
    st: &AppState,
    org: &str,
    id: &str,
    subject: &Value,
    model: &M,
    browser: B,
    system: &str,
    steps: &Arc<std::sync::Mutex<Vec<RunStep>>>,
    msgs: &mut Vec<Msg>,
    usage: &mut Usage,
) -> Result<Done, String> {
    let job: Job = serde_json::from_value(subject.clone()).map_err(|e| format!("Invalid job: {e}"))?;
    let profile = match &job.profile_id {
        Some(p) => load_profile(st, org, p).await.map_err(|e| e.to_string())?.variables.0,
        None => Vec::new(),
    };
    let (values, secrets) = vars::load_variables(st, org).await.map_err(|e| e.to_string())?;
    let mut variables = HashMap::new();
    for v in &profile {
        let value = values.get(&v.variable).ok_or(format!("The agent profile uses the global variable {}, which doesn't exist", v.variable))?;
        variables.insert(v.variable.clone(), value.clone());
    }
    msgs.push(Msg::User(opening(&job, &profile, &existing_tests(st, org).await?)));
    let tools = RunTools::new(TestingTools::new(browser, job.clone(), variables, values, secrets), Kind::Testing, false);
    let l = Loop { model, tools: &tools, system, auto_approve: false, max_steps: MAX_TURNS };
    let cancelled = AtomicBool::new(false);
    let outcome = {
        let sink = steps.clone();
        let mut emit = move |ev: agent::Event| assist_runs::apply(&mut sink.lock().unwrap(), &ev, chrono::Utc::now().timestamp_millis());
        let work = l.run(msgs, usage, &mut emit);
        let watch = assist_runs::watch(st, org, id, steps, &cancelled);
        tokio::select! {
            o = work => o,
            _ = watch => Outcome::Failed("cancelled".into()),
        }
    };
    tools.base().finish().await;
    // What was proposed is kept even when the run stops early.
    save_proposals(st, org, id, &tools.base().proposals()).await.map_err(|e| e.to_string())?;
    if cancelled.load(Ordering::Relaxed) {
        return Ok(Done { status: "cancelled", conclusion: None, error: None, change: None });
    }
    let proposed = tools.base().proposals().len();
    Ok(match (tools.conclusion(), outcome) {
        (Some(c), _) => Done { status: "concluded", conclusion: Some(c), error: None, change: None },
        (None, Outcome::Failed(why)) => Done { status: "failed", conclusion: None, error: Some(why), change: None },
        (None, _) => {
            let text = msgs.iter().rev().find_map(|m| if let Msg::Assistant { text, .. } = m { (!text.trim().is_empty()).then(|| text.clone()) } else { None }).unwrap_or_default();
            let verdict = if proposed > 0 { "tests_proposed" } else { "no_tests" };
            Done { status: "concluded", conclusion: Some(Conclusion { summary: text, verdict: verdict.into(), confidence: "low".into(), ..Default::default() }), error: None, change: None }
        }
    })
}

async fn save_proposals(st: &AppState, org: &str, run: &str, proposals: &[Proposal]) -> Result<(), sqlx::Error> {
    for p in proposals {
        sqlx::query("INSERT INTO ai_testing_proposals (org_id, run_id, suite, name, description, start_url, device_id, steps, verification) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9)")
            .bind(org)
            .bind(run)
            .bind(&p.suite)
            .bind(&p.name)
            .bind(&p.description)
            .bind(&p.start_url)
            .bind(&p.device_id)
            .bind(sqlx::types::Json(&p.steps))
            .bind(p.verification.as_ref().map(sqlx::types::Json))
            .execute(&st.postgres)
            .await?;
    }
    Ok(())
}

// ================================================================ proposals

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingProposal {
    pub id: String,
    pub run_id: String,
    /// The journey it belongs to; accepted tests of a job's suite become a test suite.
    pub suite: String,
    pub name: String,
    pub description: String,
    pub start_url: String,
    pub device_id: String,
    #[schema(value_type = Vec<Step>)]
    pub steps: sqlx::types::Json<Vec<Step>>,
    /// The sample run.
    #[schema(value_type = Option<SyntheticRunDetail>)]
    pub verification: Option<sqlx::types::Json<SyntheticRunDetail>>,
    /// pending | accepted | rejected
    pub status: String,
    /// The test made from it, once accepted.
    pub test_id: Option<String>,
    pub feedback: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingProposalSearch {
    /// pending | accepted | rejected; all when absent.
    #[serde(default)]
    pub status: Option<String>,
    /// One job's proposals.
    #[serde(default)]
    pub run_id: Option<String>,
}

const PROPOSAL: &str = "SELECT id::text, run_id::text, suite, name, description, start_url, device_id, steps, verification, status, test_id::text, feedback, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM decided_at) * 1000)::bigint AS decided_at FROM ai_testing_proposals";

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// The org's proposals, newest first (Suites to review: status pending).
#[utoipa::path(post, path = "/api/v1/synthetics/ai-testing/proposals/search", operation_id = "ai_testing_search_proposals", request_body = AiTestingProposalSearch,
    responses((status = 200, body = Vec<AiTestingProposal>)), tag = "synthetics")]
pub async fn search_proposals(State(st): State<AppState>, ctx: AuthContext, Json(q): Json<AiTestingProposalSearch>) -> ApiResult<Vec<AiTestingProposal>> {
    let run = q.run_id.filter(|r| is_uuid(r));
    let rows: Vec<AiTestingProposal> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{PROPOSAL} WHERE org_id = $1 AND ($2::text IS NULL OR status = $2) AND ($3::uuid IS NULL OR run_id = $3::uuid) ORDER BY created_at DESC, suite, name LIMIT 500"
    )))
    .bind(&ctx.org_id)
    .bind(&q.status)
    .bind(&run)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingDecision {
    pub ids: Vec<String>,
    /// Why they were ignored (rejecting only).
    #[serde(default)]
    pub feedback: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingAccepted {
    pub test_ids: Vec<String>,
    pub suite_ids: Vec<String>,
}

async fn pending(st: &AppState, org: &str, ids: &[String]) -> Result<Vec<AiTestingProposal>, ApiError> {
    let ids: Vec<String> = ids.iter().filter(|i| is_uuid(i)).cloned().collect();
    if ids.is_empty() || ids.len() > 100 {
        return Err(ApiError::BadRequest("Choose 1–100 proposals".into()));
    }
    let rows: Vec<AiTestingProposal> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{PROPOSAL} WHERE org_id = $1 AND id = ANY($2::uuid[]) AND status = 'pending' ORDER BY created_at, name")))
        .bind(org)
        .bind(&ids)
        .fetch_all(&st.postgres)
        .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound("No pending proposals among them".into()));
    }
    Ok(rows)
}

/// Accepts proposals: each becomes a browser test (every 15 minutes), and each job's suite a test suite of them.
#[utoipa::path(post, path = "/api/v1/synthetics/ai-testing/proposals/accept", operation_id = "ai_testing_accept_proposals", request_body = AiTestingDecision,
    responses((status = 200, body = AiTestingAccepted), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn accept(State(st): State<AppState>, Writer(ctx): Writer, Json(d): Json<AiTestingDecision>) -> ApiResult<AiTestingAccepted> {
    let rows = pending(&st, &ctx.org_id, &d.ids).await?;
    let mut groups: Vec<((String, String), Vec<String>)> = Vec::new();
    let mut test_ids = Vec::new();
    for p in rows {
        let input: crate::routes::synthetics::SyntheticTestInput = serde_json::from_value(json!({
            "name": p.name, "type": "browser", "request": {"url": p.start_url, "deviceId": p.device_id}, "steps": p.steps.0,
            "frequencySeconds": ACCEPTED_FREQUENCY, "tags": ["source:ai-testing"], "message": ACCEPTED_MESSAGE,
        }))
        .map_err(|e| ApiError::Internal(e.into()))?;
        let Json(test) = crate::routes::synthetics::create(State(st.clone()), Writer(ctx.clone()), Json(input)).await?;
        sqlx::query("UPDATE ai_testing_proposals SET status = 'accepted', test_id = $3::uuid, decided_at = now(), decided_by = $4::uuid WHERE org_id = $1 AND id = $2::uuid")
            .bind(&ctx.org_id)
            .bind(&p.id)
            .bind(&test.id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await?;
        let key = (p.run_id.clone(), p.suite.clone());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, ids)) => ids.push(test.id.clone()),
            None => groups.push((key, vec![test.id.clone()])),
        }
        test_ids.push(test.id);
    }
    let mut suite_ids = Vec::new();
    for ((_, suite), ids) in groups {
        let input: crate::routes::synthetics_suites::SyntheticSuiteInput =
            serde_json::from_value(json!({"name": suite, "description": "Proposed by AI Testing", "tags": ["source:ai-testing"], "testIds": ids})).map_err(|e| ApiError::Internal(e.into()))?;
        let Json(s) = crate::routes::synthetics_suites::create_suite(State(st.clone()), Writer(ctx.clone()), Json(input)).await?;
        suite_ids.push(s.id);
    }
    ctx.audit(&st, "synthetics.ai_testing_accepted", &test_ids.join(","), &format!("{} tests in {} suites", test_ids.len(), suite_ids.len())).await;
    Ok(Json(AiTestingAccepted { test_ids, suite_ids }))
}

/// Ignores proposals, with optional feedback.
#[utoipa::path(post, path = "/api/v1/synthetics/ai-testing/proposals/reject", operation_id = "ai_testing_reject_proposals", request_body = AiTestingDecision,
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn reject(State(st): State<AppState>, Writer(ctx): Writer, Json(d): Json<AiTestingDecision>) -> Result<StatusCode, ApiError> {
    let rows = pending(&st, &ctx.org_id, &d.ids).await?;
    let ids: Vec<String> = rows.into_iter().map(|p| p.id).collect();
    let feedback: String = d.feedback.trim().chars().take(2000).collect();
    sqlx::query("UPDATE ai_testing_proposals SET status = 'rejected', feedback = $3, decided_at = now(), decided_by = $4::uuid WHERE org_id = $1 AND id = ANY($2::uuid[])")
        .bind(&ctx.org_id)
        .bind(&ids)
        .bind(&feedback)
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ agent profiles

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingProfile {
    pub id: String,
    pub name: String,
    #[schema(value_type = Vec<AgentProfileVariable>)]
    pub variables: sqlx::types::Json<Vec<AgentProfileVariable>>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiTestingProfileInput {
    pub name: String,
    #[serde(default)]
    pub variables: Vec<AgentProfileVariable>,
}

/// Checks and normalizes a profile.
pub fn check_profile(input: &mut AiTestingProfileInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > 100 {
        return Err("The profile's name is 1–100 characters".into());
    }
    if input.variables.is_empty() || input.variables.len() > MAX_PROFILE_VARIABLES {
        return Err(format!("A profile lends 1–{MAX_PROFILE_VARIABLES} global variables"));
    }
    for v in &mut input.variables {
        v.usage = v.usage.trim().to_ascii_lowercase();
        if !USAGES.contains(&v.usage.as_str()) {
            return Err(format!("A variable's usage is one of {}", USAGES.join(", ")));
        }
        v.variable = vars::check_variable_name(&v.variable)?;
    }
    Ok(())
}

const PROFILE: &str = "SELECT id::text, name, variables, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM ai_testing_profiles";

async fn load_profile(st: &AppState, org: &str, id: &str) -> Result<AiTestingProfile, ApiError> {
    if !is_uuid(id) {
        return Err(ApiError::NotFound("Agent profile not found".into()));
    }
    let row: Option<AiTestingProfile> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{PROFILE} WHERE org_id = $1 AND id = $2::uuid"))).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(|| ApiError::NotFound("Agent profile not found".into()))
}

fn duplicate(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("An agent profile named {name} exists")),
        _ => e.into(),
    }
}

#[utoipa::path(get, path = "/api/v1/synthetics/ai-testing/profiles", operation_id = "ai_testing_list_profiles", responses((status = 200, body = Vec<AiTestingProfile>)), tag = "synthetics")]
pub async fn list_profiles(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AiTestingProfile>> {
    let rows: Vec<AiTestingProfile> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{PROFILE} WHERE org_id = $1 ORDER BY lower(name)"))).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v1/synthetics/ai-testing/profiles", operation_id = "ai_testing_create_profile", request_body = AiTestingProfileInput,
    responses((status = 200, body = AiTestingProfile), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn create_profile(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<AiTestingProfileInput>) -> ApiResult<AiTestingProfile> {
    check_profile(&mut input).map_err(ApiError::BadRequest)?;
    let id: String = sqlx::query_scalar("INSERT INTO ai_testing_profiles (org_id, name, variables, created_by) VALUES ($1, $2, $3, $4::uuid) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(sqlx::types::Json(&input.variables))
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await
        .map_err(|e| duplicate(e, &input.name))?;
    Ok(Json(load_profile(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/synthetics/ai-testing/profiles/{id}", operation_id = "ai_testing_update_profile", params(("id" = String, Path)), request_body = AiTestingProfileInput,
    responses((status = 200, body = AiTestingProfile), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn update_profile(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<AiTestingProfileInput>) -> ApiResult<AiTestingProfile> {
    check_profile(&mut input).map_err(ApiError::BadRequest)?;
    load_profile(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE ai_testing_profiles SET name = $3, variables = $4, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(sqlx::types::Json(&input.variables))
        .execute(&st.postgres)
        .await
        .map_err(|e| duplicate(e, &input.name))?;
    Ok(Json(load_profile(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/synthetics/ai-testing/profiles/{id}", operation_id = "ai_testing_delete_profile", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn delete_profile(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load_profile(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM ai_testing_profiles WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::client::BrowserRunResult;
    use std::sync::Arc;

    /// A scripted browser: a shop whose pages answer actions; keeps what it was asked.
    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        variables_seen: Arc<Mutex<Vec<HashMap<String, String>>>>,
        sample_passes: bool,
    }

    fn page(url: &str, extra: &str) -> ActionResult {
        ActionResult { ok: true, url: url.into(), title: "Example Shop".into(), snapshot: format!("URL: {url}\nElements:\n[e1] link \"Cart\" → /cart\n[e2] textbox \"Password\" (password)\n[e3] button \"Sign in\"\n{extra}"), ..Default::default() }
    }

    fn click_step(css: &str) -> Step {
        serde_json::from_value(json!({"name": format!("Click on {css}"), "type": "click", "params": {"element": {"userLocator": {"values": [{"type": "css", "value": css}]}}}})).unwrap()
    }

    impl Browser for Fake {
        async fn open_session(&self, req: &OpenSession<'_>) -> Result<ActionResult, String> {
            self.log.lock().unwrap().push(format!("open {} {:?}", req.start_url, req.allowed_hosts));
            Ok(ActionResult { id: Some("s1".into()), ..page(req.start_url, "") })
        }
        async fn act(&self, id: &str, action: &Value, variables: &HashMap<String, String>) -> Result<ActionResult, String> {
            self.log.lock().unwrap().push(format!("act {id} {action}"));
            self.variables_seen.lock().unwrap().push(variables.clone());
            let before = "https://shop.example.com/".to_string();
            Ok(match (action["type"].as_str(), action["ref"].as_str()) {
                (Some("click"), Some("e1")) => ActionResult { url_before: before, step: Some(click_step("#cart")), ..page("https://shop.example.com/cart", "") },
                (Some("click"), Some("e9")) => ActionResult { ok: false, error: Some("No element e9 on the page".into()), url_before: before.clone(), ..page(&before, "") },
                (Some("type"), _) => {
                    let step = serde_json::from_value(json!({"name": "Type text on Password", "type": "typeText", "params": {"element": {"userLocator": {"values": [{"type": "css", "value": "#password"}]}}, "value": action["text"]}})).unwrap();
                    // A careless page could echo the value: the tools hide it anyway.
                    ActionResult { url_before: before, step: Some(step), ..page("https://shop.example.com/", &format!("Text: typed {}", variables.values().cloned().collect::<String>())) }
                }
                (Some("assert"), _) => ActionResult {
                    url_before: before,
                    step: Some(serde_json::from_value(json!({"name": "Test page contains 'Your cart'", "type": "assertPageContains", "params": {"value": "Your cart"}})).unwrap()),
                    ..page("https://shop.example.com/cart", "")
                },
                _ => page(&before, ""),
            })
        }
        async fn end_session(&self, id: &str) {
            self.log.lock().unwrap().push(format!("end {id}"));
        }
        async fn run_test(&self, req: &RunRequest<'_>) -> Result<BrowserRunResult, String> {
            self.log.lock().unwrap().push(format!("run {} {} steps", req.start_url, req.steps.len()));
            Ok(serde_json::from_value(json!({
                "passed": self.sample_passes, "durationMs": 1800, "startUrl": req.start_url, "device": {"id": "chrome.laptop_large", "width": 1440, "height": 1100},
                "error": if self.sample_passes { Value::Null } else { json!("Cannot locate element #cart") },
                "steps": [{"name": "Navigate", "type": "goToUrl", "status": "passed", "durationMs": 900, "screenshot": "/9j/x"}],
                "video": "GkXf"
            }))
            .unwrap())
        }
    }

    fn job() -> Job {
        Job { start_url: "https://www.shop.example.com/".into(), restrict_domain: true, device_id: "chrome.laptop_large".into(), prompt: "Check that shoppers can open their cart".into(), profile_id: None }
    }

    fn tools(fake: Fake) -> TestingTools<Fake> {
        let vars = HashMap::from([("SHOP_PASSWORD".to_string(), "hunter2".to_string())]);
        TestingTools::new(fake, job(), vars.clone(), vars, vec!["hunter2".into()])
    }

    #[test]
    fn a_restricted_job_stays_on_its_domain() {
        assert_eq!(job().allowed_hosts(), Some(vec!["shop.example.com".to_string()]));
        assert_eq!(Job { restrict_domain: false, ..job() }.allowed_hosts(), None);
    }

    #[test]
    fn the_opening_names_the_goal_the_variables_and_the_existing_tests_but_no_secret() {
        let profile = vec![AgentProfileVariable { usage: "password".into(), variable: "SHOP_PASSWORD".into() }];
        let o = opening(&job(), &profile, &["Checkout flow: https://shop.example.com/ (7 steps)".into()]);
        assert!(o.contains("https://www.shop.example.com/") && o.contains("Check that shoppers can open their cart"));
        assert!(o.contains("password: {{ SHOP_PASSWORD }}"));
        assert!(o.contains("Checkout flow"));
        assert!(o.contains("shop.example.com and its subdomains"));
        let explore = opening(&Job { prompt: String::new(), ..job() }, &[], &[]);
        assert!(explore.contains("key user journeys") && explore.contains("anonymous visitor"));
    }

    #[tokio::test]
    async fn the_browser_opens_at_the_start_url_and_actions_are_numbered() {
        let fake = Fake::default();
        let t = tools(fake.clone());
        let names: Vec<String> = t.specs().into_iter().map(|s| s.name).collect();
        for n in ["browser_snapshot", "browser_goto", "browser_click", "browser_type", "browser_select", "browser_press", "browser_back", "browser_scroll", "browser_check", "propose_test"] {
            assert!(names.contains(&n.to_string()), "{n} in {names:?}");
        }
        let first = t.run("browser_snapshot", &json!({})).await.unwrap();
        assert!(first.contains("[e1] link \"Cart\""));
        let clicked = t.run("browser_click", &json!({"ref": "e1"})).await.unwrap();
        assert!(clicked.starts_with("Action #1 recorded: Click on #cart"), "{clicked}");
        let failed = t.run("browser_click", &json!({"ref": "e9"})).await.unwrap_err();
        assert!(failed.contains("No element e9") && failed.contains("URL:"), "the model still sees the page: {failed}");
        let log = fake.log.lock().unwrap().clone();
        assert_eq!(log[0], "open https://www.shop.example.com/ Some([\"shop.example.com\"])");
        assert!(log[1].starts_with("act s1 ") && log[1].contains("\"e1\""), "the first snapshot comes from opening: {log:?}");
        t.finish().await;
        assert_eq!(fake.log.lock().unwrap().last().unwrap(), "end s1");
    }

    #[tokio::test]
    async fn credentials_are_typed_from_variables_the_model_never_sees() {
        let fake = Fake::default();
        let t = tools(fake.clone());
        let out = t.run("browser_type", &json!({"ref": "e2", "text": "{{ SHOP_PASSWORD }}"})).await.unwrap();
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("Action #1 recorded: Type text on Password"));
        let seen = fake.variables_seen.lock().unwrap().clone();
        assert_eq!(seen.last().unwrap()["SHOP_PASSWORD"], "hunter2");
        // The model can't type a value it read somewhere else into a variable it doesn't know.
        assert!(t.run("browser_type", &json!({"ref": "e2", "text": "{{ OTHER_SECRET }}"})).await.unwrap_err().contains("OTHER_SECRET"));
    }

    #[tokio::test]
    async fn a_proposed_test_is_made_of_recorded_steps_and_run_once() {
        let fake = Fake { sample_passes: true, ..Default::default() };
        let t = tools(fake.clone());
        t.run("browser_click", &json!({"ref": "e1"})).await.unwrap();
        t.run("browser_check", &json!({"check": "page_contains", "text": "Your cart"})).await.unwrap();
        let out = t.run("propose_test", &json!({"suite": "Cart", "name": "Open the cart", "description": "A shopper opens the cart from the home page", "action_ids": [1, 2]})).await.unwrap();
        assert!(out.contains("sample run passed"), "{out}");
        let p = &t.proposals()[0];
        assert_eq!((p.suite.as_str(), p.name.as_str(), p.start_url.as_str()), ("Cart", "Open the cart", "https://shop.example.com/"));
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].params.element.as_ref().unwrap().user_locator.values[0].value, "#cart", "oc-browser's locator, not the model's");
        let v = p.verification.as_ref().unwrap();
        assert!(v.passed && v.browser.as_ref().unwrap().steps[0].screenshot.is_none(), "no screenshots kept");
        assert!(fake.log.lock().unwrap().iter().any(|l| l == "run https://shop.example.com/ 2 steps"));
        // Unknown actions and empty tests are refused.
        assert!(t.run("propose_test", &json!({"suite": "Cart", "name": "x", "action_ids": [7]})).await.unwrap_err().contains("#7"));
        assert!(t.run("propose_test", &json!({"suite": "Cart", "name": "x", "action_ids": []})).await.is_err());
    }

    fn job_input(v: Value) -> AiTestingJobInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn jobs_are_checked() {
        let (job, title) = check_job(job_input(json!({"startUrl": " https://staging.shop.example.com/ "}))).unwrap();
        assert_eq!((job.start_url.as_str(), job.device_id.as_str(), job.restrict_domain), ("https://staging.shop.example.com/", "chrome.laptop_large", true));
        assert_eq!(title, "Explore staging.shop.example.com");
        let (_, title) = check_job(job_input(json!({"startUrl": "https://shop.example.com", "prompt": "Check the contact form\nwithout submitting it"}))).unwrap();
        assert_eq!(title, "Check the contact form");
        for (bad, word) in [
            (json!({"startUrl": "shop.example.com"}), "starting URL"),
            (json!({"startUrl": "ftp://shop.example.com"}), "starting URL"),
            (json!({"startUrl": "https://{{ HOST }}/"}), "variables"),
            (json!({"startUrl": "https://shop.example.com", "deviceId": "nokia"}), "device"),
            (json!({"startUrl": "https://shop.example.com", "prompt": "x".repeat(4001)}), "4000"),
        ] {
            assert!(check_job(job_input(bad.clone())).unwrap_err().contains(word), "{bad}");
        }
    }

    #[test]
    fn profiles_are_checked() {
        let mut ok = AiTestingProfileInput { name: " QA shopper ".into(), variables: vec![AgentProfileVariable { usage: "Password".into(), variable: " SHOP_PASSWORD ".into() }] };
        check_profile(&mut ok).unwrap();
        assert_eq!((ok.name.as_str(), ok.variables[0].usage.as_str(), ok.variables[0].variable.as_str()), ("QA shopper", "password", "SHOP_PASSWORD"));
        let bad = |name: &str, usage: &str, variable: &str| check_profile(&mut AiTestingProfileInput { name: name.into(), variables: vec![AgentProfileVariable { usage: usage.into(), variable: variable.into() }] }).unwrap_err();
        assert!(bad("", "password", "X").contains("name"));
        assert!(bad("p", "pin", "X").contains("usage"));
        assert!(bad("p", "password", "shop password").contains("upper-case"));
        assert!(check_profile(&mut AiTestingProfileInput { name: "p".into(), variables: vec![] }).unwrap_err().contains("1–10"));
    }

    /// A model that plays a script of tool calls.
    struct Script(Mutex<Vec<crate::assist::llm::Turn>>);

    impl Model for Script {
        async fn turn(&self, _: &str, _: &[ToolSpec], _: &[Msg], _: &mut (dyn FnMut(&str) + Send)) -> Result<crate::assist::llm::Turn, String> {
            self.0.lock().unwrap().pop().ok_or_else(|| "no more turns".to_string())
        }
    }

    fn call(id: &str, name: &str, input: Value) -> crate::assist::llm::Turn {
        use crate::assist::llm::{Stop, ToolCall, Turn};
        Turn { text: String::new(), calls: vec![ToolCall { id: id.into(), name: name.into(), input: Ok(input) }], blocks: None, stop: Stop::ToolUse, usage: Usage { input: 50, output: 5 } }
    }

    /// A job end to end on the live databases: the scripted agent explores, proposes a test and concludes; the
    /// proposal waits for review, then becomes a test in a suite; nothing of it shows in another org.
    #[tokio::test]
    async fn a_job_proposes_tests_that_become_a_suite_once_accepted() {
        let settings = crate::browser::client::BrowserSettings { url: "http://127.0.0.1:1".into(), token: "0123456789abcdef".into() };
        let Some((_, states)) = crate::test_support::replicas_with(1, move |c| c.browser = Some(settings.clone())).await else { return };
        let st = &states[0];
        let mut ctx = crate::test_support::admin_of_new_org(st, "ai-testing").await;
        ctx.user_id = sqlx::query_scalar("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, 'qa@example.com', 'QA') RETURNING id::text").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        let (job, title) = check_job(job_input(json!({"startUrl": "https://shop.example.com/", "prompt": "Check that shoppers can open their cart"}))).unwrap();
        let run = assist_runs::insert_run(st, &ctx.org_id, Kind::Testing, &title, "manual", &serde_json::to_value(&job).unwrap(), &ctx.user_id, &ctx.name).await.unwrap();
        let mut turns = vec![
            call("1", "browser_snapshot", json!({})),
            call("2", "browser_click", json!({"ref": "e1"})),
            call("3", "browser_check", json!({"check": "page_contains", "text": "Your cart"})),
            call("4", "propose_test", json!({"suite": "Cart", "name": "Open the cart", "description": "A shopper opens the cart", "action_ids": [1, 2]})),
            call("5", "conclude", json!({"summary": "Proposed one test for the cart journey.", "verdict": "tests_proposed"})),
        ];
        turns.reverse();
        let model = Script(Mutex::new(turns));
        let fake = Fake { sample_passes: true, ..Default::default() };
        let steps = Arc::new(Mutex::new(Vec::new()));
        let (mut msgs, mut usage) = (Vec::new(), Usage::default());
        let done = run_job_with(st, &ctx.org_id, &run.id, &serde_json::to_value(&job).unwrap(), &model, fake.clone(), "system", &steps, &mut msgs, &mut usage).await.unwrap();
        assert_eq!((done.status, done.conclusion.unwrap().verdict.as_str()), ("concluded", "tests_proposed"));
        assert_eq!(fake.log.lock().unwrap().last().unwrap(), "end s1", "the browser session is closed");
        let opening = match &msgs[0] {
            Msg::User(t) => t.clone(),
            _ => panic!("the job starts with its opening"),
        };
        assert!(opening.contains("Check that shoppers can open their cart"));

        let Json(pending) = search_proposals(State(st.clone()), ctx.clone(), Json(AiTestingProposalSearch { status: Some("pending".into()), run_id: Some(run.id.clone()) })).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!((pending[0].suite.as_str(), pending[0].steps.0.len()), ("Cart", 2));
        assert!(pending[0].verification.as_ref().unwrap().0.passed);
        let other = crate::test_support::admin_of_new_org(st, "ai-testing-other").await;
        assert!(search_proposals(State(st.clone()), other.clone(), Json(AiTestingProposalSearch::default())).await.unwrap().0.is_empty());
        assert!(matches!(accept(State(st.clone()), Writer(other), Json(AiTestingDecision { ids: vec![pending[0].id.clone()], feedback: String::new() })).await, Err(ApiError::NotFound(_))));

        let Json(accepted) = accept(State(st.clone()), Writer(ctx.clone()), Json(AiTestingDecision { ids: vec![pending[0].id.clone()], feedback: String::new() })).await.unwrap();
        assert_eq!((accepted.test_ids.len(), accepted.suite_ids.len()), (1, 1));
        let Json(test) = crate::routes::synthetics::get_one(State(st.clone()), ctx.clone(), Path(accepted.test_ids[0].clone())).await.unwrap();
        assert_eq!((test.name.as_str(), test.request.url.as_str(), test.steps.len(), test.frequency_seconds), ("Open the cart", "https://shop.example.com/", 2, 900));
        assert!(test.tags.contains(&"source:ai-testing".to_string()));
        let Json(suite) = crate::routes::synthetics_suites::get_suite(State(st.clone()), ctx.clone(), Path(accepted.suite_ids[0].clone())).await.unwrap();
        assert_eq!((suite.name.as_str(), suite.tests.len()), ("Cart", 1));
        // Decided proposals can't be decided again.
        assert!(reject(State(st.clone()), Writer(ctx.clone()), Json(AiTestingDecision { ids: vec![pending[0].id.clone()], feedback: "dup".into() })).await.is_err());
        let Json(all) = search_proposals(State(st.clone()), ctx.clone(), Json(AiTestingProposalSearch::default())).await.unwrap();
        assert_eq!((all[0].status.as_str(), all[0].test_id.as_deref()), ("accepted", Some(accepted.test_ids[0].as_str())));
    }

    #[tokio::test]
    async fn profiles_belong_to_their_org() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let mut ctx = crate::test_support::admin_of_new_org(st, "ai-profiles").await;
        ctx.user_id = sqlx::query_scalar("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, 'qa@example.com', 'QA') RETURNING id::text").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        let input = || AiTestingProfileInput { name: "QA shopper".into(), variables: vec![AgentProfileVariable { usage: "password".into(), variable: "SHOP_PASSWORD".into() }] };
        let Json(p) = create_profile(State(st.clone()), Writer(ctx.clone()), Json(input())).await.unwrap();
        assert!(matches!(create_profile(State(st.clone()), Writer(ctx.clone()), Json(input())).await, Err(ApiError::BadRequest(m)) if m.contains("exists")));
        assert_eq!(list_profiles(State(st.clone()), ctx.clone()).await.unwrap().0.len(), 1);
        let other = crate::test_support::admin_of_new_org(st, "ai-profiles-other").await;
        assert!(list_profiles(State(st.clone()), other.clone()).await.unwrap().0.is_empty());
        assert!(matches!(delete_profile(State(st.clone()), Writer(other), Path(p.id.clone())).await, Err(ApiError::NotFound(_))));
        assert_eq!(delete_profile(State(st.clone()), Writer(ctx), Path(p.id)).await.unwrap(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn a_failing_sample_run_is_reported_to_the_model_and_kept() {
        let t = tools(Fake::default());
        t.run("browser_click", &json!({"ref": "e1"})).await.unwrap();
        let out = t.run("propose_test", &json!({"suite": "Cart", "name": "Open the cart", "action_ids": [1]})).await.unwrap();
        assert!(out.contains("sample run failed") && out.contains("Cannot locate element #cart"), "{out}");
        assert!(!t.proposals()[0].verification.as_ref().unwrap().passed);
    }
}
