//! The log as processors see it: the reserved fields (message, status, service, host, source,
//! date, trace id), tags and nested attributes, read from and written back to the indexer's row
//! (`oc.logs` shape, see `deploy/vector/normalize.vrl`).

use std::collections::HashMap;

use oc_query::Fields;
use serde_json::{Map, Value};

use super::dates::to_rfc3339;
use super::filter::Filter;

#[derive(Debug, Default, Clone)]
pub struct Log {
    pub message: String,
    pub status: String,
    pub service: String,
    pub host: String,
    pub source: String,
    pub trace_id: String,
    /// Set when a date remapper changed the log's date (epoch ms).
    pub timestamp_ms: Option<i64>,
    pub tags: HashMap<String, String>,
    /// Attributes as nested JSON (`{"http": {"status_code": 200}}`); keys may also hold dots, as
    /// OpenTelemetry's flat attributes do (`{"http.method": "GET"}`).
    pub attrs: Map<String, Value>,
    /// `attrs` flattened for search filters, rebuilt after a change.
    flat: Option<HashMap<String, String>>,
}

/// Equal content (the search cache aside).
impl PartialEq for Log {
    fn eq(&self, o: &Log) -> bool {
        (&self.message, &self.status, &self.service, &self.host, &self.source, &self.trace_id, self.timestamp_ms, &self.tags, &self.attrs)
            == (&o.message, &o.status, &o.service, &o.host, &o.source, &o.trace_id, o.timestamp_ms, &o.tags, &o.attrs)
    }
}

impl Log {
    /// A log from the indexer's row.
    pub fn from_row(row: &Value) -> Log {
        let s = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let tags = row.get("tags").and_then(Value::as_object).map(|o| o.iter().map(|(k, v)| (k.clone(), text(v))).collect()).unwrap_or_default();
        // The JSON attributes keep their types; rows without them have the flattened text ones.
        let attrs = row
            .get("attributes")
            .and_then(Value::as_str)
            .and_then(|a| serde_json::from_str::<Map<String, Value>>(a).ok())
            .or_else(|| row.get("attr").and_then(Value::as_object).cloned())
            .unwrap_or_default();
        Log { message: s("message"), status: s("status"), service: s("service"), host: s("host"), source: s("source"), trace_id: s("trace_id"), timestamp_ms: None, tags, attrs, flat: None }
    }

    /// Writes the processed log back into the row: columns, `tags`, `attr` (flattened, string
    /// values) and `attributes` (JSON); `timestamp` / `ts_ms` when the date changed.
    pub fn write_row(&self, row: &mut Value) {
        let Some(o) = row.as_object_mut() else { return };
        for (k, v) in [("message", &self.message), ("status", &self.status), ("service", &self.service), ("host", &self.host), ("source", &self.source), ("trace_id", &self.trace_id)] {
            o.insert(k.into(), Value::from(v.as_str()));
        }
        o.insert("tags".into(), Value::Object(self.tags.iter().map(|(k, v)| (k.clone(), Value::from(v.as_str()))).collect()));
        o.insert("attr".into(), Value::Object(flatten(&self.attrs).into_iter().map(|(k, v)| (k, Value::from(v))).collect()));
        o.insert("attributes".into(), Value::from(Value::Object(self.attrs.clone()).to_string()));
        if let Some(ms) = self.timestamp_ms {
            o.insert("timestamp".into(), Value::from(to_rfc3339(ms)));
            o.insert("ts_ms".into(), Value::from(ms));
        }
    }

    /// An attribute by path (`http.status_code`). `message` is the log's message; `service`,
    /// `status`, `host` and `source` fall back to the log's own when no attribute has the name.
    pub fn get(&self, path: &str) -> Option<Value> {
        if path == "message" {
            return Some(Value::from(self.message.as_str()));
        }
        if let Some(v) = lookup(&self.attrs, path) {
            return Some(v.clone());
        }
        let reserved = match path {
            "service" => &self.service,
            "status" => &self.status,
            "host" => &self.host,
            "source" => &self.source,
            _ => return None,
        };
        (!reserved.is_empty()).then(|| Value::from(reserved.as_str()))
    }

