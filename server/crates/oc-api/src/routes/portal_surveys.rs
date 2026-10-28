//! Developer Portal → Surveys: questionnaires sent to the org's developers, and their results.
//!
//! - **Surveys** have Likert (1–5), single choice, multiple choice, free text and NPS (0–10)
//!   questions, an audience (every member, or the members of some teams), optional open and close
//!   dates, and may be anonymous. Standard users and admins create, edit, open and close them.
//! - **Responding** happens in the app (`/portal/surveys/{id}/respond`): any signed-in member of the
//!   org in the audience answers once. Each response carries a respondent key, SHA-256 of the
//!   survey's salt and the user id, unique per survey; anonymous responses keep only that key (no
//!   user id), so duplicates are refused without recording who answered. Responses aren't audited.
//! - **Results** aggregate the answers per question (distributions, Likert averages, the NPS score,
//!   free-text answers) and the response rate per team; the raw responses feed the CSV export.
//! - **On submission** the survey's recipients (notification channel handles or member emails) get a
//!   message through the monitor notifier's channels.

use std::collections::{BTreeMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::notifications::{self, Outgoing};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_SURVEYS: i64 = 200;
const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_QUESTIONS: usize = 50;
const MAX_QUESTION_CHARS: usize = 300;
const MAX_QUESTION_ID_CHARS: usize = 40;
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 20;
const MAX_OPTION_CHARS: usize = 200;
const MAX_AUDIENCE_TEAMS: usize = 50;
const MAX_NOTIFY: usize = 10;
const MAX_TEXT_ANSWER_CHARS: usize = 5000;
/// Free-text answers listed per question in the results.
const MAX_TEXTS: usize = 500;
/// Responses read for results and the export.
const MAX_RESPONSES: i64 = 20_000;

/// Likert answers 1–5, in order.
pub const LIKERT_LABELS: [&str; 5] = ["Strongly disagree", "Disagree", "Neutral", "Agree", "Strongly agree"];

// ================================================================ API types

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SurveyQuestionKind {
    /// Strongly disagree (1) … strongly agree (5).
    Likert,
    /// One of `options`.
    Single,
    /// Any of `options`.
    Multi,
    Text,
    /// Net Promoter Score: 0–10.
    Nps,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SurveyQuestion {
    /// Stable key of the question within the survey (`[a-z0-9_-]`).
    pub id: String,
    pub kind: SurveyQuestionKind,
    pub title: String,
    #[serde(default)]
    #[schema(required = true)]
    pub description: String,
    #[serde(default)]
    #[schema(required = true)]
    pub required: bool,
    /// Single and multiple choice only.
    #[serde(default)]
    #[schema(required = true)]
    pub options: Vec<String>,
}

/// What the author sets: draft, open (accepting responses between the dates) or closed.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SurveyStatus {
    #[default]
    Draft,
    Open,
    Closed,
}

impl SurveyStatus {
    fn as_str(self) -> &'static str {
        match self {
            SurveyStatus::Draft => "draft",
            SurveyStatus::Open => "open",
            SurveyStatus::Closed => "closed",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "open" => SurveyStatus::Open,
            "closed" => SurveyStatus::Closed,
            _ => SurveyStatus::Draft,
        }
    }
}

