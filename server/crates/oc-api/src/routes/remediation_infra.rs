//! Fix in infrastructure (ADR-0008 decision 4, level 3) and tagging with Assist (decision 5): which level a cloud
//! resource, Kubernetes object or image finding gets from its provenance, what tags it lacks and the snippet that adds
//! them, where its definition is in the source (the Terraform resource by address or name, the Helm values of the Argo CD
//! Application, its manifests, the Dockerfile), the small deterministic editors (S3 public access block, the AWS
//! provider's `default_tags`, OCI labels) and the tools a `fix_infra` / `add_tags` run reads the repository with.
//! Everything but [`InfraTools`] is pure.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::provenance::{norm_key, Confidence, Provenance, ProvenanceKind, ProvenanceVia, TagTarget, TaggedResource, OCI_SOURCE};
use super::remediation::RepoFiles;
use super::remediation_model::Level;
use super::tagging_rules::{recommended_set, CompiledRule};
use crate::assist::agent::Tools;
use crate::assist::code::check_path;
use crate::assist::llm::ToolSpec;

/// What a cloud finding is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subject {
    Cloud,
    Kubernetes,
    Image,
    Host,
}

impl Subject {
    pub fn target(self) -> Option<TagTarget> {
        match self {
            Subject::Cloud => Some(TagTarget::Cloud),
            Subject::Kubernetes => Some(TagTarget::Kubernetes),
            Subject::Image => Some(TagTarget::Image),
            Subject::Host => None,
        }
    }

    /// A misconfiguration's resource type (`kube_deployment`, `host`, `AwsS3Bucket`) or a vulnerability's kind.
    pub fn of_type(resource_type: &str) -> Subject {
        match resource_type {
            "host" => Subject::Host,
            "container_image" => Subject::Image,
            t if t.starts_with("kube_") => Subject::Kubernetes,
            _ => Subject::Cloud,
        }
    }
}

/// The level of a cloud finding and what it rests on.
#[derive(Debug, Clone, PartialEq)]
pub struct InfraLevel {
    pub level: Level,
    pub reason: String,
    /// The resolved source (also shown when no connection reaches it).
    pub source: Option<Provenance>,
    /// Inferred provenance: the person confirms it before Assist edits.
    pub confirm: bool,
    /// Why no connection reaches the source's repository.
    pub connection: Option<String>,
}

/// Fix in infrastructure when the provenance names a repository and a path (images: the repository) that one of the
/// org's connections reaches; else what is missing.
pub fn infra_level(subject: Subject, provenance: Option<&Provenance>, connection: impl Fn(&str) -> Result<(), String>) -> InfraLevel {
    let plain = |level, reason: String, source: Option<&Provenance>| InfraLevel { level, reason, source: source.cloned(), confirm: false, connection: None };
    if subject == Subject::Host {
        return plain(Level::Explain, "A host's packages are upgraded on the host, or in the machine image it boots from: no repository is known to define them.".into(), None);
    }
    let Some(p) = provenance else {
        return plain(
            Level::NeedsProvenance,
            "No tag or label on the resource names the repository and file that define it, so Assist can explain the finding but not fix it at its source. Add the tags below where the resource is defined; Tagging Policies list them.".into(),
            None,
        );
    };
    let app = p.application.as_deref().unwrap_or_default();
    let Some(repo) = p.repo.as_deref() else {
        return plain(
            Level::NeedsProvenance,
            format!("Argo CD Application `{app}` deploys this object, but the Application itself isn't collected, so its repository and path are unknown. Collect Argo CD Applications with the Cluster Agent (orchestrator custom resources: argoproj.io/v1alpha1/applications)."),
            Some(p),
        );
    };
    let path = p.path.as_deref().map(str::trim).filter(|x| !x.is_empty());
    if path.is_none() && p.kind != ProvenanceKind::Dockerfile {
        let why = match p.kind {
            ProvenanceKind::Terraform => format!("The resource names its repository ({repo}) but not its module path: add tf_path (or Yor's git_file) so Assist knows which Terraform files to open."),
            _ => format!("The source names {repo} but no path in it, so Assist can't tell which files define the resource."),
        };
        return plain(Level::NeedsProvenance, why, Some(p));
    }
    if p.kind == ProvenanceKind::Code {
        return plain(Level::Explain, "The source is a service's code repository, not the infrastructure code that defines the resource.".into(), Some(p));
    }
    if let Err(e) = connection(repo) {
        return InfraLevel {
            level: Level::Explain,
            reason: format!("The resource comes from {repo}, which none of the organization's source code connections reaches, so Assist can explain the finding but not open its source."),
            source: Some(p.clone()),
            confirm: false,
            connection: Some(e),
        };
    }
    let at = path.map(|x| format!("{repo}/{}", x.trim_start_matches("./"))).unwrap_or_else(|| repo.to_string());
    let of_app = if app.is_empty() { String::new() } else { format!(" of Argo CD Application {app}") };
    let mut reason = match p.kind {
        ProvenanceKind::Terraform => format!("Assist opens the Terraform resource{} in {at} and proposes the change.", p.address.as_ref().map(|a| format!(" {a}")).unwrap_or_default()),
        ProvenanceKind::Helm => format!("Assist opens the Helm values{of_app} in {at} and proposes the change."),
        ProvenanceKind::Kustomize | ProvenanceKind::Manifest => format!("Assist opens the manifests{of_app} in {at} and proposes the change."),
        _ => format!("Assist opens the Dockerfile in {at} and proposes a newer base image that fixes the image's OS packages; the image must then be rebuilt and redeployed."),
    };
    let inferred = p.confidence == Confidence::Inferred;
    if inferred {
        reason.push_str(" The source was inferred, not named by the resource: confirm it before Assist edits it.");
    }
    InfraLevel { level: Level::FixInInfra, reason, source: Some(p.clone()), confirm: inferred, connection: None }
}

/// The place the run may read and change: the provenance's directory (a file's parent).
pub fn scope_dir(p: &Provenance) -> String {
    let path = p.path.as_deref().unwrap_or("").trim().trim_start_matches("./").trim_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    let file = [".tf", ".tf.json", ".yaml", ".yml", ".json", ".hcl"].iter().any(|e| last.ends_with(e)) || last.to_ascii_lowercase().contains("dockerfile");
    if file {
        path.rsplit_once('/').map_or("", |(d, _)| d).to_string()
    } else {
        path.to_string()
    }
}

/// `path` is in one of `dirs` (an empty dir is the repository root).
pub fn under(path: &str, dirs: &[String]) -> bool {
    check_path(path).is_ok() && dirs.iter().any(|d| d.is_empty() || path == d || path.starts_with(&format!("{d}/")))
}

// ================================================================ missing tags and their snippet

/// A provenance tag a resource lacks, per the policy requiring it.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MissingTag {
    pub policy: String,
    /// The key or its alternatives (`tf_repo|git_repo`).
    pub key: String,
    /// The value found when it isn't allowed.
    pub value: Option<String>,
}

/// How to add the missing tags where the resource is defined.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceSnippet {
    pub title: String,
    pub language: String,
    pub code: String,
}

/// Tag keys that say where a resource comes from.
const PROVENANCE_KEYS: [&str; 12] = [
    "tf_repo",
    "tf_path",
    "git_repo",
    "git_file",
    "git_org",
    "argocd.argoproj.io/tracking-id",
    "app.kubernetes.io/instance",
    "argocd.argoproj.io/instance",
    OCI_SOURCE,
    "org.opencontainers.image.revision",
    "git.repository_url",
    "git.commit.sha",
];

fn about_provenance(key: &str) -> bool {
    key.split('|').any(|k| PROVENANCE_KEYS.iter().any(|p| norm_key(p) == norm_key(k)))
}

/// The provenance tags the resource lacks per the org's enabled policies applying to it, else per the recommended set.
pub fn missing_tags(target: TagTarget, resource: Option<&TaggedResource>, policies: &[(String, CompiledRule)]) -> Vec<MissingTag> {
    let mut out: Vec<MissingTag> = Vec::new();
    let mut push = |policy: &str, key: String, value: Option<String>| {
        if about_provenance(&key) && !out.iter().any(|m| m.key == key) {
            out.push(MissingTag { policy: policy.into(), key, value });
        }
    };
    let mut covered = false;
    if let Some(r) = resource {
        for (name, rule) in policies.iter().filter(|(_, c)| c.applies(r)) {
            covered |= rule.rule().required.iter().any(|t| about_provenance(&t.key));
            for g in rule.gaps(r) {
                push(name, g.key, g.value);
            }
        }
    }
    if !covered {
        for p in recommended_set().into_iter().filter(|p| p.rule.target == target) {
            let Ok(c) = CompiledRule::new(&p.rule) else { continue };
            let gaps: Vec<(String, Option<String>)> = match resource {
                Some(r) => c.gaps(r).into_iter().map(|g| (g.key, g.value)).collect(),
                None => c.rule().required.iter().map(|t| (t.key.clone(), None)).collect(),
            };
            for (key, value) in gaps {
                push(p.name, key, value);
            }
        }
    }
    out
}

const TF_TAGS: &str = "managed-by = \"terraform\"\n      tf_repo    = \"github.com/<owner>/<repository>\"\n      tf_path    = \"<this root module's directory>\"";

