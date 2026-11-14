//! Tagging policies (docs/adr/0008-remediation-and-provenance.md, decision 2): which tags the resources of a scope
//! must carry, and with which values. Pure functions over [`TaggedResource`]s; [`super::tagging_policies`] stores the
//! policies, their daily coverage and serves the API.
//!
//! A policy targets cloud resources, Kubernetes objects, images or services, narrowed by a scope (cloud provider,
//! account, cluster, resource type, names; each a comma-separated list of `*` patterns, empty for any). Every
//! required tag must be set; a key may list alternatives (`tf_repo|git_repo`: our Terraform tags or Yor's), and
//! allowed values, when given, restrict its values (without case). Keys compare as `provenance::norm_key` does.

use std::collections::BTreeMap;

use oc_query::glob;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::provenance::{tag_values, TagTarget, TaggedResource};
use super::resource_policy_rules::score;

pub const MAX_REQUIRED: usize = 20;
pub const MAX_ALLOWED: usize = 50;
const MAX_KEY_CHARS: usize = 300;
const MAX_VALUE_CHARS: usize = 200;
const MAX_SCOPE_CHARS: usize = 1000;

/// A tag every resource in scope must carry.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequiredTag {
    /// The key, or alternatives separated by `|` (`tf_path|git_file`).
    pub key: String,
    /// Allowed values; any value when empty.
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

/// Which resources of the target a policy covers. Each field is a comma-separated list of `*` patterns; empty for
/// any.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TagScope {
    /// aws | azure | gcp (cloud resources).
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub cluster: String,
    /// `AwsS3Bucket`, `kube_deployment`, …
    #[serde(default)]
    pub resource_type: String,
    /// Resource, image or service names.
    #[serde(default)]
    pub names: String,
}

/// What a policy checks.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagRule {
    pub target: TagTarget,
    #[serde(default)]
    pub scope: TagScope,
    pub required: Vec<RequiredTag>,
}

/// A required tag a resource lacks, or carries with a value not allowed.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagGap {
    pub key: String,
    /// The value found when it isn't allowed; null when the tag is missing.
    pub value: Option<String>,
}

impl TagGap {
    /// `tf_repo missing`, `env=production not allowed`.
    pub fn describe(&self) -> String {
        match &self.value {
            None => format!("{} missing", self.key),
            Some(v) => format!("{}={v} not allowed", self.key),
        }
    }
}

/// A rule checked and ready to evaluate.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    rule: TagRule,
    scope: [Vec<String>; 5],
}

