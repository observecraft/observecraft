//! Standard attributes (Logs → Configuration → Standard Attributes): the attribute names an
//! organization agrees on (`http.status_code`, `network.client.ip`, …), each with a type and the
//! aliases its senders use instead (`status_code`, OpenTelemetry's `http.response.status_code`).
//! The indexer (`logproc`) copies an alias's value to the standard attribute after the org's
//! pipelines and before generated metrics and index routing, so searches, facets and metrics can
//! rely on one name.
//!
//! Storage: the default list ships with the server (`server/seeds/standard_attributes.json`) and
//! Postgres (`log_standard_attributes`) keeps only an org's changes: a row per attribute it added
//! or edited, or a `hidden` row for a default it removed. An org that changed nothing has no rows
//! and follows the defaults as they improve; "Reset to defaults" deletes its rows.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::pipelines::processors::{cast, Format};
use crate::pipelines::Log;

/// Attributes an organization can have (defaults included).
pub const MAX_ATTRIBUTES: usize = 500;
pub const MAX_ALIASES: usize = 20;
const MAX_PATH_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 500;
/// The log's own fields; remappers and pipelines handle them (`trace_id` with the trace id remapper).
const RESERVED: [&str; 9] = ["message", "status", "service", "host", "source", "date", "timestamp", "trace_id", "span_id"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StandardAttributeType {
    String,
    Integer,
    Double,
    Boolean,
}

impl StandardAttributeType {
    pub fn as_str(self) -> &'static str {
        match self {
            StandardAttributeType::String => "string",
            StandardAttributeType::Integer => "integer",
            StandardAttributeType::Double => "double",
            StandardAttributeType::Boolean => "boolean",
        }
    }

    pub fn parse(s: &str) -> Option<StandardAttributeType> {
        [StandardAttributeType::String, StandardAttributeType::Integer, StandardAttributeType::Double, StandardAttributeType::Boolean].into_iter().find(|t| t.as_str() == s)
    }

    fn format(self) -> Format {
        match self {
            StandardAttributeType::String => Format::Text,
            StandardAttributeType::Integer => Format::Integer,
            StandardAttributeType::Double => Format::Double,
            StandardAttributeType::Boolean => Format::Boolean,
        }
    }
}

fn yes() -> bool {
    true
}

/// A standard attribute as an org defines it.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StandardAttributeInput {
    /// The attribute's path (`http.status_code`), without `@`.
    pub path: String,
    #[serde(rename = "type")]
    pub kind: StandardAttributeType,
    #[serde(default)]
    pub description: String,
    /// Attribute paths whose value is copied to `path` when a log lacks it; the first one present wins.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Keep the alias attribute after copying its value (default); otherwise it is moved.
    #[serde(default = "yes")]
    pub preserve_source: bool,
}

/// A standard attribute of the org's list.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StandardAttribute {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: StandardAttributeType,
    pub description: String,
    pub aliases: Vec<String>,
    pub preserve_source: bool,
    /// Shipped with ObserveCraft (removing it hides it until a reset).
    pub is_default: bool,
    /// A default the org edited.
    pub modified: bool,
}

/// An org's change to the list: its definition of `path`, or `None` for a default it removed.
pub struct Override {
    pub path: String,
    pub definition: Option<StandardAttributeInput>,
}

/// A `log_standard_attributes` row.
#[derive(sqlx::FromRow)]
pub struct StoredRow {
    pub org_id: String,
    pub path: String,
    pub hidden: bool,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub description: String,
    pub aliases: Value,
    pub preserve_source: bool,
}

impl From<StoredRow> for Override {
    fn from(r: StoredRow) -> Override {
        let definition = (!r.hidden).then(|| StandardAttributeInput {
            path: r.path.clone(),
            kind: StandardAttributeType::parse(&r.kind).unwrap_or(StandardAttributeType::String),
            description: r.description,
            aliases: serde_json::from_value(r.aliases).unwrap_or_default(),
            preserve_source: r.preserve_source,
        });
        Override { path: r.path, definition }
    }
}

