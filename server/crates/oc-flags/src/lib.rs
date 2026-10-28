//! Feature flags (Software Delivery → Feature Flags): the flag model shared by the management API
//! (`oc-api`, which stores it in Postgres) and the intake (`oc-intake`, which evaluates it for
//! OpenFeature SDKs over OFREP).
//!
//! A flag has typed variants. Each environment has its own [`FlagEnvConfig`]: on or off, ordered
//! targeting rules (every condition of a rule must hold; the first matching rule serves), what
//! the rest gets (a variant or a percentage split) and an optional progressive rollout of one
//! variant over the traffic that no rule matched.
//!
//! Percentages are decided by [`bucket`]: a hash of the flag key and the subject's
//! `targetingKey`, so a subject keeps its variant between evaluations and SDKs, and a subject
//! inside a 10% rollout stays inside it at 20%.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Buckets per 100%: splits and rollouts are precise to 0.01%.
pub const BUCKETS: u32 = 10_000;
pub const MAX_VARIANTS: usize = 20;
pub const MAX_RULES: usize = 50;
pub const MAX_CONDITIONS: usize = 20;
pub const MAX_VALUES: usize = 500;
pub const MAX_ROLLOUT_STEPS: usize = 20;
const MAX_KEY_CHARS: usize = 100;

/// The type of a flag's values.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum FlagType {
    Boolean,
    String,
    Integer,
    Number,
    Json,
}

impl FlagType {
    pub fn as_str(self) -> &'static str {
        match self {
            FlagType::Boolean => "boolean",
            FlagType::String => "string",
            FlagType::Integer => "integer",
            FlagType::Number => "number",
            FlagType::Json => "json",
        }
    }

    pub fn parse(s: &str) -> Option<FlagType> {
        [FlagType::Boolean, FlagType::String, FlagType::Integer, FlagType::Number, FlagType::Json].into_iter().find(|t| t.as_str() == s)
    }

    /// Whether `v` is a value of this type.
    pub fn accepts(self, v: &Value) -> bool {
        match self {
            FlagType::Boolean => v.is_boolean(),
            FlagType::String => v.is_string(),
            FlagType::Integer => v.is_i64() || v.is_u64(),
            FlagType::Number => v.is_number(),
            FlagType::Json => v.is_object() || v.is_array(),
        }
    }
}

/// One value a flag can serve; `key` is the variant OpenFeature reports.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagVariant {
    pub key: String,
    pub value: Value,
}

/// How a condition compares the context attribute with its values.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FlagOperator {
    /// Equal to one of the values.
    OneOf,
    NotOneOf,
    /// Contains one of the values.
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    /// Numeric comparisons with the first value.
    Gt,
    Gte,
    Lt,
    Lte,
    /// The attribute is present (values ignored).
    Exists,
    NotExists,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagCondition {
    /// A context attribute: `targetingKey`, `country`, or a dotted path into an object (`user.plan`).
    pub attribute: String,
    pub operator: FlagOperator,
    #[serde(default)]
    pub values: Vec<String>,
}

/// One part of a percentage split.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagSplit {
    pub variant: String,
    /// Percent of subjects (0–100, two decimals).
    pub weight: f64,
}

/// What a rule (or the default) serves: one variant, or a split between variants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagServe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Weights add up to 100.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split: Vec<FlagSplit>,
}

impl FlagServe {
    pub fn variant(key: &str) -> FlagServe {
        FlagServe { variant: Some(key.to_string()), split: Vec::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagRule {
    #[serde(default)]
    pub name: String,
    /// All must hold; none matches every subject.
    #[serde(default)]
    pub conditions: Vec<FlagCondition>,
    pub serve: FlagServe,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagRolloutStep {
    /// Percent of the remaining traffic that gets the rollout's variant during this step.
    pub percent: f64,
    /// How long the step lasts before the next one starts (the last step lasts until changed).
    pub duration_minutes: u32,
}

/// A progressive rollout: the traffic no rule matched moves to `variant` step by step.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagRollout {
    pub variant: String,
    pub steps: Vec<FlagRolloutStep>,
    /// Epoch milliseconds the first step began (set by the server when the rollout starts).
    #[serde(default)]
    pub started_at: i64,
}

/// A flag's configuration in one environment.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FlagEnvConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<FlagRule>,
    /// Served when no rule matches (and the rollout doesn't take the subject).
    #[serde(default)]
    pub default_serve: FlagServe,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<FlagRollout>,
}

/// Why a value was served, as OpenFeature names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The default variant, served to everyone.
    Static,
    /// A rule matched and served one variant.
    TargetingMatch,
    /// A percentage split or a rollout decided.
    Split,
    /// The flag is off in this environment: the application's code default applies.
    Disabled,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Static => "STATIC",
            Reason::TargetingMatch => "TARGETING_MATCH",
            Reason::Split => "SPLIT",
            Reason::Disabled => "DISABLED",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    /// Absent when disabled.
    pub variant: Option<String>,
    pub value: Option<Value>,
    pub reason: Reason,
}

/// Why a flag couldn't be evaluated, as OpenFeature error codes.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// A split needs the subject's `targetingKey`.
    TargetingKeyMissing,
    InvalidContext(String),
    /// The stored configuration names a variant that doesn't exist.
    General(String),
}

