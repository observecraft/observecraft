//! Workflow Automation: what a workflow is made of, how it is checked, and the run state machine.
//!
//! A workflow has form **inputs** (as self-service actions do), **triggers** and a tree of **steps**:
//!
//! - action steps shared with self-service actions ([`super::portal_action_steps`]): HTTP requests,
//!   declare an incident, page an on-call team, comment on an incident, run a synthetic test;
//! - workflow steps: notify (notification channels and members), query a metric, count logs, wait;
//! - control steps: **if** (an [expression](super::workflow_expr) choosing `steps` or `else_steps`)
//!   and **for each** (runs `steps` once per item of a list expression, `Loop.item` / `Loop.index`).
//!
//! Templates and expressions read `Trigger.…` (what started the run), `inputs.…`, `Steps.<id>.status`,
//! `Steps.<id>.output.…` and `Steps.<id>.error` (the latest result of a step), `Loop.item` /
//! `Loop.index` (the innermost "for each") and `Workflow.id` / `Workflow.name` / `Workflow.runId`.
//!
//! A run is a [`RunState`]: a stack of the blocks being run and the results so far. [`advance`] walks
//! the control steps without side effects and hands back the next action step; the executor performs
//! it and reports with [`complete`]. The state is plain JSON, kept after every step, so a run resumes
//! where it stopped (after a wait, or on another server).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::portal_action_steps::{self as steps, ActionHttpRequest, ActionInput, ActionStep, ActionStepKind, TemplateScope};
use super::workflow_cron::{zone, Cron};
use super::workflow_expr as expr;

pub const MAX_TRIGGERS: usize = 5;
pub const MAX_STEPS: usize = 100;
pub const MAX_NESTING: usize = 6;
pub const MAX_LOOP_NESTING: usize = 2;
pub const MAX_LOOP_ITEMS: usize = 100;
/// Action steps one run may execute (loops included).
pub const MAX_EXECUTED: u32 = 500;
pub const MAX_RETRIES: u8 = 3;
pub const MAX_WAIT_SECONDS: i64 = 3600;
/// Names templates and expressions may start with.
pub const ROOTS: &[&str] = &["Trigger", "Steps", "inputs", "Loop", "Workflow", "user"];
const TRANSITIONS: &[&str] = &["alert", "warn", "nodata", "ok"];
const INCIDENT_EVENTS: &[&str] = &["declared", "updated", "resolved"];
const SEVERITIES: &[&str] = &["SEV-1", "SEV-2", "SEV-3", "SEV-4", "SEV-5", "UNKNOWN"];
const AGGREGATIONS: &[&str] = &["avg", "sum", "min", "max", "last"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerKind {
    /// Run from the editor or the workflow list.
    Manual,
    /// A monitor whose message mentions `@workflow-<handle>` changes state.
    Monitor,
    /// A cron schedule.
    Schedule,
    /// An incident is declared, updated or resolved.
    Incident,
    /// `POST /api/v1/workflows/{id}/trigger` with an application key and the workflow's token.
    Api,
    /// A button of an app, or a self-service action.
    App,
}

impl WorkflowTriggerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowTriggerKind::Manual => "manual",
            WorkflowTriggerKind::Monitor => "monitor",
            WorkflowTriggerKind::Schedule => "schedule",
            WorkflowTriggerKind::Incident => "incident",
            WorkflowTriggerKind::Api => "api",
            WorkflowTriggerKind::App => "app",
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTrigger {
    pub kind: WorkflowTriggerKind,
    /// Schedule: `minute hour day-of-month month day-of-week`.
    #[serde(default)]
    pub cron: String,
    /// Schedule: IANA time zone; UTC when empty.
    #[serde(default)]
    pub timezone: String,
    /// Monitor: the states that start a run (alert, warn, nodata, ok); alert when empty.
    #[serde(default)]
    pub transitions: Vec<String>,
    /// Incident: declared, updated, resolved; all when empty.
    #[serde(default)]
    pub incident_events: Vec<String>,
    /// Incident: severities that start a run; any when empty.
    #[serde(default)]
    pub severities: Vec<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKind {
    /// HTTP requests sent in order (`requests`).
    Http,
    /// Fields `title`, `severity`, `summary`, `teams`, `services`.
    CreateIncident,
    /// Fields `team`, `title`, `description`, `urgency`.
    PageOnCall,
    /// Fields `incident`, `body`.
    IncidentComment,
    /// Field `test`.
    SyntheticRun,
    /// Fields `recipients` (`@slack-…`, `@webhook-…`, member emails), `title`, `message`.
    Notify,
    /// Fields `query` (a metric query or formula), `minutes` (15), `aggregation` (avg, sum, min, max, last).
    MetricQuery,
    /// Fields `query` (a log search), `minutes` (15), `group_by` (a facet, optional).
    LogCount,
    /// Field `seconds` (1–3600).
    Wait,
    /// `expression` chooses `steps` or `else_steps`.
    If,
    /// `expression` is a list; `steps` run per item.
    ForEach,
}

impl WorkflowStepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowStepKind::Http => "http",
            WorkflowStepKind::CreateIncident => "create_incident",
            WorkflowStepKind::PageOnCall => "page_on_call",
            WorkflowStepKind::IncidentComment => "incident_comment",
            WorkflowStepKind::SyntheticRun => "synthetic_run",
            WorkflowStepKind::Notify => "notify",
            WorkflowStepKind::MetricQuery => "metric_query",
            WorkflowStepKind::LogCount => "log_count",
            WorkflowStepKind::Wait => "wait",
            WorkflowStepKind::If => "if",
            WorkflowStepKind::ForEach => "for_each",
        }
    }

    /// The self-service action step it runs as, for the shared ones.
    pub fn action_kind(self) -> Option<ActionStepKind> {
        Some(match self {
            WorkflowStepKind::Http => ActionStepKind::Http,
            WorkflowStepKind::CreateIncident => ActionStepKind::CreateIncident,
            WorkflowStepKind::PageOnCall => ActionStepKind::PageOnCall,
            WorkflowStepKind::IncidentComment => ActionStepKind::IncidentComment,
            WorkflowStepKind::SyntheticRun => ActionStepKind::SyntheticRun,
            _ => return None,
        })
    }

    /// Fields of the workflow-only action steps, and whether each is required.
    fn own_fields(self) -> &'static [(&'static str, bool)] {
        match self {
            WorkflowStepKind::Notify => &[("recipients", true), ("title", false), ("message", true)],
            WorkflowStepKind::MetricQuery => &[("query", true), ("minutes", false), ("aggregation", false)],
            WorkflowStepKind::LogCount => &[("query", true), ("minutes", false), ("group_by", false)],
            WorkflowStepKind::Wait => &[("seconds", true)],
            _ => &[],
        }
    }

    pub fn is_control(self) -> bool {
        matches!(self, WorkflowStepKind::If | WorkflowStepKind::ForEach)
    }
}