    /// Sets an attribute by path, creating nested objects (`message` sets the message).
    pub fn set(&mut self, path: &str, v: Value) {
        if path == "message" {
            self.message = text(&v);
            return;
        }
        self.flat = None;
        set_in(&mut self.attrs, path, v);
    }

    pub fn remove(&mut self, path: &str) -> Option<Value> {
        if path == "message" {
            return Some(Value::from(std::mem::take(&mut self.message)));
        }
        self.flat = None;
        remove_in(&mut self.attrs, path)
    }

    /// Whether the log matches a search filter in its current state.
    pub fn matches(&mut self, filter: &Filter) -> bool {
        let flat = self.flat.get_or_insert_with(|| flatten(&self.attrs));
        let f = Fields { service: &self.service, status: &self.status, host: &self.host, source: &self.source, message: &self.message, tags: &self.tags, attr: flat, index: None };
        filter.matches(&f)
    }
}

/// The value at `path`: a key holding the whole path, or a key holding a prefix whose object holds the rest.
fn lookup<'a>(m: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if let Some(v) = m.get(path) {
        return Some(v);
    }
    path.match_indices('.').find_map(|(i, _)| match m.get(&path[..i]) {
        Some(Value::Object(inner)) => lookup(inner, &path[i + 1..]),
        _ => None,
    })
}

/// The index of the first `.` in `path` whose prefix is an object key of `m`.
fn object_prefix(m: &Map<String, Value>, path: &str) -> Option<usize> {
    path.match_indices('.').map(|(i, _)| i).find(|&i| matches!(m.get(&path[..i]), Some(Value::Object(_))))
}