/// How to add the missing tags where the resource is defined, for its kind (Terraform per cloud, Argo CD, the image build).
pub fn provenance_snippet(target: TagTarget, resource: Option<&TaggedResource>, missing: &[MissingTag]) -> ProvenanceSnippet {
    let missing = if missing.is_empty() { String::new() } else { format!("Missing: {}\n", missing.iter().map(|m| m.key.as_str()).collect::<Vec<_>>().join(", ")) };
    let hash = |s: &str| if s.is_empty() { String::new() } else { format!("# {s}") };
    let snippet = |title: &str, language: &str, code: String| ProvenanceSnippet { title: title.into(), language: language.into(), code };
    match target {
        TagTarget::Cloud => match resource.and_then(|r| r.provider.as_deref()) {
            Some("azure") | Some("gcp") => {
                let (attr, title) = if resource.and_then(|r| r.provider.as_deref()) == Some("gcp") {
                    ("labels", "Terraform on Google Cloud: a merged locals map (label values allow only a-z, 0-9, - and _; connect the Terraform state if the repository can't be written)")
                } else {
                    ("tags", "Terraform on Azure: a merged locals map (the azurerm provider has no default tags)")
                };
                let name = resource.map(|r| r.name.as_str()).unwrap_or("example");
                snippet(
                    title,
                    "hcl",
                    format!("{}locals {{\n  default_tags = {{\n    {}\n  }}\n}}\n\n# In every resource of the module, e.g. {name}:\n  {attr} = merge(local.default_tags, {{ purpose = \"...\" }})\n", hash(&missing), TF_TAGS.replace("\n      ", "\n    ")),
                )
            }
            _ => snippet(
                "Terraform on AWS: the provider's default_tags, once per root module",
                "hcl",
                format!("{}provider \"aws\" {{\n  default_tags {{\n    tags = {{\n      {TF_TAGS}\n    }}\n  }}\n}}\n", hash(&missing)),
            ),
        },
        TagTarget::Kubernetes => snippet(
            "Argo CD: annotation tracking, and the Applications collected by the Cluster Agent",
            "yaml",
            format!(
                "{}# argocd-cm\ndata:\n  application.resourceTrackingMethod: annotation\n\n# Datadog Helm chart values\ndatadog:\n  orchestratorExplorer:\n    enabled: true\n    customResources:\n      - argoproj.io/v1alpha1/applications\n",
                hash(&missing)
            ),
        ),
        TagTarget::Image => {
            let name = resource.map(|r| r.name.as_str()).unwrap_or("registry.example.com/<image>:<tag>");
            snippet(
                "Images: the OCI source and revision labels, set at build time",
                "shell",
                format!("{}docker build \\\n  --label {OCI_SOURCE}=\"https://github.com/<owner>/<repository>\" \\\n  --label org.opencontainers.image.revision=\"$(git rev-parse HEAD)\" \\\n  -t {name} .\n", hash(&missing)),
            )
        }
        TagTarget::Service => snippet(
            "Services: git metadata for the tracers",
            "yaml",
            format!("{}env:\n  - name: DD_GIT_REPOSITORY_URL\n    value: https://github.com/<owner>/<repository>\n  - name: DD_GIT_COMMIT_SHA\n    value: \"<commit, set by CI>\"\n", hash(&missing)),
        ),
    }
}

// ================================================================ Terraform

/// A top-level block of an HCL file.
#[derive(Debug, Clone, PartialEq)]
pub struct HclBlock {
    /// resource | data | module | provider | …
    pub kind: String,
    pub labels: Vec<String>,
    /// Byte offsets of the block's first character and just past its closing brace.
    pub start: usize,
    pub end: usize,
    /// 1-based lines.
    pub start_line: usize,
    pub end_line: usize,
}

fn line_no(src: &str, byte: usize) -> usize {
    src[..byte.min(src.len())].matches('\n').count() + 1
}

fn line_start(src: &str, byte: usize) -> usize {
    src[..byte].rfind('\n').map_or(0, |n| n + 1)
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Past a string's closing quote (`i` at its opening quote); a string doesn't span lines.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            b'\n' => return i,
            _ => i += 1,
        }
    }
    b.len()
}

/// The end of the line closing a heredoc opened at `i` (`<<EOF`, `<<-EOF`).
fn heredoc_end(s: &str, i: usize) -> Option<usize> {
    let rest = &s[i + 2..];
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    let marker: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if marker.is_empty() {
        return None;
    }
    let mut pos = s[i..].find('\n')? + i + 1;
    while pos < s.len() {
        let end = s[pos..].find('\n').map_or(s.len(), |n| pos + n);
        if s[pos..end].trim() == marker {
            return Some(end);
        }
        pos = end + 1;
    }
    None
}

/// Where the next token at `i` ends when it is a string, comment or heredoc (none: an ordinary character).
fn skip_opaque(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    match b[i] {
        b'"' => Some(skip_string(b, i)),
        b'#' => Some(s[i..].find('\n').map_or(s.len(), |n| i + n)),
        b'/' if b.get(i + 1) == Some(&b'/') => Some(s[i..].find('\n').map_or(s.len(), |n| i + n)),
        b'/' if b.get(i + 1) == Some(&b'*') => Some(s[i + 2..].find("*/").map_or(s.len(), |n| i + n + 4)),
        b'<' if b.get(i + 1) == Some(&b'<') => heredoc_end(s, i),
        _ => None,
    }
}

/// Just past the `}` closing the `{` at `open`.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        if let Some(next) = skip_opaque(s, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A block's header line (`resource "aws_s3_bucket" "assets" {`): its kind, labels and the brace's offset in the line.
fn block_header(line: &str) -> Option<(String, Vec<String>, usize)> {
    let t = line.trim_start();
    let mut at = line.len() - t.len();
    let kind: String = t.chars().take_while(|c| ident_char(*c)).collect();
    if kind.is_empty() {
        return None;
    }
    at += kind.len();
    let mut labels = Vec::new();
    loop {
        let rest = &line[at..];
        let trimmed = rest.trim_start();
        at += rest.len() - trimmed.len();
        if trimmed.starts_with('{') {
            return Some((kind, labels, at));
        }
        if let Some(r) = trimmed.strip_prefix('"') {
            let end = r.find('"')?;
            labels.push(r[..end].to_string());
            at += end + 2;
        } else {
            let id: String = trimmed.chars().take_while(|c| ident_char(*c)).collect();
            if id.is_empty() {
                return None;
            }
            at += id.len();
            labels.push(id);
        }
    }
}

/// The blocks between two offsets (a file, or a block's body).
fn blocks_between(src: &str, from: usize, to: usize) -> Vec<HclBlock> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos < to {
        let line_end = src[pos..to].find('\n').map_or(to, |n| pos + n);
        if let Some((kind, labels, brace)) = block_header(&src[pos..line_end]) {
            if let Some(end) = matching_brace(src, pos + brace).filter(|e| *e <= to) {
                out.push(HclBlock { kind, labels, start: pos, end, start_line: line_no(src, pos), end_line: line_no(src, end - 1) });
                pos = src[end..to].find('\n').map_or(to, |n| end + n + 1);
                continue;
            }
        }
        pos = line_end + 1;
    }
    out
}

pub fn hcl_blocks(src: &str) -> Vec<HclBlock> {
    blocks_between(src, 0, src.len())
}

/// The body of a block, between its braces.
fn body(src: &str, b: &HclBlock) -> (usize, usize) {
    let open = src[b.start..].find('{').map_or(b.start, |n| b.start + n);
    (open + 1, b.end - 1)
}

/// The lines of a body at its own level: nested blocks, maps and heredocs are skipped. `(start, end)` offsets.
fn top_lines(src: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos < to {
        let line_end = src[pos..to].find('\n').map_or(to, |n| pos + n);
        out.push((pos, line_end));
        let mut next = line_end + 1;
        let mut i = pos;
        while i < line_end {
            if src.as_bytes()[i] == b'<' && src.as_bytes().get(i + 1) == Some(&b'<') {
                if let Some(e) = heredoc_end(src, i) {
                    next = e + 1;
                    break;
                }
            }
            if let Some(n) = skip_opaque(src, i) {
                i = n;
                continue;
            }
            if src.as_bytes()[i] == b'{' {
                if let Some(e) = matching_brace(src, i) {
                    if e > line_end {
                        next = src[e..to].find('\n').map_or(to, |n| e + n + 1);
                        break;
                    }
                    i = e;
                    continue;
                }
            }
            i += 1;
        }
        pos = next;
    }
    out
}

/// `key = value` on a line: the key (quotes removed) and the value.
fn key_value(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once('=')?;
    if v.starts_with('=') {
        return None;
    }
    let k = k.trim().trim_matches('"');
    (!k.is_empty() && k.chars().all(|c| ident_char(c) || c == '.' || c == '/')).then(|| (k.to_string(), v.trim().to_string()))
}

/// A top-level attribute's raw value in a block (`aws_s3_bucket.assets.id`, `"example-shop-assets"`).
pub fn attribute(src: &str, b: &HclBlock, key: &str) -> Option<String> {
    let (from, to) = body(src, b);
    top_lines(src, from, to).into_iter().find_map(|(s, e)| key_value(&src[s..e]).filter(|(k, _)| k == key).map(|(_, v)| v))
}

/// A Terraform address: modules, the resource type (none for Yor's `yor_name`) and name, without an index.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    pub modules: Vec<String>,
    pub resource_type: Option<String>,
    pub name: String,
}

pub fn parse_address(a: &str) -> Option<Address> {
    let mut clean = String::new();
    let mut depth = 0;
    for c in a.trim().chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    let parts: Vec<&str> = clean.split('.').collect();
    let mut modules = Vec::new();
    let mut i = 0;
    while i + 1 < parts.len() && parts[i] == "module" {
        modules.push(parts[i + 1].to_string());
        i += 2;
    }
    let ok = |s: &str| !s.is_empty() && s.chars().all(ident_char);
    match &parts[i..] {
        [name] if modules.is_empty() && ok(name) => Some(Address { modules, resource_type: None, name: name.to_string() }),
        [t, name] if ok(t) && ok(name) && *t != "module" => Some(Address { modules, resource_type: Some(t.to_string()), name: name.to_string() }),
        _ => None,
    }
}

/// `rel` from `dir` in the repository; none when it leaves the repository.
fn join_dir(dir: &str, rel: &str) -> Option<String> {
    let mut segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            s => segs.push(s),
        }
    }
    Some(segs.join("/"))
}