impl EvalError {
    pub fn code(&self) -> &'static str {
        match self {
            EvalError::TargetingKeyMissing => "TARGETING_KEY_MISSING",
            EvalError::InvalidContext(_) => "INVALID_CONTEXT",
            EvalError::General(_) => "GENERAL",
        }
    }

    pub fn details(&self) -> String {
        match self {
            EvalError::TargetingKeyMissing => "a percentage split needs the context's targetingKey".into(),
            EvalError::InvalidContext(d) | EvalError::General(d) => d.clone(),
        }
    }
}

// ================================================================ bucketing

/// The subject's bucket in `[0, BUCKETS)` for `salt` (the flag key): the first 8 bytes of
/// SHA-256(`salt` + "." + `targeting_key`), big-endian, modulo [`BUCKETS`].
pub fn bucket(salt: &str, targeting_key: &str) -> u32 {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b".");
    h.update(targeting_key.as_bytes());
    let digest = h.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(first) % u64::from(BUCKETS)) as u32
}

/// Percent → buckets (rounded to 0.01%).
fn buckets_of(percent: f64) -> u32 {
    (percent.clamp(0.0, 100.0) * f64::from(BUCKETS) / 100.0).round() as u32
}

/// The variant of `split` for bucket `b`; the last part takes what rounding leaves.
pub fn pick(split: &[FlagSplit], b: u32) -> Option<&str> {
    let mut upto = 0.0;
    for s in split {
        upto += s.weight;
        if b < buckets_of(upto) {
            return Some(&s.variant);
        }
    }
    split.last().map(|s| s.variant.as_str())
}

/// The salt of a flag's rollout, so the rollout's subjects are independent of its splits.
fn rollout_salt(flag_key: &str) -> String {
    format!("{flag_key}/rollout")
}

// ================================================================ rollout

/// The rollout's percent at `now_ms`: each step's percent while it lasts, the last one after.
/// 0 before it starts.
pub fn rollout_percent(r: &FlagRollout, now_ms: i64) -> f64 {
    if r.started_at <= 0 || now_ms < r.started_at {
        return 0.0;
    }
    let elapsed = now_ms - r.started_at;
    let mut end = 0i64;
    for s in &r.steps {
        end = end.saturating_add(i64::from(s.duration_minutes) * 60_000);
        if elapsed < end {
            return s.percent;
        }
    }
    r.steps.last().map_or(0.0, |s| s.percent)
}

// ================================================================ environments

/// Whether an environment's query (`production`, `staging-*`, `*-eu`) matches the `env` an SDK
/// sends: case-insensitive, `*` matches any run of characters.
pub fn env_matches(query: &str, env: &str) -> bool {
    let (query, env) = (query.trim().to_lowercase(), env.trim().to_lowercase());
    if query.is_empty() {
        return false;
    }
    let parts: Vec<&str> = query.split('*').collect();
    if parts.len() == 1 {
        return query == env;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !env.starts_with(first) || env.len() < first.len() + last.len() || !env.ends_with(last) {
        return false;
    }
    let mut rest = &env[first.len()..env.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

// ================================================================ evaluation

/// The context attribute `name`, looked up as a key first and then as a dotted path.
fn attribute<'a>(ctx: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    if let Some(v) = ctx.get(name) {
        return Some(v);
    }
    let mut parts = name.split('.');
    let mut v = ctx.get(parts.next()?)?;
    for p in parts {
        v = v.as_object()?.get(p)?;
    }
    Some(v)
}

/// Scalars as the text conditions compare (`true`, `42`, `CA`); objects and null are not values.
fn scalars(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Bool(b) => vec![b.to_string()],
        Value::Number(n) => vec![n.to_string()],
        Value::Array(items) => items.iter().flat_map(scalars).collect(),
        Value::Null | Value::Object(_) => Vec::new(),
    }
}

