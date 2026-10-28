//! Resource policies: what a policy checks and how it is evaluated against the resource catalog.
//! Pure functions over [`CatalogResource`]s; [`super::resource_policies`] stores policies and
//! their daily compliance snapshots.
//!
//! A policy targets one resource type, narrows it with a scope (catalog query syntax, e.g.
//! `env:prod cluster:shop-*`) and requires every condition to hold on an attribute: a field
//! (`env`, `team`, …), a tag key or one of the resource's attributes (`replicas`, `image_tags`, …).
//! Attributes with several values (tags, image tags) must all satisfy a positive condition
//! (equals, one of, matches, comparisons) and none may match a negative one.

use std::cmp::Ordering;

use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::catalog::{enum_str_static, CatalogResource, CatalogType, Faceted};

/// Conditions per policy, values per condition, and their sizes.
pub const MAX_CONDITIONS: usize = 10;
pub const MAX_VALUES: usize = 50;
const MAX_VALUE_CHARS: usize = 200;
const MAX_PATTERN_CHARS: usize = 500;
const BACKTRACK_LIMIT: usize = 100_000;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOperator {
    Exists,
    NotExists,
    Equals,
    NotEquals,
    OneOf,
    NotOneOf,
    Matches,
    Gt,
    Gte,
    Lt,
    Lte,
    VersionGt,
    VersionGte,
    VersionLt,
    VersionLte,
}

impl PolicyOperator {
    /// How many values the operator takes: none, exactly one, or a list.
    fn arity(self) -> (usize, usize) {
        match self {
            PolicyOperator::Exists | PolicyOperator::NotExists => (0, 0),
            PolicyOperator::OneOf | PolicyOperator::NotOneOf => (1, MAX_VALUES),
            _ => (1, 1),
        }
    }

    /// Negative conditions hold when no value matches (so a missing attribute complies).
    fn negative(self) -> bool {
        matches!(self, PolicyOperator::NotExists | PolicyOperator::NotEquals | PolicyOperator::NotOneOf)
    }

    fn symbol(self) -> &'static str {
        match self {
            PolicyOperator::Exists => "set",
            PolicyOperator::NotExists => "not set",
            PolicyOperator::Equals => "=",
            PolicyOperator::NotEquals => "≠",
            PolicyOperator::OneOf => "one of",
            PolicyOperator::NotOneOf => "not one of",
            PolicyOperator::Matches => "matches",
            PolicyOperator::Gt | PolicyOperator::VersionGt => ">",
            PolicyOperator::Gte | PolicyOperator::VersionGte => "≥",
            PolicyOperator::Lt | PolicyOperator::VersionLt => "<",
            PolicyOperator::Lte | PolicyOperator::VersionLte => "≤",
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PolicyCategory {
    CostOptimization,
    Reliability,
    Security,
    Versioning,
    Tagging,
}

impl PolicyCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyCategory::CostOptimization => "cost_optimization",
            PolicyCategory::Reliability => "reliability",
            PolicyCategory::Security => "security",
            PolicyCategory::Versioning => "versioning",
            PolicyCategory::Tagging => "tagging",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [PolicyCategory::CostOptimization, PolicyCategory::Reliability, PolicyCategory::Security, PolicyCategory::Versioning, PolicyCategory::Tagging].into_iter().find(|c| c.as_str() == s)
    }
}

/// One requirement on an attribute of the resource.
#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PolicyCondition {
    /// A field (`env`, `team`, `region`, …), a tag key or an attribute (`replicas`, `image_tags`, `version`, …).
    pub attribute: String,
    pub operator: PolicyOperator,
    /// None for exists / not exists, one for comparisons, a list for one of / not one of.
    #[serde(default)]
    pub values: Vec<String>,
}

impl PolicyCondition {
    /// `replicas ≥ 2`, `team set`, `image_tags not one of latest`.
    pub fn describe(&self) -> String {
        if self.values.is_empty() {
            format!("{} {}", self.attribute, self.operator.symbol())
        } else {
            format!("{} {} {}", self.attribute, self.operator.symbol(), self.values.join(", "))
        }
    }
}