/// Local module calls of a file in `dir`: their name and directory in the repository (`source = "../modules/x"`).
pub fn local_modules(src: &str, dir: &str) -> Vec<(String, String)> {
    hcl_blocks(src)
        .into_iter()
        .filter(|b| b.kind == "module" && b.labels.len() == 1)
        .filter_map(|b| {
            let source = attribute(src, &b, "source")?;
            let source = source.trim_matches('"');
            if !(source.starts_with("./") || source.starts_with("../")) {
                return None;
            }
            Some((b.labels[0].clone(), join_dir(dir, source)?)).filter(|(_, d)| !d.is_empty())
        })
        .collect()
}

/// Where the resource is defined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Located {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    /// `aws_s3_bucket.assets` (in its module's file).
    pub address: String,
    /// address | name (a block naming the cloud resource)
    pub by: String,
}

/// A resource block sets an attribute to the cloud resource's name (`bucket = "example-shop-logs"`).
fn names_resource(src: &str, b: &HclBlock, name: &str) -> bool {
    let (from, to) = body(src, b);
    let quoted = format!("\"{name}\"");
    !name.is_empty() && top_lines(src, from, to).into_iter().any(|(s, e)| key_value(&src[s..e]).is_some_and(|(_, v)| v == quoted))
}

/// The resource's block among files: by its address, else by the cloud resource's name in a block's attributes.
pub fn find_resource(files: &[(String, String)], address: Option<&Address>, cloud_name: &str) -> Option<Located> {
    let resources: Vec<(&String, &String, HclBlock)> = files.iter().flat_map(|(p, src)| hcl_blocks(src).into_iter().filter(|b| b.kind == "resource" && b.labels.len() == 2).map(move |b| (p, src, b))).collect();
    let loc = |p: &String, b: &HclBlock, by: &str| Located { file: p.clone(), start_line: b.start_line, end_line: b.end_line, address: format!("{}.{}", b.labels[0], b.labels[1]), by: by.into() };
    if let Some(a) = address {
        let hits: Vec<&(&String, &String, HclBlock)> = resources.iter().filter(|(_, _, b)| b.labels[1] == a.name && a.resource_type.as_ref().is_none_or(|t| *t == b.labels[0])).collect();
        if let Some((p, _, b)) = hits.iter().find(|(_, src, b)| names_resource(src, b, cloud_name)).or(hits.first()) {
            return Some(loc(p, b, "address"));
        }
    }
    resources.iter().find(|(_, src, b)| names_resource(src, b, cloud_name)).map(|(p, _, b)| loc(p, b, "name"))
}

// ================================================================ Helm, manifests, Dockerfiles

fn join(dir: &str, file: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    }
}

/// The values files of a Helm source, the environment's first: the Application's `helm.valueFiles` when it lists
/// them, else `values-<env>.yaml` and the like, then `values.yaml`.
pub fn helm_values_files(listing: &[String], dir: &str, env: Option<&str>, app_value_files: &[String]) -> Vec<String> {
    let listed: Vec<String> = app_value_files.iter().map(|f| f.trim().trim_start_matches("./")).filter(|f| !f.starts_with('$') && !f.contains("..")).map(|f| join(dir, f)).filter(|p| listing.contains(p)).collect();
    if !listed.is_empty() {
        return listed;
    }
    let mut out: Vec<String> = Vec::new();
    let envs: Vec<&str> = match env.map(|e| e.trim().to_ascii_lowercase()) {
        Some(e) if e == "prod" || e == "production" => vec!["prod", "production"],
        Some(e) if e == "staging" || e == "stage" => vec!["staging", "stage"],
        Some(e) if e == "dev" || e == "development" => vec!["dev", "development"],
        Some(e) if !e.is_empty() => vec![env.unwrap_or_default().trim()],
        _ => vec![],
    };
    let candidates = envs.iter().flat_map(|e| [format!("values-{e}.yaml"), format!("values-{e}.yml"), format!("values.{e}.yaml"), format!("values/{e}.yaml"), format!("env/{e}/values.yaml"), format!("environments/{e}/values.yaml")]);
    for f in candidates.chain(["values.yaml".to_string(), "values.yml".to_string()]) {
        let p = join(dir, &f);
        if listing.contains(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn depth(p: &str) -> usize {
    p.matches('/').count()
}

/// YAML files of a kustomize or plain manifest source: `kustomization.yaml` first, shallow before deep.
pub fn yaml_files(listing: &[String], dir: &str) -> Vec<String> {
    let dirs = vec![dir.trim_matches('/').to_string()];
    let mut v: Vec<String> = listing.iter().filter(|p| (p.ends_with(".yaml") || p.ends_with(".yml")) && under(p, &dirs)).cloned().collect();
    let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    v.sort_by_key(|p| (!name(p).starts_with("kustomization."), depth(p), p.clone()));
    v
}

/// A YAML file declares the object (a document with `kind: <kind>` and `name: <name>`).
pub fn declares_object(content: &str, kind: &str, name: &str) -> bool {
    let unquote = |v: &str| v.trim().trim_matches(['"', '\'']).to_string();
    content.split("\n---").any(|doc| {
        let (mut kinded, mut named, mut in_meta, mut meta_indent) = (false, false, false, None::<usize>);
        for l in doc.lines() {
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            let indent = l.len() - l.trim_start().len();
            if indent == 0 {
                in_meta = l.trim_end() == "metadata:";
                meta_indent = None;
                if let Some(v) = l.strip_prefix("kind:") {
                    kinded |= unquote(v) == kind;
                }
            } else if in_meta && *meta_indent.get_or_insert(indent) == indent {
                if let Some(v) = l.trim().strip_prefix("name:") {
                    named |= unquote(v) == name;
                }
            }
        }
        kinded && named
    })
}

fn is_dockerfile(path: &str) -> bool {
    let n = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    n == "dockerfile" || n.starts_with("dockerfile.") || n.ends_with(".dockerfile")
}

/// Dockerfiles under `dir`, the one in a directory named after the image first, then the shallowest.
pub fn dockerfiles(listing: &[String], dir: &str, image: &str) -> Vec<String> {
    let dirs = vec![dir.trim_matches('/').to_string()];
    let repo = image.rsplit('/').next().unwrap_or(image).split([':', '@']).next().unwrap_or("").to_string();
    let named = |p: &str| p.rsplit_once('/').is_some_and(|(d, _)| d.rsplit('/').next() == Some(repo.as_str()));
    let mut v: Vec<String> = listing.iter().filter(|p| is_dockerfile(p) && under(p, &dirs)).cloned().collect();
    v.sort_by_key(|p| (!named(p), depth(p), p.clone()));
    v
}

/// A Dockerfile's base images (`FROM`), stage references left out: (line, image).
pub fn base_images(content: &str) -> Vec<(usize, String)> {
    let mut stages: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let mut words = line.split_whitespace();
        if !words.next().is_some_and(|w| w.eq_ignore_ascii_case("from")) {
            continue;
        }
        let rest: Vec<&str> = words.filter(|w| !w.starts_with("--")).collect();
        let Some(image) = rest.first() else { continue };
        if !stages.iter().any(|s| s.eq_ignore_ascii_case(image)) {
            out.push((i + 1, image.to_string()));
        }
        if rest.len() >= 3 && rest[1].eq_ignore_ascii_case("as") {
            stages.push(rest[2].to_string());
        }
    }
    out
}

/// OS packages are fixed by a newer base image; language packages in the application's manifest.
pub fn os_package(ecosystem: &str) -> bool {
    let base = ecosystem.split(':').next().unwrap_or("").to_ascii_lowercase();
    ecosystem.contains(':')
        || ["debian", "ubuntu", "alpine", "red hat", "redhat", "rhel", "centos", "amazon linux", "rocky", "alma", "wolfi", "chainguard", "suse", "opensuse", "fedora", "oracle"].iter().any(|d| base.starts_with(d))
}

// ================================================================ deterministic editors

/// Prowler's checks fixed by blocking public access on the bucket.
pub const S3_PUBLIC_ACCESS_CHECKS: [&str; 2] = ["s3_bucket_level_public_access_block", "s3_bucket_public_access"];
const PUBLIC_ACCESS_FLAGS: [&str; 4] = ["block_public_acls", "block_public_policy", "ignore_public_acls", "restrict_public_buckets"];

/// Applies `(offset, length, replacement)` edits, the latest first.
fn apply(src: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = src.to_string();
    for (at, len, text) in edits {
        out.replace_range(at..at + len, &text);
    }
    out
}

/// Blocks public access on the bucket at `bucket`: sets the four flags of its `aws_s3_bucket_public_access_block`, or
/// adds that resource after the bucket. `(path, before, after)`.
pub fn block_public_access(files: &[(String, String)], bucket: &Located) -> Result<(String, String, String), String> {
    let name = bucket.address.strip_prefix("aws_s3_bucket.").ok_or_else(|| format!("{} isn't an aws_s3_bucket", bucket.address))?;
    let (_, bsrc) = files.iter().find(|(p, _)| *p == bucket.file).ok_or("The bucket's file wasn't read")?;
    let bblock = hcl_blocks(bsrc).into_iter().find(|b| b.kind == "resource" && b.labels == ["aws_s3_bucket", name]).ok_or("The bucket's block wasn't found")?;
    let literal = attribute(bsrc, &bblock, "bucket");
    let refs = [format!("aws_s3_bucket.{name}.id"), format!("aws_s3_bucket.{name}.bucket")];
    for (path, src) in files {
        for b in hcl_blocks(src).into_iter().filter(|b| b.kind == "resource" && b.labels.first().map(String::as_str) == Some("aws_s3_bucket_public_access_block")) {
            let Some(v) = attribute(src, &b, "bucket") else { continue };
            if !refs.contains(&v) && literal.as_ref() != Some(&v) {
                continue;
            }
            let (from, to) = body(src, &b);
            let lines = top_lines(src, from, to);
            let indent = lines.iter().map(|(s, e)| &src[*s..*e]).find(|l| key_value(l).is_some()).map(|l| l[..l.len() - l.trim_start().len()].to_string()).unwrap_or_else(|| "  ".into());
            let mut edits = Vec::new();
            let mut added = String::new();
            for flag in PUBLIC_ACCESS_FLAGS {
                match lines.iter().find(|(s, e)| key_value(&src[*s..*e]).is_some_and(|(k, _)| k == flag)) {
                    Some((s, e)) => {
                        let line = &src[*s..*e];
                        if key_value(line).is_some_and(|(_, v)| v == "true") {
                            continue;
                        }
                        let eq = line.find('=').unwrap_or(0);
                        edits.push((*s, e - s, format!("{} true", &line[..=eq])));
                    }
                    None => added.push_str(&format!("{indent}{flag} = true\n")),
                }
            }
            if !added.is_empty() {
                edits.push((line_start(src, b.end - 1), 0, added));
            }
            if edits.is_empty() {
                return Err(format!("{path} already blocks public access on {} (aws_s3_bucket_public_access_block.{}): the finding may predate its apply (compare with terraform plan), or an account-level setting or bucket policy grants access.", bucket.address, b.labels.get(1).cloned().unwrap_or_default()));
            }
            return Ok((path.clone(), src.clone(), apply(src, edits)));
        }
    }
    let block = format!(
        "\n\nresource \"aws_s3_bucket_public_access_block\" \"{name}\" {{\n  bucket = aws_s3_bucket.{name}.id\n\n  block_public_acls       = true\n  block_public_policy     = true\n  ignore_public_acls      = true\n  restrict_public_buckets = true\n}}"
    );
    Ok((bucket.file.clone(), bsrc.clone(), apply(bsrc, vec![(bblock.end, 0, block)])))
}

/// A map key as HCL writes it (quoted unless an identifier).
fn hcl_key(k: &str) -> String {
    if k.starts_with(|c: char| c.is_ascii_alphabetic()) && k.chars().all(ident_char) {
        k.to_string()
    } else {
        format!("\"{k}\"")
    }
}

/// Adds the tags to every `provider "aws"` block's `default_tags` (creating it), keeping the tags already there.
pub fn add_default_tags(src: &str, tags: &[(String, String)]) -> Result<String, String> {
    let providers: Vec<HclBlock> = hcl_blocks(src).into_iter().filter(|b| b.kind == "provider" && b.labels == ["aws"]).collect();
    if providers.is_empty() {
        return Err("No provider \"aws\" block in this file".into());
    }
    let mut edits = Vec::new();
    for p in &providers {
        if p.start_line == p.end_line {
            return Err("The provider \"aws\" block is written on one line: write it on several lines first".into());
        }
        let (from, to) = body(src, p);
        match blocks_between(src, from, to).into_iter().find(|b| b.kind == "default_tags") {
            Some(d) => {
                let (dfrom, dto) = body(src, &d);
                let (s, e) = top_lines(src, dfrom, dto).into_iter().find(|(s, e)| key_value(&src[*s..*e]).is_some_and(|(k, _)| k == "tags")).ok_or("default_tags has no tags")?;
                let (_, value) = key_value(&src[s..e]).unwrap_or_default();
                if !value.starts_with('{') {
                    return Err(format!("default_tags.tags isn't a literal map ({value}): add the tags where it is defined"));
                }
                let open = s + src[s..e].find('{').unwrap_or(0);
                let close = matching_brace(src, open).ok_or("default_tags.tags isn't closed")?;
                let entries = top_lines(src, open + 1, close - 1);
                let have: Vec<String> = entries.iter().filter_map(|(a, b)| key_value(&src[*a..*b]).map(|(k, _)| norm_key(&k))).collect();
                let indent = entries.iter().map(|(a, b)| &src[*a..*b]).find(|l| key_value(l).is_some()).map(|l| l[..l.len() - l.trim_start().len()].to_string()).unwrap_or_else(|| "      ".into());
                let missing: String = tags.iter().filter(|(k, _)| !have.contains(&norm_key(k))).map(|(k, v)| format!("{indent}{} = \"{v}\"\n", hcl_key(k))).collect();
                if !missing.is_empty() {
                    edits.push((line_start(src, close - 1), 0, missing));
                }
            }
            None => {
                let close_line = line_start(src, p.end - 1);
                let b = &src[close_line..p.end - 1];
                let width = tags.iter().map(|(k, _)| hcl_key(k).len()).max().unwrap_or(0);
                let entries: String = tags.iter().map(|(k, v)| format!("{b}      {:<width$} = \"{v}\"\n", hcl_key(k))).collect();
                edits.push((close_line, 0, format!("\n{b}  default_tags {{\n{b}    tags = {{\n{entries}{b}    }}\n{b}  }}\n")));
            }
        }
    }
    if edits.is_empty() {
        return Err("The aws provider's default_tags already carry these tags".into());
    }
    Ok(apply(src, edits))
}

/// Labels the final stage with the OCI source (the repository's URL) and revision (a `GIT_COMMIT` build argument).
pub fn add_oci_labels(src: &str, source_url: &str) -> Result<String, String> {
    if src.contains(OCI_SOURCE) {
        return Err(format!("The Dockerfile already sets {OCI_SOURCE}"));
    }
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let last = lines.iter().rposition(|l| l.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("from"))).ok_or("No FROM instruction: this isn't a Dockerfile")?;
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(l);
        if i == last {
            if !l.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("ARG GIT_COMMIT=unknown\nLABEL {OCI_SOURCE}=\"{source_url}\" \\\n      org.opencontainers.image.revision=\"$GIT_COMMIT\"\n"));
        }
    }
    Ok(out)
}