/// Whether one condition holds for the context.
pub fn condition_holds(c: &FlagCondition, ctx: &Map<String, Value>) -> bool {
    let found = attribute(ctx, &c.attribute).map(scalars).unwrap_or_default();
    let any = |f: &dyn Fn(&str, &str) -> bool| found.iter().any(|have| c.values.iter().any(|want| f(have, want)));
    let numeric = |f: fn(f64, f64) -> bool| {
        let Some(want) = c.values.first().and_then(|w| w.trim().parse::<f64>().ok()) else {
            return false;
        };
        found.iter().filter_map(|have| have.trim().parse::<f64>().ok()).any(|have| f(have, want))
    };
    match c.operator {
        FlagOperator::OneOf => any(&|have, want| have == want),
        FlagOperator::NotOneOf => !any(&|have, want| have == want),
        FlagOperator::Contains => any(&|have, want| have.contains(want)),
        FlagOperator::NotContains => !any(&|have, want| have.contains(want)),
        FlagOperator::StartsWith => any(&|have, want| have.starts_with(want)),
        FlagOperator::EndsWith => any(&|have, want| have.ends_with(want)),
        FlagOperator::Gt => numeric(|a, b| a > b),
        FlagOperator::Gte => numeric(|a, b| a >= b),
        FlagOperator::Lt => numeric(|a, b| a < b),
        FlagOperator::Lte => numeric(|a, b| a <= b),
        FlagOperator::Exists => !found.is_empty(),
        FlagOperator::NotExists => found.is_empty(),
    }
}

/// The context's `targetingKey`, which must be a string when present.
pub fn targeting_key(ctx: &Map<String, Value>) -> Result<Option<&str>, EvalError> {
    match ctx.get("targetingKey") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str()).filter(|s| !s.is_empty())),
        Some(_) => Err(EvalError::InvalidContext("targetingKey must be a string".into())),
    }
}

fn served(variants: &[FlagVariant], key: &str, reason: Reason) -> Result<Resolution, EvalError> {
    let v = variants.iter().find(|v| v.key == key).ok_or_else(|| EvalError::General(format!("variant '{key}' is not defined")))?;
    Ok(Resolution { variant: Some(v.key.clone()), value: Some(v.value.clone()), reason })
}

fn serve(flag_key: &str, variants: &[FlagVariant], s: &FlagServe, tk: Option<&str>, fixed: Reason) -> Result<Resolution, EvalError> {
    if let Some(key) = &s.variant {
        return served(variants, key, fixed);
    }
    let tk = tk.ok_or(EvalError::TargetingKeyMissing)?;
    let key = pick(&s.split, bucket(flag_key, tk)).ok_or_else(|| EvalError::General("the split has no variants".into()))?;
    served(variants, key, Reason::Split)
}

/// Evaluates a flag in one environment for a context at `now_ms`.
pub fn evaluate(flag_key: &str, variants: &[FlagVariant], cfg: &FlagEnvConfig, ctx: &Map<String, Value>, now_ms: i64) -> Result<Resolution, EvalError> {
    let tk = targeting_key(ctx)?;
    if !cfg.enabled {
        return Ok(Resolution { variant: None, value: None, reason: Reason::Disabled });
    }
    if let Some(rule) = cfg.rules.iter().find(|r| r.conditions.iter().all(|c| condition_holds(c, ctx))) {
        return serve(flag_key, variants, &rule.serve, tk, Reason::TargetingMatch);
    }
    if let Some(r) = &cfg.rollout {
        let percent = rollout_percent(r, now_ms);
        if percent >= 100.0 {
            return served(variants, &r.variant, Reason::Static);
        }
        if percent > 0.0 {
            let tk = tk.ok_or(EvalError::TargetingKeyMissing)?;
            if bucket(&rollout_salt(flag_key), tk) < buckets_of(percent) {
                return served(variants, &r.variant, Reason::Split);
            }
        }
    }
    serve(flag_key, variants, &cfg.default_serve, tk, Reason::Static)
}

// ================================================================ validation