/// A step of a workflow.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[schema(no_recursion)]
pub struct WorkflowStep {
    /// `[a-z0-9_]`, unique in the workflow; others read its result as `Steps.<id>`.
    pub id: String,
    pub kind: WorkflowStepKind,
    /// Shown on the canvas; the kind's name when empty.
    #[serde(default)]
    pub name: String,
    /// Http only.
    #[serde(default)]
    pub requests: Vec<ActionHttpRequest>,
    /// Action steps: a template per field.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// If: the condition. For each: the list.
    #[serde(default)]
    pub expression: String,
    /// If: run when the condition holds. For each: run per item.
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
    /// If: run otherwise.
    #[serde(default)]
    pub else_steps: Vec<WorkflowStep>,
    /// Action steps: attempts after a failure (0–3), 5 s apart.
    #[serde(default)]
    pub retries: u8,
    /// Action steps: a failure is recorded and the run goes on.
    #[serde(default)]
    pub continue_on_error: bool,
}

/// What a workflow does: its form, triggers and steps.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSpec {
    #[serde(default)]
    pub inputs: Vec<ActionInput>,
    #[serde(default)]
    pub triggers: Vec<WorkflowTrigger>,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
}

impl WorkflowSpec {
    pub fn has_trigger(&self, kind: WorkflowTriggerKind) -> bool {
        self.triggers.iter().any(|t| t.kind == kind)
    }
}

// ================================================================ checks