/// A numeric version: `v1.31.2` → [1, 31, 2], `1.29.3-eks-a1b2` → [1, 29, 3], `22.04` → [22, 4].
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core: &str = s.split(|c: char| !(c.is_ascii_digit() || c == '.')).next().unwrap_or("");
    let parts: Vec<u64> = core.split('.').filter(|p| !p.is_empty()).map(str::parse).collect::<Result<_, _>>().ok()?;
    (!parts.is_empty()).then_some(parts)
}

/// Compares versions component by component; missing components count as 0 (`1.30` = `1.30.0`).
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse_version(a)?, parse_version(b)?);
    let n = a.len().max(b.len());
    Some((0..n).map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal))
}

/// A condition ready to run (its pattern compiled, its threshold parsed).
#[derive(Debug)]
struct Compiled {
    condition: PolicyCondition,
    regex: Option<fancy_regex::Regex>,
    number: Option<f64>,
}

impl Compiled {
    fn new(c: &PolicyCondition) -> Result<Compiled, String> {
        let attribute = c.attribute.trim();
        if attribute.is_empty() || attribute.len() > 200 || !attribute.chars().all(|ch| ch.is_alphanumeric() || "_.-/@".contains(ch)) {
            return Err(format!("'{attribute}' is not a valid attribute name"));
        }
        let (min, max) = c.operator.arity();
        let values: Vec<String> = c.values.iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
        if values.len() < min || values.len() > max {
            return Err(match (min, max) {
                (0, 0) => format!("{attribute}: '{}' takes no value", c.operator.symbol()),
                (1, 1) => format!("{attribute}: '{}' takes one value", c.operator.symbol()),
                _ => format!("{attribute}: '{}' takes 1 to {max} values", c.operator.symbol()),
            });
        }
        if values.iter().any(|v| v.chars().count() > if c.operator == PolicyOperator::Matches { MAX_PATTERN_CHARS } else { MAX_VALUE_CHARS }) {
            return Err(format!("{attribute}: value too long"));
        }
        let first = values.first().map(String::as_str).unwrap_or("");
        let mut out = Compiled { condition: PolicyCondition { attribute: attribute.to_string(), operator: c.operator, values: values.clone() }, regex: None, number: None };
        match c.operator {
            PolicyOperator::Matches => {
                out.regex = Some(fancy_regex::RegexBuilder::new(first).backtrack_limit(BACKTRACK_LIMIT).build().map_err(|e| format!("{attribute}: invalid pattern ({e})"))?);
            }
            PolicyOperator::Gt | PolicyOperator::Gte | PolicyOperator::Lt | PolicyOperator::Lte => {
                out.number = Some(first.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("{attribute}: '{first}' is not a number"))?);
            }
            PolicyOperator::VersionGt | PolicyOperator::VersionGte | PolicyOperator::VersionLt | PolicyOperator::VersionLte => {
                parse_version(first).ok_or_else(|| format!("{attribute}: '{first}' is not a version"))?;
            }
            _ => {}
        }
        Ok(out)
    }

    /// Whether one value satisfies the (positive form of the) condition.
    fn value_ok(&self, v: &str) -> bool {
        let want = self.condition.values.first().map(String::as_str).unwrap_or("");
        let ord = |o: Ordering| match self.condition.operator {
            PolicyOperator::Gt | PolicyOperator::VersionGt => o == Ordering::Greater,
            PolicyOperator::Gte | PolicyOperator::VersionGte => o != Ordering::Less,
            PolicyOperator::Lt | PolicyOperator::VersionLt => o == Ordering::Less,
            _ => o != Ordering::Greater,
        };
        match self.condition.operator {
            PolicyOperator::Exists | PolicyOperator::NotExists => true,
            PolicyOperator::Equals | PolicyOperator::NotEquals => v.eq_ignore_ascii_case(want),
            PolicyOperator::OneOf | PolicyOperator::NotOneOf => self.condition.values.iter().any(|w| v.eq_ignore_ascii_case(w)),
            PolicyOperator::Matches => self.regex.as_ref().is_some_and(|r| r.is_match(v).unwrap_or(false)),
            PolicyOperator::Gt | PolicyOperator::Gte | PolicyOperator::Lt | PolicyOperator::Lte => v.parse::<f64>().ok().zip(self.number).and_then(|(a, b)| a.partial_cmp(&b)).is_some_and(ord),
            _ => compare_versions(v, want).is_some_and(ord),
        }
    }

    /// None when the condition holds, else why not.
    fn failure(&self, values: &[&str]) -> Option<String> {
        let attr = &self.condition.attribute;
        let shown = || values.join(", ");
        let op = self.condition.operator;
        if op.negative() {
            let hits: Vec<&str> = values.iter().copied().filter(|v| self.value_ok(v)).collect();
            return (!hits.is_empty()).then(|| format!("{attr} is {}", if op == PolicyOperator::NotExists { shown() } else { hits.join(", ") }));
        }
        if values.is_empty() {
            return Some(format!("{attr} is missing"));
        }
        (!values.iter().all(|v| self.value_ok(v))).then(|| format!("{attr} is {}", shown()))
    }
}