#[derive(Deserialize)]
struct Seed {
    attributes: Vec<StandardAttributeInput>,
}

/// The default list, by path.
pub fn defaults() -> &'static [StandardAttributeInput] {
    static DEFAULTS: OnceLock<Vec<StandardAttributeInput>> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let seed: Seed = serde_json::from_str(include_str!("../../../seeds/standard_attributes.json")).expect("seeds/standard_attributes.json is valid");
        let mut list = seed.attributes;
        list.sort_by(|a, b| a.path.cmp(&b.path));
        list
    })
}

/// The org's list: the defaults it kept (with its edits) and the attributes it added, by path.
pub fn merge(overrides: &[Override]) -> Vec<StandardAttribute> {
    let changed = |path: &str| overrides.iter().find(|o| o.path == path);
    let entry = |d: &StandardAttributeInput, is_default: bool, modified: bool| StandardAttribute {
        path: d.path.clone(),
        kind: d.kind,
        description: d.description.clone(),
        aliases: d.aliases.clone(),
        preserve_source: d.preserve_source,
        is_default,
        modified,
    };
    let mut list: Vec<StandardAttribute> = defaults()
        .iter()
        .filter_map(|d| match changed(&d.path) {
            None => Some(entry(d, true, false)),
            Some(Override { definition: Some(own), .. }) => Some(entry(own, true, own != d)),
            Some(Override { definition: None, .. }) => None,
        })
        .collect();
    let added = overrides.iter().filter(|o| !defaults().iter().any(|d| d.path == o.path));
    list.extend(added.filter_map(|o| o.definition.as_ref()).map(|d| entry(d, false, false)));
    list.sort_by(|a, b| a.path.cmp(&b.path));
    list
}

/// `http.status_code`: letters, digits, `_`, `-` and dots between them.
fn valid_path(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_PATH_CHARS && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) && !p.starts_with('.') && !p.ends_with('.') && !p.contains("..")
}

/// Whether `inner` is `outer` or below it (`logger` contains `logger.name`).
fn contains(outer: &str, inner: &str) -> bool {
    inner == outer || inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with('.'))
}

/// Trims and checks a definition against the org's other attributes (`others` excludes `input.path`).
pub fn validate(input: &mut StandardAttributeInput, others: &[StandardAttribute]) -> Result<(), String> {
    input.path = input.path.trim().trim_start_matches('@').to_string();
    input.description = input.description.trim().to_string();
    let path = input.path.as_str();
    if !valid_path(path) {
        return Err(format!("Attribute paths are up to {MAX_PATH_CHARS} letters, digits, '_', '-' and dots (e.g. http.status_code)"));
    }
    if RESERVED.contains(&path) {
        return Err(format!("'{path}' is a reserved field of the log, not an attribute"));
    }
    if let Some(o) = others.iter().find(|o| o.aliases.iter().any(|a| a == path)) {
        return Err(format!("'{path}' is an alias of {}", o.path));
    }
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    if input.aliases.len() > MAX_ALIASES {
        return Err(format!("At most {MAX_ALIASES} aliases"));
    }
    let mut seen = HashSet::new();
    for alias in &mut input.aliases {
        *alias = alias.trim().trim_start_matches('@').to_string();
        let a = alias.as_str();
        if !valid_path(a) {
            return Err(format!("'{a}' isn't a valid attribute path"));
        }
        if RESERVED.contains(&a) {
            return Err(format!("'{a}' is a reserved field of the log, not an attribute"));
        }
        if contains(a, &input.path) {
            return Err(format!("The alias '{a}' contains the attribute itself"));
        }
        if !seen.insert(a.to_string()) {
            return Err(format!("'{a}' is listed twice"));
        }
        if others.iter().any(|o| o.path == a) {
            return Err(format!("'{a}' is a standard attribute itself"));
        }
        if let Some(o) = others.iter().find(|o| o.aliases.iter().any(|x| x == a)) {
            return Err(format!("'{a}' is already an alias of {}", o.path));
        }
    }
    Ok(())
}