// ================================================================ the merge request

fn via_label(v: ProvenanceVia) -> &'static str {
    match v {
        ProvenanceVia::Tag => "tags",
        ProvenanceVia::State => "Terraform state",
        ProvenanceVia::Argocd => "Argo CD",
        ProvenanceVia::Oci => "OCI labels",
        ProvenanceVia::GitMetadata => "git metadata",
    }
}

pub fn kind_label(k: ProvenanceKind) -> &'static str {
    match k {
        ProvenanceKind::Terraform => "Terraform",
        ProvenanceKind::Helm => "Helm",
        ProvenanceKind::Kustomize => "Kustomize",
        ProvenanceKind::Manifest => "Kubernetes manifests",
        ProvenanceKind::Dockerfile => "Dockerfile",
        ProvenanceKind::Code => "code",
    }
}

/// The source lines of a merge request's description.
pub fn source_note(resource: &str, p: &Provenance, located: Option<&Located>) -> String {
    let mut s = format!("- Resource: `{resource}`\n");
    let mut src = format!("- Source ({}): `{}`", kind_label(p.kind), p.repo.as_deref().unwrap_or("?"));
    if let Some(path) = p.path.as_deref().filter(|x| !x.is_empty()) {
        src.push_str(&format!(" `{path}`"));
    }
    if let Some(r) = p.git_ref.as_deref().filter(|x| !x.is_empty()) {
        src.push_str(&format!(" (ref `{r}`)"));
    }
    if let Some(app) = &p.application {
        src.push_str(&format!(", Argo CD Application `{app}`"));
    }
    s.push_str(&src);
    s.push('\n');
    let confidence = match p.confidence {
        Confidence::Exact => "exact",
        Confidence::Inferred => "inferred, confirmed before the fix",
    };
    s.push_str(&format!("- Provenance: {confidence}, via {}\n", via_label(p.via)));
    if let Some(l) = located {
        s.push_str(&format!("- Definition: `{}` in `{}` (lines {}–{})\n", l.address, l.file, l.start_line, l.end_line));
    }
    s
}

/// What the repository's CI must show before the merge.
pub fn ci_note(kind: ProvenanceKind) -> String {
    match kind {
        ProvenanceKind::Terraform => "The repository's CI must pass before this is merged: `terraform plan` must show only the intended change (and `terraform fmt -check` / `validate` pass). Nothing is merged or applied automatically.",
        ProvenanceKind::Helm => "The repository's CI must pass before this is merged: `helm template` (or Argo CD's diff) must render only the intended change. Nothing is merged or synced automatically.",
        ProvenanceKind::Kustomize => "The repository's CI must pass before this is merged: `kustomize build` (or Argo CD's diff) must render only the intended change. Nothing is merged or synced automatically.",
        ProvenanceKind::Manifest => "The repository's CI must pass before this is merged: the manifests must validate and Argo CD's diff show only the intended change. Nothing is merged or synced automatically.",
        ProvenanceKind::Dockerfile => "The image must build in the repository's CI, then be rebuilt, rescanned and redeployed for the finding to close. Nothing is merged automatically.",
        ProvenanceKind::Code => "The repository's CI must pass before this is merged. Nothing is merged automatically.",
    }
    .to_string()
}

// ================================================================ the run's tools

/// Reads allowed to a `fix_infra` or `add_tags` run, the server's own reads of the source included.
pub const MAX_INFRA_READS: usize = 12;
const MAX_LISTED: usize = 300;

/// A run's tools: the source's directory (and the local modules it calls), listed and read, at most
/// [`MAX_INFRA_READS`] files.
pub struct InfraTools<R: RepoFiles> {
    pub files: R,
    scopes: Mutex<Vec<String>>,
    reads: Mutex<usize>,
}

impl<R: RepoFiles> InfraTools<R> {
    /// `read` files were already read for the run's opening.
    pub fn new(files: R, scopes: Vec<String>, read: usize) -> Self {
        InfraTools { files, scopes: Mutex::new(scopes), reads: Mutex::new(read) }
    }

    pub fn scopes(&self) -> Vec<String> {
        self.scopes.lock().unwrap().clone()
    }