fn patterns(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

fn in_scope(patterns: &[String], value: Option<&str>) -> bool {
    patterns.is_empty() || value.is_some_and(|v| patterns.iter().any(|p| glob(p, v)))
}

impl CompiledRule {
    pub fn new(rule: &TagRule) -> Result<CompiledRule, String> {
        if rule.required.is_empty() {
            return Err("A policy needs at least one required tag".into());
        }
        if rule.required.len() > MAX_REQUIRED {
            return Err(format!("A policy can require at most {MAX_REQUIRED} tags"));
        }
        let s = &rule.scope;
        if [&s.provider, &s.account, &s.cluster, &s.resource_type, &s.names].iter().any(|f| f.chars().count() > MAX_SCOPE_CHARS) {
            return Err("The scope is too long".into());
        }
        let mut required = Vec::new();
        for t in &rule.required {
            let alternatives: Vec<&str> = t.key.split('|').map(str::trim).filter(|k| !k.is_empty()).collect();
            if alternatives.is_empty() || t.key.chars().count() > MAX_KEY_CHARS || alternatives.iter().any(|k| k.contains(':') || k.contains(char::is_whitespace)) {
                return Err(format!("'{}' is not a valid tag key", t.key.trim()));
            }
            let values: Vec<String> = t.allowed_values.iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
            if values.len() > MAX_ALLOWED || values.iter().any(|v| v.chars().count() > MAX_VALUE_CHARS) {
                return Err(format!("{}: at most {MAX_ALLOWED} allowed values of {MAX_VALUE_CHARS} characters", alternatives.join("|")));
            }
            required.push(RequiredTag { key: alternatives.join("|"), allowed_values: values });
        }
        let scope = [patterns(&s.provider), patterns(&s.account), patterns(&s.cluster), patterns(&s.resource_type), patterns(&s.names)];
        let rule = TagRule { target: rule.target, scope: TagScope { provider: s.provider.trim().into(), account: s.account.trim().into(), cluster: s.cluster.trim().into(), resource_type: s.resource_type.trim().into(), names: s.names.trim().into() }, required };
        Ok(CompiledRule { rule, scope })
    }

    /// The rule as checked (keys and values trimmed).
    pub fn rule(&self) -> &TagRule {
        &self.rule
    }

    /// The resource is of the rule's target and in its scope.
    pub fn applies(&self, r: &TaggedResource) -> bool {
        let [provider, account, cluster, resource_type, names] = &self.scope;
        r.target == self.rule.target
            && in_scope(provider, r.provider.as_deref())
            && in_scope(account, r.account.as_deref())
            && in_scope(cluster, r.cluster.as_deref())
            && in_scope(resource_type, Some(&r.resource_type))
            && in_scope(names, Some(&r.name))
    }

    /// The required tags the resource lacks or carries with values not allowed; empty when it complies.
    pub fn gaps(&self, r: &TaggedResource) -> Vec<TagGap> {
        let mut out = Vec::new();
        for t in &self.rule.required {
            let values: Vec<&str> = t.key.split('|').flat_map(|k| tag_values(&r.tags, k)).collect();
            if values.is_empty() {
                out.push(TagGap { key: t.key.clone(), value: None });
            } else if !t.allowed_values.is_empty() {
                if let Some(bad) = values.iter().find(|v| !t.allowed_values.iter().any(|a| a.eq_ignore_ascii_case(v))) {
                    out.push(TagGap { key: t.key.clone(), value: Some(bad.to_string()) });
                }
            }
        }
        out
    }
}

/// Resources evaluated and compliant, and the compliant share in percent (one decimal; null when nothing is
/// evaluated).
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CoverageCount {
    pub evaluated: usize,
    pub compliant: usize,
    pub score: Option<f64>,
}

impl CoverageCount {
    pub fn of(evaluated: usize, compliant: usize) -> CoverageCount {
        CoverageCount { evaluated, compliant, score: score(evaluated, evaluated - compliant.min(evaluated)) }
    }

    fn add(&mut self, compliant: bool) {
        *self = CoverageCount::of(self.evaluated + 1, self.compliant + usize::from(compliant));
    }
}

/// Coverage of one policy, account, cluster, team or target.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagCoverageRow {
    /// The policy id, account, cluster, team or target.
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub count: CoverageCount,
}

/// One policy's gaps on a resource.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyGaps {
    pub policy_id: String,
    pub policy_name: String,
    pub gaps: Vec<TagGap>,
}

/// A resource failing at least one policy, with what it lacks and where it comes from.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NonCompliantResource {
    pub resource: TaggedResource,
    pub policies: Vec<PolicyGaps>,
}

/// Compliance of the org's resources with its enabled policies.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// Resources at least one policy applies to, and those complying with all of theirs.
    pub total: CoverageCount,
    /// Evaluated resources with a known source.
    pub with_provenance: usize,
    pub by_policy: Vec<TagCoverageRow>,
    pub by_target: Vec<TagCoverageRow>,
    pub by_account: Vec<TagCoverageRow>,
    pub by_cluster: Vec<TagCoverageRow>,
    pub by_team: Vec<TagCoverageRow>,
    /// Failing resources, those of the lowest-scoring groups first (at most the limit).
    pub noncompliant: Vec<NonCompliantResource>,
    pub noncompliant_total: usize,
}

/// A policy as evaluated: its id, name and rule.
pub struct EvaluatedPolicy<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub rule: &'a CompiledRule,
}

pub const NO_TEAM: &str = "(no team)";

fn rows(m: BTreeMap<String, CoverageCount>) -> Vec<TagCoverageRow> {
    let mut out: Vec<TagCoverageRow> = m.into_iter().map(|(key, count)| TagCoverageRow { label: key.clone(), key, count }).collect();
    // Worst first: the lowest score, then the most resources.
    out.sort_by(|a, b| a.count.score.unwrap_or(100.0).total_cmp(&b.count.score.unwrap_or(100.0)).then(b.count.evaluated.cmp(&a.count.evaluated)).then(a.key.cmp(&b.key)));
    out
}

