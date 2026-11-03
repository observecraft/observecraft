//! The tool loop: the model answers, calls tools, sees their results and answers again, until it
//! is done. Tools that change something (create a monitor, declare an incident) wait for the user's
//! approval unless the conversation auto-approves: the loop then stops with the calls pending and is
//! resumed with the user's decisions. All results of one turn go back in one message.
use std::future::Future;

use serde::Serialize;
use serde_json::Value;

use super::llm::{Msg, Stop, ToolCall, ToolResult, ToolSpec, Turn, Usage};

/// The model of a conversation (the provider over HTTP; a script in tests).
pub trait Model {
    /// One turn; `on_text` gets the text as it streams.
    fn turn(&self, system: &str, tools: &[ToolSpec], msgs: &[Msg], on_text: &mut (dyn FnMut(&str) + Send)) -> impl Future<Output = Result<Turn, String>> + Send;
}

/// What the model may call.
pub trait Tools {
    fn specs(&self) -> Vec<ToolSpec>;
    /// Whether the tool changes something (it then needs the user's approval).
    fn writes(&self, name: &str) -> bool;
    /// Runs one call with a valid input; `Err` is reported to the model as the tool's error.
    fn run(&self, name: &str, input: &Value) -> impl Future<Output = Result<String, String>> + Send;
}

/// What the client sees while a reply is produced.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Text { text: String },
    ToolStart { id: String, name: String, input: Value },
    ToolEnd { id: String, ok: bool, summary: String },
    /// A call waits for the user's approval.
    Approval { id: String, name: String, input: Value },
}

/// How a reply ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done,
    /// Calls wait for approval; the conversation resumes with [`resume`].
    Paused(Vec<ToolCall>),
    Failed(String),
}

/// Model turns per reply, at most (a model calling tools in circles stops here).
pub const MAX_STEPS: usize = 24;
/// A tool result sent to the model, at most (characters).
pub const MAX_RESULT: usize = 16_000;

pub struct Loop<'a, M: Model, T: Tools> {
    pub model: &'a M,
    pub tools: &'a T,
    pub system: &'a str,
    pub auto_approve: bool,
}

impl<M: Model + Sync, T: Tools + Sync> Loop<'_, M, T> {
    /// Answers the conversation's last message, appending the turns to `msgs`.
    pub async fn run(&self, msgs: &mut Vec<Msg>, usage: &mut Usage, emit: &mut (dyn FnMut(Event) + Send)) -> Outcome {
        let specs = self.tools.specs();
        for _ in 0..MAX_STEPS {
            let mut on_text = |t: &str| emit(Event::Text { text: t.to_string() });
            let turn = match self.model.turn(self.system, &specs, msgs, &mut on_text).await {
                Ok(t) => t,
                Err(e) => return Outcome::Failed(e),
            };
            usage.input += turn.usage.input;
            usage.output += turn.usage.output;
            let stop = turn.stop.clone();
            let calls = turn.calls.clone();
            msgs.push(turn.into_msg());
            match stop {
                Stop::Refusal(why) => return Outcome::Failed(if why.is_empty() { "The model declined to answer this request.".into() } else { format!("The model declined to answer: {why}") }),
                Stop::ToolUse if !calls.is_empty() => {}
                Stop::MaxTokens if calls.is_empty() => return Outcome::Failed("The answer was cut off at the model's output limit.".into()),
                Stop::MaxTokens => {}
                _ => return Outcome::Done,
            }
            let waiting: Vec<ToolCall> = calls.iter().filter(|c| c.input.is_ok() && self.tools.writes(&c.name) && !self.auto_approve).cloned().collect();
            if !waiting.is_empty() {
                for c in &waiting {
                    emit(Event::Approval { id: c.id.clone(), name: c.name.clone(), input: c.input.clone().unwrap_or_default() });
                }
                return Outcome::Paused(calls);
            }
            let results = self.execute(&calls, &[], emit).await;
            msgs.push(Msg::Results(results));
        }
        Outcome::Failed(format!("Stopped after {MAX_STEPS} steps without an answer."))
    }

    /// Runs the paused calls with the user's decisions (`approved` ids), then continues the reply.
    pub async fn resume(&self, msgs: &mut Vec<Msg>, pending: &[ToolCall], approved: &[String], usage: &mut Usage, emit: &mut (dyn FnMut(Event) + Send)) -> Outcome {
        let declined: Vec<String> = pending.iter().filter(|c| self.tools.writes(&c.name) && !approved.contains(&c.id)).map(|c| c.id.clone()).collect();
        let results = self.execute(pending, &declined, emit).await;
        msgs.push(Msg::Results(results));
        self.run(msgs, usage, emit).await
    }

    async fn execute(&self, calls: &[ToolCall], declined: &[String], emit: &mut (dyn FnMut(Event) + Send)) -> Vec<ToolResult> {
        let mut results = Vec::with_capacity(calls.len());
        for c in calls {
            let input = c.input.clone().unwrap_or(Value::Null);
            emit(Event::ToolStart { id: c.id.clone(), name: c.name.clone(), input: input.clone() });
            let outcome = match &c.input {
                Err(raw) => Err(format!("The input was not a valid JSON object: {}", truncate(raw, 200))),
                Ok(_) if declined.contains(&c.id) => Err("The user declined this action. Don't retry it; say what you would have done instead.".to_string()),
                Ok(input) if specs_has(&self.tools.specs(), &c.name) => self.tools.run(&c.name, input).await,
                Ok(_) => Err(format!("Unknown tool {}", c.name)),
            };
            let (ok, content) = match outcome {
                Ok(s) => (true, truncate(&s, MAX_RESULT)),
                Err(e) => (false, e),
            };
            emit(Event::ToolEnd { id: c.id.clone(), ok, summary: summary(&content) });
            results.push(ToolResult { id: c.id.clone(), content, is_error: !ok });
        }
        results
    }
}