/// What a policy evaluates: which resources and what they must satisfy.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTarget {
    pub resource_type: CatalogType,
    /// Catalog query narrowing the resources (`env:prod cluster:shop-*`); empty for all of the type.
    #[serde(default)]
    pub scope: String,
    pub conditions: Vec<PolicyCondition>,
}

/// A policy target checked and ready to evaluate.
#[derive(Debug)]
pub struct CompiledPolicy {
    resource_type: CatalogType,
    scope: Vec<Term>,
    conditions: Vec<Compiled>,
}

impl CompiledPolicy {
    pub fn new(t: &PolicyTarget) -> Result<CompiledPolicy, String> {
        if t.conditions.is_empty() {
            return Err("A policy needs at least one condition".into());
        }
        if t.conditions.len() > MAX_CONDITIONS {
            return Err(format!("A policy can have at most {MAX_CONDITIONS} conditions"));
        }
        if t.scope.chars().count() > 1000 {
            return Err("The scope is too long".into());
        }
        Ok(CompiledPolicy { resource_type: t.resource_type, scope: parse_query(&t.scope), conditions: t.conditions.iter().map(Compiled::new).collect::<Result<_, _>>()? })
    }

    /// The resource is of the policy's type and in its scope.
    pub fn applies(&self, r: &CatalogResource) -> bool {
        r.resource_type == self.resource_type && r.matches(&self.scope)
    }

    /// None when the resource complies, else the first failing condition.
    pub fn failure(&self, r: &CatalogResource) -> Option<String> {
        self.conditions.iter().find_map(|c| c.failure(&r.values(&c.condition.attribute)))
    }
}

/// A noncompliant resource and why.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyViolation {
    pub resource_id: String,
    pub reason: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PolicyEvaluation {
    /// Resources in the policy's type and scope (and the page's filters).
    pub evaluated: usize,
    pub noncompliant: usize,
    /// Compliant share in percent, one decimal; null when nothing is evaluated.
    pub score: Option<f64>,
}

/// Compliant share in percent with one decimal, None when there is nothing to evaluate.
pub fn score(evaluated: usize, noncompliant: usize) -> Option<f64> {
    (evaluated > 0).then(|| ((evaluated - noncompliant.min(evaluated)) as f64 * 1000.0 / evaluated as f64).round() / 10.0)
}

/// Evaluates a policy on the catalog, limited to resources matching `filter` (the page's
/// header filters), and returns the counts and every violation.
pub fn evaluate(policy: &CompiledPolicy, resources: &[CatalogResource], filter: &[Term]) -> (PolicyEvaluation, Vec<PolicyViolation>) {
    let mut evaluated = 0;
    let mut violations = Vec::new();
    for r in resources.iter().filter(|r| policy.applies(r) && r.matches(filter)) {
        evaluated += 1;
        if let Some(reason) = policy.failure(r) {
            violations.push(PolicyViolation { resource_id: r.id.clone(), reason });
        }
    }
    (PolicyEvaluation { evaluated, noncompliant: violations.len(), score: score(evaluated, violations.len()) }, violations)
}

/// A ready-made policy for what the catalog knows about.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTemplate {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: PolicyCategory,
    pub priority: u8,
    pub target: PolicyTarget,
}

fn cond(attribute: &str, operator: PolicyOperator, values: &[&str]) -> PolicyCondition {
    PolicyCondition { attribute: attribute.into(), operator, values: values.iter().map(|v| v.to_string()).collect() }
}