    fn shown(&self) -> String {
        self.scopes().iter().map(|d| if d.is_empty() { "the repository root".to_string() } else { d.clone() }).collect::<Vec<_>>().join(", ")
    }
}

impl<R: RepoFiles + Sync> Tools for InfraTools<R> {
    fn specs(&self) -> Vec<ToolSpec> {
        let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
        vec![
            ToolSpec {
                name: "list_source_files".into(),
                description: format!("Files of the resource's source on the default branch: {} (and local modules it calls, once read).", self.shown()),
                schema: obj(json!({"dir": {"type": "string", "description": "One of those directories or below; all when empty"}}), &[]),
            },
            ToolSpec { name: "read_source_file".into(), description: format!("A file of the resource's source, at most {MAX_INFRA_READS} files per run."), schema: obj(json!({"path": {"type": "string"}}), &["path"]) },
        ]
    }

    fn writes(&self, _: &str) -> bool {
        false
    }

    async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
        let scopes = self.scopes();
        match name {
            "list_source_files" => {
                let dir = input["dir"].as_str().unwrap_or("").trim().trim_matches('/');
                let dirs = if dir.is_empty() { scopes.clone() } else if under(dir, &scopes) { vec![dir.to_string()] } else { return Err(format!("Only files under {} can be listed in this run", self.shown())) };
                let mut files = Vec::new();
                for d in &dirs {
                    files.extend(self.files.list(d).await?.into_iter().filter(|p| under(p, &scopes)));
                }
                files.dedup();
                files.truncate(MAX_LISTED);
                Ok(json!({"files": files}).to_string())
            }
            "read_source_file" => {
                let path = input["path"].as_str().unwrap_or_default().trim().trim_start_matches("./");
                if !under(path, &scopes) {
                    return Err(format!("Only files under {} can be read in this run", self.shown()));
                }
                {
                    let mut n = self.reads.lock().unwrap();
                    if *n >= MAX_INFRA_READS {
                        return Err(format!("At most {MAX_INFRA_READS} files can be read: propose the change now"));
                    }
                    *n += 1;
                }
                let content = self.files.read(path).await?;
                if path.ends_with(".tf") {
                    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
                    let mut s = self.scopes.lock().unwrap();
                    for (_, m) in local_modules(&content, dir) {
                        if !s.contains(&m) {
                            s.push(m);
                        }
                    }
                }
                Ok(json!({"path": path, "content": content}).to_string())
            }
            _ => Err(format!("Unknown tool {name}")),
        }
    }
}

// ================================================================ the source, read for the run

/// What the run needs to know about the finding's resource to find it in its source.
#[derive(Debug, Clone, Default)]
pub struct InfraFinding {
    /// The cloud resource's, object's or image's name (`example-shop-assets`, `catalog-service`, `registry.example.com/x:1.2`).
    pub resource_name: String,
    /// `AwsS3Bucket`, `kube_statefulset`, `container_image`.
    pub resource_type: String,
    pub env: Option<String>,
    /// Helm: the Application's `spec.source.helm.valueFiles`.
    pub value_files: Vec<String>,
}

/// The files read for a run's opening and where it may read and write.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prepared {
    /// Directories the run may read (the source's, the local modules it calls).
    pub scopes: Vec<String>,
    /// Directories a change may touch: the source's, and the module where the resource is defined.
    pub write_dirs: Vec<String>,
    /// Dockerfiles: the only files a change may touch.
    pub write_files: Option<Vec<String>>,
    /// Files read, with their content.
    pub files: Vec<(String, String)>,
    pub located: Option<Located>,
    /// Files read so far (they count against [`MAX_INFRA_READS`]).
    pub reads: usize,
}

/// Files read for the opening, at most (the run keeps the rest of [`MAX_INFRA_READS`]).
const OPENING_READS: usize = 6;

/// The Kubernetes kind of a `kube_*` type (`kube_statefulset` → `StatefulSet`).
pub fn kube_kind(resource_type: &str) -> Option<&'static str> {
    Some(match resource_type.strip_prefix("kube_")? {
        "deployment" => "Deployment",
        "statefulset" => "StatefulSet",
        "daemonset" => "DaemonSet",
        "cronjob" => "CronJob",
        "job" => "Job",
        "pod" => "Pod",
        "service" => "Service",
        "ingress" => "Ingress",
        _ => return None,
    })
}

/// The `.tf` files directly in `dir`.
fn tf_files(listing: &[String], dir: &str) -> Vec<String> {
    let dir = dir.trim_matches('/');
    listing.iter().filter(|p| p.ends_with(".tf") && p.rsplit_once('/').map_or("", |(d, _)| d) == dir).cloned().collect()
}

/// Reads the source of the finding's resource: the Terraform files of its module (and the local modules on the way to
/// its address) with the resource located, the Helm values files of the environment, the manifests declaring the object,
/// or the image's Dockerfile.
pub async fn prepare<R: RepoFiles>(files: &R, p: &Provenance, f: &InfraFinding) -> Prepared {
    let dir = scope_dir(p);
    let mut out = Prepared { scopes: vec![dir.clone()], write_dirs: vec![dir.clone()], ..Default::default() };
    let listing = files.list(&dir).await.unwrap_or_default();
    let read = |path: String, out: &mut Prepared| {
        let ok = out.reads < OPENING_READS && !out.files.iter().any(|(p, _)| *p == path);
        async move {
            if !ok {
                return None;
            }
            files.read(&path).await.ok().map(|c| (path, c))
        }
    };
    match p.kind {
        ProvenanceKind::Terraform => {
            let address = p.address.as_deref().and_then(parse_address);
            let mut queue = tf_files(&listing, &dir);
            // A file named by the tags (Yor's git_file) first.
            if let Some(file) = p.path.as_deref().map(|x| x.trim().trim_start_matches("./").to_string()).filter(|x| x.ends_with(".tf")) {
                queue.retain(|q| *q != file);
                queue.insert(0, file);
            }
            let mut modules: Vec<(String, String)> = Vec::new();
            while let Some(path) = (!queue.is_empty()).then(|| queue.remove(0)) {
                let Some((path, content)) = read(path, &mut out).await else { continue };
                out.reads += 1;
                let here = path.rsplit_once('/').map_or("", |(d, _)| d).to_string();
                for (name, mdir) in local_modules(&content, &here) {
                    if !out.scopes.contains(&mdir) {
                        out.scopes.push(mdir.clone());
                        modules.push((name, mdir));
                    }
                }
                out.files.push((path, content));
                if queue.is_empty() && find_resource(&out.files, address.as_ref(), &f.resource_name).is_none() {
                    // The modules on the way to the address first, then the others.
                    let next = match address.as_ref().and_then(|a| modules.iter().position(|(n, _)| a.modules.contains(n))) {
                        Some(i) => Some(modules.remove(i)),
                        None => (!modules.is_empty()).then(|| modules.remove(0)),
                    };
                    if let Some((_, mdir)) = next {
                        queue = tf_files(&files.list(&mdir).await.unwrap_or_default(), &mdir);
                    }
                }
            }
            out.located = find_resource(&out.files, address.as_ref(), &f.resource_name);
            if let Some(l) = &out.located {
                let d = l.file.rsplit_once('/').map_or("", |(d, _)| d).to_string();
                if !under(&l.file, &out.write_dirs) {
                    out.write_dirs.push(d);
                }
            }
        }
        ProvenanceKind::Helm => {
            for v in helm_values_files(&listing, &dir, f.env.as_deref(), &f.value_files).into_iter().take(3) {
                if let Some(x) = read(v, &mut out).await {
                    out.reads += 1;
                    out.files.push(x);
                }
            }
        }
        ProvenanceKind::Kustomize | ProvenanceKind::Manifest => {
            let kind = kube_kind(&f.resource_type).unwrap_or("");
            for y in yaml_files(&listing, &dir) {
                let Some((path, content)) = read(y, &mut out).await else { continue };
                out.reads += 1;
                let name = path.rsplit('/').next().unwrap_or("");
                if name.starts_with("kustomization.") || declares_object(&content, kind, &f.resource_name) {
                    out.files.push((path, content));
                }
            }
        }
        _ => {
            let found = dockerfiles(&listing, &dir, &f.resource_name);
            if let Some(first) = found.first().cloned() {
                if let Some(x) = read(first, &mut out).await {
                    out.reads += 1;
                    out.files.push(x);
                }
            }
            out.write_files = Some(found);
        }
    }
    out
}

