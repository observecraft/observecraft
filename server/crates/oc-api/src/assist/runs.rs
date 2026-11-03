//! Background runs: an investigation of an alert, a verdict on a security signal, a custom agent's run
//! or a coding session. Each is the same tool loop as the chat, without a user waiting: tools that
//! change something are left out unless the run allows them, and the model ends by calling `conclude`,
//! whose input is the run's structured conclusion.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::agent::Tools;
use super::llm::ToolSpec;

/// What a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Why a monitor alerted (or any problem described by a user).
    Investigation,
    /// Whether a Cloud SIEM signal is a real threat.
    Security,
    /// A custom agent doing what its instructions say.
    Agent,
    /// A coding session: a change proposed to a repository.
    Code,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Investigation => "investigation",
            Kind::Security => "security",
            Kind::Agent => "agent",
            Kind::Code => "code",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Investigation, Kind::Security, Kind::Agent, Kind::Code].into_iter().find(|k| k.as_str() == s)
    }

    /// The verdicts `conclude` accepts.
    pub fn verdicts(self) -> &'static [&'static str] {
        match self {
            Kind::Investigation => &["conclusive", "inconclusive", "no_issue"],
            Kind::Security => &["benign", "suspicious", "malicious", "inconclusive"],
            Kind::Agent => &["done", "needs_attention", "failed"],
            Kind::Code => &["change_proposed", "no_change", "failed"],
        }
    }
}

/// A finding that supports the conclusion.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// A link inside the app (`/logs?query=…`) showing it.
    #[serde(default)]
    pub link: Option<String>,
}

/// How a run ended, as the model concluded it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Conclusion {
    /// Markdown.
    pub summary: String,
    pub verdict: String,
    /// high | medium | low
    pub confidence: String,
    #[serde(default)]
    pub root_cause: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub next_steps: Vec<String>,
}

const CONFIDENCE: &[&str] = &["high", "medium", "low"];

/// A file of a proposed change: its content before (none: a new file) and after.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    #[serde(default)]
    pub before: Option<String>,
    pub after: String,
}

/// The change a coding session proposes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeChange {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub files: Vec<ChangedFile>,
}

fn propose_spec() -> ToolSpec {
    ToolSpec {
        name: "propose_change".into(),
        description: "Propose your change to the repository: each changed file with its whole new content (not a diff). The user reviews it and opens a pull request from it. Call it once, before conclude.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "Pull request title, imperative (Use an index for cart lookups)"},
                "description": {"type": "string", "description": "What changes and why, in Markdown, citing the evidence"},
                "files": {"type": "array", "items": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}},
            },
            "required": ["title", "files"],
        }),
    }
}

fn conclude_spec(kind: Kind) -> ToolSpec {
    ToolSpec {
        name: "conclude".into(),
        description: "Record your conclusion once you have looked at enough data. Call it exactly once, as your last action; the user reads it as the result of this run. Cite the evidence you actually saw, with links inside the app where you have them.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string", "description": "What happened and why, in Markdown, a few sentences."},
                "verdict": {"type": "string", "enum": kind.verdicts()},
                "confidence": {"type": "string", "enum": CONFIDENCE},
                "root_cause": {"type": "string", "description": "The root cause in one line, when known."},
                "evidence": {"type": "array", "items": {"type": "object", "properties": {"title": {"type": "string"}, "detail": {"type": "string"}, "link": {"type": "string"}}, "required": ["title"]}},
                "next_steps": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["summary", "verdict"],
        }),
    }
}

/// A run's tools: the base tools (without those that change something unless the run allows them)
/// plus `conclude`.
pub struct RunTools<T: Tools> {
    base: T,
    kind: Kind,
    actions: bool,
    conclusion: Mutex<Option<Conclusion>>,
    change: Mutex<Option<CodeChange>>,
}

impl<T: Tools> RunTools<T> {
    pub fn new(base: T, kind: Kind, actions: bool) -> Self {
        RunTools { base, kind, actions, conclusion: Mutex::new(None), change: Mutex::new(None) }
    }

    pub fn conclusion(&self) -> Option<Conclusion> {
        self.conclusion.lock().unwrap().clone()
    }

    /// The change a coding session proposed (files' `before` not filled yet).
    pub fn change(&self) -> Option<CodeChange> {
        self.change.lock().unwrap().clone()
    }