/// Evaluates every policy on every resource: coverage per policy, target, account, cluster and team, and up to
/// `limit` non-compliant resources.
pub fn coverage(policies: &[EvaluatedPolicy], resources: &[TaggedResource], limit: usize) -> Coverage {
    let mut c = Coverage::default();
    let mut by_policy: Vec<CoverageCount> = vec![CoverageCount::default(); policies.len()];
    let (mut by_target, mut by_account, mut by_cluster, mut by_team) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for r in resources {
        let mut applied = false;
        let mut failing = Vec::new();
        for (i, p) in policies.iter().enumerate().filter(|(_, p)| p.rule.applies(r)) {
            applied = true;
            let gaps = p.rule.gaps(r);
            by_policy[i].add(gaps.is_empty());
            if !gaps.is_empty() {
                failing.push(PolicyGaps { policy_id: p.id.into(), policy_name: p.name.into(), gaps });
            }
        }
        if !applied {
            continue;
        }
        let ok = failing.is_empty();
        c.total.add(ok);
        c.with_provenance += usize::from(r.provenance.is_some());
        by_target.entry(r.target.as_str().to_string()).or_insert_with(CoverageCount::default).add(ok);
        if let Some(a) = &r.account {
            by_account.entry(a.clone()).or_insert_with(CoverageCount::default).add(ok);
        }
        if let Some(cl) = &r.cluster {
            by_cluster.entry(cl.clone()).or_insert_with(CoverageCount::default).add(ok);
        }
        by_team.entry(r.team.clone().unwrap_or_else(|| NO_TEAM.into())).or_insert_with(CoverageCount::default).add(ok);
        if !ok {
            c.noncompliant_total += 1;
            if c.noncompliant.len() < limit {
                c.noncompliant.push(NonCompliantResource { resource: r.clone(), policies: failing });
            }
        }
    }
    c.by_policy = policies.iter().zip(by_policy).map(|(p, count)| TagCoverageRow { key: p.id.into(), label: p.name.into(), count }).collect();
    c.by_target = rows(by_target);
    c.by_account = rows(by_account);
    c.by_cluster = rows(by_cluster);
    c.by_team = rows(by_team);
    c
}

/// A policy of the recommended set.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedPolicy {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub rule: TagRule,
}

fn req(key: &str, allowed: &[&str]) -> RequiredTag {
    RequiredTag { key: key.into(), allowed_values: allowed.iter().map(|v| v.to_string()).collect() }
}

fn recommended(key: &'static str, name: &'static str, description: &'static str, target: TagTarget, required: Vec<RequiredTag>) -> RecommendedPolicy {
    RecommendedPolicy { key, name, description, rule: TagRule { target, scope: TagScope::default(), required } }
}