/// A flag key: 1–100 characters of lowercase letters, digits, `-`, `_` and `.`, starting with a letter or digit.
pub fn valid_key(key: &str) -> bool {
    (1..=MAX_KEY_CHARS).contains(&key.len())
        && key.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
}

/// Checks a flag's variants: 1–20 unique keys and values of the flag's type (exactly `true` and `false` for booleans).
pub fn validate_variants(t: FlagType, variants: &[FlagVariant]) -> Result<(), String> {
    if variants.is_empty() {
        return Err("A flag needs at least one variant".into());
    }
    if variants.len() > MAX_VARIANTS {
        return Err(format!("A flag has at most {MAX_VARIANTS} variants"));
    }
    let mut seen = std::collections::HashSet::new();
    for v in variants {
        if v.key.trim().is_empty() || v.key.chars().count() > MAX_KEY_CHARS {
            return Err(format!("Variant names are 1–{MAX_KEY_CHARS} characters"));
        }
        if !seen.insert(v.key.as_str()) {
            return Err(format!("Variant '{}' appears twice", v.key));
        }
        if !t.accepts(&v.value) {
            return Err(format!("Variant '{}' must be a {} value", v.key, t.as_str()));
        }
    }
    if t == FlagType::Boolean {
        let values: std::collections::HashSet<bool> = variants.iter().filter_map(|v| v.value.as_bool()).collect();
        if variants.len() != 2 || values.len() != 2 {
            return Err("A boolean flag has exactly two variants: true and false".into());
        }
    }
    Ok(())
}

fn validate_serve(s: &FlagServe, variants: &[FlagVariant], what: &str) -> Result<(), String> {
    let known = |k: &str| variants.iter().any(|v| v.key == k);
    match (&s.variant, s.split.is_empty()) {
        (Some(k), true) if known(k) => Ok(()),
        (Some(k), true) => Err(format!("{what} serves '{k}', which is not a variant of this flag")),
        (None, false) => {
            if s.split.len() > MAX_VARIANTS {
                return Err(format!("{what}: a split has at most {MAX_VARIANTS} parts"));
            }
            if let Some(bad) = s.split.iter().find(|p| !known(&p.variant)) {
                return Err(format!("{what} splits to '{}', which is not a variant of this flag", bad.variant));
            }
            if s.split.iter().any(|p| !p.weight.is_finite() || p.weight < 0.0) {
                return Err(format!("{what}: split weights are between 0 and 100"));
            }
            let total: f64 = s.split.iter().map(|p| p.weight).sum();
            if (total - 100.0).abs() > 0.011 {
                return Err(format!("{what}: split weights add up to {total}, not 100"));
            }
            Ok(())
        }
        (Some(_), false) => Err(format!("{what} serves either a variant or a split, not both")),
        (None, true) => Err(format!("{what} must serve a variant or a split")),
    }
}