    fn propose(&self, input: &Value) -> Result<String, String> {
        let title = input["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).ok_or("Give the change a title")?;
        let mut files = Vec::new();
        for f in input["files"].as_array().ok_or("List the changed files")? {
            let path = f["path"].as_str().unwrap_or_default().trim_start_matches("./");
            super::code::check_path(path)?;
            let after = f["content"].as_str().ok_or_else(|| format!("{path}: give its whole new content"))?;
            files.push(ChangedFile { path: path.to_string(), before: None, after: after.to_string() });
        }
        if files.is_empty() || files.len() > 20 {
            return Err("A change has 1 to 20 files".into());
        }
        *self.change.lock().unwrap() = Some(CodeChange { title: title.to_string(), description: input["description"].as_str().unwrap_or_default().to_string(), files });
        Ok("Proposed. Now conclude.".into())
    }

    fn record(&self, input: &Value) -> Result<String, String> {
        let mut c: Conclusion = serde_json::from_value(json!({
            "summary": input["summary"],
            "verdict": input["verdict"],
            "confidence": input["confidence"].as_str().unwrap_or("medium"),
            "rootCause": input.get("root_cause"),
            "evidence": input.get("evidence").cloned().unwrap_or(json!([])),
            "nextSteps": input.get("next_steps").cloned().unwrap_or(json!([])),
        }))
        .map_err(|e| format!("Invalid conclusion: {e}"))?;
        if !self.kind.verdicts().contains(&c.verdict.as_str()) {
            return Err(format!("The verdict must be one of {}", self.kind.verdicts().join(", ")));
        }
        if !CONFIDENCE.contains(&c.confidence.as_str()) {
            c.confidence = "medium".into();
        }
        c.evidence.retain(|e| !e.title.trim().is_empty());
        *self.conclusion.lock().unwrap() = Some(c);
        Ok("Recorded. Stop here.".into())
    }
}

impl<T: Tools + Sync> Tools for RunTools<T> {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self.base.specs().into_iter().filter(|s| self.actions || !self.base.writes(&s.name)).collect();
        if self.kind == Kind::Code {
            specs.push(propose_spec());
        }
        specs.push(conclude_spec(self.kind));
        specs
    }

    fn writes(&self, name: &str) -> bool {
        self.base.writes(name)
    }

    async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
        if name == "conclude" {
            return self.record(input);
        }
        if name == "propose_change" && self.kind == Kind::Code {
            return self.propose(input);
        }
        if self.base.writes(name) && !self.actions {
            return Err(format!("{name} changes something, which this run may not do"));
        }
        self.base.run(name, input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::agent::Tools;
    use crate::assist::llm::ToolSpec;
    use serde_json::{json, Value};

    /// Two tools, one of which changes something.
    struct Base;
    impl Tools for Base {
        fn specs(&self) -> Vec<ToolSpec> {
            ["search_logs", "create_monitor"].iter().map(|n| ToolSpec { name: n.to_string(), description: String::new(), schema: json!({"type": "object", "properties": {}}) }).collect()
        }
        fn writes(&self, name: &str) -> bool {
            name == "create_monitor"
        }
        async fn run(&self, name: &str, _: &Value) -> Result<String, String> {
            Ok(format!("ran {name}"))
        }
    }

    fn names(t: &impl Tools) -> Vec<String> {
        t.specs().into_iter().map(|s| s.name).collect()
    }

    #[tokio::test]
    async fn a_run_reads_only_and_ends_with_its_conclusion() {
        let t = RunTools::new(Base, Kind::Investigation, false);
        assert_eq!(names(&t), vec!["search_logs", "conclude"]);
        assert!(t.run("create_monitor", &json!({})).await.is_err());
        assert_eq!(t.run("search_logs", &json!({})).await.unwrap(), "ran search_logs");

        // The verdict must be one of the kind's.
        assert!(t.run("conclude", &json!({"summary": "x", "verdict": "malicious"})).await.unwrap_err().contains("conclusive"));
        let input = json!({"summary": "inventory-service timed out after its deploy", "verdict": "conclusive", "confidence": "high", "root_cause": "deploy 3.2.1", "evidence": [{"title": "412 timeouts", "detail": "logs"}], "next_steps": ["Roll back"]});
        t.run("conclude", &input).await.unwrap();
        let c = t.conclusion().unwrap();
        assert_eq!((c.verdict.as_str(), c.confidence.as_str(), c.evidence.len()), ("conclusive", "high", 1));
        assert_eq!(c.next_steps, vec!["Roll back"]);
    }

    #[tokio::test]
    async fn a_run_that_allows_actions_keeps_them_and_security_runs_judge_signals() {
        let t = RunTools::new(Base, Kind::Agent, true);
        assert_eq!(names(&t), vec!["search_logs", "create_monitor", "conclude"]);
        let s = RunTools::new(Base, Kind::Security, false);
        s.run("conclude", &json!({"summary": "Known VPN", "verdict": "benign"})).await.unwrap();
        assert_eq!(s.conclusion().unwrap().verdict, "benign");
        assert_eq!(s.conclusion().unwrap().confidence, "medium", "defaults to medium");
    }

    #[tokio::test]
    async fn a_coding_session_proposes_whole_files() {
        let t = RunTools::new(Base, Kind::Code, false);
        assert!(names(&t).contains(&"propose_change".to_string()));
        assert!(!names(&RunTools::new(Base, Kind::Investigation, false)).contains(&"propose_change".to_string()));
        assert!(t.run("propose_change", &json!({"title": "x", "files": [{"path": "../etc/passwd", "content": "x"}]})).await.is_err());
        let ok = json!({"title": "Use an index for cart lookups", "description": "find_by scans", "files": [{"path": "app/models/cart.rb", "content": "class Cart; end\n"}]});
        t.run("propose_change", &ok).await.unwrap();
        let c = t.change().unwrap();
        assert_eq!((c.title.as_str(), c.files.len(), c.files[0].path.as_str()), ("Use an index for cart lookups", 1, "app/models/cart.rb"));
    }
}