/// The recommended set installed for every org (and offered again in the editor): unified service tagging and git
/// metadata for services, the Terraform source for cloud resources, Argo CD tracking for Kubernetes objects and the
/// OCI source labels for images.
pub fn recommended_set() -> Vec<RecommendedPolicy> {
    vec![
        recommended(
            "services_unified_tagging",
            "Services carry unified service tags and git metadata",
            "service, env and version link a service's traces, logs and metrics; the repository and commit (DD_GIT_REPOSITORY_URL, DD_GIT_COMMIT_SHA) tell where to fix it.",
            TagTarget::Service,
            vec![req("service", &[]), req("env", &[]), req("version", &[]), req("git.repository_url", &[]), req("git.commit.sha", &[])],
        ),
        recommended(
            "cloud_terraform_source",
            "Cloud resources name their Terraform source",
            "managed-by, and the repository and module path (tf_repo, tf_path, or Yor's git_repo, git_file) on every resource Terraform creates.",
            TagTarget::Cloud,
            vec![req("managed-by", &[]), req("tf_repo|git_repo", &[]), req("tf_path|git_file", &[])],
        ),
        recommended(
            "cloud_ownership",
            "Cloud resources have an owning team and an environment",
            "Alerts, findings and costs reach the right people when every resource carries team and env.",
            TagTarget::Cloud,
            vec![req("team", &[]), req("env", &["prod", "staging", "dev"])],
        ),
        recommended(
            "kubernetes_argocd_tracking",
            "Kubernetes objects are tracked by Argo CD",
            "Argo CD's tracking annotation (or label) names the Application whose repository, path and revision define the object.",
            TagTarget::Kubernetes,
            vec![req("argocd.argoproj.io/tracking-id|app.kubernetes.io/instance", &[])],
        ),
        recommended(
            "images_oci_source",
            "Images carry their OCI source and revision",
            "The org.opencontainers.image.source and .revision labels, set at build time, name the repository and commit an image was built from.",
            TagTarget::Image,
            vec![req("org.opencontainers.image.source|git.repository_url", &[]), req("org.opencontainers.image.revision|git.commit.sha", &[])],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::provenance::{from_tags, TaggedResource};

    fn res(target: TagTarget, name: &str, tags: &[&str]) -> TaggedResource {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        let first = |k: &str| tag_values(&tags, k).first().map(|v| v.to_string());
        TaggedResource {
            id: format!("id-{name}"),
            target,
            name: name.into(),
            resource_type: if target == TagTarget::Cloud { "AwsS3Bucket".into() } else { "kube_deployment".into() },
            provider: (target == TagTarget::Cloud).then(|| "aws".into()),
            account: first("account"),
            region: None,
            cluster: first("cluster"),
            namespace: None,
            env: first("env"),
            team: first("team"),
            provenance: from_tags(&tags),
            tags,
        }
    }

    fn rule(target: TagTarget, scope: TagScope, required: Vec<RequiredTag>) -> CompiledRule {
        CompiledRule::new(&TagRule { target, scope, required }).unwrap()
    }

    #[test]
    fn required_tags_need_a_value_from_any_alternative() {
        let r = rule(TagTarget::Cloud, TagScope::default(), vec![req("managed-by", &[]), req("tf_repo|git_repo", &[]), req("tf_path|git_file", &[])]);
        assert_eq!(r.gaps(&res(TagTarget::Cloud, "a", &["Managed_By:terraform", "tf_repo:example-org/infra-terraform", "tf_path:envs/prod"])), vec![]);
        // Yor's tags satisfy the alternatives.
        assert_eq!(r.gaps(&res(TagTarget::Cloud, "b", &["managed-by:terraform", "git_repo:infra-terraform", "git_file:main.tf"])), vec![]);
        let gaps = r.gaps(&res(TagTarget::Cloud, "c", &["managed-by:terraform", "tf_repo:"]));
        assert_eq!(gaps, vec![TagGap { key: "tf_repo|git_repo".into(), value: None }, TagGap { key: "tf_path|git_file".into(), value: None }]);
        assert_eq!(gaps[0].describe(), "tf_repo|git_repo missing");
    }

    #[test]
    fn allowed_values_ignore_case_and_report_the_bad_value() {
        let r = rule(TagTarget::Cloud, TagScope::default(), vec![req("env", &["prod", "staging"])]);
        assert!(r.gaps(&res(TagTarget::Cloud, "a", &["env:PROD"])).is_empty());
        let g = r.gaps(&res(TagTarget::Cloud, "b", &["env:production"]));
        assert_eq!(g, vec![TagGap { key: "env".into(), value: Some("production".into()) }]);
        assert_eq!(g[0].describe(), "env=production not allowed");
        assert_eq!(r.gaps(&res(TagTarget::Cloud, "c", &[])), vec![TagGap { key: "env".into(), value: None }]);
    }

    #[test]
    fn scopes_match_target_provider_account_cluster_type_and_names() {
        let scoped = |scope: TagScope| rule(TagTarget::Cloud, scope, vec![req("team", &[])]);
        let a = res(TagTarget::Cloud, "example-shop-assets", &["account:123456789012"]);
        assert!(scoped(TagScope::default()).applies(&a));
        assert!(!rule(TagTarget::Service, TagScope::default(), vec![req("team", &[])]).applies(&a));
        assert!(scoped(TagScope { provider: "aws".into(), account: "1234*, 999".into(), ..Default::default() }).applies(&a));
        assert!(!scoped(TagScope { account: "999".into(), ..Default::default() }).applies(&a));
        assert!(scoped(TagScope { resource_type: "aws*bucket".into(), names: "example-shop-*".into(), ..Default::default() }).applies(&a));
        // A pattern on a field the resource lacks leaves it out.
        assert!(!scoped(TagScope { cluster: "shop-*".into(), ..Default::default() }).applies(&a));
    }

    #[test]
    fn invalid_rules_are_rejected() {
        let check = |required: Vec<RequiredTag>| CompiledRule::new(&TagRule { target: TagTarget::Cloud, scope: TagScope::default(), required }).err();
        assert!(check(vec![]).unwrap().contains("at least one"));
        assert!(check(vec![req(" | ", &[])]).unwrap().contains("not a valid tag key"));
        assert!(check(vec![req("team:x", &[])]).unwrap().contains("not a valid tag key"));
        assert!(check(vec![req("cost center", &[])]).is_some());
        assert!(check((0..=MAX_REQUIRED).map(|i| req(&format!("k{i}"), &[])).collect()).unwrap().contains("at most"));
        let many: Vec<String> = (0..=MAX_ALLOWED).map(|i| format!("v{i}")).collect();
        assert!(check(vec![RequiredTag { key: "env".into(), allowed_values: many }]).is_some());
        // Keys and values are trimmed; blank values dropped.
        let r = CompiledRule::new(&TagRule { target: TagTarget::Cloud, scope: TagScope { account: " 1234 ".into(), ..Default::default() }, required: vec![req(" tf_repo | git_repo ", &[" prod ", ""])] }).unwrap();
        assert_eq!(r.rule().required, vec![req("tf_repo|git_repo", &["prod"])]);
        assert_eq!(r.rule().scope.account, "1234");
    }

    #[test]
    fn coverage_counts_per_policy_target_account_cluster_and_team() {
        let owners = rule(TagTarget::Cloud, TagScope::default(), vec![req("team", &[])]);
        let source = rule(TagTarget::Cloud, TagScope { account: "111".into(), ..Default::default() }, vec![req("tf_repo", &[])]);
        let argo = rule(TagTarget::Kubernetes, TagScope::default(), vec![req("argocd.argoproj.io/tracking-id", &[])]);
        let policies = [EvaluatedPolicy { id: "p1", name: "Owners", rule: &owners }, EvaluatedPolicy { id: "p2", name: "Source", rule: &source }, EvaluatedPolicy { id: "p3", name: "Argo", rule: &argo }];
        let resources = vec![
            res(TagTarget::Cloud, "a", &["account:111", "team:storefront", "tf_repo:example-org/infra-terraform"]),
            res(TagTarget::Cloud, "b", &["account:111", "team:storefront"]),
            res(TagTarget::Cloud, "c", &["account:222"]),
            res(TagTarget::Kubernetes, "d", &["cluster:shop-prod-eu", "team:checkout", "argocd.argoproj.io/tracking-id:shop:apps/Deployment:checkout/d"]),
            res(TagTarget::Image, "e", &[]),
        ];
        let c = coverage(&policies, &resources, 1);
        // The image is in no policy's scope.
        assert_eq!(c.total, CoverageCount { evaluated: 4, compliant: 2, score: Some(50.0) });
        assert_eq!(c.with_provenance, 1);
        let count = |rows: &[TagCoverageRow], key: &str| rows.iter().find(|r| r.key == key).map(|r| (r.count.evaluated, r.count.compliant)).unwrap();
        assert_eq!(count(&c.by_policy, "p1"), (3, 2));
        assert_eq!(count(&c.by_policy, "p2"), (2, 1));
        assert_eq!(count(&c.by_policy, "p3"), (1, 1));
        assert_eq!(count(&c.by_target, "cloud"), (3, 1));
        assert_eq!(count(&c.by_account, "111"), (2, 1));
        assert_eq!(count(&c.by_account, "222"), (1, 0));
        assert_eq!(count(&c.by_cluster, "shop-prod-eu"), (1, 1));
        assert_eq!(count(&c.by_team, NO_TEAM), (1, 0));
        // Worst groups first.
        assert_eq!(c.by_account[0].key, "222");
        // Failing resources up to the limit, with each policy's gaps.
        assert_eq!((c.noncompliant_total, c.noncompliant.len()), (2, 1));
        assert_eq!(c.noncompliant[0].resource.name, "b");
        assert_eq!(c.noncompliant[0].policies, vec![PolicyGaps { policy_id: "p2".into(), policy_name: "Source".into(), gaps: vec![TagGap { key: "tf_repo".into(), value: None }] }]);
    }

    #[test]
    fn no_policies_or_resources_have_no_score() {
        let c = coverage(&[], &[res(TagTarget::Cloud, "a", &[])], 10);
        assert_eq!(c.total, CoverageCount::default());
        assert!(c.by_team.is_empty() && c.noncompliant.is_empty());
    }

    #[test]
    fn the_recommended_set_is_valid_and_recognises_each_convention() {
        let set = recommended_set();
        let mut keys = std::collections::HashSet::new();
        for p in &set {
            assert!(keys.insert(p.key), "duplicate {}", p.key);
            CompiledRule::new(&p.rule).unwrap_or_else(|e| panic!("{}: {e}", p.key));
        }
        let get = |k: &str| CompiledRule::new(&set.iter().find(|p| p.key == k).unwrap().rule).unwrap();
        let mut img = res(TagTarget::Image, "registry.example.com/billing:3.2.1", &["org.opencontainers.image.source:https://github.com/example-org/billing", "git.commit.sha:ab12"]);
        assert!(get("images_oci_source").gaps(&img).is_empty());
        img.tags.pop();
        assert_eq!(get("images_oci_source").gaps(&img).len(), 1);
        let dep = res(TagTarget::Kubernetes, "order-service", &["app.kubernetes.io/instance:order-service"]);
        assert!(get("kubernetes_argocd_tracking").gaps(&dep).is_empty());
    }
}
