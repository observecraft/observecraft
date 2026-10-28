//! Metric monitor queries with arithmetic between metric queries, as in integrations-core's recommended
//! monitors: `sum:a{*} by {x} - sum:b{*} by {x}`, `default_zero(sum:a{s:ok} by {c} * 100 / sum:a{*} by {c})`.

use std::collections::{BTreeMap, BTreeSet};

use oc_query::parse_metric_query;

use crate::routes::metrics::Group;

type Tags = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    /// The n-th metric query of the formula.
    Query(usize),
    Num(f64),
    Bin(Op, Box<Expr>, Box<Expr>),
    /// Missing values (no series, gaps, division by zero) count as 0.
    DefaultZero(Box<Expr>),
}

#[derive(Debug)]
pub(crate) struct Formula {
    pub expr: Expr,
    /// The metric queries, each valid for `parse_metric_query`.
    pub queries: Vec<String>,
    /// Where each query is in the trimmed text (byte offsets).
    pub spans: Vec<(usize, usize)>,
}

/// Parses a monitor's metric query: one metric query, or arithmetic between metric queries and numbers.
pub(crate) fn parse_formula(text: &str) -> Result<Formula, String> {
    let mut p = Parser { s: text.trim(), pos: 0, queries: Vec::new(), spans: Vec::new() };
    let expr = p.expr()?;
    p.skip_ws();
    if p.pos < p.s.len() {
        return Err(format!("unexpected '{}' at {}", &p.s[p.pos..], p.pos + 1));
    }
    Ok(Formula { expr, queries: p.queries, spans: p.spans })
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    queries: Vec<String>,
    spans: Vec<(usize, usize)>,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.s[self.pos..]
    }

    fn skip_ws(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.s.len() - trimmed.len();
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.rest().starts_with(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        loop {
            let op = if self.eat('+') {
                Op::Add
            } else if self.eat('-') {
                Op::Sub
            } else {
                return Ok(left);
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.term()?));
        }
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.factor()?;
        loop {
            let op = if self.eat('*') {
                Op::Mul
            } else if self.eat('/') {
                Op::Div
            } else {
                return Ok(left);
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.factor()?));
        }
    }

    fn factor(&mut self) -> Result<Expr, String> {
        self.skip_ws();
        if self.eat('(') {
            let e = self.expr()?;
            return if self.eat(')') { Ok(e) } else { Err("missing ')'".into()) };
        }
        if let Some(after) = self.rest().strip_prefix("default_zero(") {
            self.pos = self.s.len() - after.len();
            let e = self.expr()?;
            return if self.eat(')') { Ok(Expr::DefaultZero(Box::new(e))) } else { Err("missing ')' after default_zero(".into()) };
        }
        let number: String = self.rest().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if !number.is_empty() {
            self.pos += number.len();
            return number.parse().map(Expr::Num).map_err(|_| format!("invalid number '{number}'"));
        }
        if self.rest().is_empty() {
            return Err("expected a metric query or a number at the end".into());
        }
        self.metric_query()
    }

    /// `<agg>:<metric>{filters}` and an optional ` by {tags}`.
    fn metric_query(&mut self) -> Result<Expr, String> {
        let start = self.pos;
        let close = |from: usize, s: &str| s[from..].find('}').map(|i| from + i + 1);
        let open = self.rest().find('{').map(|i| self.pos + i).ok_or("expected <agg>:<metric>{…}")?;
        let mut end = close(open, self.s).ok_or("unclosed '{'")?;
        let tail = &self.s[end..];
        let by = tail.trim_start();
        if let Some(after_by) = by.strip_prefix("by").filter(|a| a.trim_start().starts_with('{')) {
            let brace = self.s.len() - after_by.trim_start().len();
            end = close(brace, self.s).ok_or("unclosed 'by {'")?;
        }
        let text = self.s[start..end].trim();
        parse_metric_query(text)?;
        let raw = &self.s[start..end];
        self.spans.push((start + raw.len() - raw.trim_start().len(), end - (raw.len() - raw.trim_end().len())));
        self.pos = end;
        self.queries.push(text.to_string());
        Ok(Expr::Query(self.queries.len() - 1))
    }
}

/// The formula's series: queries' groups joined by their tags (a query without `by` applies to every
/// group) and timestamps, groups and points without a value left out.
pub(crate) fn combine(expr: &Expr, results: &[Vec<Group>]) -> Vec<Group> {
    // Lookup per query: tags → (timestamp → value).
    let series: Vec<BTreeMap<&Tags, BTreeMap<i64, f64>>> = results.iter().map(|groups| groups.iter().map(|g| (&g.tags, g.points.iter().copied().collect())).collect()).collect();
    let keys: BTreeSet<&Tags> = results.iter().flatten().map(|g| &g.tags).filter(|t| !t.is_empty()).collect();
    let keys: Vec<&Tags> = if keys.is_empty() { results.iter().flatten().map(|g| &g.tags).take(1).collect() } else { keys.into_iter().collect() };
    let times: BTreeSet<i64> = results.iter().flatten().flat_map(|g| g.points.iter().map(|p| p.0)).collect();
    let empty = BTreeMap::new();
    let mut out = Vec::new();
    for key in keys {
        let points: Vec<(i64, f64)> = times
            .iter()
            .filter_map(|&t| {
                let value = |i: usize| -> Option<f64> {
                    let s = &series[i];
                    s.get(key).or_else(|| s.get(&empty)).and_then(|points| points.get(&t)).copied()
                };
                eval(expr, &value).filter(|v| v.is_finite()).map(|v| (t, v))
            })
            .collect();
        if !points.is_empty() {
            out.push(Group { tags: key.clone(), points });
        }
    }
    out
}