/// The opening message of a `fix_infra` run: the finding, its source and the files read.
pub fn infra_opening(what: &str, detail: &Value, p: &Provenance, prep: &Prepared, resource: &str) -> String {
    let mut s = format!("Fix this {what} on {resource} in the code that defines it.\n\nThe finding:\n```json\n{}\n```\n\n", crate::assist::agent::truncate(&serde_json::to_string_pretty(detail).unwrap_or_default(), 6000));
    s.push_str(&format!("Its source:\n{}", source_note(resource, p, prep.located.as_ref())));
    if p.kind == ProvenanceKind::Terraform && prep.located.is_none() {
        s.push_str("- The resource's block wasn't found in the files read: look for it with list_source_files and read_source_file (local modules become readable once their caller is read).\n");
    }
    if let Some(files) = &prep.write_files {
        s.push_str(&format!("\nOnly these files may change: {}.", if files.is_empty() { "none found (conclude with no_change and say where the Dockerfile should be)".to_string() } else { files.join(", ") }));
        if let Some(eco) = detail.get("ecosystem").and_then(Value::as_str) {
            s.push_str(&if os_package(eco) {
                format!("\nThe vulnerable package ({eco}) comes with the base image: a newer tag of the same base image fixes it.")
            } else {
                format!("\nThe vulnerable package is a {eco} library the application declares, not part of the base image: a base image bump doesn't fix it. Conclude with no_change and say to fix it in the application's manifest.")
            });
        }
        if let Some((_, content)) = prep.files.first() {
            let bases: Vec<String> = base_images(content).into_iter().map(|(l, i)| format!("line {l}: {i}")).collect();
            s.push_str(&format!("\nBase images: {}.", if bases.is_empty() { "none".to_string() } else { bases.join("; ") }));
        }
    } else {
        s.push_str(&format!("\nOnly files under {} may change.", prep.write_dirs.iter().map(|d| if d.is_empty() { "the repository root" } else { d.as_str() }).collect::<Vec<_>>().join(", ")));
    }
    s.push_str(" Propose their whole new contents with propose_change, then conclude.\n");
    for (path, content) in &prep.files {
        s.push_str(&format!("\n{path}:\n```\n{}\n```\n", crate::assist::agent::truncate(content, 30_000)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::provenance::TaggedResource;
    use crate::routes::tagging_rules::{RequiredTag, TagRule, TagScope};
    use std::collections::HashMap;

    fn prov(kind: ProvenanceKind, repo: Option<&str>, path: Option<&str>, confidence: Confidence, via: ProvenanceVia) -> Provenance {
        Provenance { repo: repo.map(str::to_string), path: path.map(str::to_string), git_ref: None, address: None, application: None, kind, confidence, via }
    }

    fn connected(repo: &str) -> Result<(), String> {
        if repo.starts_with("github.com/") {
            Ok(())
        } else {
            Err(format!("No GitLab connection reaches {}", repo.split('/').next().unwrap()))
        }
    }

    #[test]
    fn the_level_follows_the_provenance_its_confidence_and_the_connection() {
        let tf = prov(ProvenanceKind::Terraform, Some("github.com/example-org/infra-terraform"), Some("envs/prod/storage"), Confidence::Exact, ProvenanceVia::Tag);
        let l = infra_level(Subject::Cloud, Some(&tf), connected);
        assert_eq!((l.level, l.confirm, l.connection.clone()), (Level::FixInInfra, false, None));
        assert!(l.reason.contains("Terraform") && l.reason.contains("envs/prod/storage"), "{}", l.reason);
        // Inferred: still fixable, after a confirmation.
        let mut inferred = prov(ProvenanceKind::Helm, Some("github.com/example-org/gitops"), Some("apps/catalog"), Confidence::Inferred, ProvenanceVia::Argocd);
        inferred.application = Some("catalog".into());
        let l = infra_level(Subject::Kubernetes, Some(&inferred), connected);
        assert_eq!((l.level, l.confirm), (Level::FixInInfra, true));
        assert!(l.reason.contains("inferred") && l.reason.contains("catalog"), "{}", l.reason);
        // No connection reaches the repository: explained, the source shown.
        let gl = prov(ProvenanceKind::Terraform, Some("gitlab.com/example-org/infra"), Some("envs/prod"), Confidence::Exact, ProvenanceVia::State);
        let l = infra_level(Subject::Cloud, Some(&gl), connected);
        assert_eq!(l.level, Level::Explain);
        assert!(l.source.is_some() && l.connection.as_deref().unwrap().contains("No GitLab connection"));
        // No provenance, or only the Argo CD Application's name, or Terraform without its module path: tags missing.
        assert_eq!(infra_level(Subject::Cloud, None, connected).level, Level::NeedsProvenance);
        let mut app_only = prov(ProvenanceKind::Manifest, None, None, Confidence::Exact, ProvenanceVia::Argocd);
        app_only.application = Some("payments".into());
        let l = infra_level(Subject::Kubernetes, Some(&app_only), connected);
        assert_eq!(l.level, Level::NeedsProvenance);
        assert!(l.reason.contains("payments") && l.reason.contains("argoproj.io/v1alpha1/applications"), "{}", l.reason);
        let no_path = prov(ProvenanceKind::Terraform, Some("github.com/example-org/infra-terraform"), None, Confidence::Exact, ProvenanceVia::Tag);
        assert!(infra_level(Subject::Cloud, Some(&no_path), connected).reason.contains("tf_path"));
        // Images need only the repository (the Dockerfile is searched for); hosts have no repository.
        let img = prov(ProvenanceKind::Dockerfile, Some("github.com/example-org/storefront-api"), None, Confidence::Exact, ProvenanceVia::Oci);
        let l = infra_level(Subject::Image, Some(&img), connected);
        assert_eq!(l.level, Level::FixInInfra);
        assert!(l.reason.contains("rebuilt"), "{}", l.reason);
        assert_eq!(infra_level(Subject::Host, Some(&img), connected).level, Level::Explain);
    }

    #[test]
    fn the_scope_is_the_source_directory() {
        let p = |path: &str, kind| prov(kind, Some("github.com/x/y"), Some(path), Confidence::Exact, ProvenanceVia::Tag);
        assert_eq!(scope_dir(&p("envs/prod/storage", ProvenanceKind::Terraform)), "envs/prod/storage");
        assert_eq!(scope_dir(&p("modules/orders-db/main.tf", ProvenanceKind::Terraform)), "modules/orders-db");
        assert_eq!(scope_dir(&p("./apps/catalog/", ProvenanceKind::Helm)), "apps/catalog");
        assert_eq!(scope_dir(&prov(ProvenanceKind::Dockerfile, Some("github.com/x/y"), None, Confidence::Exact, ProvenanceVia::Oci)), "");
        let dirs = vec!["envs/prod/storage".to_string(), "modules/storage".to_string()];
        assert!(under("envs/prod/storage/main.tf", &dirs) && under("modules/storage/s3.tf", &dirs));
        assert!(!under("envs/prod/storagex/main.tf", &dirs) && !under("envs/prod/main.tf", &dirs) && !under("envs/prod/storage/../x.tf", &dirs));
        assert!(under("Dockerfile", &["".to_string()]));
    }

    fn resource(target: TagTarget, provider: Option<&str>, name: &str, tags: &[&str]) -> TaggedResource {
        TaggedResource {
            id: format!("id-{name}"),
            target,
            name: name.into(),
            resource_type: "AwsS3Bucket".into(),
            provider: provider.map(str::to_string),
            account: Some("123456789012".into()),
            region: None,
            cluster: None,
            namespace: None,
            env: None,
            team: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            provenance: None,
        }
    }

    #[test]
    fn missing_tags_come_from_the_policies_else_the_recommended_set() {
        let rule = |required: &[&str]| CompiledRule::new(&TagRule { target: TagTarget::Cloud, scope: TagScope::default(), required: required.iter().map(|k| RequiredTag { key: k.to_string(), allowed_values: vec![] }).collect() }).unwrap();
        let policies = vec![("Terraform source".to_string(), rule(&["managed-by", "tf_repo|git_repo", "tf_path|git_file"])), ("Owners".to_string(), rule(&["team"]))];
        let r = resource(TagTarget::Cloud, Some("aws"), "example-shop-logs", &["managed-by:terraform", "tf_repo:github.com/example-org/infra-terraform"]);
        // Only provenance tags: tf_path is missing, team isn't about provenance.
        assert_eq!(missing_tags(TagTarget::Cloud, Some(&r), &policies), vec![MissingTag { policy: "Terraform source".into(), key: "tf_path|git_file".into(), value: None }]);
        // No policy: the recommended set's provenance tags.
        let m = missing_tags(TagTarget::Image, None, &[]);
        assert_eq!(m.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["org.opencontainers.image.source|git.repository_url", "org.opencontainers.image.revision|git.commit.sha"]);
        assert!(m[0].policy.contains("OCI"));
        let k = missing_tags(TagTarget::Kubernetes, None, &[]);
        assert_eq!(k[0].key, "argocd.argoproj.io/tracking-id|app.kubernetes.io/instance");
    }

    #[test]
    fn the_snippet_adds_the_missing_tags_where_the_resource_is_defined() {
        let missing = vec![MissingTag { policy: "p".into(), key: "tf_repo|git_repo".into(), value: None }, MissingTag { policy: "p".into(), key: "tf_path|git_file".into(), value: None }];
        let aws = provenance_snippet(TagTarget::Cloud, Some(&resource(TagTarget::Cloud, Some("aws"), "example-shop-logs", &[])), &missing);
        assert_eq!(aws.language, "hcl");
        assert!(aws.code.contains("default_tags") && aws.code.contains("tf_repo") && aws.code.contains("tf_path") && aws.code.contains("Missing: tf_repo|git_repo, tf_path|git_file"), "{}", aws.code);
        let az = provenance_snippet(TagTarget::Cloud, Some(&resource(TagTarget::Cloud, Some("azure"), "shop-secrets", &[])), &missing);
        assert!(az.code.contains("merge(local.default_tags"), "{}", az.code);
        let img = provenance_snippet(TagTarget::Image, Some(&TaggedResource { name: "registry.example.com/catalog-service:2.4.0".into(), ..resource(TagTarget::Image, None, "x", &[]) }), &[]);
        assert!(img.code.contains("org.opencontainers.image.source") && img.code.contains("-t registry.example.com/catalog-service:2.4.0"), "{}", img.code);
        assert!(provenance_snippet(TagTarget::Kubernetes, None, &[]).code.contains("resourceTrackingMethod: annotation"));
    }

    const MAIN_TF: &str = r#"# Storage for the storefront
module "storage" {
  source = "../../../modules/storage"
  name   = "example-shop"
}

module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
}

resource "aws_s3_bucket" "logs" {
  bucket = "example-shop-logs"
  tags = {
    purpose = "logs" # } not a brace
  }
  policy = <<EOF
{"Statement": [{"Effect": "Allow"}]}
EOF
}
"#;

    const MODULE_TF: &str = r#"resource "aws_s3_bucket" "assets" {
  bucket = "${var.name}-assets"
}

resource "aws_s3_bucket_public_access_block" "assets" {
  bucket = aws_s3_bucket.assets.id

  block_public_acls       = false
  block_public_policy     = true
}
"#;

    #[test]
    fn hcl_blocks_skip_strings_comments_and_heredocs() {
        let b = hcl_blocks(MAIN_TF);
        let kinds: Vec<(String, Vec<String>)> = b.iter().map(|x| (x.kind.clone(), x.labels.clone())).collect();
        assert_eq!(kinds, vec![("module".into(), vec!["storage".into()]), ("module".into(), vec!["vpc".into()]), ("resource".into(), vec!["aws_s3_bucket".into(), "logs".into()])]);
        assert_eq!((b[2].start_line, b[2].end_line), (11, 19));
        assert_eq!(attribute(MAIN_TF, &b[2], "bucket").as_deref(), Some("\"example-shop-logs\""));
        assert_eq!(attribute(MAIN_TF, &b[2], "purpose"), None, "nested attributes aren't the block's");
        assert_eq!(local_modules(MAIN_TF, "envs/prod/storage"), vec![("storage".to_string(), "modules/storage".to_string())]);
        assert_eq!(local_modules("module \"x\" {\n  source = \"../../../../etc\"\n}\n", "envs/prod"), vec![], "no escape from the repository");
    }

    #[test]
    fn terraform_resources_are_found_by_address_or_name() {
        let a = parse_address("module.storage.aws_s3_bucket.assets[\"eu\"]").unwrap();
        assert_eq!(a, Address { modules: vec!["storage".into()], resource_type: Some("aws_s3_bucket".into()), name: "assets".into() });
        assert_eq!(parse_address("assets").unwrap(), Address { modules: vec![], resource_type: None, name: "assets".into() });
        assert_eq!(parse_address("module.storage"), None);
        let files = vec![("envs/prod/storage/main.tf".to_string(), MAIN_TF.to_string()), ("modules/storage/main.tf".to_string(), MODULE_TF.to_string())];
        let l = find_resource(&files, Some(&a), "example-shop-assets").unwrap();
        assert_eq!((l.file.as_str(), l.start_line, l.end_line, l.address.as_str(), l.by.as_str()), ("modules/storage/main.tf", 1, 3, "aws_s3_bucket.assets", "address"));
        // Yor's resource name alone.
        assert_eq!(find_resource(&files, parse_address("logs").as_ref(), "").unwrap().address, "aws_s3_bucket.logs");
        // No address: the block naming the cloud resource.
        let l = find_resource(&files, None, "example-shop-logs").unwrap();
        assert_eq!((l.address.as_str(), l.by.as_str()), ("aws_s3_bucket.logs", "name"));
        assert_eq!(find_resource(&files, None, "nothing-like-it"), None);
    }

    #[test]
    fn helm_values_files_take_the_environment_first() {
        let listing: Vec<String> = ["apps/catalog/Chart.yaml", "apps/catalog/values.yaml", "apps/catalog/values-prod.yaml", "apps/catalog/values-staging.yaml", "apps/catalog/templates/deployment.yaml", "apps/other/values.yaml"].iter().map(|s| s.to_string()).collect();
        assert_eq!(helm_values_files(&listing, "apps/catalog", Some("prod"), &[]), vec!["apps/catalog/values-prod.yaml", "apps/catalog/values.yaml"]);
        assert_eq!(helm_values_files(&listing, "apps/catalog", Some("production"), &[]), vec!["apps/catalog/values-prod.yaml", "apps/catalog/values.yaml"], "production is prod");
        assert_eq!(helm_values_files(&listing, "apps/catalog", None, &[]), vec!["apps/catalog/values.yaml"]);
        // The Application's valueFiles win (relative to the source path).
        assert_eq!(helm_values_files(&listing, "apps/catalog", Some("prod"), &["values-staging.yaml".into(), "missing.yaml".into()]), vec!["apps/catalog/values-staging.yaml"]);
        let manifests: Vec<String> = ["apps/checkout/overlays/prod/patch.yaml", "apps/checkout/overlays/prod/kustomization.yaml", "apps/checkout/overlays/prod/sub/deep.yml", "apps/checkout/README.md"].iter().map(|s| s.to_string()).collect();
        assert_eq!(yaml_files(&manifests, "apps/checkout/overlays/prod"), vec!["apps/checkout/overlays/prod/kustomization.yaml", "apps/checkout/overlays/prod/patch.yaml", "apps/checkout/overlays/prod/sub/deep.yml"]);
        let doc = "apiVersion: v1\nkind: Service\nmetadata:\n  name: billing\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: billing\n  namespace: payments\n";
        assert!(declares_object(doc, "Deployment", "billing") && declares_object(doc, "Service", "billing"));
        assert!(!declares_object(doc, "Deployment", "order-service") && !declares_object("kind: Deployment\nmetadata:\n  name: billing-worker\n", "Deployment", "billing"));
    }

    #[test]
    fn dockerfiles_and_their_base_images() {
        let listing: Vec<String> = ["services/api/Dockerfile", "Dockerfile", "docs/x.md", "services/storefront-api/Dockerfile", "tools/ci.Dockerfile"].iter().map(|s| s.to_string()).collect();
        assert_eq!(dockerfiles(&listing, "", "registry.example.com/storefront-api:1.42.0"), vec!["services/storefront-api/Dockerfile", "Dockerfile", "tools/ci.Dockerfile", "services/api/Dockerfile"]);
        let df = "# syntax=docker/dockerfile:1\nFROM --platform=$BUILDPLATFORM maven:3.9-eclipse-temurin-21 AS build\nRUN mvn package\nFROM build AS test\nfrom debian:12.4-slim\nCOPY --from=build /app.jar /app.jar\n";
        assert_eq!(base_images(df), vec![(2, "maven:3.9-eclipse-temurin-21".to_string()), (5, "debian:12.4-slim".to_string())]);
        assert!(os_package("Debian:12") && os_package("Alpine:v3.19") && !os_package("Maven") && !os_package("npm"));
    }

    #[test]
    fn public_access_is_blocked_in_the_existing_block_or_a_new_one() {
        let files = vec![("modules/storage/main.tf".to_string(), MODULE_TF.to_string())];
        let bucket = find_resource(&files, parse_address("aws_s3_bucket.assets").as_ref(), "").unwrap();
        let (path, before, after) = block_public_access(&files, &bucket).unwrap();
        assert_eq!((path.as_str(), before.as_str()), ("modules/storage/main.tf", MODULE_TF));
        assert!(after.contains("  block_public_acls       = true\n") && after.contains("  block_public_policy     = true\n"), "{after}");
        assert!(after.contains("  ignore_public_acls = true\n") && after.contains("  restrict_public_buckets = true\n}\n"), "{after}");
        assert_eq!(after.matches("aws_s3_bucket_public_access_block").count(), 1);
        // Already blocked: nothing to change here.
        let done = vec![("m.tf".to_string(), after.clone())];
        let b = find_resource(&done, parse_address("aws_s3_bucket.assets").as_ref(), "").unwrap();
        assert!(block_public_access(&done, &b).unwrap_err().contains("already"));
        // No block: one is added after the bucket.
        let files = vec![("envs/prod/storage/main.tf".to_string(), MAIN_TF.to_string())];
        let logs = find_resource(&files, None, "example-shop-logs").unwrap();
        let (_, _, after) = block_public_access(&files, &logs).unwrap();
        assert!(after.contains("EOF\n}\n\nresource \"aws_s3_bucket_public_access_block\" \"logs\" {\n  bucket = aws_s3_bucket.logs.id\n\n  block_public_acls       = true\n"), "{after}");
        assert_eq!(hcl_blocks(&after).len(), 4, "still valid blocks");
    }

    #[test]
    fn default_tags_are_added_to_the_aws_provider() {
        let tags = vec![("managed-by".to_string(), "terraform".to_string()), ("tf_repo".to_string(), "github.com/example-org/infra-terraform".to_string()), ("tf_path".to_string(), "envs/prod/storage".to_string())];
        let src = "terraform {\n  required_version = \">= 1.5\"\n}\n\nprovider \"aws\" {\n  region = \"eu-west-1\"\n}\n";
        let out = add_default_tags(src, &tags).unwrap();
        assert_eq!(out, "terraform {\n  required_version = \">= 1.5\"\n}\n\nprovider \"aws\" {\n  region = \"eu-west-1\"\n\n  default_tags {\n    tags = {\n      managed-by = \"terraform\"\n      tf_repo    = \"github.com/example-org/infra-terraform\"\n      tf_path    = \"envs/prod/storage\"\n    }\n  }\n}\n");
        // Existing default_tags keep their tags; only the missing ones are added.
        let existing = "provider \"aws\" {\n  default_tags {\n    tags = {\n      team = \"storefront\"\n      Managed_By = \"terraform\"\n    }\n  }\n}\n";
        let out = add_default_tags(existing, &tags).unwrap();
        assert!(out.contains("      team = \"storefront\"\n      Managed_By = \"terraform\"\n      tf_repo = \"github.com/example-org/infra-terraform\"\n      tf_path = \"envs/prod/storage\"\n    }"), "{out}");
        assert!(add_default_tags(&out, &tags).unwrap_err().contains("already"));
        assert!(add_default_tags("provider \"aws\" {\n  default_tags {\n    tags = var.tags\n  }\n}\n", &tags).unwrap_err().contains("literal"));
        assert!(add_default_tags("provider \"azurerm\" {\n  features {}\n}\n", &tags).unwrap_err().contains("provider \"aws\""));
    }

    #[test]
    fn oci_labels_go_on_the_final_stage() {
        let df = "FROM golang:1.22 AS build\nRUN go build -o /app\nFROM gcr.io/distroless/base\nCOPY --from=build /app /app\nENTRYPOINT [\"/app\"]\n";
        let out = add_oci_labels(df, "https://github.com/example-org/order-service").unwrap();
        assert_eq!(
            out,
            "FROM golang:1.22 AS build\nRUN go build -o /app\nFROM gcr.io/distroless/base\nARG GIT_COMMIT=unknown\nLABEL org.opencontainers.image.source=\"https://github.com/example-org/order-service\" \\\n      org.opencontainers.image.revision=\"$GIT_COMMIT\"\nCOPY --from=build /app /app\nENTRYPOINT [\"/app\"]\n"
        );
        assert!(add_oci_labels(&out, "https://github.com/example-org/order-service").unwrap_err().contains("already"));
        assert!(add_oci_labels("RUN x\n", "u").is_err());
    }

    #[test]
    fn the_merge_request_names_the_source_and_the_ci_to_pass() {
        let mut p = prov(ProvenanceKind::Helm, Some("github.com/example-org/gitops"), Some("apps/catalog"), Confidence::Inferred, ProvenanceVia::Argocd);
        p.application = Some("catalog".into());
        p.git_ref = Some("main".into());
        let s = source_note("StatefulSet/catalog/catalog-service", &p, None);
        assert!(s.contains("- Resource: `StatefulSet/catalog/catalog-service`") && s.contains("`github.com/example-org/gitops` `apps/catalog` (ref `main`)"), "{s}");
        assert!(s.contains("Argo CD Application `catalog`") && s.contains("inferred, confirmed before the fix"), "{s}");
        let tf = prov(ProvenanceKind::Terraform, Some("github.com/example-org/infra-terraform"), Some("envs/prod/storage"), Confidence::Exact, ProvenanceVia::Tag);
        let l = Located { file: "modules/storage/main.tf".into(), start_line: 1, end_line: 3, address: "aws_s3_bucket.assets".into(), by: "address".into() };
        let s = source_note("arn:aws:s3:::example-shop-assets", &tf, Some(&l));
        assert!(s.contains("exact, via tags") && s.contains("`aws_s3_bucket.assets` in `modules/storage/main.tf`"), "{s}");
        assert!(ci_note(ProvenanceKind::Terraform).contains("terraform plan"));
        assert!(ci_note(ProvenanceKind::Helm).contains("helm template"));
        assert!(ci_note(ProvenanceKind::Dockerfile).contains("rebuilt"));
    }

    struct MapFiles(HashMap<String, String>);

    impl RepoFiles for MapFiles {
        async fn read(&self, path: &str) -> Result<String, String> {
            self.0.get(path).cloned().ok_or_else(|| format!("{path} not found"))
        }
        async fn list(&self, dir: &str) -> Result<Vec<String>, String> {
            let mut v: Vec<String> = self.0.keys().filter(|k| dir.is_empty() || k.starts_with(&format!("{dir}/"))).cloned().collect();
            v.sort();
            Ok(v)
        }
    }

    #[tokio::test]
    async fn infra_tools_read_only_the_source_and_the_modules_it_calls() {
        let files = MapFiles(HashMap::from([
            ("envs/prod/storage/main.tf".to_string(), MAIN_TF.to_string()),
            ("modules/storage/main.tf".to_string(), MODULE_TF.to_string()),
            ("envs/staging/main.tf".to_string(), "x".to_string()),
        ]));
        let t = InfraTools::new(files, vec!["envs/prod/storage".into()], 2);
        let names: Vec<String> = t.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["list_source_files", "read_source_file"]);
        assert!(t.run("list_source_files", &json!({})).await.unwrap().contains("envs/prod/storage/main.tf"));
        assert!(t.run("read_source_file", &json!({"path": "envs/staging/main.tf"})).await.unwrap_err().contains("Only files under"));
        assert!(t.run("read_source_file", &json!({"path": "modules/storage/main.tf"})).await.is_err(), "the module isn't known yet");
        // Reading the root module makes its local modules readable.
        t.run("read_source_file", &json!({"path": "envs/prod/storage/main.tf"})).await.unwrap();
        assert_eq!(t.scopes(), vec!["envs/prod/storage", "modules/storage"]);
        assert!(t.run("read_source_file", &json!({"path": "modules/storage/main.tf"})).await.unwrap().contains("aws_s3_bucket_public_access_block"));
        assert!(t.run("read_source_file", &json!({"path": "envs/prod/storage/../staging/main.tf"})).await.is_err());
        for _ in 4..MAX_INFRA_READS {
            t.run("read_source_file", &json!({"path": "envs/prod/storage/main.tf"})).await.unwrap();
        }
        assert!(t.run("read_source_file", &json!({"path": "envs/prod/storage/main.tf"})).await.unwrap_err().contains("At most 12"));
    }

    fn repo(files: &[(&str, &str)]) -> MapFiles {
        MapFiles(files.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect())
    }

    #[tokio::test]
    async fn terraform_sources_are_read_down_to_the_module_defining_the_address() {
        let files = repo(&[("envs/prod/storage/main.tf", MAIN_TF), ("envs/prod/storage/providers.tf", "provider \"aws\" {\n  region = \"eu-west-1\"\n}\n"), ("modules/storage/main.tf", MODULE_TF), ("modules/other/main.tf", "x")]);
        let mut p = prov(ProvenanceKind::Terraform, Some("github.com/example-org/infra-terraform"), Some("envs/prod/storage"), Confidence::Exact, ProvenanceVia::State);
        p.address = Some("module.storage.aws_s3_bucket.assets".into());
        let prep = prepare(&files, &p, &InfraFinding { resource_name: "example-shop-assets".into(), resource_type: "AwsS3Bucket".into(), ..Default::default() }).await;
        assert_eq!(prep.scopes, vec!["envs/prod/storage", "modules/storage"]);
        assert_eq!(prep.write_dirs, vec!["envs/prod/storage", "modules/storage"], "the module defining it may change too");
        let l = prep.located.clone().unwrap();
        assert_eq!((l.file.as_str(), l.address.as_str()), ("modules/storage/main.tf", "aws_s3_bucket.assets"));
        assert_eq!(prep.reads, 3);
        assert!(!prep.files.iter().any(|(p, _)| p.starts_with("modules/other")));
        // The fix is deterministic here.
        let (path, _, after) = block_public_access(&prep.files, &l).unwrap();
        assert!(path == "modules/storage/main.tf" && after.contains("restrict_public_buckets = true"));
        let o = infra_opening("cloud misconfiguration", &json!({"ruleId": "s3_bucket_level_public_access_block"}), &p, &prep, "arn:aws:s3:::example-shop-assets");
        assert!(o.contains("`aws_s3_bucket.assets` in `modules/storage/main.tf`") && o.contains("Only files under envs/prod/storage, modules/storage may change") && o.contains("modules/storage/main.tf:\n```"), "{o}");
        // Tags without an address: the block naming the bucket.
        let p = prov(ProvenanceKind::Terraform, Some("github.com/example-org/infra-terraform"), Some("envs/prod/storage"), Confidence::Exact, ProvenanceVia::Tag);
        let prep = prepare(&files, &p, &InfraFinding { resource_name: "example-shop-logs".into(), ..Default::default() }).await;
        assert_eq!(prep.located.unwrap().address, "aws_s3_bucket.logs");
    }

    #[tokio::test]
    async fn helm_manifest_and_dockerfile_sources_read_their_files() {
        let files = repo(&[
            ("apps/catalog/values.yaml", "securityContext: {}\n"),
            ("apps/catalog/values-prod.yaml", "replicaCount: 3\n"),
            ("apps/catalog/templates/statefulset.yaml", "x"),
            ("apps/billing/kustomization.yaml", "resources: [deployment.yaml]\n"),
            ("apps/billing/deployment.yaml", "kind: Deployment\nmetadata:\n  name: billing\n"),
            ("apps/billing/service.yaml", "kind: Service\nmetadata:\n  name: billing\n"),
            ("Dockerfile", "FROM debian:12.4-slim\n"),
        ]);
        let mut helm = prov(ProvenanceKind::Helm, Some("github.com/example-org/gitops"), Some("apps/catalog"), Confidence::Exact, ProvenanceVia::Argocd);
        helm.application = Some("catalog".into());
        let prep = prepare(&files, &helm, &InfraFinding { resource_name: "catalog-service".into(), resource_type: "kube_statefulset".into(), env: Some("prod".into()), ..Default::default() }).await;
        assert_eq!(prep.files.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["apps/catalog/values-prod.yaml", "apps/catalog/values.yaml"]);
        assert_eq!(prep.write_dirs, vec!["apps/catalog"]);
        let k = prov(ProvenanceKind::Kustomize, Some("github.com/example-org/gitops"), Some("apps/billing"), Confidence::Exact, ProvenanceVia::Argocd);
        let prep = prepare(&files, &k, &InfraFinding { resource_name: "billing".into(), resource_type: "kube_deployment".into(), ..Default::default() }).await;
        assert_eq!(prep.files.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["apps/billing/kustomization.yaml", "apps/billing/deployment.yaml"]);
        let d = prov(ProvenanceKind::Dockerfile, Some("github.com/example-org/storefront-api"), None, Confidence::Exact, ProvenanceVia::Oci);
        let prep = prepare(&files, &d, &InfraFinding { resource_name: "registry.example.com/storefront-api:1.42.0".into(), ..Default::default() }).await;
        assert_eq!((prep.write_files.clone(), prep.files.len()), (Some(vec!["Dockerfile".to_string()]), 1));
        let o = infra_opening("infrastructure vulnerability", &json!({"ecosystem": "Debian:12", "package": "libssl3"}), &d, &prep, "registry.example.com/storefront-api:1.42.0");
        assert!(o.contains("Only these files may change: Dockerfile.") && o.contains("line 1: debian:12.4-slim") && o.contains("comes with the base image"), "{o}");
        let o = infra_opening("infrastructure vulnerability", &json!({"ecosystem": "Maven"}), &d, &prep, "x");
        assert!(o.contains("doesn't fix it"), "{o}");
        assert_eq!(kube_kind("kube_statefulset"), Some("StatefulSet"));
    }
}