/// Whether a survey accepts responses now: an open survey before its open date is scheduled, after
/// its close date closed.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SurveyState {
    Draft,
    Scheduled,
    Open,
    Closed,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SurveyInput {
    pub title: String,
    pub description: String,
    /// Template the survey started from; empty for a custom survey.
    pub template: String,
    pub questions: Vec<SurveyQuestion>,
    /// Team handles asked; empty for every member.
    pub audience_teams: Vec<String>,
    pub anonymous: bool,
    pub status: SurveyStatus,
    /// Epoch milliseconds.
    pub opens_at: Option<i64>,
    pub closes_at: Option<i64>,
    /// Notified of each response: channel handles (`slack-devex`, `webhook-portal`) or member emails.
    pub notify: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Survey {
    pub id: String,
    pub title: String,
    pub description: String,
    pub template: String,
    pub questions: Vec<SurveyQuestion>,
    pub audience_teams: Vec<String>,
    pub anonymous: bool,
    pub status: SurveyStatus,
    pub state: SurveyState,
    pub opens_at: Option<i64>,
    pub closes_at: Option<i64>,
    pub notify: Vec<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub responses: i64,
    /// Members in the audience.
    pub audience_size: i64,
    /// The signed-in user has responded.
    pub responded: bool,
}

/// What a respondent sees.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SurveyForm {
    pub id: String,
    pub title: String,
    pub description: String,
    pub questions: Vec<SurveyQuestion>,
    pub anonymous: bool,
    pub state: SurveyState,
    pub closes_at: Option<i64>,
    /// The signed-in user is in the audience.
    pub in_audience: bool,
    pub responded: bool,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
pub struct SurveyResponseInput {
    /// Answer per question id: 1–5 (Likert), 0–10 (NPS), an option (single choice), options (multiple
    /// choice) or text.
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub answers: Map<String, Value>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SurveyOptionCount {
    pub label: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SurveyNps {
    /// % promoters (9–10) − % detractors (0–6), −100 … 100.
    pub score: f64,
    pub promoters: u64,
    pub passives: u64,
    pub detractors: u64,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SurveyQuestionResult {
    pub question_id: String,
    pub kind: SurveyQuestionKind,
    pub title: String,
    /// Responses that answered it.
    pub answered: u64,
    /// Likert labels, 0–10 for NPS, or the options; empty for text.
    pub counts: Vec<SurveyOptionCount>,
    /// Likert: mean answer (1–5).
    pub average: Option<f64>,
    pub nps: Option<SurveyNps>,
    /// Text: the answers, newest first.
    pub texts: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SurveyTeamRate {
    pub team: String,
    pub name: String,
    pub members: i64,
    pub responded: i64,
    /// Percent of members who responded.
    pub rate: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SurveyResults {
    pub survey_id: String,
    pub responses: i64,
    pub audience_size: i64,
    /// Percent of the audience who responded.
    pub response_rate: Option<f64>,
    pub questions: Vec<SurveyQuestionResult>,
    pub teams: Vec<SurveyTeamRate>,
}

/// One response, for the export. Anonymous surveys never name the respondent.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SurveyResponseRow {
    pub id: i64,
    pub submitted_at: i64,
    pub respondent: Option<String>,
    pub teams: Vec<String>,
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub answers: Map<String, Value>,
}

// ================================================================ validation and aggregation

fn text(value: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(format!("{what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(v.to_string())
}

fn valid_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 100 && h.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// A notification recipient: a member email or a channel handle (`slack-…`, `webhook-…`).
fn valid_recipient(r: &str) -> bool {
    if let Some((local, domain)) = r.split_once('@') {
        return !local.is_empty() && domain.contains('.') && !r.contains(char::is_whitespace) && r.len() <= 254;
    }
    (r.starts_with("slack-") || r.starts_with("webhook-")) && valid_handle(r)
}

/// Checks and normalises a survey (trimmed texts, no options on questions that have none).
pub fn check_survey(input: &SurveyInput) -> Result<SurveyInput, String> {
    let title = text(&input.title, "The title", MAX_TITLE_CHARS, true)?;
    let description = text(&input.description, "The description", MAX_DESCRIPTION_CHARS, false)?;
    if input.questions.is_empty() {
        return Err("Add at least one question".into());
    }
    if input.questions.len() > MAX_QUESTIONS {
        return Err(format!("A survey has at most {MAX_QUESTIONS} questions"));
    }
    let mut ids = HashSet::new();
    let mut questions = Vec::with_capacity(MAX_QUESTIONS);
    for (i, q) in input.questions.iter().enumerate() {
        let n = i + 1;
        let id = q.id.trim().to_string();
        if id.is_empty() || id.len() > MAX_QUESTION_ID_CHARS || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(format!("Question {n} needs an id of up to {MAX_QUESTION_ID_CHARS} characters a–z, 0–9, _ or -"));
        }
        if !ids.insert(id.clone()) {
            return Err(format!("The question id {id} is used twice"));
        }
        let title = text(&q.title, &format!("The title of question {n}"), MAX_QUESTION_CHARS, true)?;
        let description = text(&q.description, &format!("The description of question {n}"), MAX_DESCRIPTION_CHARS, false)?;
        let options = match q.kind {
            SurveyQuestionKind::Single | SurveyQuestionKind::Multi => {
                let opts: Vec<String> = q.options.iter().map(|o| o.trim().to_string()).collect();
                if opts.len() < MIN_OPTIONS || opts.len() > MAX_OPTIONS || opts.iter().any(|o| o.is_empty() || o.chars().count() > MAX_OPTION_CHARS) {
                    return Err(format!("Question {n} needs {MIN_OPTIONS}–{MAX_OPTIONS} options of up to {MAX_OPTION_CHARS} characters"));
                }
                let mut seen = HashSet::new();
                if let Some(dup) = opts.iter().find(|o| !seen.insert(o.as_str())) {
                    return Err(format!("Question {n} lists the option {dup} twice"));
                }
                opts
            }
            _ => Vec::new(),
        };
        questions.push(SurveyQuestion { id, kind: q.kind, title, description, required: q.required, options });
    }
    if input.audience_teams.len() > MAX_AUDIENCE_TEAMS {
        return Err(format!("At most {MAX_AUDIENCE_TEAMS} teams"));
    }
    let mut audience_teams: Vec<String> = Vec::new();
    for t in &input.audience_teams {
        let t = t.trim();
        if !valid_handle(t) {
            return Err(format!("{t} isn't a team handle"));
        }
        if !audience_teams.iter().any(|x| x == t) {
            audience_teams.push(t.to_string());
        }
    }
    if input.notify.len() > MAX_NOTIFY {
        return Err(format!("At most {MAX_NOTIFY} recipients"));
    }
    let mut notify: Vec<String> = Vec::new();
    for r in &input.notify {
        let r = r.trim().trim_start_matches('@');
        if !valid_recipient(r) {
            return Err(format!("{r} isn't a member email or a channel handle (slack-…, webhook-…)"));
        }
        if !notify.iter().any(|x| x == r) {
            notify.push(r.to_string());
        }
    }
    if let (Some(o), Some(c)) = (input.opens_at, input.closes_at) {
        if c <= o {
            return Err("The survey must close after it opens".into());
        }
    }
    let template = text(&input.template, "The template", 100, false)?;
    Ok(SurveyInput { title, description, template, questions, audience_teams, anonymous: input.anonymous, status: input.status, opens_at: input.opens_at, closes_at: input.closes_at, notify })
}

/// A whole JSON number (1 and 1.0, not 1.5 or "1").
fn whole_number(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 1e15).map(|f| f as i64))
}

/// Checks a response against the questions; returns the answers to store (unanswered ones dropped).
pub fn check_answers(questions: &[SurveyQuestion], answers: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(unknown) = answers.keys().find(|k| !questions.iter().any(|q| &q.id == *k)) {
        return Err(format!("Unknown question {unknown}"));
    }
    let mut out = Map::new();
    for q in questions {
        let given = answers.get(&q.id).filter(|v| !v.is_null());
        let value = match (q.kind, given) {
            (_, None) => None,
            (SurveyQuestionKind::Likert, Some(v)) => {
                let n = whole_number(v).filter(|n| (1..=5).contains(n)).ok_or_else(|| format!("Answer “{}” from 1 to 5", q.title))?;
                Some(json!(n))
            }
            (SurveyQuestionKind::Nps, Some(v)) => {
                let n = whole_number(v).filter(|n| (0..=10).contains(n)).ok_or_else(|| format!("Answer “{}” from 0 to 10", q.title))?;
                Some(json!(n))
            }
            (SurveyQuestionKind::Single, Some(v)) => {
                let s = v.as_str().ok_or_else(|| format!("Pick one option for “{}”", q.title))?;
                if !q.options.iter().any(|o| o == s) {
                    return Err(format!("{s} isn't an option of “{}”", q.title));
                }
                Some(json!(s))
            }
            (SurveyQuestionKind::Multi, Some(v)) => {
                let list = v.as_array().ok_or_else(|| format!("Answer “{}” with a list of options", q.title))?;
                let mut picked: Vec<&str> = Vec::new();
                for item in list {
                    let s = item.as_str().ok_or_else(|| format!("Answer “{}” with a list of options", q.title))?;
                    if !q.options.iter().any(|o| o == s) {
                        return Err(format!("{s} isn't an option of “{}”", q.title));
                    }
                    if picked.contains(&s) {
                        return Err(format!("{s} is picked twice for “{}”", q.title));
                    }
                    picked.push(s);
                }
                (!picked.is_empty()).then(|| json!(picked))
            }
            (SurveyQuestionKind::Text, Some(v)) => {
                let s = v.as_str().ok_or_else(|| format!("Answer “{}” with text", q.title))?.trim();
                if s.chars().count() > MAX_TEXT_ANSWER_CHARS {
                    return Err(format!("The answer to “{}” is longer than {MAX_TEXT_ANSWER_CHARS} characters", q.title));
                }
                (!s.is_empty()).then(|| json!(s))
            }
        };
        match value {
            Some(v) => {
                out.insert(q.id.clone(), v);
            }
            None if q.required => return Err(format!("Answer “{}”", q.title)),
            None => {}
        }
    }
    Ok(out)
}

/// The effective state at `now` (epoch ms).
pub fn survey_state(status: SurveyStatus, opens_at: Option<i64>, closes_at: Option<i64>, now: i64) -> SurveyState {
    match status {
        SurveyStatus::Draft => SurveyState::Draft,
        SurveyStatus::Closed => SurveyState::Closed,
        SurveyStatus::Open if closes_at.is_some_and(|c| c <= now) => SurveyState::Closed,
        SurveyStatus::Open if opens_at.is_some_and(|o| o > now) => SurveyState::Scheduled,
        SurveyStatus::Open => SurveyState::Open,
    }
}

/// The duplicate guard: hex SHA-256 of the survey's salt and the user id.
pub fn respondent_key(salt: &str, user_id: &str) -> String {
    Sha256::digest(format!("{salt}:{user_id}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// `part` of `whole` in percent, one decimal.
fn percent(part: i64, whole: i64) -> Option<f64> {
    (whole > 0).then(|| (part as f64 * 1000.0 / whole as f64).round() / 10.0)
}

fn scale_result(q: &SurveyQuestion, out: &mut SurveyQuestionResult, given: &[&Value]) {
    let likert = q.kind == SurveyQuestionKind::Likert;
    let (lo, hi) = if likert { (1, 5) } else { (0, 10) };
    let mut counts = vec![0u64; (hi - lo + 1) as usize];
    let mut sum = 0i64;
    for n in given.iter().filter_map(|v| whole_number(v)).filter(|n| (lo..=hi).contains(n)) {
        counts[(n - lo) as usize] += 1;
        sum += n;
        out.answered += 1;
    }
    out.counts = counts.iter().enumerate().map(|(i, c)| SurveyOptionCount { label: if likert { LIKERT_LABELS[i].to_string() } else { i.to_string() }, count: *c }).collect();
    if out.answered == 0 {
        return;
    }
    if likert {
        out.average = Some((sum as f64 / out.answered as f64 * 100.0).round() / 100.0);
    } else {
        let promoters = counts[9] + counts[10];
        let passives = counts[7] + counts[8];
        let detractors = counts[..7].iter().sum::<u64>();
        let score = ((promoters as f64 - detractors as f64) * 1000.0 / out.answered as f64).round() / 10.0;
        out.nps = Some(SurveyNps { score, promoters, passives, detractors });
    }
}

/// Per-question results from the stored answers, newest response first.
pub fn aggregate(questions: &[SurveyQuestion], responses: &[Map<String, Value>]) -> Vec<SurveyQuestionResult> {
    questions
        .iter()
        .map(|q| {
            let mut out = SurveyQuestionResult { question_id: q.id.clone(), kind: q.kind, title: q.title.clone(), answered: 0, counts: Vec::new(), average: None, nps: None, texts: Vec::new() };
            let given: Vec<&Value> = responses.iter().filter_map(|r| r.get(&q.id)).collect();
            match q.kind {
                SurveyQuestionKind::Likert | SurveyQuestionKind::Nps => scale_result(q, &mut out, &given),
                SurveyQuestionKind::Single | SurveyQuestionKind::Multi => {
                    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
                    for v in given {
                        let picked: Vec<&str> = match v {
                            Value::String(s) => vec![s.as_str()],
                            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                            _ => Vec::new(),
                        };
                        let known: Vec<&str> = picked.into_iter().filter(|p| q.options.iter().any(|o| o == p)).collect();
                        if !known.is_empty() {
                            out.answered += 1;
                        }
                        for p in known {
                            *counts.entry(p).or_default() += 1;
                        }
                    }
                    out.counts = q.options.iter().map(|o| SurveyOptionCount { label: o.clone(), count: counts.get(o.as_str()).copied().unwrap_or(0) }).collect();
                }
                SurveyQuestionKind::Text => {
                    for s in given.into_iter().filter_map(Value::as_str) {
                        out.answered += 1;
                        if out.texts.len() < MAX_TEXTS {
                            out.texts.push(s.to_string());
                        }
                    }
                }
            }
            out
        })
        .collect()
}

/// Response rate per team from the teams' sizes (handle, name, members) and each response's teams.
pub fn team_rates(teams: &[(String, String, i64)], responses: &[Vec<String>]) -> Vec<SurveyTeamRate> {
    teams
        .iter()
        .map(|(team, name, members)| {
            let responded = responses.iter().filter(|r| r.contains(team)).count() as i64;
            SurveyTeamRate { team: team.clone(), name: name.clone(), members: *members, responded, rate: percent(responded.min(*members), *members) }
        })
        .collect()
}

// ================================================================ storage

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("Survey not found".into())
}

#[derive(sqlx::FromRow)]
struct SurveyRow {
    id: String,
    title: String,
    description: String,
    template: String,
    questions: Value,
    audience_teams: Vec<String>,
    anonymous: bool,
    status: String,
    opens_at: Option<i64>,
    closes_at: Option<i64>,
    notify: Vec<String>,
    salt: String,
    created_by: String,
    created_at: i64,
    updated_at: i64,
    responses: i64,
    audience_size: i64,
    responded: bool,
}

impl SurveyRow {
    fn questions(&self) -> Vec<SurveyQuestion> {
        serde_json::from_value(self.questions.clone()).unwrap_or_default()
    }

    fn state(&self) -> SurveyState {
        survey_state(SurveyStatus::parse(&self.status), self.opens_at, self.closes_at, now_ms())
    }

    fn view(&self) -> Survey {
        Survey {
            id: self.id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            template: self.template.clone(),
            questions: self.questions(),
            audience_teams: self.audience_teams.clone(),
            anonymous: self.anonymous,
            status: SurveyStatus::parse(&self.status),
            state: self.state(),
            opens_at: self.opens_at,
            closes_at: self.closes_at,
            notify: self.notify.clone(),
            created_by: self.created_by.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            responses: self.responses,
            audience_size: self.audience_size,
            responded: self.responded,
        }
    }
}

/// The org's surveys (one when `id` is set), with counts and whether `user` has responded.
async fn survey_rows(st: &AppState, org: &str, user: &str, id: Option<&str>) -> Result<Vec<SurveyRow>, ApiError> {
    let rows = sqlx::query_as(
        "SELECT s.id::text, s.title, s.description, s.template, s.questions, s.audience_teams, s.anonymous, s.status, \
         (extract(epoch FROM s.opens_at) * 1000)::bigint AS opens_at, (extract(epoch FROM s.closes_at) * 1000)::bigint AS closes_at, \
         s.notify, s.salt, COALESCE(u.name, '') AS created_by, \
         (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM s.updated_at) * 1000)::bigint AS updated_at, \
         (SELECT count(*) FROM portal_survey_responses r WHERE r.org_id = s.org_id AND r.survey_id = s.id) AS responses, \
         CASE WHEN cardinality(s.audience_teams) = 0 THEN (SELECT count(*) FROM users m WHERE m.org_id = s.org_id AND m.status = 'active') \
              ELSE (SELECT count(DISTINCT tm.user_id) FROM team_members tm JOIN users m ON m.id = tm.user_id AND m.status = 'active' \
                    WHERE tm.org_id = s.org_id AND tm.handle = ANY(s.audience_teams)) END AS audience_size, \
         EXISTS (SELECT 1 FROM portal_survey_responses r WHERE r.org_id = s.org_id AND r.survey_id = s.id \
                 AND r.respondent_key = encode(sha256(convert_to(s.salt || ':' || $2, 'UTF8')), 'hex')) AS responded \
         FROM portal_surveys s LEFT JOIN users u ON u.id = s.created_by \
         WHERE s.org_id = $1 AND ($3::text IS NULL OR s.id::text = $3) ORDER BY s.created_at DESC",
    )
    .bind(org)
    .bind(user)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows)
}

async fn survey_row(st: &AppState, org: &str, user: &str, id: &str) -> Result<SurveyRow, ApiError> {
    if !valid_uuid(id) {
        return Err(not_found());
    }
    survey_rows(st, org, user, Some(id)).await?.pop().ok_or_else(not_found)
}

async fn user_teams(st: &AppState, org: &str, user: &str) -> Result<Vec<String>, ApiError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT handle FROM team_members WHERE org_id = $1 AND user_id::text = $2 ORDER BY handle").bind(org).bind(user).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

fn in_audience(row: &SurveyRow, teams: &[String]) -> bool {
    row.audience_teams.is_empty() || row.audience_teams.iter().any(|t| teams.contains(t))
}

/// The questions' shape (ids, kinds, options): what responses depend on.
fn shape(questions: &[SurveyQuestion]) -> Vec<(&str, SurveyQuestionKind, &[String])> {
    questions.iter().map(|q| (q.id.as_str(), q.kind, q.options.as_slice())).collect()
}

/// Tells the survey's recipients about a new response, in the background.
fn notify_response(st: &AppState, org: &str, row: &SurveyRow, respondent: &str) {
    if row.notify.is_empty() {
        return;
    }
    let link = format!("{}/portal/surveys/{}/results", st.config.public_url, row.id);
    let title = format!("New response to “{}”", row.title);
    let count = row.responses + 1;
    let out = Outgoing {
        org: org.to_string(),
        title: title.clone(),
        email_body: format!("{title}\n\nFrom: {respondent}\nResponses so far: {count}\n\nResults: {link}\n"),
        slack_text: format!("*{title}*\nFrom: {respondent} · {count} responses so far\n<{link}|View results>"),
        webhook: json!({ "event": "survey.response", "survey": { "id": row.id, "title": row.title, "url": link }, "respondent": respondent, "responses": count }),
    };
    let recipients = row.notify.clone();
    let st = st.clone();
    tokio::spawn(async move {
        for r in recipients {
            match notifications::send(&st, &r, &out).await {
                notifications::Outcome::Sent => {}
                notifications::Outcome::Unknown(e) | notifications::Outcome::Failed(e) => tracing::warn!(recipient = %r, error = %e, "survey response notification not sent"),
            }
        }
    });
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/portal/surveys", operation_id = "list_portal_surveys", responses((status = 200, body = Vec<Survey>)), tag = "portal")]
pub async fn list_surveys(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<Survey>> {
    Ok(Json(survey_rows(&st, &ctx.org_id, &ctx.user_id, None).await?.iter().map(SurveyRow::view).collect()))
}

#[utoipa::path(get, path = "/api/v1/portal/surveys/{id}", operation_id = "get_portal_survey", params(("id" = String, Path)),
    responses((status = 200, body = Survey), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn get_survey(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Survey> {
    Ok(Json(survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?.view()))
}

#[utoipa::path(post, path = "/api/v1/portal/surveys", operation_id = "create_portal_survey", request_body = SurveyInput,
    responses((status = 200, body = Survey), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn create_survey(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SurveyInput>) -> ApiResult<Survey> {
    let s = check_survey(&input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM portal_surveys WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_SURVEYS {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_SURVEYS} surveys")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO portal_surveys (org_id, title, description, template, questions, audience_teams, anonymous, status, opens_at, closes_at, notify, salt, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, to_timestamp($9::bigint / 1000.0), to_timestamp($10::bigint / 1000.0), $11, $12, \
         (SELECT id FROM users WHERE org_id = $1 AND id::text = $13)) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&s.title)
    .bind(&s.description)
    .bind(&s.template)
    .bind(serde_json::to_value(&s.questions).map_err(anyhow::Error::from)?)
    .bind(&s.audience_teams)
    .bind(s.anonymous)
    .bind(s.status.as_str())
    .bind(s.opens_at)
    .bind(s.closes_at)
    .bind(&s.notify)
    .bind(crate::auth::crypto::random_token(""))
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "portal.survey_created", &id, &format!("{} ({} questions, {})", s.title, s.questions.len(), s.status.as_str())).await;
    Ok(Json(survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?.view()))
}

#[utoipa::path(put, path = "/api/v1/portal/surveys/{id}", operation_id = "update_portal_survey", params(("id" = String, Path)), request_body = SurveyInput,
    responses((status = 200, body = Survey), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn update_survey(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SurveyInput>) -> ApiResult<Survey> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    let s = check_survey(&input).map_err(ApiError::BadRequest)?;
    if row.responses > 0 {
        if shape(&row.questions()) != shape(&s.questions) {
            return Err(ApiError::BadRequest("Questions can't be added, removed or changed in kind or options once responses arrived; duplicate the survey instead".into()));
        }
        if row.anonymous != s.anonymous {
            return Err(ApiError::BadRequest("Whether the survey is anonymous can't change once responses arrived".into()));
        }
    }
    sqlx::query(
        "UPDATE portal_surveys SET title = $3, description = $4, template = $5, questions = $6, audience_teams = $7, anonymous = $8, status = $9, \
         opens_at = to_timestamp($10::bigint / 1000.0), closes_at = to_timestamp($11::bigint / 1000.0), notify = $12, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&s.title)
    .bind(&s.description)
    .bind(&s.template)
    .bind(serde_json::to_value(&s.questions).map_err(anyhow::Error::from)?)
    .bind(&s.audience_teams)
    .bind(s.anonymous)
    .bind(s.status.as_str())
    .bind(s.opens_at)
    .bind(s.closes_at)
    .bind(&s.notify)
    .execute(&st.postgres)
    .await?;
    let detail = if row.status == s.status.as_str() { s.title.clone() } else { format!("{} ({} → {})", s.title, row.status, s.status.as_str()) };
    ctx.audit(&st, "portal.survey_updated", &id, &detail).await;
    Ok(Json(survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?.view()))
}

#[utoipa::path(delete, path = "/api/v1/portal/surveys/{id}", operation_id = "delete_portal_survey", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn delete_survey(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    sqlx::query("DELETE FROM portal_surveys WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "portal.survey_deleted", &id, &format!("{} ({} responses)", row.title, row.responses)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The survey as its respondents see it.
#[utoipa::path(get, path = "/api/v1/portal/surveys/{id}/form", operation_id = "get_portal_survey_form", params(("id" = String, Path)),
    responses((status = 200, body = SurveyForm), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn survey_form(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<SurveyForm> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    let teams = user_teams(&st, &ctx.org_id, &ctx.user_id).await?;
    Ok(Json(SurveyForm {
        id: row.id.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        questions: row.questions(),
        anonymous: row.anonymous,
        state: row.state(),
        closes_at: row.closes_at,
        in_audience: in_audience(&row, &teams),
        responded: row.responded,
    }))
}

/// Records the signed-in member's response; each member responds once.
#[utoipa::path(post, path = "/api/v1/portal/surveys/{id}/responses", operation_id = "submit_portal_survey_response", params(("id" = String, Path)), request_body = SurveyResponseInput,
    responses((status = 204), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn submit_response(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(input): Json<SurveyResponseInput>) -> Result<StatusCode, ApiError> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    if row.state() != SurveyState::Open {
        return Err(ApiError::BadRequest("This survey isn't accepting responses".into()));
    }
    let teams = user_teams(&st, &ctx.org_id, &ctx.user_id).await?;
    if !in_audience(&row, &teams) {
        return Err(ApiError::Forbidden("This survey is for other teams".into()));
    }
    let answers = check_answers(&row.questions(), &input.answers).map_err(ApiError::BadRequest)?;
    let key = respondent_key(&row.salt, &ctx.user_id);
    let user = if row.anonymous { None } else { Some(ctx.user_id.clone()) };
    let inserted = sqlx::query(
        "INSERT INTO portal_survey_responses (org_id, survey_id, user_id, respondent_key, teams, answers) \
         VALUES ($1, $2::uuid, (SELECT id FROM users WHERE org_id = $1 AND id::text = $3), $4, $5, $6) ON CONFLICT (survey_id, respondent_key) DO NOTHING",
    )
    .bind(&ctx.org_id)
    .bind(&row.id)
    .bind(user)
    .bind(&key)
    .bind(&teams)
    .bind(Value::Object(answers))
    .execute(&st.postgres)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::BadRequest("You have already responded to this survey".into()));
    }
    notify_response(&st, &ctx.org_id, &row, if row.anonymous { "Anonymous" } else { &ctx.name });
    Ok(StatusCode::NO_CONTENT)
}

async fn stored_responses(st: &AppState, org: &str, id: &str) -> Result<Vec<(Value, Vec<String>)>, ApiError> {
    let rows = sqlx::query_as("SELECT answers, teams FROM portal_survey_responses WHERE org_id = $1 AND survey_id::text = $2 ORDER BY submitted_at DESC, id DESC LIMIT $3")
        .bind(org)
        .bind(id)
        .bind(MAX_RESPONSES)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows)
}

#[utoipa::path(get, path = "/api/v1/portal/surveys/{id}/results", operation_id = "get_portal_survey_results", params(("id" = String, Path)),
    responses((status = 200, body = SurveyResults), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn survey_results(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<SurveyResults> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    let stored = stored_responses(&st, &ctx.org_id, &row.id).await?;
    let answers: Vec<Map<String, Value>> = stored.iter().map(|(a, _)| a.as_object().cloned().unwrap_or_default()).collect();
    let response_teams: Vec<Vec<String>> = stored.into_iter().map(|(_, t)| t).collect();
    let teams: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT t.handle, t.name, count(u.id) FROM teams t \
         LEFT JOIN team_members tm ON tm.org_id = t.org_id AND tm.handle = t.handle \
         LEFT JOIN users u ON u.id = tm.user_id AND u.status = 'active' \
         WHERE t.org_id = $1 AND (cardinality($2::text[]) = 0 OR t.handle = ANY($2)) GROUP BY t.handle, t.name ORDER BY t.name",
    )
    .bind(&ctx.org_id)
    .bind(&row.audience_teams)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(SurveyResults {
        survey_id: row.id.clone(),
        responses: row.responses,
        audience_size: row.audience_size,
        response_rate: percent(row.responses.min(row.audience_size), row.audience_size),
        questions: aggregate(&row.questions(), &answers),
        teams: team_rates(&teams, &response_teams),
    }))
}

#[derive(sqlx::FromRow)]
struct ResponseRow {
    id: i64,
    submitted_at: i64,
    respondent: Option<String>,
    teams: Vec<String>,
    answers: Value,
}

/// Every response, newest first, for the CSV export. Anonymous surveys name no one and list no teams.
#[utoipa::path(get, path = "/api/v1/portal/surveys/{id}/responses", operation_id = "list_portal_survey_responses", params(("id" = String, Path)),
    responses((status = 200, body = Vec<SurveyResponseRow>), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn list_responses(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Vec<SurveyResponseRow>> {
    let row = survey_row(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    let rows: Vec<ResponseRow> = sqlx::query_as(
        "SELECT r.id, (extract(epoch FROM r.submitted_at) * 1000)::bigint AS submitted_at, CASE WHEN u.id IS NULL THEN NULL ELSE u.name || ' <' || u.email || '>' END AS respondent, r.teams, r.answers \
         FROM portal_survey_responses r LEFT JOIN users u ON u.id = r.user_id AND u.org_id = r.org_id \
         WHERE r.org_id = $1 AND r.survey_id::text = $2 ORDER BY r.submitted_at DESC, r.id DESC LIMIT $3",
    )
    .bind(&ctx.org_id)
    .bind(&row.id)
    .bind(MAX_RESPONSES)
    .fetch_all(&st.postgres)
    .await?;
    ctx.audit(&st, "portal.survey_exported", &row.id, &format!("{} ({} responses)", row.title, rows.len())).await;
    Ok(Json(
        rows.into_iter()
            .map(|r| SurveyResponseRow {
                id: r.id,
                submitted_at: r.submitted_at,
                respondent: if row.anonymous { None } else { r.respondent },
                teams: if row.anonymous { Vec::new() } else { r.teams },
                answers: r.answers.as_object().cloned().unwrap_or_default(),
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(id: &str, kind: SurveyQuestionKind, required: bool, options: &[&str]) -> SurveyQuestion {
        SurveyQuestion { id: id.into(), kind, title: format!("Question {id}"), description: String::new(), required, options: options.iter().map(|s| s.to_string()).collect() }
    }

    fn survey(questions: Vec<SurveyQuestion>) -> SurveyInput {
        SurveyInput { title: " Developer Experience Survey ".into(), questions, ..Default::default() }
    }

    fn answers(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn surveys_are_checked_and_normalised() {
        let ok = check_survey(&survey(vec![q("tools", SurveyQuestionKind::Likert, true, &["ignored"]), q("lang", SurveyQuestionKind::Single, false, &[" Rust ", "Go"])])).unwrap();
        assert_eq!(ok.title, "Developer Experience Survey");
        assert!(ok.questions[0].options.is_empty(), "Likert questions carry no options");
        assert_eq!(ok.questions[1].options, vec!["Rust", "Go"]);

        let err = |s: SurveyInput| check_survey(&s).unwrap_err();
        assert!(err(SurveyInput { title: " ".into(), ..survey(vec![q("a", SurveyQuestionKind::Text, false, &[])]) }).contains("title"));
        assert!(err(survey(vec![])).contains("at least one question"));
        assert!(err(survey(vec![q("a", SurveyQuestionKind::Text, false, &[]), q("a", SurveyQuestionKind::Nps, false, &[])])).contains("twice"));
        assert!(err(survey(vec![q("Bad Id", SurveyQuestionKind::Text, false, &[])])).contains("id"));
        assert!(err(survey(vec![q("a", SurveyQuestionKind::Single, false, &["only"])])).contains("2–20 options"));
        assert!(err(survey(vec![q("a", SurveyQuestionKind::Multi, false, &["x", "x"])])).contains("twice"));
        let mut untitled = q("a", SurveyQuestionKind::Text, false, &[]);
        untitled.title = String::new();
        assert!(err(survey(vec![untitled])).contains("title"));
        let dates = SurveyInput { opens_at: Some(2000), closes_at: Some(1000), ..survey(vec![q("a", SurveyQuestionKind::Text, false, &[])]) };
        assert!(err(dates).contains("close after"));
        let notify = SurveyInput { notify: vec!["slack-devex".into(), "lead@example.com".into(), "#general".into()], ..survey(vec![q("a", SurveyQuestionKind::Text, false, &[])]) };
        assert!(err(notify).contains("#general"));
        let teams = SurveyInput { audience_teams: vec!["checkout".into(), "bad team".into()], ..survey(vec![q("a", SurveyQuestionKind::Text, false, &[])]) };
        assert!(err(teams).contains("bad team"));
    }

    #[test]
    fn answers_are_checked_against_the_questions() {
        let qs = vec![
            q("tools", SurveyQuestionKind::Likert, true, &[]),
            q("nps", SurveyQuestionKind::Nps, false, &[]),
            q("lang", SurveyQuestionKind::Single, false, &["Rust", "Go"]),
            q("areas", SurveyQuestionKind::Multi, false, &["CI", "Docs", "Local dev"]),
            q("more", SurveyQuestionKind::Text, false, &[]),
        ];
        let ok = check_answers(&qs, &answers(json!({"tools": 4, "nps": 10, "lang": "Go", "areas": ["CI", "Docs"], "more": "  Faster builds  "}))).unwrap();
        assert_eq!(Value::Object(ok), json!({"tools": 4, "nps": 10, "lang": "Go", "areas": ["CI", "Docs"], "more": "Faster builds"}));
        // Unanswered optional questions and empty texts are dropped.
        let sparse = check_answers(&qs, &answers(json!({"tools": 1, "more": " ", "nps": null, "areas": []}))).unwrap();
        assert_eq!(Value::Object(sparse), json!({"tools": 1}));

        let err = |v: Value| check_answers(&qs, &answers(v)).unwrap_err();
        assert!(err(json!({})).contains("Question tools"), "required");
        assert!(err(json!({"tools": 6})).contains("1 to 5"));
        assert!(err(json!({"tools": 2.5})).contains("1 to 5"));
        assert!(err(json!({"tools": 3, "nps": 11})).contains("0 to 10"));
        assert!(err(json!({"tools": 3, "lang": "Java"})).contains("Java"));
        assert!(err(json!({"tools": 3, "areas": ["CI", "CI"]})).contains("twice"));
        assert!(err(json!({"tools": 3, "areas": "CI"})).contains("list"));
        assert!(err(json!({"tools": 3, "more": "x".repeat(5001)})).contains("5000"));
        assert!(err(json!({"tools": 3, "other": 1})).contains("Unknown question other"));
    }

    #[test]
    fn states_follow_status_and_dates() {
        use SurveyState as S;
        use SurveyStatus::*;
        assert_eq!(survey_state(Draft, None, None, 100), S::Draft);
        assert_eq!(survey_state(Open, None, None, 100), S::Open);
        assert_eq!(survey_state(Open, Some(200), None, 100), S::Scheduled);
        assert_eq!(survey_state(Open, Some(50), Some(100), 100), S::Closed);
        assert_eq!(survey_state(Open, Some(50), Some(101), 100), S::Open);
        assert_eq!(survey_state(Closed, None, None, 100), S::Closed);
    }

    #[test]
    fn respondent_keys_are_per_survey_and_per_user() {
        let k = respondent_key("salt-a", "0b6f6f0e-7c1d-4b55-9a35-6d2d2d1f0001");
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(k, respondent_key("salt-a", "0b6f6f0e-7c1d-4b55-9a35-6d2d2d1f0001"), "the same user maps to the same key");
        assert_ne!(k, respondent_key("salt-b", "0b6f6f0e-7c1d-4b55-9a35-6d2d2d1f0001"), "another survey gives another key");
        assert_ne!(k, respondent_key("salt-a", "0b6f6f0e-7c1d-4b55-9a35-6d2d2d1f0002"));
        assert!(!k.contains("0b6f6f0e"), "the key doesn't reveal the user");
    }

    #[test]
    fn results_aggregate_distributions_averages_and_nps() {
        let qs = vec![
            q("tools", SurveyQuestionKind::Likert, true, &[]),
            q("nps", SurveyQuestionKind::Nps, false, &[]),
            q("areas", SurveyQuestionKind::Multi, false, &["CI", "Docs"]),
            q("more", SurveyQuestionKind::Text, false, &[]),
        ];
        let rs = vec![
            answers(json!({"tools": 5, "nps": 10, "areas": ["CI", "Docs"], "more": "newest"})),
            answers(json!({"tools": 4, "nps": 9, "areas": ["CI"]})),
            answers(json!({"tools": 2, "nps": 7})),
            answers(json!({"tools": 1, "nps": 3, "more": "oldest"})),
            // Answers to removed options or of the wrong shape are ignored.
            answers(json!({"tools": "x", "areas": ["Gone"]})),
        ];
        let out = aggregate(&qs, &rs);
        let likert = &out[0];
        assert_eq!(likert.answered, 4);
        assert_eq!(likert.counts.iter().map(|c| c.count).collect::<Vec<_>>(), vec![1, 1, 0, 1, 1]);
        assert_eq!(likert.counts[4].label, "Strongly agree");
        assert_eq!(likert.average, Some(3.0));

        let nps = out[1].nps.as_ref().unwrap();
        assert_eq!((nps.promoters, nps.passives, nps.detractors), (2, 1, 1));
        assert_eq!(nps.score, 25.0);
        assert_eq!(out[1].counts.len(), 11);
        assert_eq!(out[1].counts[10].count, 1);

        assert_eq!(out[2].counts, vec![SurveyOptionCount { label: "CI".into(), count: 2 }, SurveyOptionCount { label: "Docs".into(), count: 1 }]);
        assert_eq!(out[2].answered, 2);
        assert_eq!(out[3].texts, vec!["newest", "oldest"]);
        assert!(out[3].counts.is_empty());

        let empty = aggregate(&qs, &[]);
        assert_eq!(empty[0].average, None);
        assert_eq!(empty[1].nps, None);
    }

    #[test]
    fn response_rates_per_team() {
        let teams = vec![("checkout".to_string(), "Checkout".to_string(), 4), ("platform".to_string(), "Platform".to_string(), 0)];
        let rs = vec![vec!["checkout".to_string()], vec!["checkout".to_string(), "platform".to_string()], vec![]];
        let out = team_rates(&teams, &rs);
        assert_eq!(out[0], SurveyTeamRate { team: "checkout".into(), name: "Checkout".into(), members: 4, responded: 2, rate: Some(50.0) });
        assert_eq!(out[1].responded, 1);
        assert_eq!(out[1].rate, None, "no members, no rate");
    }
}