/// Checks an environment configuration against the flag's variants.
pub fn validate_config(cfg: &FlagEnvConfig, variants: &[FlagVariant]) -> Result<(), String> {
    if cfg.rules.len() > MAX_RULES {
        return Err(format!("At most {MAX_RULES} targeting rules"));
    }
    for (i, r) in cfg.rules.iter().enumerate() {
        let what = format!("Rule {}", i + 1);
        if r.conditions.len() > MAX_CONDITIONS {
            return Err(format!("{what} has more than {MAX_CONDITIONS} conditions"));
        }
        for c in &r.conditions {
            if c.attribute.trim().is_empty() {
                return Err(format!("{what}: every condition needs an attribute"));
            }
            if c.values.len() > MAX_VALUES {
                return Err(format!("{what}: a condition has at most {MAX_VALUES} values"));
            }
            let needs_values = !matches!(c.operator, FlagOperator::Exists | FlagOperator::NotExists);
            if needs_values && c.values.iter().all(|v| v.is_empty()) {
                return Err(format!("{what}: the condition on '{}' needs a value", c.attribute));
            }
            let numeric = matches!(c.operator, FlagOperator::Gt | FlagOperator::Gte | FlagOperator::Lt | FlagOperator::Lte);
            if numeric && c.values.first().and_then(|v| v.trim().parse::<f64>().ok()).is_none() {
                return Err(format!("{what}: the condition on '{}' compares with a number", c.attribute));
            }
        }
        validate_serve(&r.serve, variants, &what)?;
    }
    validate_serve(&cfg.default_serve, variants, "The default rule")?;
    if let Some(r) = &cfg.rollout {
        if !variants.iter().any(|v| v.key == r.variant) {
            return Err(format!("The rollout serves '{}', which is not a variant of this flag", r.variant));
        }
        if r.steps.is_empty() || r.steps.len() > MAX_ROLLOUT_STEPS {
            return Err(format!("A rollout has 1–{MAX_ROLLOUT_STEPS} steps"));
        }
        if r.steps.iter().any(|s| !s.percent.is_finite() || !(0.0..=100.0).contains(&s.percent)) {
            return Err("Rollout percentages are between 0 and 100".into());
        }
        if r.steps.windows(2).any(|w| w[1].percent < w[0].percent) {
            return Err("Rollout steps must not decrease".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn bool_variants() -> Vec<FlagVariant> {
        vec![FlagVariant { key: "on".into(), value: json!(true) }, FlagVariant { key: "off".into(), value: json!(false) }]
    }

    fn split(parts: &[(&str, f64)]) -> FlagServe {
        FlagServe { variant: None, split: parts.iter().map(|(v, w)| FlagSplit { variant: v.to_string(), weight: *w }).collect() }
    }

    fn cond(attribute: &str, operator: FlagOperator, values: &[&str]) -> FlagCondition {
        FlagCondition { attribute: attribute.into(), operator, values: values.iter().map(|v| v.to_string()).collect() }
    }

    #[test]
    fn bucket_is_stable_and_in_range() {
        let b = bucket("checkout-new-payment-flow", "user-123");
        assert_eq!(b, bucket("checkout-new-payment-flow", "user-123"));
        assert!(b < BUCKETS);
        // Pinned: SDKs and servers must keep assigning the same bucket across releases.
        let digest = Sha256::digest(b"checkout-new-payment-flow.user-123");
        let expected = (u64::from_be_bytes(digest[..8].try_into().unwrap()) % u64::from(BUCKETS)) as u32;
        assert_eq!(b, expected);
        // Another flag shuffles subjects independently.
        let moved = (0..200).filter(|i| bucket("a", &format!("u{i}")) != bucket("b", &format!("u{i}"))).count();
        assert!(moved > 190);
    }

    #[test]
    fn buckets_spread_evenly() {
        let n = 20_000;
        let parts = [("a", 50.0), ("b", 30.0), ("c", 20.0)];
        let s = split(&parts).split;
        let mut counts = std::collections::HashMap::new();
        for i in 0..n {
            *counts.entry(pick(&s, bucket("catalog-search-v2", &format!("user-{i}"))).unwrap()).or_insert(0) += 1;
        }
        for (v, w) in parts {
            let share = f64::from(counts[v]) / f64::from(n) * 100.0;
            assert!((share - w).abs() < 1.5, "{v}: {share}% for {w}%");
        }
    }

    #[test]
    fn pick_follows_cumulative_weights() {
        let s = split(&[("a", 10.0), ("b", 90.0)]).split;
        assert_eq!(pick(&s, 0), Some("a"));
        assert_eq!(pick(&s, 999), Some("a"));
        assert_eq!(pick(&s, 1000), Some("b"));
        assert_eq!(pick(&s, BUCKETS - 1), Some("b"));
        // Rounding leaves nobody out: the last part takes the remainder.
        let thirds = split(&[("a", 33.33), ("b", 33.33), ("c", 33.33)]).split;
        assert_eq!(pick(&thirds, BUCKETS - 1), Some("c"));
        assert_eq!(pick(&[], 5), None);
        assert_eq!(buckets_of(12.345), 1235);
    }

    #[test]
    fn rollout_moves_through_its_steps() {
        let r = FlagRollout {
            variant: "on".into(),
            steps: vec![FlagRolloutStep { percent: 10.0, duration_minutes: 60 }, FlagRolloutStep { percent: 50.0, duration_minutes: 30 }, FlagRolloutStep { percent: 100.0, duration_minutes: 0 }],
            started_at: 1_000_000,
        };
        let min = 60_000;
        assert_eq!(rollout_percent(&r, 999_999), 0.0);
        assert_eq!(rollout_percent(&r, 1_000_000), 10.0);
        assert_eq!(rollout_percent(&r, 1_000_000 + 59 * min), 10.0);
        assert_eq!(rollout_percent(&r, 1_000_000 + 60 * min), 50.0);
        assert_eq!(rollout_percent(&r, 1_000_000 + 90 * min), 100.0);
        assert_eq!(rollout_percent(&r, i64::MAX), 100.0);
        assert_eq!(rollout_percent(&FlagRollout { steps: vec![], ..r.clone() }, 2_000_000), 0.0);
        // Not started yet.
        assert_eq!(rollout_percent(&FlagRollout { started_at: 0, ..r }, 2_000_000), 0.0);
    }

    #[test]
    fn environment_queries() {
        assert!(env_matches("production", "production"));
        assert!(env_matches("production", "Production"));
        assert!(!env_matches("production", "production-eu"));
        assert!(env_matches("staging-*", "staging-eu"));
        assert!(env_matches("staging-*", "staging-"));
        assert!(!env_matches("staging-*", "stage"));
        assert!(env_matches("*-local", "dev-local"));
        assert!(env_matches("prod*eu", "prod-west-eu"));
        assert!(!env_matches("prod*eu", "prod-west-us"));
        assert!(env_matches("*", "anything"));
        assert!(!env_matches("", "x"));
    }

    #[test]
    fn conditions() {
        let c = ctx(json!({"targetingKey": "user-1", "country": "CA", "plan": {"tier": "pro"}, "age": 42, "beta": true, "groups": ["qa", "staff"], "email": "ann@example.com"}));
        assert!(condition_holds(&cond("country", FlagOperator::OneOf, &["US", "CA"]), &c));
        assert!(!condition_holds(&cond("country", FlagOperator::OneOf, &["ca"]), &c));
        assert!(condition_holds(&cond("country", FlagOperator::NotOneOf, &["US"]), &c));
        assert!(condition_holds(&cond("plan.tier", FlagOperator::OneOf, &["pro"]), &c));
        assert!(condition_holds(&cond("targetingKey", FlagOperator::StartsWith, &["user-"]), &c));
        assert!(condition_holds(&cond("email", FlagOperator::EndsWith, &["@example.com"]), &c));
        assert!(condition_holds(&cond("email", FlagOperator::Contains, &["ann"]), &c));
        assert!(condition_holds(&cond("email", FlagOperator::NotContains, &["bob"]), &c));
        assert!(condition_holds(&cond("age", FlagOperator::Gte, &["42"]), &c));
        assert!(!condition_holds(&cond("age", FlagOperator::Gt, &["42"]), &c));
        assert!(condition_holds(&cond("age", FlagOperator::Lt, &["50.5"]), &c));
        assert!(!condition_holds(&cond("age", FlagOperator::Lte, &["abc"]), &c));
        assert!(condition_holds(&cond("beta", FlagOperator::OneOf, &["true"]), &c));
        assert!(condition_holds(&cond("groups", FlagOperator::OneOf, &["staff"]), &c));
        assert!(condition_holds(&cond("beta", FlagOperator::Exists, &[]), &c));
        assert!(condition_holds(&cond("missing", FlagOperator::NotExists, &[]), &c));
        // A missing attribute matches no positive operator, and every negative one.
        assert!(!condition_holds(&cond("missing", FlagOperator::OneOf, &["x"]), &c));
        assert!(condition_holds(&cond("missing", FlagOperator::NotOneOf, &["x"]), &c));
        assert!(!condition_holds(&cond("plan", FlagOperator::OneOf, &["pro"]), &c));
    }

    fn cfg(rules: Vec<FlagRule>, default_serve: FlagServe) -> FlagEnvConfig {
        FlagEnvConfig { enabled: true, rules, default_serve, rollout: None }
    }

    #[test]
    fn disabled_serves_the_code_default() {
        let c = FlagEnvConfig { enabled: false, ..cfg(vec![], FlagServe::variant("on")) };
        let r = evaluate("f", &bool_variants(), &c, &ctx(json!({})), 0).unwrap();
        assert_eq!(r, Resolution { variant: None, value: None, reason: Reason::Disabled });
    }

    #[test]
    fn default_variant_is_static() {
        let r = evaluate("f", &bool_variants(), &cfg(vec![], FlagServe::variant("off")), &ctx(json!({})), 0).unwrap();
        assert_eq!(r, Resolution { variant: Some("off".into()), value: Some(json!(false)), reason: Reason::Static });
    }

    #[test]
    fn first_matching_rule_serves() {
        let rules = vec![
            FlagRule { name: "staff".into(), conditions: vec![cond("email", FlagOperator::EndsWith, &["@example.com"])], serve: FlagServe::variant("on") },
            FlagRule { name: "canada".into(), conditions: vec![cond("country", FlagOperator::OneOf, &["CA"])], serve: FlagServe::variant("off") },
        ];
        let c = cfg(rules, FlagServe::variant("off"));
        let staff = evaluate("f", &bool_variants(), &c, &ctx(json!({"email": "ann@example.com", "country": "CA"})), 0).unwrap();
        assert_eq!((staff.variant.as_deref(), staff.reason), (Some("on"), Reason::TargetingMatch));
        let other = evaluate("f", &bool_variants(), &c, &ctx(json!({"email": "bob@example.org", "country": "US"})), 0).unwrap();
        assert_eq!((other.variant.as_deref(), other.reason), (Some("off"), Reason::Static));
        // Every condition of a rule must hold.
        let both = cfg(
            vec![FlagRule { name: String::new(), conditions: vec![cond("country", FlagOperator::OneOf, &["CA"]), cond("beta", FlagOperator::OneOf, &["true"])], serve: FlagServe::variant("on") }],
            FlagServe::variant("off"),
        );
        assert_eq!(evaluate("f", &bool_variants(), &both, &ctx(json!({"country": "CA"})), 0).unwrap().variant.as_deref(), Some("off"));
        assert_eq!(evaluate("f", &bool_variants(), &both, &ctx(json!({"country": "CA", "beta": true})), 0).unwrap().variant.as_deref(), Some("on"));
    }

    #[test]
    fn splits_need_a_targeting_key_and_are_sticky() {
        let c = cfg(vec![], split(&[("on", 50.0), ("off", 50.0)]));
        assert_eq!(evaluate("f", &bool_variants(), &c, &ctx(json!({})), 0), Err(EvalError::TargetingKeyMissing));
        let first = evaluate("f", &bool_variants(), &c, &ctx(json!({"targetingKey": "user-7"})), 0).unwrap();
        assert_eq!(first.reason, Reason::Split);
        for _ in 0..5 {
            assert_eq!(evaluate("f", &bool_variants(), &c, &ctx(json!({"targetingKey": "user-7"})), 0).unwrap(), first);
        }
        let expected = pick(&c.default_serve.split, bucket("f", "user-7")).unwrap();
        assert_eq!(first.variant.as_deref(), Some(expected));
        assert_eq!(evaluate("f", &bool_variants(), &c, &ctx(json!({"targetingKey": 7})), 0), Err(EvalError::InvalidContext("targetingKey must be a string".into())));
    }

    #[test]
    fn rollout_takes_a_growing_share_of_the_default_traffic() {
        let rollout =
            FlagRollout { variant: "on".into(), steps: vec![FlagRolloutStep { percent: 10.0, duration_minutes: 60 }, FlagRolloutStep { percent: 40.0, duration_minutes: 60 }], started_at: 1 };
        let mut c = cfg(vec![FlagRule { name: "qa".into(), conditions: vec![cond("qa", FlagOperator::Exists, &[])], serve: FlagServe::variant("off") }], FlagServe::variant("off"));
        c.rollout = Some(rollout);
        let on_at = |now: i64| {
            (0..2000)
                .filter(|i| {
                    let r = evaluate("f", &bool_variants(), &c, &ctx(json!({"targetingKey": format!("u{i}")})), now).unwrap();
                    r.variant.as_deref() == Some("on")
                })
                .map(|i| format!("u{i}"))
                .collect::<std::collections::HashSet<_>>()
        };
        let early = on_at(1);
        let later = on_at(1 + 61 * 60_000);
        assert!((150..250).contains(&early.len()), "{}", early.len());
        assert!((700..900).contains(&later.len()), "{}", later.len());
        // Subjects inside the rollout stay inside as it grows.
        assert!(early.is_subset(&later));
        // Rules come first.
        let qa = evaluate("f", &bool_variants(), &c, &ctx(json!({"targetingKey": "u1", "qa": 1})), 1 + 61 * 60_000).unwrap();
        assert_eq!((qa.variant.as_deref(), qa.reason), (Some("off"), Reason::TargetingMatch));
    }

    #[test]
    fn a_missing_variant_is_a_general_error() {
        let c = cfg(vec![], FlagServe::variant("gone"));
        assert!(matches!(evaluate("f", &bool_variants(), &c, &ctx(json!({})), 0), Err(EvalError::General(_))));
    }

    #[test]
    fn keys() {
        assert!(valid_key("checkout-new-payment-flow"));
        assert!(valid_key("catalog.search_v2"));
        assert!(!valid_key(""));
        assert!(!valid_key("-x"));
        assert!(!valid_key("Upper"));
        assert!(!valid_key("has space"));
        assert!(!valid_key(&"a".repeat(101)));
    }

    #[test]
    fn variant_validation() {
        assert!(validate_variants(FlagType::Boolean, &bool_variants()).is_ok());
        let one_bool = vec![FlagVariant { key: "on".into(), value: json!(true) }];
        assert!(validate_variants(FlagType::Boolean, &one_bool).is_err());
        let strings = vec![FlagVariant { key: "blue".into(), value: json!("blue") }, FlagVariant { key: "blue".into(), value: json!("navy") }];
        assert_eq!(validate_variants(FlagType::String, &strings), Err("Variant 'blue' appears twice".into()));
        let wrong = vec![FlagVariant { key: "n".into(), value: json!(1.5) }];
        assert!(validate_variants(FlagType::Integer, &wrong).is_err());
        assert!(validate_variants(FlagType::Number, &wrong).is_ok());
        assert!(validate_variants(FlagType::Json, &[FlagVariant { key: "cfg".into(), value: json!({"limit": 3}) }]).is_ok());
        assert!(validate_variants(FlagType::String, &[]).is_err());
    }

    #[test]
    fn config_validation() {
        let v = bool_variants();
        assert!(validate_config(&cfg(vec![], FlagServe::variant("on")), &v).is_ok());
        assert!(validate_config(&cfg(vec![], FlagServe::variant("maybe")), &v).is_err());
        assert!(validate_config(&cfg(vec![], FlagServe::default()), &v).is_err());
        assert!(validate_config(&cfg(vec![], split(&[("on", 60.0), ("off", 30.0)])), &v).is_err());
        assert!(validate_config(&cfg(vec![], split(&[("on", 66.67), ("off", 33.33)])), &v).is_ok());
        let both = FlagServe { variant: Some("on".into()), split: split(&[("on", 100.0)]).split };
        assert!(validate_config(&cfg(vec![], both), &v).is_err());
        let empty_attr = FlagRule { name: String::new(), conditions: vec![cond("", FlagOperator::OneOf, &["x"])], serve: FlagServe::variant("on") };
        assert!(validate_config(&cfg(vec![empty_attr], FlagServe::variant("on")), &v).is_err());
        let no_values = FlagRule { name: String::new(), conditions: vec![cond("country", FlagOperator::OneOf, &[])], serve: FlagServe::variant("on") };
        assert!(validate_config(&cfg(vec![no_values], FlagServe::variant("on")), &v).is_err());
        let numeric = FlagRule { name: String::new(), conditions: vec![cond("age", FlagOperator::Gt, &["old"])], serve: FlagServe::variant("on") };
        assert!(validate_config(&cfg(vec![numeric], FlagServe::variant("on")), &v).is_err());
        let mut c = cfg(vec![], FlagServe::variant("off"));
        c.rollout = Some(FlagRollout { variant: "on".into(), steps: vec![FlagRolloutStep { percent: 120.0, duration_minutes: 10 }], started_at: 0 });
        assert!(validate_config(&c, &v).is_err());
        c.rollout =
            Some(FlagRollout { variant: "on".into(), steps: vec![FlagRolloutStep { percent: 20.0, duration_minutes: 10 }, FlagRolloutStep { percent: 10.0, duration_minutes: 10 }], started_at: 0 });
        assert_eq!(validate_config(&c, &v), Err("Rollout steps must not decrease".into()));
    }

    #[test]
    fn serde_shapes() {
        let c: FlagEnvConfig = serde_json::from_value(json!({
            "enabled": true,
            "rules": [{"name": "beta", "conditions": [{"attribute": "country", "operator": "one_of", "values": ["CA"]}], "serve": {"split": [{"variant": "on", "weight": 50}, {"variant": "off", "weight": 50}]}}],
            "defaultServe": {"variant": "off"},
            "rollout": {"variant": "on", "steps": [{"percent": 5, "durationMinutes": 30}], "startedAt": 0}
        }))
        .unwrap();
        assert_eq!(c.rules[0].serve.split.len(), 2);
        assert_eq!(serde_json::to_value(&c.default_serve).unwrap(), json!({"variant": "off"}));
        assert_eq!(FlagType::parse("json"), Some(FlagType::Json));
    }
}