fn set_in(m: &mut Map<String, Value>, path: &str, v: Value) {
    if m.contains_key(path) {
        m.insert(path.to_string(), v);
        return;
    }
    if let Some(i) = object_prefix(m, path) {
        if let Some(Value::Object(inner)) = m.get_mut(&path[..i]) {
            set_in(inner, &path[i + 1..], v);
        }
        return;
    }
    match path.split_once('.') {
        None => {
            m.insert(path.to_string(), v);
        }
        Some((head, rest)) => {
            let entry = m.entry(head.to_string()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(inner) = entry {
                set_in(inner, rest, v);
            }
        }
    }
}

fn remove_in(m: &mut Map<String, Value>, path: &str) -> Option<Value> {
    if let Some(v) = m.remove(path) {
        return Some(v);
    }
    let i = object_prefix(m, path)?;
    match m.get_mut(&path[..i]) {
        Some(Value::Object(inner)) => remove_in(inner, &path[i + 1..]),
        _ => None,
    }
}

/// An attribute value as text: strings as they are, `null` as empty, others as JSON.
pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

/// Nested attributes flattened to `a.b.c` keys with text values (normalize.vrl's `attr`).
pub fn flatten(attrs: &Map<String, Value>) -> HashMap<String, String> {
    fn walk(prefix: &str, m: &Map<String, Value>, out: &mut HashMap<String, String>) {
        for (k, v) in m {
            let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            match v {
                Value::Object(inner) if !inner.is_empty() => walk(&key, inner, out),
                _ => {
                    out.insert(key, text(v));
                }
            }
        }
    }
    let mut out = HashMap::new();
    walk("", attrs, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> Value {
        json!({
            "org_id": "acme", "id": "1", "timestamp": "2026-10-25T13:00:00.000Z", "ts_ms": 1_792_933_200_000_i64,
            "status": "info", "service": "storefront-api", "host": "i-0a1b2c3d4e5f60001", "source": "nginx",
            "message": "GET /cart 200", "tags": {"env": "prod"},
            "attr": {"http.method": "GET", "user.id": "7"},
            "attributes": "{\"http.method\":\"GET\",\"user\":{\"id\":7}}",
            "trace_id": "", "span_id": "", "severity_number": 0, "scope": ""
        })
    }

    #[test]
    fn reads_the_row() {
        let log = Log::from_row(&row());
        assert_eq!((log.message.as_str(), log.service.as_str(), log.source.as_str()), ("GET /cart 200", "storefront-api", "nginx"));
        assert_eq!(log.tags["env"], "prod");
        assert_eq!(log.get("user.id"), Some(json!(7)));
        assert_eq!(log.get("http.method"), Some(json!("GET")));
        assert_eq!(log.get("message"), Some(json!("GET /cart 200")));
        assert_eq!(log.get("service"), Some(json!("storefront-api")));
        assert_eq!(log.get("nope"), None);
        // Rows without the JSON attributes fall back to the flattened ones.
        let mut r = row();
        r.as_object_mut().unwrap().remove("attributes");
        assert_eq!(Log::from_row(&r).get("user.id"), Some(json!("7")));
    }

    #[test]
    fn paths_create_nested_attributes() {
        let mut log = Log::from_row(&row());
        log.set("http.status_code", json!(200));
        log.set("user.name", json!("jane"));
        log.set("message", json!("rewritten"));
        assert_eq!(log.attrs["http.method"], json!("GET"));
        assert_eq!(log.attrs["http"], json!({"status_code": 200}));
        assert_eq!(log.attrs["user"], json!({"id": 7, "name": "jane"}));
        assert_eq!(log.message, "rewritten");
        assert_eq!(log.remove("user.id"), Some(json!(7)));
        assert_eq!(log.remove("http.method"), Some(json!("GET")));
        // A path through a non-object replaces it.
        log.set("user.name.first", json!("Jane"));
        assert_eq!(log.attrs["user"], json!({"name": {"first": "Jane"}}));
    }

    #[test]
    fn writes_the_row_back() {
        let mut r = row();
        let mut log = Log::from_row(&r);
        log.set("http.status_code", json!(200));
        log.set("duration", json!(1.5));
        log.set("ok", json!(true));
        log.status = "error".into();
        log.timestamp_ms = Some(1_792_933_200_123);
        log.tags.insert("db".into(), "orders".into());
        log.write_row(&mut r);
        assert_eq!(r["status"], "error");
        assert_eq!(r["timestamp"], "2026-10-25T13:00:00.123Z");
        assert_eq!(r["ts_ms"], 1_792_933_200_123_i64);
        assert_eq!(r["tags"], json!({"env": "prod", "db": "orders"}));
        assert_eq!(r["attr"], json!({"http.method": "GET", "user.id": "7", "http.status_code": "200", "duration": "1.5", "ok": "true"}));
        let attributes: Value = serde_json::from_str(r["attributes"].as_str().unwrap()).unwrap();
        assert_eq!(attributes["http"]["status_code"], 200);
        assert_eq!(r["org_id"], "acme", "other fields stay");
    }

    #[test]
    fn matches_search_filters_on_the_current_state() {
        let mut log = Log::from_row(&row());
        let f = Filter::parse("source:nginx @http.status_code:200");
        assert!(!log.matches(&f));
        log.set("http.status_code", json!(200));
        assert!(log.matches(&f));
    }

    #[test]
    fn flattens_like_the_normalizer() {
        let attrs = json!({"a": {"b": 1, "c": [1, 2], "d": null}, "e.f": "x"});
        let flat = flatten(attrs.as_object().unwrap());
        assert_eq!(flat.len(), 4);
        assert_eq!((flat["a.b"].as_str(), flat["a.c"].as_str(), flat["a.d"].as_str(), flat["e.f"].as_str()), ("1", "[1,2]", "", "x"));
    }
}