struct Rule {
    target: String,
    aliases: Vec<String>,
    format: Format,
    preserve_source: bool,
}

/// An org's list compiled for the indexer.
pub struct Remapper {
    rules: Vec<Rule>,
    /// Every alias, to skip logs that have none without parsing them.
    aliases: HashSet<String>,
}

impl Remapper {
    pub fn compile(attributes: &[StandardAttribute]) -> Remapper {
        let rules: Vec<Rule> = attributes
            .iter()
            .filter(|a| !a.aliases.is_empty())
            .map(|a| Rule { target: a.path.clone(), aliases: a.aliases.clone(), format: a.kind.format(), preserve_source: a.preserve_source })
            .collect();
        let aliases = rules.iter().flat_map(|r| r.aliases.iter().cloned()).collect();
        Remapper { rules, aliases }
    }

    /// Copies (or moves) alias values to their standard attributes; whether the log changed.
    ///
    /// Per attribute: a log that already has it keeps its value; otherwise the first alias, in
    /// order, holding a value of the attribute's type (a scalar that casts) gives it.
    pub fn apply(&self, log: &mut Log) -> bool {
        let scalar = |v: &Value| !v.is_object() && !v.is_array() && !v.is_null();
        let mut changed = false;
        for r in &self.rules {
            if log.get(&r.target).is_some() {
                continue;
            }
            let found = r.aliases.iter().find_map(|a| log.get(a).filter(scalar).and_then(|v| cast(v, &r.format)).map(|v| (a, v)));
            let Some((alias, v)) = found else { continue };
            if !r.preserve_source {
                log.remove(alias);
            }
            log.set(&r.target, v);
            changed = true;
        }
        changed
    }

    /// Whether the indexer row may hold an alias: its flattened `attr` names one, or it has no `attr`.
    pub fn may_apply(&self, row: &Value) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        match row.get("attr").and_then(Value::as_object) {
            Some(attr) => attr.keys().any(|k| self.aliases.contains(k)),
            None => true,
        }
    }
}