fn step_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn check_trigger(t: &WorkflowTrigger) -> Result<WorkflowTrigger, String> {
    let mut out = WorkflowTrigger { kind: t.kind, cron: String::new(), timezone: String::new(), transitions: vec![], incident_events: vec![], severities: vec![] };
    let subset = |values: &[String], allowed: &[&str], what: &str| -> Result<Vec<String>, String> {
        let mut v: Vec<String> = Vec::new();
        for x in values {
            let x = x.trim().to_string();
            if !allowed.contains(&x.as_str()) {
                return Err(format!("{x} isn't a {what} ({})", allowed.join(", ")));
            }
            if !v.contains(&x) {
                v.push(x);
            }
        }
        Ok(v)
    };
    match t.kind {
        WorkflowTriggerKind::Schedule => {
            out.cron = t.cron.split_whitespace().collect::<Vec<_>>().join(" ");
            Cron::parse(&out.cron)?;
            out.timezone = t.timezone.trim().to_string();
            zone(&out.timezone)?;
        }
        WorkflowTriggerKind::Monitor => out.transitions = subset(&t.transitions, TRANSITIONS, "monitor state")?,
        WorkflowTriggerKind::Incident => {
            out.incident_events = subset(&t.incident_events, INCIDENT_EVENTS, "incident event")?;
            out.severities = subset(&t.severities, SEVERITIES, "severity")?;
        }
        _ => {}
    }
    Ok(out)
}

struct Walk<'a> {
    inputs: &'a [ActionInput],
    secrets: &'a HashSet<String>,
    ids: HashSet<String>,
    count: usize,
}

fn check_steps(list: &[WorkflowStep], w: &mut Walk, depth: usize, loops: usize) -> Result<Vec<WorkflowStep>, String> {
    if depth > MAX_NESTING {
        return Err(format!("Steps can be nested at most {MAX_NESTING} levels deep"));
    }
    let mut out = Vec::with_capacity(list.len());
    for s in list {
        w.count += 1;
        if w.count > MAX_STEPS {
            return Err(format!("A workflow has at most {MAX_STEPS} steps"));
        }
        let id = s.id.trim().to_string();
        if !step_id_ok(&id) {
            return Err(format!("{id} isn't a step id (a–z, 0–9 and _, up to 40)"));
        }
        if !w.ids.insert(id.clone()) {
            return Err(format!("The step id {id} is used twice"));
        }
        let at = |e: String| format!("Step {id}: {e}");
        let name = s.name.trim().chars().take(200).collect::<String>();
        if s.retries > MAX_RETRIES {
            return Err(at(format!("retries are 0–{MAX_RETRIES}")));
        }
        let mut step = WorkflowStep {
            id: id.clone(),
            kind: s.kind,
            name,
            requests: vec![],
            fields: BTreeMap::new(),
            expression: String::new(),
            steps: vec![],
            else_steps: vec![],
            retries: 0,
            continue_on_error: false,
        };
        if s.kind.is_control() {
            step.expression = s.expression.trim().to_string();
            expr::check(&step.expression, ROOTS).map_err(at)?;
            let inner_loops = loops + usize::from(s.kind == WorkflowStepKind::ForEach);
            if inner_loops > MAX_LOOP_NESTING {
                return Err(at(format!("“for each” steps can be nested {MAX_LOOP_NESTING} deep at most")));
            }
            step.steps = check_steps(&s.steps, w, depth + 1, inner_loops)?;
            if s.kind == WorkflowStepKind::If {
                step.else_steps = check_steps(&s.else_steps, w, depth + 1, loops)?;
            } else if !s.else_steps.is_empty() {
                return Err(at("only if/else steps have an else branch".into()));
            }
        } else {
            step.retries = s.retries;
            step.continue_on_error = s.continue_on_error;
            if !s.steps.is_empty() || !s.else_steps.is_empty() {
                return Err(at("only if/else and “for each” steps contain steps".into()));
            }
            if let Some(kind) = s.kind.action_kind() {
                let action = ActionStep { kind, requests: s.requests.clone(), fields: s.fields.clone() };
                let checked = steps::check_step_in(&action, w.inputs, w.secrets, ROOTS).map_err(at)?;
                step.requests = checked.requests;
                step.fields = checked.fields;
            } else {
                let allowed = s.kind.own_fields();
                if let Some(unknown) = s.fields.keys().find(|k| !allowed.iter().any(|(f, _)| f == k)) {
                    return Err(at(format!("there's no field {unknown} (fields: {})", allowed.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", "))));
                }
                let scope = TemplateScope { inputs: w.inputs, secrets: w.secrets, allow_secrets: false, prior_steps: 0, roots: ROOTS };
                for (field, required) in allowed {
                    let v = s.fields.get(*field).map(|v| v.trim().to_string()).unwrap_or_default();
                    if *required && v.is_empty() {
                        return Err(at(format!("the {field} field is required")));
                    }
                    if v.chars().count() > steps::MAX_TEMPLATE_CHARS {
                        return Err(at(format!("the {field} field is too long")));
                    }
                    steps::check_template(&v, &scope).map_err(at)?;
                    if !v.is_empty() {
                        step.fields.insert(field.to_string(), v);
                    }
                }
                check_literals(&step).map_err(at)?;
            }
        }
        out.push(step);
    }
    Ok(out)
}