fn template(
    key: &'static str,
    name: &'static str,
    description: &'static str,
    (category, priority): (PolicyCategory, u8),
    resource_type: CatalogType,
    scope: &str,
    conditions: Vec<PolicyCondition>,
) -> PolicyTemplate {
    PolicyTemplate { key, name, description, category, priority, target: PolicyTarget { resource_type, scope: scope.into(), conditions } }
}

/// The built-in templates, in display order. Every attribute they check is one the catalog fills
/// from what the collectors send (see `catalog.rs`).
pub fn templates() -> Vec<PolicyTemplate> {
    use PolicyCategory::*;
    use PolicyOperator::*;
    vec![
        template(
            "kube_no_latest_tag",
            "Kubernetes workloads don't run the latest image tag",
            "A floating tag changes what runs on every restart. Pin each container image to a release tag or a digest.",
            (Versioning, 2),
            CatalogType::KubeDeployment,
            "",
            vec![cond("image_tags", NotOneOf, &["latest"])],
        ),
        template(
            "kube_requests_limits",
            "Kubernetes workloads set CPU and memory requests and limits",
            "Without requests the scheduler can't place pods well; without limits one pod can starve its neighbours.",
            (Reliability, 2),
            CatalogType::KubeDeployment,
            "",
            vec![cond("cpu_requests", Exists, &[]), cond("cpu_limits", Exists, &[]), cond("memory_requests", Exists, &[]), cond("memory_limits", Exists, &[])],
        ),
        template(
            "kube_prod_replicas",
            "Production deployments run at least two replicas",
            "A single replica means downtime on every rollout or node failure.",
            (Reliability, 1),
            CatalogType::KubeDeployment,
            "env:prod",
            vec![cond("replicas", Gte, &["2"])],
        ),
        template(
            "kube_unified_tags",
            "Kubernetes deployments carry team, service and env tags",
            "Unified service tags link a workload to its owners, its traces and its logs.",
            (Tagging, 3),
            CatalogType::KubeDeployment,
            "",
            vec![cond("team", Exists, &[]), cond("service", Exists, &[]), cond("env", Exists, &[])],
        ),
        template(
            "host_team_tag",
            "Hosts have an owning team",
            "Every host should carry a team tag so alerts and costs reach the right people.",
            (Tagging, 3),
            CatalogType::Host,
            "",
            vec![cond("team", Exists, &[])],
        ),
        template(
            "host_supported_ubuntu",
            "Ubuntu hosts run a supported LTS release",
            "Ubuntu 20.04 and older are out of standard support. Upgrade hosts to 22.04 or later.",
            (Security, 2),
            CatalogType::Host,
            "distro:ubuntu",
            vec![cond("os_release", VersionGte, &["22.04"])],
        ),
        template(
            "kube_node_version",
            "Kubernetes nodes run a supported kubelet version",
            "Nodes on old minor versions miss security fixes. Adjust the minimum to your provider's support window.",
            (Versioning, 2),
            CatalogType::KubeNode,
            "",
            vec![cond("version", VersionGte, &["1.31"])],
        ),
        template(
            "kube_service_no_nodeport",
            "Kubernetes services aren't exposed through node ports",
            "NodePort services open a port on every node. Prefer a load balancer or an ingress.",
            (Security, 2),
            CatalogType::KubeService,
            "",
            vec![cond("service_type", NotEquals, &["NodePort"])],
        ),
        template(
            "kube_staging_replicas",
            "Staging deployments run at most two replicas",
            "Non-production environments rarely need more capacity than two replicas give.",
            (CostOptimization, 3),
            CatalogType::KubeDeployment,
            "env:staging",
            vec![cond("replicas", Lte, &["2"])],
        ),
        template(
            "ecs_service_tasks",
            "ECS services run at least two tasks",
            "A service with one running task goes down with its task.",
            (Reliability, 2),
            CatalogType::EcsService,
            "",
            vec![cond("tasks", Gte, &["2"])],
        ),
    ]
}