fn specs_has(specs: &[ToolSpec], name: &str) -> bool {
    specs.iter().any(|s| s.name == name)
}

/// The first `max` characters, marked when cut.
pub fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}\n… (truncated: {} more characters)", &s[..i], s.len() - i),
        None => s.to_string(),
    }
}

/// A one-line summary of a tool's output for the client.
fn summary(content: &str) -> String {
    truncate(content.lines().next().unwrap_or(""), 160)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Replies with scripted turns, recording the conversations it was sent.
    struct Script {
        turns: Mutex<Vec<Turn>>,
        seen: Mutex<Vec<Vec<Msg>>>,
    }

    impl Script {
        fn new(turns: Vec<Turn>) -> Self {
            Script { turns: Mutex::new(turns.into_iter().rev().collect()), seen: Mutex::new(Vec::new()) }
        }
    }

    impl Model for Script {
        async fn turn(&self, _: &str, _: &[ToolSpec], msgs: &[Msg], on_text: &mut (dyn FnMut(&str) + Send)) -> Result<Turn, String> {
            self.seen.lock().unwrap().push(msgs.to_vec());
            let t = self.turns.lock().unwrap().pop().ok_or("no more turns")?;
            if !t.text.is_empty() {
                on_text(&t.text);
            }
            Ok(t)
        }
    }

    struct Fake;

    impl Tools for Fake {
        fn specs(&self) -> Vec<ToolSpec> {
            ["search_logs", "create_monitor"].iter().map(|n| ToolSpec { name: n.to_string(), description: String::new(), schema: json!({"type": "object"}) }).collect()
        }
        fn writes(&self, name: &str) -> bool {
            name == "create_monitor"
        }
        async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
            match name {
                "search_logs" if input["query"] == "boom" => Err("Invalid query".into()),
                "search_logs" => Ok(format!("3 logs for {}", input["query"].as_str().unwrap_or(""))),
                _ => Ok("Monitor 7 created".into()),
            }
        }
    }

    fn text(t: &str) -> Turn {
        Turn { text: t.into(), calls: vec![], blocks: None, stop: Stop::EndTurn, usage: Usage { input: 10, output: 2 } }
    }

    fn calls(calls: &[(&str, &str, Result<Value, String>)]) -> Turn {
        Turn {
            text: String::new(),
            calls: calls.iter().map(|(id, name, input)| ToolCall { id: id.to_string(), name: name.to_string(), input: input.clone() }).collect(),
            blocks: None,
            stop: Stop::ToolUse,
            usage: Usage { input: 10, output: 2 },
        }
    }

    async fn run(model: &Script, auto_approve: bool, msgs: &mut Vec<Msg>) -> (Outcome, Vec<Event>, Usage) {
        let mut events = Vec::new();
        let mut usage = Usage::default();
        let outcome = Loop { model, tools: &Fake, system: "s", auto_approve }.run(msgs, &mut usage, &mut |e| events.push(e)).await;
        (outcome, events, usage)
    }

    #[tokio::test]
    async fn tools_run_and_their_results_go_back_in_one_message() {
        let model = Script::new(vec![calls(&[("a", "search_logs", Ok(json!({"query": "status:error"}))), ("b", "search_logs", Ok(json!({"query": "boom"})))]), text("Two services fail.")]);
        let mut msgs = vec![Msg::User("What fails?".into())];
        let (outcome, events, usage) = run(&model, false, &mut msgs).await;
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(usage, Usage { input: 20, output: 4 });
        assert_eq!(
            msgs[2],
            Msg::Results(vec![
                ToolResult { id: "a".into(), content: "3 logs for status:error".into(), is_error: false },
                ToolResult { id: "b".into(), content: "Invalid query".into(), is_error: true },
            ])
        );
        assert_eq!(msgs.last(), Some(&Msg::Assistant { text: "Two services fail.".into(), calls: vec![], blocks: None }));
        assert!(events.contains(&Event::ToolEnd { id: "b".into(), ok: false, summary: "Invalid query".into() }));
        assert_eq!(events.last(), Some(&Event::Text { text: "Two services fail.".into() }));
        // The second turn saw the results.
        assert_eq!(model.seen.lock().unwrap()[1].len(), 3);
    }

    #[tokio::test]
    async fn a_write_waits_for_approval_and_a_declined_one_is_reported() {
        let create = || calls(&[("r", "search_logs", Ok(json!({"query": "x"}))), ("w", "create_monitor", Ok(json!({"name": "Checkout errors"})))]);
        // Paused: nothing ran yet, the write is announced.
        let model = Script::new(vec![create(), text("Done.")]);
        let mut msgs = vec![Msg::User("Create a monitor".into())];
        let (outcome, events, _) = run(&model, false, &mut msgs).await;
        let Outcome::Paused(pending) = outcome else { panic!("{outcome:?}") };
        assert_eq!(pending.len(), 2);
        assert_eq!(events, vec![Event::Approval { id: "w".into(), name: "create_monitor".into(), input: json!({"name": "Checkout errors"}) }]);
        assert_eq!(msgs.len(), 2);
        // Approved: both run, then the reply continues.
        let mut usage = Usage::default();
        let outcome = Loop { model: &model, tools: &Fake, system: "s", auto_approve: false }.resume(&mut msgs, &pending, &["w".into()], &mut usage, &mut |_| {}).await;
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(msgs[2], Msg::Results(vec![ToolResult { id: "r".into(), content: "3 logs for x".into(), is_error: false }, ToolResult { id: "w".into(), content: "Monitor 7 created".into(), is_error: false }]));
        // Declined: the model is told, the read still runs.
        let model = Script::new(vec![create(), text("Okay.")]);
        let mut msgs = vec![Msg::User("Create a monitor".into())];
        let Outcome::Paused(pending) = run(&model, false, &mut msgs).await.0 else { panic!() };
        Loop { model: &model, tools: &Fake, system: "s", auto_approve: false }.resume(&mut msgs, &pending, &[], &mut usage, &mut |_| {}).await;
        let Msg::Results(results) = &msgs[2] else { panic!() };
        assert!(!results[0].is_error && results[1].is_error && results[1].content.starts_with("The user declined"));
        // Auto-approve: no pause.
        let model = Script::new(vec![create(), text("Created.")]);
        let mut msgs = vec![Msg::User("Create a monitor".into())];
        assert_eq!(run(&model, true, &mut msgs).await.0, Outcome::Done);
    }

    #[tokio::test]
    async fn bad_inputs_unknown_tools_refusals_and_endless_loops_end_cleanly() {
        let model = Script::new(vec![calls(&[("a", "search_logs", Err("{\"query\": \"x".into())), ("b", "drop_tables", Ok(json!({})))]), text("Sorry.")]);
        let mut msgs = vec![Msg::User("q".into())];
        run(&model, false, &mut msgs).await;
        let Msg::Results(results) = &msgs[2] else { panic!() };
        assert!(results[0].is_error && results[0].content.starts_with("The input was not a valid JSON object"));
        assert_eq!(results[1].content, "Unknown tool drop_tables");

        let refused = Turn { stop: Stop::Refusal("policy".into()), ..text("") };
        assert_eq!(run(&Script::new(vec![refused]), false, &mut vec![Msg::User("q".into())]).await.0, Outcome::Failed("The model declined to answer: policy".into()));

        let cut = Turn { stop: Stop::MaxTokens, ..text("half an ans") };
        assert!(matches!(run(&Script::new(vec![cut]), false, &mut vec![Msg::User("q".into())]).await.0, Outcome::Failed(m) if m.contains("output limit")));

        let endless = (0..MAX_STEPS).map(|i| calls(&[(&format!("c{i}"), "search_logs", Ok(json!({"query": "x"})))])).collect();
        assert!(matches!(run(&Script::new(endless), false, &mut vec![Msg::User("q".into())]).await.0, Outcome::Failed(m) if m.contains("Stopped after")));
    }

    #[test]
    fn long_results_are_cut_on_a_character_boundary() {
        assert_eq!(truncate("ăîș", 2), "ăî\n… (truncated: 2 more characters)");
        assert_eq!(truncate("short", 10), "short");
    }
}