/// Fields written without placeholders are checked now rather than when the step runs.
fn check_literals(step: &WorkflowStep) -> Result<(), String> {
    let literal = |f: &str| step.fields.get(f).filter(|v| !v.contains("{{")).cloned();
    if let Some(s) = literal("seconds") {
        wait_seconds(&s)?;
    }
    if let Some(m) = literal("minutes") {
        window_minutes(&m)?;
    }
    if let Some(a) = literal("aggregation") {
        if !AGGREGATIONS.contains(&a.as_str()) {
            return Err(format!("the aggregation is one of {}", AGGREGATIONS.join(", ")));
        }
    }
    Ok(())
}

/// The seconds of a wait step (1–3600).
pub fn wait_seconds(v: &str) -> Result<i64, String> {
    let n: f64 = v.trim().parse().map_err(|_| format!("{v} isn't a number of seconds"))?;
    if !(1.0..=MAX_WAIT_SECONDS as f64).contains(&n) {
        return Err(format!("A wait lasts 1–{MAX_WAIT_SECONDS} seconds"));
    }
    Ok(n.round() as i64)
}

/// The window of a query step, in minutes (1–1440, 15 when empty).
pub fn window_minutes(v: &str) -> Result<i64, String> {
    if v.trim().is_empty() {
        return Ok(15);
    }
    let n: i64 = v.trim().parse().map_err(|_| format!("{v} isn't a number of minutes"))?;
    if !(1..=1440).contains(&n) {
        return Err("A query looks back 1–1440 minutes".into());
    }
    Ok(n)
}

/// Checks and normalizes a workflow's form, triggers and steps against its secret names.
pub fn check_spec(spec: &WorkflowSpec, secrets: &HashSet<String>) -> Result<WorkflowSpec, String> {
    let inputs = steps::check_inputs(&spec.inputs)?;
    if spec.triggers.len() > MAX_TRIGGERS {
        return Err(format!("A workflow has at most {MAX_TRIGGERS} triggers"));
    }
    let mut triggers = Vec::with_capacity(spec.triggers.len());
    for t in &spec.triggers {
        if t.kind != WorkflowTriggerKind::Schedule && spec.triggers.iter().filter(|x| x.kind == t.kind).count() > 1 {
            return Err(format!("A workflow has one {} trigger at most", t.kind.as_str()));
        }
        triggers.push(check_trigger(t)?);
    }
    let mut walk = Walk { inputs: &inputs, secrets, ids: HashSet::new(), count: 0 };
    let checked = check_steps(&spec.steps, &mut walk, 1, 0)?;
    Ok(WorkflowSpec { inputs, triggers, steps: checked })
}

// ================================================================ triggers

/// A monitor trigger starts a run on a transition to `to`.
pub fn monitor_matches(t: &WorkflowTrigger, to: &str) -> bool {
    t.kind == WorkflowTriggerKind::Monitor && if t.transitions.is_empty() { to == "alert" } else { t.transitions.iter().any(|x| x == to) }
}

/// An incident trigger starts a run on `event` (declared, updated, resolved) of an incident of `severity`.
pub fn incident_matches(t: &WorkflowTrigger, event: &str, severity: &str) -> bool {
    t.kind == WorkflowTriggerKind::Incident && (t.incident_events.is_empty() || t.incident_events.iter().any(|e| e == event)) && (t.severities.is_empty() || t.severities.iter().any(|s| s == severity))
}

/// The next time after `after_ms` one of the schedule triggers fires.
pub fn next_schedule(spec: &WorkflowSpec, after_ms: i64) -> Option<i64> {
    spec.triggers.iter().filter(|t| t.kind == WorkflowTriggerKind::Schedule).filter_map(|t| Cron::parse(&t.cron).ok().zip(zone(&t.timezone).ok()).and_then(|(c, z)| c.next_after(after_ms, z))).min()
}

// ================================================================ run state machine

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Branch {
    Root,
    Then,
    Else,
    Body,
}

