//! Log pipelines (Logs → Configuration → Pipelines): ordered per organization, each with a search
//! filter and ordered processors (grok parsing, remappers, categories, …) in Datadog's format. The
//! indexer runs them on every log before index routing (`logproc`), so index filters, exclusion
//! filters and stored rows see the processed log.

pub mod dates;
pub mod filter;
pub mod grok;
pub mod library;
pub mod log;
pub mod processors;

use serde_json::Value;

pub use filter::Filter;
pub use log::Log;
use processors::Step;

/// Nested pipelines go one level deep, as in Datadog.
pub const MAX_NESTING: usize = 1;

pub struct Pipeline {
    pub name: String,
    filter: Filter,
    steps: Vec<Step>,
}

impl Pipeline {
    /// Compiles a pipeline; processors that don't compile are left out and reported (named).
    pub fn compile(name: &str, filter: &str, processors: &[Value]) -> (Pipeline, Vec<String>) {
        Pipeline::compile_at(name, filter, processors, 0)
    }

    pub(crate) fn compile_at(name: &str, filter: &str, processors: &[Value], depth: usize) -> (Pipeline, Vec<String>) {
        let mut steps = Vec::new();
        let mut errors = Vec::new();
        for (i, def) in processors.iter().enumerate() {
            match processors::compile(def, depth) {
                Ok(Some(step)) => steps.push(step),
                Ok(None) => {}
                Err(e) => {
                    let label = def.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).or_else(|| def.get("type").and_then(Value::as_str)).unwrap_or("processor");
                    errors.push(format!("Processor {} ({label}): {e}", i + 1));
                }
            }
        }
        (Pipeline { name: name.to_string(), filter: Filter::parse(filter), steps }, errors)
    }

    /// Runs the processors when the log matches the filter; whether it did.
    pub fn apply(&self, log: &mut Log, now_ms: i64) -> bool {
        if !log.matches(&self.filter) {
            return false;
        }
        for step in &self.steps {
            processors::apply(step, log, now_ms);
        }
        true
    }
}

/// Runs an org's pipelines, in order, on one indexer row; whether any matched (and the row was rewritten).
pub fn process_row(pipelines: &[Pipeline], row: &mut Value, now_ms: i64) -> bool {
    if pipelines.is_empty() {
        return false;
    }
    let mut log = Log::from_row(row);
    let mut matched = false;
    for p in pipelines {
        matched |= p.apply(&mut log, now_ms);
    }
    if matched {
        log.write_row(row);
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pipeline(name: &str, filter: &str, processors: Value) -> Pipeline {
        let (p, errors) = Pipeline::compile(name, filter, processors.as_array().unwrap());
        assert!(errors.is_empty());
        p
    }

    #[test]
    fn pipelines_run_in_order_on_the_logs_they_match() {
        let pipelines = [
            pipeline("nginx", "source:nginx", json!([{"type": "grok-parser", "grok": {"matchRules": "r %{word:http.method} %{notSpace:http.url} %{integer:http.status_code}"}}])),
            // Sees what the first pipeline extracted.
            pipeline(
                "errors",
                "@http.status_code:[500 TO 599]",
                json!([{"type": "string-builder-processor", "template": "error", "target": "level"}, {"type": "status-remapper", "sources": ["level"]}]),
            ),
            pipeline("billing", "service:billing", json!([{"type": "string-builder-processor", "template": "x", "target": "never"}])),
        ];
        let mut row = json!({"org_id": "acme", "id": "1", "status": "info", "service": "api-gateway", "source": "nginx", "message": "GET /cart 503", "tags": {}, "attr": {}, "attributes": "{}"});
        assert!(process_row(&pipelines, &mut row, 0));
        assert_eq!(row["status"], "error");
        assert_eq!(row["attr"]["http.status_code"], "503");
        assert!(row["attr"].get("never").is_none());
        // A log no pipeline matches is left as it is.
        let mut other = json!({"org_id": "acme", "id": "2", "status": "info", "service": "billing-worker", "source": "go", "message": "m", "attr": {"a": "1"}});
        let before = other.clone();
        assert!(!process_row(&pipelines, &mut other, 0));
        assert_eq!(other, before);
    }
}