/// The serialized name of a resource type (`kube_deployment`).
pub fn type_str(t: CatalogType) -> &'static str {
    enum_str_static(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn res(t: CatalogType, id: &str, tags: &[&str], attrs: &[(&str, &str)]) -> CatalogResource {
        CatalogResource {
            id: id.into(),
            resource_type: t,
            resource_category: super::super::catalog::CatalogCategory::Kubernetes,
            name: id.into(),
            cloud: None,
            account: None,
            region: None,
            cluster: Some("shop-prod-eu".into()),
            namespace: Some("checkout".into()),
            env: tags.iter().find_map(|t| t.strip_prefix("env:")).map(str::to_string),
            service: None,
            team: tags.iter().find_map(|t| t.strip_prefix("team:")).map(str::to_string),
            status: None,
            health: None,
            created_at: None,
            last_seen: 0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(),
            kube_kind: None,
        }
    }

    fn policy(t: CatalogType, scope: &str, conditions: Vec<PolicyCondition>) -> CompiledPolicy {
        CompiledPolicy::new(&PolicyTarget { resource_type: t, scope: scope.into(), conditions }).unwrap()
    }

    #[test]
    fn versions_compare_numerically_ignoring_prefixes_and_suffixes() {
        assert_eq!(parse_version("v1.31.2"), Some(vec![1, 31, 2]));
        assert_eq!(parse_version("1.29.3-eks-a1b2c3"), Some(vec![1, 29, 3]));
        assert_eq!(parse_version("22.04"), Some(vec![22, 4]));
        assert_eq!(parse_version("latest"), None);
        assert_eq!(compare_versions("v1.9", "1.10"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.30", "v1.30.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("20.04.6", "22.04"), Some(Ordering::Less));
        assert_eq!(compare_versions("2.4", "nope"), None);
    }

    #[test]
    fn operators_check_every_value_and_missing_attributes() {
        let dep = res(CatalogType::KubeDeployment, "a", &["env:prod", "team:checkout"], &[("replicas", "1"), ("image_tags", "2.4, latest"), ("version", "v1.30.6")]);
        let fail = |c: PolicyCondition| policy(CatalogType::KubeDeployment, "", vec![c]).failure(&dep);
        use PolicyOperator::*;
        assert_eq!(fail(cond("team", Exists, &[])), None);
        assert_eq!(fail(cond("service", Exists, &[])).as_deref(), Some("service is missing"));
        assert_eq!(fail(cond("service", NotExists, &[])), None);
        assert_eq!(fail(cond("team", NotExists, &[])).as_deref(), Some("team is checkout"));
        assert_eq!(fail(cond("env", Equals, &["PROD"])), None, "equality ignores case");
        assert_eq!(fail(cond("env", NotEquals, &["prod"])).as_deref(), Some("env is prod"));
        // A list attribute: every value must satisfy positive conditions, none may hit negative ones.
        assert_eq!(fail(cond("image_tags", NotOneOf, &["latest", "main"])).as_deref(), Some("image_tags is latest"));
        assert_eq!(fail(cond("image_tags", OneOf, &["2.4", "latest"])), None);
        assert_eq!(fail(cond("image_tags", Matches, &[r"^\d+\.\d+$"])).as_deref(), Some("image_tags is 2.4, latest"));
        assert_eq!(fail(cond("replicas", Gte, &["2"])).as_deref(), Some("replicas is 1"));
        assert_eq!(fail(cond("replicas", Lt, &["2"])), None);
        assert_eq!(fail(cond("replicas", Gt, &["0.5"])), None);
        assert_eq!(fail(cond("version", VersionGte, &["1.31"])).as_deref(), Some("version is v1.30.6"));
        assert_eq!(fail(cond("version", VersionLt, &["1.31"])), None);
        // Comparisons on missing or unparsable values fail; negative conditions on missing pass.
        assert_eq!(fail(cond("cpu_limits", Lte, &["4"])).as_deref(), Some("cpu_limits is missing"));
        assert_eq!(fail(cond("env", Gte, &["1"])).as_deref(), Some("env is prod"));
        assert_eq!(fail(cond("cpu_limits", NotEquals, &["0"])), None);
    }

    #[test]
    fn invalid_conditions_are_rejected_with_a_reason() {
        let check = |c: PolicyCondition| CompiledPolicy::new(&PolicyTarget { resource_type: CatalogType::Host, scope: String::new(), conditions: vec![c] }).err();
        use PolicyOperator::*;
        assert!(check(cond("team", Exists, &[])).is_none());
        assert!(check(cond("team", Exists, &["x"])).unwrap().contains("takes no value"));
        assert!(check(cond("team", Equals, &[])).unwrap().contains("takes one value"));
        assert!(check(cond("replicas", Gte, &["two"])).unwrap().contains("not a number"));
        assert!(check(cond("version", VersionGte, &["new"])).unwrap().contains("not a version"));
        assert!(check(cond("name", Matches, &["("])).unwrap().contains("invalid pattern"));
        assert!(check(cond("bad key!", Exists, &[])).unwrap().contains("not a valid attribute"));
        assert!(CompiledPolicy::new(&PolicyTarget { resource_type: CatalogType::Host, scope: String::new(), conditions: vec![] }).is_err());
    }

    #[test]
    fn scope_and_page_filters_narrow_the_evaluated_resources() {
        let all = vec![
            res(CatalogType::KubeDeployment, "prod-1", &["env:prod"], &[("replicas", "1")]),
            res(CatalogType::KubeDeployment, "prod-3", &["env:prod", "team:payments"], &[("replicas", "3")]),
            res(CatalogType::KubeDeployment, "staging-1", &["env:staging"], &[("replicas", "1")]),
            res(CatalogType::KubeNode, "node", &["env:prod"], &[]),
        ];
        let p = policy(CatalogType::KubeDeployment, "env:prod", vec![cond("replicas", PolicyOperator::Gte, &["2"])]);
        let (e, v) = evaluate(&p, &all, &[]);
        assert_eq!((e.evaluated, e.noncompliant, e.score), (2, 1, Some(50.0)));
        assert_eq!(v, vec![PolicyViolation { resource_id: "prod-1".into(), reason: "replicas is 1".into() }]);
        // The page's filters apply on top of the scope.
        let (e, _) = evaluate(&p, &all, &parse_query("team:payments"));
        assert_eq!((e.evaluated, e.noncompliant, e.score), (1, 0, Some(100.0)));
        // Scopes may use attributes and wildcards.
        let p = policy(CatalogType::KubeDeployment, "-replicas:3 cluster:shop-*", vec![cond("env", PolicyOperator::Equals, &["prod"])]);
        assert_eq!(evaluate(&p, &all, &[]).0.evaluated, 2);
        // Nothing in scope: no score.
        let p = policy(CatalogType::Host, "", vec![cond("team", PolicyOperator::Exists, &[])]);
        assert_eq!(evaluate(&p, &all, &[]).0, PolicyEvaluation { evaluated: 0, noncompliant: 0, score: None });
    }

    #[test]
    fn scores_round_to_one_decimal() {
        assert_eq!(score(3, 1), Some(66.7));
        assert_eq!(score(3, 0), Some(100.0));
        assert_eq!(score(7, 7), Some(0.0));
        assert_eq!(score(0, 0), None);
    }

    #[test]
    fn templates_are_valid_and_unique() {
        let mut keys = std::collections::HashSet::new();
        for t in templates() {
            assert!(keys.insert(t.key), "duplicate template {}", t.key);
            assert!(t.priority <= 4);
            CompiledPolicy::new(&t.target).unwrap_or_else(|e| panic!("{}: {e}", t.key));
        }
        let tags = templates().into_iter().find(|t| t.key == "kube_no_latest_tag").unwrap();
        let p = CompiledPolicy::new(&tags.target).unwrap();
        assert!(p.failure(&res(CatalogType::KubeDeployment, "x", &[], &[("image_tags", "latest")])).is_some());
        assert!(p.failure(&res(CatalogType::KubeDeployment, "x", &[], &[("image_tags", "2.4, digest")])).is_none());
    }

    #[test]
    fn categories_round_trip() {
        for c in [PolicyCategory::CostOptimization, PolicyCategory::Reliability, PolicyCategory::Security, PolicyCategory::Versioning, PolicyCategory::Tagging] {
            assert_eq!(PolicyCategory::parse(c.as_str()), Some(c));
        }
        assert_eq!(PolicyCategory::parse("other"), None);
        assert_eq!(type_str(CatalogType::KubeDeployment), "kube_deployment");
        assert_eq!(cond("replicas", PolicyOperator::Gte, &["2"]).describe(), "replicas ≥ 2");
    }
}