/// A block being run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Frame {
    /// The if/else or "for each" step the block belongs to; none for the top level.
    pub parent: Option<String>,
    pub branch: Branch,
    /// The next step of the block.
    pub index: usize,
    /// For each: the items, and which one is being run.
    #[serde(default)]
    pub items: Vec<Value>,
    #[serde(default)]
    pub item: usize,
}

/// Where a run is: the blocks being run (innermost last) and the latest result of each step.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RunState {
    pub frames: Vec<Frame>,
    /// `Steps.<id>`: `{"status", "output", "error"}`.
    pub steps: Map<String, Value>,
    /// Action steps run so far.
    pub executed: u32,
}

/// A decision taken by a control step, for the run's history.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub step_id: String,
    pub kind: WorkflowStepKind,
    pub name: String,
    pub output: Value,
    pub iteration: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// Run this action step (in the innermost loop's `iteration`).
    Execute(Box<WorkflowStep>, Option<i32>),
    Done,
    Failed(String),
}

pub fn start() -> RunState {
    RunState { frames: vec![Frame { parent: None, branch: Branch::Root, index: 0, items: vec![], item: 0 }], ..RunState::default() }
}

fn find<'a>(list: &'a [WorkflowStep], id: &str) -> Option<&'a WorkflowStep> {
    for s in list {
        if s.id == id {
            return Some(s);
        }
        if let Some(f) = find(&s.steps, id).or_else(|| find(&s.else_steps, id)) {
            return Some(f);
        }
    }
    None
}

fn block<'a>(spec: &'a WorkflowSpec, f: &Frame) -> &'a [WorkflowStep] {
    let Some(parent) = &f.parent else { return &spec.steps };
    match (find(&spec.steps, parent), f.branch) {
        (Some(s), Branch::Else) => &s.else_steps,
        (Some(s), _) => &s.steps,
        (None, _) => &[],
    }
}

/// The data templates and expressions read: `base` (Trigger, inputs, Workflow, user) plus Steps and Loop.
pub fn vars(state: &RunState, base: &Value) -> Value {
    let mut v = base.as_object().cloned().unwrap_or_default();
    v.insert("Steps".into(), Value::Object(state.steps.clone()));
    if let Some(f) = state.frames.iter().rev().find(|f| f.branch == Branch::Body) {
        v.insert("Loop".into(), json!({"item": f.items.get(f.item).cloned().unwrap_or(Value::Null), "index": f.item}));
    }
    Value::Object(v)
}

fn iteration(state: &RunState) -> Option<i32> {
    state.frames.iter().rev().find(|f| f.branch == Branch::Body).map(|f| f.item as i32)
}

