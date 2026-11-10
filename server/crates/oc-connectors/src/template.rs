//! Templates: manifest strings with `{{ expr }}` parts (docs/connectors.md, "Templates"). A string that is exactly one
//! `{{ expr }}` renders to the expression's value with its type (a list, a number, an object); any other string
//! renders to text, `null` parts as "". JSON trees (request bodies, log attributes) are templates at every string leaf.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::expr::{self, Expr};

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Expr(Expr),
}

/// A compiled template string.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    pub fn parse(src: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut rest = src;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_string()));
            }
            let after = &rest[open + 2..];
            let close = after.find("}}").ok_or_else(|| format!("{{{{ without }}}} in {src:?}"))?;
            let e = expr::parse(&after[..close]).map_err(|e| format!("{e} in {{{{{}}}}}", &after[..close]))?;
            parts.push(Part::Expr(e));
            rest = &after[close + 2..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_string()));
        }
        Ok(Template { parts })
    }

    /// A template with no expression.
    pub fn is_static(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Text(_)))
    }

    pub fn roots(&self, out: &mut BTreeSet<String>) {
        for p in &self.parts {
            if let Part::Expr(e) = p {
                e.roots(out);
            }
        }
    }

    pub fn paths(&self, out: &mut Vec<(String, Option<String>)>) {
        for p in &self.parts {
            if let Part::Expr(e) = p {
                e.paths(out);
            }
        }
    }

    pub fn render(&self, scope: &Map<String, Value>) -> Value {
        match self.parts.as_slice() {
            [] => Value::String(String::new()),
            [Part::Expr(e)] => e.eval(scope),
            parts => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        Part::Text(t) => out.push_str(t),
                        Part::Expr(e) => out.push_str(&expr::text(&e.eval(scope))),
                    }
                }
                Value::String(out)
            }
        }
    }

    /// The rendered value as text ("" for null).
    pub fn text(&self, scope: &Map<String, Value>) -> String {
        expr::text(&self.render(scope))
    }
}

/// A compiled JSON tree whose string leaves are templates.
#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    Str(Template),
    List(Vec<Tree>),
    Obj(Vec<(String, Tree)>),
    Other(Value),
}

impl Tree {
    pub fn parse(v: &Value) -> Result<Tree, String> {
        Ok(match v {
            Value::String(s) => Tree::Str(Template::parse(s)?),
            Value::Array(a) => Tree::List(a.iter().map(Tree::parse).collect::<Result<_, _>>()?),
            Value::Object(o) => Tree::Obj(o.iter().map(|(k, v)| Ok((k.clone(), Tree::parse(v)?))).collect::<Result<_, String>>()?),
            other => Tree::Other(other.clone()),
        })
    }

    pub fn paths(&self, out: &mut Vec<(String, Option<String>)>) {
        match self {
            Tree::Str(t) => t.paths(out),
            Tree::List(items) => items.iter().for_each(|t| t.paths(out)),
            Tree::Obj(fields) => fields.iter().for_each(|(_, t)| t.paths(out)),
            Tree::Other(_) => {}
        }
    }

    pub fn roots(&self, out: &mut BTreeSet<String>) {
        match self {
            Tree::Str(t) => t.roots(out),
            Tree::List(items) => items.iter().for_each(|t| t.roots(out)),
            Tree::Obj(fields) => fields.iter().for_each(|(_, t)| t.roots(out)),
            Tree::Other(_) => {}
        }
    }

    /// Renders every leaf; with `prune`, `null` and "" leaves and the objects left empty are dropped (log attributes).
    pub fn render(&self, scope: &Map<String, Value>, prune: bool) -> Value {
        match self {
            Tree::Str(t) => t.render(scope),
            Tree::Other(v) => v.clone(),
            Tree::List(items) => Value::Array(items.iter().map(|t| t.render(scope, prune)).collect()),
            Tree::Obj(fields) => {
                let mut out = Map::new();
                for (k, t) in fields {
                    let v = t.render(scope, prune);
                    if prune && empty(&v) {
                        continue;
                    }
                    out.insert(k.clone(), v);
                }
                Value::Object(out)
            }
        }
    }
}

/// Dropped from pruned trees: `null`, "", and objects with nothing left.
pub fn empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> Map<String, Value> {
        json!({ "config": { "site": "eu", "ids": [1, 2] }, "record": { "id": 7, "who": "" } }).as_object().unwrap().clone()
    }

    #[test]
    fn a_single_expression_keeps_its_type_and_others_are_text() {
        assert_eq!(Template::parse("{{ config.ids }}").unwrap().render(&scope()), json!([1, 2]));
        assert_eq!(Template::parse("https://api.{{config.site}}.example.com/v1").unwrap().render(&scope()), json!("https://api.eu.example.com/v1"));
        assert_eq!(Template::parse("id={{ record.id }}{{ record.none }}").unwrap().render(&scope()), json!("id=7"));
        assert_eq!(Template::parse("plain").unwrap().render(&scope()), json!("plain"));
        assert!(Template::parse("plain").unwrap().is_static());
        assert!(Template::parse("{{ open").unwrap_err().contains("without"));
        assert!(Template::parse("{{ a | nope }}").unwrap_err().contains("unknown filter"));
    }

    #[test]
    fn trees_render_every_leaf_and_prune_empties() {
        let tree = Tree::parse(&json!({ "evt": { "id": "{{ record.id }}", "who": "{{ record.who }}" }, "usr": { "name": "{{ record.who }}" }, "n": 3, "list": ["{{ config.site }}"] })).unwrap();
        assert_eq!(tree.render(&scope(), true), json!({ "evt": { "id": 7 }, "n": 3, "list": ["eu"] }));
        assert_eq!(tree.render(&scope(), false)["usr"], json!({ "name": "" }));
        let mut roots = BTreeSet::new();
        tree.roots(&mut roots);
        assert_eq!(roots.into_iter().collect::<Vec<_>>(), ["config", "record"]);
    }
}