fn eval(expr: &Expr, value: &dyn Fn(usize) -> Option<f64>) -> Option<f64> {
    match expr {
        Expr::Query(i) => value(*i),
        Expr::Num(n) => Some(*n),
        Expr::DefaultZero(e) => Some(eval(e, &|i| Some(value(i).unwrap_or(0.0))).filter(|v| v.is_finite()).unwrap_or(0.0)),
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a, value)?, eval(b, value)?);
            match op {
                Op::Add => Some(a + b),
                Op::Sub => Some(a - b),
                Op::Mul => Some(a * b),
                Op::Div => (b != 0.0).then(|| a / b),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn group(pairs: &[(&str, &str)], points: &[(i64, f64)]) -> Group {
        Group { tags: tags(pairs), points: points.to_vec() }
    }

    #[test]
    fn a_plain_metric_query_is_one_query() {
        let f = parse_formula("avg:system.load.1{env:prod} by {host}").unwrap();
        assert_eq!(f.queries, vec!["avg:system.load.1{env:prod} by {host}"]);
        assert_eq!(f.expr, Expr::Query(0));
    }

    #[test]
    fn arithmetic_between_queries_keeps_precedence_and_default_zero() {
        let f = parse_formula("sum:a.desired{*} by {kube_namespace,kube_deployment} - sum:a.available{*} by {kube_namespace,kube_deployment}").unwrap();
        assert_eq!(f.queries, vec!["sum:a.desired{*} by {kube_namespace,kube_deployment}", "sum:a.available{*} by {kube_namespace,kube_deployment}"]);
        assert_eq!(f.expr, Expr::Bin(Op::Sub, Box::new(Expr::Query(0)), Box::new(Expr::Query(1))));

        let f = parse_formula("default_zero(sum:node.status{status:schedulable} by {kube_cluster_name} * 100 / sum:node.status{*} by {kube_cluster_name})").unwrap();
        assert_eq!(f.queries.len(), 2);
        let ratio = Expr::Bin(Op::Div, Box::new(Expr::Bin(Op::Mul, Box::new(Expr::Query(0)), Box::new(Expr::Num(100.0)))), Box::new(Expr::Query(1)));
        assert_eq!(f.expr, Expr::DefaultZero(Box::new(ratio)));

        assert_eq!(
            parse_formula("(sum:a{*} + sum:b{*}) / 2").unwrap().expr,
            Expr::Bin(Op::Div, Box::new(Expr::Bin(Op::Add, Box::new(Expr::Query(0)), Box::new(Expr::Query(1)))), Box::new(Expr::Num(2.0)))
        );
    }

    #[test]
    fn invalid_formulas_say_why() {
        assert!(parse_formula("sum:a{*} -").is_err());
        assert!(parse_formula("sum:a{*} by {x").is_err());
        assert!(parse_formula("median:a{*}").is_err());
        assert!(parse_formula("(sum:a{*}").is_err());
    }

    #[test]
    fn groups_join_by_their_tags_and_timestamps() {
        let f = parse_formula("sum:a{*} by {d} - sum:b{*} by {d}").unwrap();
        let desired = vec![group(&[("d", "web")], &[(0, 3.0), (60, 3.0)]), group(&[("d", "api")], &[(0, 2.0)])];
        let available = vec![group(&[("d", "web")], &[(0, 1.0), (60, 3.0)])];
        let out = combine(&f.expr, &[desired, available]);
        // api has no available series: no value without default_zero.
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].tags.clone(), out[0].points.clone()), (tags(&[("d", "web")]), vec![(0, 2.0), (60, 0.0)]));
    }

    #[test]
    fn default_zero_fills_missing_values_and_a_tagless_query_applies_to_every_group() {
        let f = parse_formula("default_zero(sum:a{s:ok} by {c} * 100 / sum:a{*} by {c})").unwrap();
        let ok = vec![group(&[("c", "one")], &[(0, 3.0)])];
        let all = vec![group(&[("c", "one")], &[(0, 4.0)]), group(&[("c", "two")], &[(0, 5.0)])];
        let out = combine(&f.expr, &[ok, all]);
        let value = |c: &str| out.iter().find(|g| g.tags["c"] == c).map(|g| g.points.clone());
        assert_eq!(value("one"), Some(vec![(0, 75.0)]));
        assert_eq!(value("two"), Some(vec![(0, 0.0)]));

        let f = parse_formula("sum:a{*} by {c} / sum:b{*}").unwrap();
        let out = combine(&f.expr, &[vec![group(&[("c", "one")], &[(0, 6.0)])], vec![group(&[], &[(0, 2.0)])]]);
        assert_eq!(out[0].points, vec![(0, 3.0)]);
        // Division by zero has no value.
        let out = combine(&f.expr, &[vec![group(&[("c", "one")], &[(0, 6.0)])], vec![group(&[], &[(0, 0.0)])]]);
        assert!(out.is_empty());
    }
}