/// Walks control steps from where the run is to the next action step. Decisions taken on the way go
/// to `decisions`.
pub fn advance(spec: &WorkflowSpec, state: &mut RunState, base: &Value, decisions: &mut Vec<Decision>) -> Next {
    // Every pass moves the run forward; the bound only guards against a malformed state.
    for _ in 0..100_000 {
        let Some(frame) = state.frames.last() else { return Next::Done };
        let list = block(spec, frame);
        if frame.index >= list.len() {
            let frame = state.frames.last_mut().expect("a frame");
            if frame.branch == Branch::Body && frame.item + 1 < frame.items.len() {
                frame.item += 1;
                frame.index = 0;
                continue;
            }
            state.frames.pop();
            if let Some(outer) = state.frames.last_mut() {
                outer.index += 1;
            }
            continue;
        }
        let step = &list[frame.index];
        match step.kind {
            WorkflowStepKind::If | WorkflowStepKind::ForEach => {
                let parsed = match expr::parse(&step.expression) {
                    Ok(e) => e,
                    Err(e) => return Next::Failed(format!("Step {}: {e}", step.id)),
                };
                let value = expr::eval(&parsed, &vars(state, base));
                let at = iteration(state);
                if step.kind == WorkflowStepKind::If {
                    let holds = expr::truthy(&value);
                    let branch = if holds { "then" } else { "else" };
                    decisions.push(Decision { step_id: step.id.clone(), kind: step.kind, name: step.name.clone(), output: json!({"result": holds, "branch": branch}), iteration: at });
                    state.steps.insert(step.id.clone(), json!({"status": "succeeded", "output": {"result": holds, "branch": branch}}));
                    state.frames.push(Frame { parent: Some(step.id.clone()), branch: if holds { Branch::Then } else { Branch::Else }, index: 0, items: vec![], item: 0 });
                } else {
                    let items = match value {
                        Value::Array(a) => a,
                        Value::Null => vec![],
                        other => return Next::Failed(format!("Step {}: “for each” needs a list, and {} isn't one", step.id, short(&other))),
                    };
                    if items.len() > MAX_LOOP_ITEMS {
                        return Next::Failed(format!("Step {}: “for each” runs over {MAX_LOOP_ITEMS} items at most ({} given)", step.id, items.len()));
                    }
                    decisions.push(Decision { step_id: step.id.clone(), kind: step.kind, name: step.name.clone(), output: json!({"items": items.len()}), iteration: at });
                    state.steps.insert(step.id.clone(), json!({"status": "succeeded", "output": {"items": items.len()}}));
                    if items.is_empty() {
                        state.frames.last_mut().expect("a frame").index += 1;
                    } else {
                        state.frames.push(Frame { parent: Some(step.id.clone()), branch: Branch::Body, index: 0, items, item: 0 });
                    }
                }
            }
            _ => {
                if state.executed >= MAX_EXECUTED {
                    return Next::Failed(format!("The run stopped after {MAX_EXECUTED} steps"));
                }
                return Next::Execute(Box::new(step.clone()), iteration(state));
            }
        }
    }
    Next::Failed("The run's state is inconsistent".into())
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

/// Records the result of the action step [`advance`] handed out and moves past it.
pub fn complete(state: &mut RunState, step_id: &str, ok: bool, output: Value, error: Option<String>) {
    state.steps.insert(step_id.to_string(), json!({"status": if ok { "succeeded" } else { "failed" }, "output": output, "error": error}));
    state.executed += 1;
    if let Some(f) = state.frames.last_mut() {
        f.index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            kind: WorkflowStepKind::Notify,
            name: String::new(),
            requests: vec![],
            fields: BTreeMap::from([("recipients".to_string(), "@slack-checkout".to_string()), ("message".to_string(), "Hello".to_string())]),
            expression: String::new(),
            steps: vec![],
            else_steps: vec![],
            retries: 0,
            continue_on_error: false,
        }
    }

    fn control(id: &str, kind: WorkflowStepKind, expression: &str, steps: Vec<WorkflowStep>, else_steps: Vec<WorkflowStep>) -> WorkflowStep {
        WorkflowStep { id: id.into(), kind, expression: expression.into(), steps, else_steps, fields: BTreeMap::new(), ..action(id) }
    }

    fn spec(steps: Vec<WorkflowStep>) -> WorkflowSpec {
        WorkflowSpec {
            inputs: vec![],
            triggers: vec![WorkflowTrigger { kind: WorkflowTriggerKind::Manual, cron: String::new(), timezone: String::new(), transitions: vec![], incident_events: vec![], severities: vec![] }],
            steps,
        }
    }

    /// Runs a workflow to the end, answering every action step with `answer`; returns the executed ids.
    fn run_all(spec: &WorkflowSpec, base: &Value, answer: impl Fn(&str, &RunState) -> (bool, Value)) -> (Vec<String>, Vec<Decision>, Next) {
        let mut state = start();
        let mut done = Vec::new();
        let mut decisions = Vec::new();
        loop {
            match advance(spec, &mut state, base, &mut decisions) {
                Next::Execute(step, _) => {
                    let (ok, out) = answer(&step.id, &state);
                    done.push(step.id.clone());
                    complete(&mut state, &step.id, ok, out, None);
                    // The state survives a round trip through the database.
                    state = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
                }
                other => return (done, decisions, other),
            }
        }
    }

    #[test]
    fn steps_run_in_order() {
        let s = spec(vec![action("a"), action("b"), action("c")]);
        let (done, _, end) = run_all(&s, &json!({}), |_, _| (true, json!({})));
        assert_eq!(done, vec!["a", "b", "c"]);
        assert_eq!(end, Next::Done);
    }

    #[test]
    fn if_else_chooses_a_branch_from_earlier_results() {
        let s = spec(vec![
            action("check"),
            control("still_alerting", WorkflowStepKind::If, "Steps.check.output.state == 'alert' && Trigger.value > 5", vec![action("restart"), action("tell")], vec![action("skip_note")]),
            action("after"),
        ]);
        let answer = |state: &'static str| move |id: &str, _: &RunState| (true, if id == "check" { json!({"state": state}) } else { json!({}) });
        let (done, decisions, _) = run_all(&s, &json!({"Trigger": {"value": "7"}}), answer("alert"));
        assert_eq!(done, vec!["check", "restart", "tell", "after"]);
        assert_eq!(decisions[0].output, json!({"result": true, "branch": "then"}));
        let (done, decisions, _) = run_all(&s, &json!({"Trigger": {"value": "7"}}), answer("ok"));
        assert_eq!(done, vec!["check", "skip_note", "after"]);
        assert_eq!(decisions[0].output["branch"], "else");
        // An empty branch is fine.
        let s = spec(vec![control("gate", WorkflowStepKind::If, "false", vec![action("never")], vec![]), action("end")]);
        assert_eq!(run_all(&s, &json!({}), |_, _| (true, json!({}))).0, vec!["end"]);
    }

    #[test]
    fn for_each_runs_per_item_with_loop_vars() {
        let s = spec(vec![
            action("list"),
            control(
                "each",
                WorkflowStepKind::ForEach,
                "Steps.list.output.services",
                vec![control("is_billing", WorkflowStepKind::If, "Loop.item == 'billing'", vec![action("page")], vec![action("note")])],
                vec![],
            ),
            action("done"),
        ]);
        let (done, decisions, end) = run_all(&s, &json!({}), |id, st| {
            if id == "list" {
                return (true, json!({"services": ["order-service", "billing", "catalog-service"]}));
            }
            let v = vars(st, &json!({}));
            assert_eq!(v["Loop"]["item"].is_string(), id != "done", "Loop exists inside the loop only");
            (true, json!({"item": v["Loop"]["item"]}))
        });
        assert_eq!(done, vec!["list", "note", "page", "note", "done"]);
        assert_eq!(end, Next::Done);
        assert_eq!(decisions.iter().filter(|d| d.step_id == "is_billing").map(|d| d.iteration).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(2)]);
        let (done, _, _) = run_all(&s, &json!({}), |id, _| (true, if id == "list" { json!({"services": []}) } else { json!({}) }));
        assert_eq!(done, vec!["list", "done"], "an empty list skips the body");
    }

    #[test]
    fn loops_and_runs_are_bounded() {
        let s = spec(vec![control("each", WorkflowStepKind::ForEach, "Trigger.items", vec![action("x")], vec![])]);
        let many: Vec<i32> = (0..=MAX_LOOP_ITEMS as i32).collect();
        let (_, _, end) = run_all(&s, &json!({"Trigger": {"items": many}}), |_, _| (true, json!({})));
        assert!(matches!(end, Next::Failed(e) if e.contains("100 items")));
        let (_, _, end) = run_all(&s, &json!({"Trigger": {"items": "nope"}}), |_, _| (true, json!({})));
        assert!(matches!(end, Next::Failed(e) if e.contains("needs a list")));
        // 100 × 100 nested items exceed the step budget.
        let nested = spec(vec![control("outer", WorkflowStepKind::ForEach, "Trigger.items", vec![control("inner", WorkflowStepKind::ForEach, "Trigger.items", vec![action("x")], vec![])], vec![])]);
        let hundred: Vec<i32> = (0..100).collect();
        let (done, _, end) = run_all(&nested, &json!({"Trigger": {"items": hundred}}), |_, _| (true, json!({})));
        assert_eq!(done.len(), MAX_EXECUTED as usize);
        assert!(matches!(end, Next::Failed(e) if e.contains("500 steps")));
    }

    #[test]
    fn specs_are_checked() {
        let none = HashSet::new();
        let ok = check_spec(&spec(vec![action("a"), control("b", WorkflowStepKind::If, "{{ Steps.a.status == 'succeeded' }}", vec![action("c")], vec![])]), &none).unwrap();
        assert_eq!(ok.steps[1].steps[0].id, "c");
        let err = |s: WorkflowSpec| check_spec(&s, &none).unwrap_err();
        assert!(err(spec(vec![action("a"), action("a")])).contains("twice"));
        assert!(err(spec(vec![action("Bad Id")])).contains("Bad Id"));
        assert!(err(spec(vec![control("b", WorkflowStepKind::If, "Secret.x", vec![], vec![])])).contains("Secret"));
        let mut no_message = action("n");
        no_message.fields.remove("message");
        assert!(err(spec(vec![no_message])).contains("message"));
        let mut wait = action("w");
        wait.kind = WorkflowStepKind::Wait;
        wait.fields = BTreeMap::from([("seconds".to_string(), "7200".to_string())]);
        assert!(err(spec(vec![wait.clone()])).contains("1–3600"));
        wait.fields.insert("seconds".into(), "{{ inputs.delay }}".into());
        assert!(err(spec(vec![wait])).contains("inputs.delay"), "inputs must exist");
        let three = control(
            "l1",
            WorkflowStepKind::ForEach,
            "Trigger.a",
            vec![control("l2", WorkflowStepKind::ForEach, "Trigger.a", vec![control("l3", WorkflowStepKind::ForEach, "Trigger.a", vec![action("x")], vec![])], vec![])],
            vec![],
        );
        assert!(err(spec(vec![three])).contains("nested 2"));
        let mut retries = action("r");
        retries.retries = 9;
        assert!(err(spec(vec![retries])).contains("retries"));
        let too_many: Vec<WorkflowStep> = (0..=MAX_STEPS).map(|i| action(&format!("s{i}"))).collect();
        assert!(err(spec(too_many)).contains("at most 100"));
        // Shared steps go through the self-service checks, with the workflow's names.
        let mut page = action("page");
        page.kind = WorkflowStepKind::PageOnCall;
        page.fields = BTreeMap::from([("team".to_string(), "payments".to_string()), ("title".to_string(), "{{ Trigger.monitor.name }} burns".to_string())]);
        assert!(check_spec(&spec(vec![page.clone()]), &none).is_ok());
        page.fields.insert("color".into(), "red".into());
        assert!(err(spec(vec![page])).contains("color"));
    }

    #[test]
    fn triggers_are_checked() {
        let none = HashSet::new();
        let t = |kind, cron: &str| WorkflowTrigger { kind, cron: cron.into(), timezone: "Europe/Paris".into(), transitions: vec![], incident_events: vec![], severities: vec![] };
        let mut s = spec(vec![]);
        s.triggers = vec![t(WorkflowTriggerKind::Schedule, " 0  9 * * MON-FRI "), t(WorkflowTriggerKind::Schedule, "*/30 * * * *")];
        assert_eq!(check_spec(&s, &none).unwrap().triggers[0].cron, "0 9 * * MON-FRI");
        s.triggers = vec![t(WorkflowTriggerKind::Schedule, "every day")];
        assert!(check_spec(&s, &none).unwrap_err().contains("5 fields"));
        s.triggers = vec![t(WorkflowTriggerKind::Monitor, ""), t(WorkflowTriggerKind::Monitor, "")];
        assert!(check_spec(&s, &none).unwrap_err().contains("one monitor"));
        let mut m = t(WorkflowTriggerKind::Monitor, "");
        m.transitions = vec!["exploded".into()];
        s.triggers = vec![m];
        assert!(check_spec(&s, &none).unwrap_err().contains("exploded"));
    }

    #[test]
    fn trigger_matching() {
        let mut m = WorkflowTrigger { kind: WorkflowTriggerKind::Monitor, cron: String::new(), timezone: String::new(), transitions: vec![], incident_events: vec![], severities: vec![] };
        assert!(monitor_matches(&m, "alert"));
        assert!(!monitor_matches(&m, "ok"), "alerts only by default");
        m.transitions = vec!["warn".into(), "ok".into()];
        assert!(monitor_matches(&m, "ok") && monitor_matches(&m, "warn") && !monitor_matches(&m, "alert"));
        let mut i = WorkflowTrigger { kind: WorkflowTriggerKind::Incident, ..m.clone() };
        assert!(!monitor_matches(&i, "alert"));
        assert!(incident_matches(&i, "declared", "SEV-4"));
        i.incident_events = vec!["declared".into()];
        i.severities = vec!["SEV-1".into(), "SEV-2".into()];
        assert!(incident_matches(&i, "declared", "SEV-1"));
        assert!(!incident_matches(&i, "declared", "SEV-3"));
        assert!(!incident_matches(&i, "updated", "SEV-1"));
        let mut s = spec(vec![]);
        s.triggers = vec![
            WorkflowTrigger { kind: WorkflowTriggerKind::Schedule, cron: "0 9 * * *".into(), timezone: "UTC".into(), ..m.clone() },
            WorkflowTrigger { kind: WorkflowTriggerKind::Schedule, cron: "30 8 * * *".into(), timezone: "UTC".into(), ..m.clone() },
        ];
        let at = chrono::DateTime::parse_from_rfc3339("2026-03-02T08:00:00Z").unwrap().timestamp_millis();
        assert_eq!(next_schedule(&s, at), Some(at + 30 * 60_000));
        assert_eq!(next_schedule(&spec(vec![]), at), None);
    }
}