/// The defaults compiled, for orgs that changed nothing.
pub fn default_remapper() -> Arc<Remapper> {
    static REMAPPER: OnceLock<Arc<Remapper>> = OnceLock::new();
    REMAPPER.get_or_init(|| Arc::new(Remapper::compile(&merge(&[])))).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(path: &str, kind: StandardAttributeType, aliases: &[&str]) -> StandardAttributeInput {
        StandardAttributeInput { path: path.into(), kind, description: String::new(), aliases: aliases.iter().map(|a| a.to_string()).collect(), preserve_source: true }
    }

    fn attribute(path: &str, kind: StandardAttributeType, aliases: &[&str], preserve_source: bool) -> StandardAttribute {
        StandardAttribute { path: path.into(), kind, description: String::new(), aliases: aliases.iter().map(|a| a.to_string()).collect(), preserve_source, is_default: false, modified: false }
    }

    fn log(attrs: Value) -> Log {
        Log::from_row(&json!({"message": "m", "attributes": attrs.to_string()}))
    }

    use StandardAttributeType::{Boolean, Double, Integer, String as Text};

    #[test]
    fn the_defaults_are_valid_and_consistent() {
        let list = merge(&[]);
        assert_eq!(list.len(), defaults().len());
        assert!(list.len() >= 30);
        for (i, a) in list.iter().enumerate() {
            assert!(a.is_default && !a.modified);
            assert!(!a.description.is_empty(), "{} has a description", a.path);
            let others: Vec<_> = list.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, a)| a.clone()).collect();
            let mut def = StandardAttributeInput { path: a.path.clone(), kind: a.kind, description: a.description.clone(), aliases: a.aliases.clone(), preserve_source: a.preserve_source };
            assert_eq!(validate(&mut def, &others), Ok(()), "{}", a.path);
        }
        let paths: Vec<_> = list.iter().map(|a| a.path.as_str()).collect();
        for p in ["http.status_code", "http.url", "http.method", "network.client.ip", "duration", "error.kind", "error.message", "error.stack", "usr.id", "db.statement", "logger.name"] {
            assert!(paths.contains(&p), "{p} is a default");
        }
        let status = list.iter().find(|a| a.path == "http.status_code").unwrap();
        assert_eq!(status.kind, Integer);
        assert!(["status_code", "statusCode", "http.response.status_code"].iter().all(|a| status.aliases.iter().any(|x| x == a)));
    }

    #[test]
    fn org_changes_merge_onto_the_defaults() {
        let mut edited = input("http.status_code", Integer, &["code"]);
        edited.description = "Response code".into();
        let overrides = [
            Override { path: "http.status_code".into(), definition: Some(edited) },
            Override { path: "usr.email".into(), definition: None },
            Override { path: "checkout.cart_id".into(), definition: Some(input("checkout.cart_id", Text, &["cartId"])) },
        ];
        let list = merge(&overrides);
        assert_eq!(list.len(), defaults().len(), "one hidden, one added");
        assert!(list.windows(2).all(|w| w[0].path < w[1].path), "sorted by path");
        assert!(!list.iter().any(|a| a.path == "usr.email"));
        let status = list.iter().find(|a| a.path == "http.status_code").unwrap();
        assert_eq!((status.aliases.clone(), status.description.as_str(), status.is_default, status.modified), (vec!["code".to_string()], "Response code", true, true));
        let custom = list.iter().find(|a| a.path == "checkout.cart_id").unwrap();
        assert_eq!((custom.is_default, custom.modified), (false, false));
        // A hidden row for a path that isn't a default changes nothing.
        assert_eq!(merge(&[Override { path: "nope".into(), definition: None }]).len(), defaults().len());
    }

    #[test]
    fn validation() {
        let others = vec![attribute("http.status_code", Integer, &["status_code"], true)];
        let mut ok = input(" @checkout.cart_id ", Text, &[" cartId ", "cart.id"]);
        ok.description = "  Cart  ".into();
        assert_eq!(validate(&mut ok, &others), Ok(()));
        assert_eq!((ok.path.as_str(), ok.description.as_str(), ok.aliases.clone()), ("checkout.cart_id", "Cart", vec!["cartId".to_string(), "cart.id".to_string()]));
        let bad = |path: &str, aliases: &[&str]| validate(&mut input(path, Text, aliases), &others);
        for p in ["", "@", "a b", ".a", "a.", "a..b", "message", "status", "trace_id", "@service"] {
            assert!(bad(p, &[]).is_err(), "path {p:?}");
        }
        assert!(validate(&mut input(&"a".repeat(MAX_PATH_CHARS + 1), Text, &[]), &others).is_err());
        assert!(bad("x", &["x"]).is_err(), "an alias isn't the attribute itself");
        assert!(bad("x", &["y", "y"]).is_err(), "aliases are unique");
        assert!(bad("x", &["a b"]).is_err());
        assert!(bad("x", &["host"]).is_err(), "reserved fields aren't aliases");
        assert!(bad("logger.name", &["logger"]).is_err(), "an alias can't contain the attribute");
        assert!(bad("x", &["status_code"]).is_err(), "an alias maps onto one attribute");
        assert!(bad("x", &["http.status_code"]).is_err(), "a standard attribute isn't an alias");
        assert!(bad("status_code", &[]).is_err(), "an alias isn't a standard attribute");
        let many: Vec<String> = (0..=MAX_ALIASES).map(|i| format!("a{i}")).collect();
        assert!(bad("x", &many.iter().map(String::as_str).collect::<Vec<_>>()).is_err());
        let mut long = input("x", Text, &[]);
        long.description = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(validate(&mut long, &others).is_err());
    }

    #[test]
    fn aliases_are_copied_to_the_standard_attribute_with_its_type() {
        let r = Remapper::compile(&[attribute("http.status_code", Integer, &["status_code", "statusCode", "http.response.status_code"], true)]);
        let mut l = log(json!({"status_code": "503"}));
        assert!(r.apply(&mut l));
        assert_eq!((l.get("http.status_code"), l.get("status_code")), (Some(json!(503)), Some(json!("503"))));
        // Nested and dotted (OpenTelemetry) paths.
        let mut l = log(json!({"http": {"response": {"status_code": 404}}}));
        assert!(r.apply(&mut l));
        assert_eq!(l.attrs["http"]["status_code"], json!(404));
        let mut l = log(json!({"http.response.status_code": 200}));
        assert!(r.apply(&mut l));
        assert_eq!(l.get("http.status_code"), Some(json!(200)));
    }

    #[test]
    fn first_alias_wins_and_the_standard_value_is_kept() {
        let r = Remapper::compile(&[attribute("network.client.ip", Text, &["client.address", "clientip"], true)]);
        let mut l = log(json!({"clientip": "10.0.0.2", "client.address": "10.0.0.1"}));
        assert!(r.apply(&mut l));
        assert_eq!(l.get("network.client.ip"), Some(json!("10.0.0.1")));
        let mut l = log(json!({"network": {"client": {"ip": "10.0.0.9"}}, "clientip": "10.0.0.2"}));
        assert!(!r.apply(&mut l), "an existing standard value isn't overridden");
        assert_eq!(l.get("network.client.ip"), Some(json!("10.0.0.9")));
        let mut l = log(json!({"other": 1}));
        assert!(!r.apply(&mut l));
    }

    #[test]
    fn values_that_dont_cast_or_arent_scalars_are_skipped() {
        let r = Remapper::compile(&[attribute("http.status_code", Integer, &["status_code", "code"], true), attribute("cache.hit", Boolean, &["hit"], true), attribute("ratio", Double, &["r"], true)]);
        let mut l = log(json!({"status_code": "n/a", "code": "201", "hit": "TRUE", "r": "0.25"}));
        assert!(r.apply(&mut l));
        assert_eq!((l.get("http.status_code"), l.get("cache.hit"), l.get("ratio")), (Some(json!(201)), Some(json!(true)), Some(json!(0.25))));
        let s = Remapper::compile(&[attribute("http.url", Text, &["url"], true)]);
        let mut l = log(json!({"url": {"full": "https://shop.example.com/cart"}}));
        assert!(!s.apply(&mut l), "an object isn't a value");
        let mut l = log(json!({"url": 42}));
        assert!(s.apply(&mut l));
        assert_eq!(l.get("http.url"), Some(json!("42")));
    }

    #[test]
    fn aliases_move_unless_preserved() {
        let r = Remapper::compile(&[attribute("usr.id", Text, &["user_id"], false)]);
        let mut l = log(json!({"user_id": 7}));
        assert!(r.apply(&mut l));
        assert_eq!((l.get("usr.id"), l.get("user_id")), (Some(json!("7")), None));
    }

    #[test]
    fn rows_without_aliases_are_skipped_before_parsing() {
        let r = Remapper::compile(&[attribute("usr.id", Text, &["user.id"], true)]);
        assert!(r.may_apply(&json!({"attr": {"user.id": "7"}})));
        assert!(!r.may_apply(&json!({"attr": {"usr.id": "7"}})));
        assert!(r.may_apply(&json!({"message": "no attr column"})));
        assert!(!default_remapper().may_apply(&json!({"attr": {"team": "checkout"}})));
    }
}
