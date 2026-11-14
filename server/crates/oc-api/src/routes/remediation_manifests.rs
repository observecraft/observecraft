//! Fix in code for library vulnerabilities (ADR-0008): a dependency bumped to its first fixed version in the
//! manifest that declares it, without running the package manager. Pure functions over the file's text: only
//! the version is rewritten, everything else (order, spacing, comments, quotes) stays as it was.
//!
//! Formats: package.json, composer.json, requirements*.txt, pyproject.toml (PEP 621 and Poetry), Pipfile,
//! go.mod, pom.xml (the dependency's version or the property it names), build.gradle(.kts), Gemfile,
//! Cargo.toml and *.csproj (also Directory.Packages.props and packages.config). Lockfiles are never edited:
//! the package manager regenerates them in CI ([`lockfile_note`]).

use std::cmp::Ordering;
use std::ops::Range;

use regex::Regex;

use super::code_security_model::{compare_versions, normalize_name};

/// Why a manifest couldn't be bumped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BumpError {
    /// The manifest doesn't declare the package: it comes in transitively.
    NotDeclared,
    /// The declared version or range already allows the fixed version: only the lockfile pins the old one.
    AlreadyAllowed,
    /// Declared in a way we don't rewrite (a range with several bounds, a version from elsewhere…), with why.
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Npm,
    Composer,
    Requirements,
    Pyproject,
    Pipfile,
    GoMod,
    Pom,
    Gradle,
    Gemfile,
    Cargo,
    Nuget,
}

/// A manifest's format from its file name.
pub fn format_of(path: &str) -> Option<Format> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "package.json" => Format::Npm,
        "composer.json" => Format::Composer,
        "pyproject.toml" => Format::Pyproject,
        "pipfile" => Format::Pipfile,
        "go.mod" => Format::GoMod,
        "pom.xml" => Format::Pom,
        "build.gradle" | "build.gradle.kts" => Format::Gradle,
        "gemfile" | "gems.rb" => Format::Gemfile,
        "cargo.toml" => Format::Cargo,
        "directory.packages.props" | "packages.config" => Format::Nuget,
        l if l.ends_with(".csproj") || l.ends_with(".fsproj") || l.ends_with(".vbproj") => Format::Nuget,
        l if l.ends_with(".txt") && (l.contains("requirements") || l.contains("constraints") || path.to_ascii_lowercase().contains("requirements/")) => Format::Requirements,
        l if l.ends_with(".in") && l.contains("requirements") => Format::Requirements,
        _ => return None,
    })
}

/// The manifests to look at for a file a scanner reported (a lockfile names its manifest beside it).
/// `*.csproj` stands for any project file of the directory.
pub fn manifest_candidates(file: &str) -> Vec<String> {
    let file = file.trim_start_matches("./").trim_start_matches('/');
    let (dir, name) = match file.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n),
        None => (String::new(), file),
    };
    if format_of(file).is_some() {
        return vec![file.to_string()];
    }
    let beside: &[&str] = match name.to_ascii_lowercase().as_str() {
        "package-lock.json" | "npm-shrinkwrap.json" | "yarn.lock" | "pnpm-lock.yaml" | "bun.lock" => &["package.json"],
        "composer.lock" => &["composer.json"],
        "poetry.lock" | "uv.lock" | "pdm.lock" => &["pyproject.toml"],
        "pipfile.lock" => &["Pipfile"],
        "go.sum" => &["go.mod"],
        "gemfile.lock" => &["Gemfile"],
        "gems.locked" => &["gems.rb"],
        "cargo.lock" => &["Cargo.toml"],
        "gradle.lockfile" => &["build.gradle", "build.gradle.kts"],
        "packages.lock.json" => &["*.csproj"],
        _ => &[],
    };
    beside.iter().map(|m| format!("{dir}{m}")).collect()
}

/// What to say about the lockfile once the manifest is bumped (none: the format has no lockfile we know of beside it).
pub fn lockfile_note(manifest: &str) -> Option<&'static str> {
    Some(match format_of(manifest)? {
        Format::Npm => "Regenerate the lockfile (package-lock.json, yarn.lock or pnpm-lock.yaml) with the package manager, in CI or locally (`npm install`, `yarn install`, `pnpm install`): it isn't edited here.",
        Format::Composer => "Regenerate composer.lock (`composer update <package>`): it isn't edited here.",
        Format::Pyproject => "Regenerate the lockfile if the project has one (`poetry lock`, `uv lock`, `pdm lock`): it isn't edited here.",
        Format::Pipfile => "Regenerate Pipfile.lock (`pipenv lock`): it isn't edited here.",
        Format::Requirements => "If these requirements are compiled from a requirements.in (pip-tools), update that file and recompile.",
        Format::GoMod => "Update go.sum (`go mod tidy`): its checksums can't be computed without downloading the module.",
        Format::Gemfile => "Regenerate Gemfile.lock (`bundle update <gem>`): it isn't edited here.",
        Format::Cargo => "Regenerate Cargo.lock (`cargo update -p <crate>`): it isn't edited here.",
        Format::Gradle => "If the build uses dependency locking, regenerate gradle.lockfile (`./gradlew dependencies --write-locks`).",
        Format::Nuget => "If the project uses packages.lock.json, regenerate it (`dotnet restore --force-evaluate`).",
        Format::Pom => return None,
    })
}

/// Bumps `package` (as its ecosystem names it: `group:artifact` for Maven) to `to` in the manifest at `path`.
pub fn bump(path: &str, content: &str, package: &str, to: &str) -> Result<String, BumpError> {
    let format = format_of(path).ok_or_else(|| BumpError::Unsupported(format!("{path} isn't a manifest format fixes are written for")))?;
    let decls = match format {
        Format::Npm => json_deps(content, package, &["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"], false),
        Format::Composer => json_deps(content, package, &["require", "require-dev"], true),
        Format::Requirements => requirements(content, package),
        Format::Pyproject => pyproject(content, package),
        Format::Pipfile => pipfile(content, package),
        Format::GoMod => go_mod(content, package),
        Format::Pom => pom(content, package),
        Format::Gradle => gradle(content, package),
        Format::Gemfile => gemfile(content, package),
        Format::Cargo => cargo(content, package),
        Format::Nuget => nuget(content, package),
    };
    let to = if format == Format::GoMod && !to.starts_with('v') { format!("v{to}") } else { to.to_string() };
    resolve(content, decls, &to)
}

// ---------------------------------------------------------------- declarations and their versions

/// How a version or range is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// npm, Composer, Cargo, Poetry, RubyGems: an operator (`^`, `~`, `>=`, `~>`, `=`) and one version.
    Range,
    /// PEP 440 clauses (`==1.0`, `>=1.0,<2`); empty or `*` is any version.
    Pep440,
    /// One exact version (Go, Maven, Gradle, NuGet).
    Exact,
}

/// Where a declaration's version is.
#[derive(Debug, Clone)]
enum Version {
    /// The version text's span (inside its quotes or element) and how it is written.
    At(Range<usize>, Style),
    /// No version written: insert this (formatted with the fixed version at `{}`) at the offset.
    Insert(usize, String),
    /// Several constraints of one declaration (a Gemfile's `'>= 1.0', '< 2'`).
    All(Vec<Range<usize>>),
    /// Declared, but not rewritable here.
    Unsupported(String),
}

enum Outcome {
    Edit(Vec<(Range<usize>, String)>),
    Allowed,
    Unsupported(String),
}

fn resolve(content: &str, decls: Vec<Version>, to: &str) -> Result<String, BumpError> {
    if decls.is_empty() {
        return Err(BumpError::NotDeclared);
    }
    let outcomes: Vec<Outcome> = decls.into_iter().map(|d| outcome(content, d, to)).collect();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for o in &outcomes {
        if let Outcome::Edit(e) = o {
            edits.extend(e.iter().cloned());
        }
    }
    if edits.is_empty() {
        if outcomes.iter().any(|o| matches!(o, Outcome::Allowed)) {
            return Err(BumpError::AlreadyAllowed);
        }
        let why = outcomes.into_iter().find_map(|o| if let Outcome::Unsupported(w) = o { Some(w) } else { None }).unwrap_or_default();
        return Err(BumpError::Unsupported(why));
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    edits.dedup_by_key(|(r, _)| r.start);
    let mut out = content.to_string();
    for (r, text) in edits {
        out.replace_range(r, &text);
    }
    Ok(out)
}

fn outcome(content: &str, d: Version, to: &str) -> Outcome {
    match d {
        Version::Unsupported(why) => Outcome::Unsupported(why),
        Version::Insert(at, template) => Outcome::Edit(vec![(at..at, template.replace("{}", to))]),
        Version::At(r, style) => match edit_spec(&content[r.clone()], to, style) {
            Spec::Edit(s) => Outcome::Edit(vec![(r, s)]),
            Spec::Keep | Spec::Allowed => Outcome::Allowed,
            Spec::Unsupported(w) => Outcome::Unsupported(w),
        },
        Version::All(ranges) => {
            let mut edits = Vec::new();
            let mut allowed = false;
            for r in ranges {
                match edit_spec(&content[r.clone()], to, Style::Range) {
                    Spec::Edit(s) => edits.push((r, s)),
                    Spec::Allowed => allowed = true,
                    Spec::Keep => {}
                    Spec::Unsupported(w) => return Outcome::Unsupported(w),
                }
            }
            if !edits.is_empty() {
                Outcome::Edit(edits)
            } else if allowed {
                Outcome::Allowed
            } else {
                Outcome::Unsupported("no lower bound to raise".into())
            }
        }
    }
}

enum Spec {
    Edit(String),
    /// An upper bound or exclusion that doesn't stand in the fix's way.
    Keep,
    Allowed,
    Unsupported(String),
}

fn version_start(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn cmp(a: &str, b: &str) -> Ordering {
    compare_versions(a.trim_start_matches('v'), b.trim_start_matches('v'))
}

/// A version or range rewritten to allow at least `to`, in the same notation.
fn edit_spec(spec: &str, to: &str, style: Style) -> Spec {
    match style {
        Style::Pep440 => pep440(spec, to),
        Style::Exact => {
            let v = spec.trim();
            if !version_start(v.trim_start_matches('v')) || v.contains(['[', '(', ',', '+', '*', '$']) || v.starts_with("latest") {
                return Spec::Unsupported(format!("the version `{v}` isn't a plain version"));
            }
            if cmp(v, to) != Ordering::Less {
                return Spec::Allowed;
            }
            Spec::Edit(spec.replacen(v, to, 1))
        }
        Style::Range => range(spec, to),
    }
}

/// npm, Cargo, Composer, Poetry and RubyGems ranges with one bound: the version moves to the fix, the operator stays.
fn range(spec: &str, to: &str) -> Spec {
    let s = spec.trim();
    if s.is_empty() || s == "*" || s == "x" || s == "latest" {
        return Spec::Edit(format!("^{to}"));
    }
    if s.contains("||") || s.contains('|') || s.contains(',') || s.contains(" - ") || s.split_whitespace().filter(|w| version_start(w.trim_start_matches(['^', '~', '>', '<', '=', 'v']))).count() > 1 {
        return Spec::Unsupported(format!("the range `{s}` has several bounds"));
    }
    let op_len = s.find(|c: char| !"^~><=! ".contains(c)).unwrap_or(s.len());
    let op = s[..op_len].trim();
    let v = s[op_len..].trim_start_matches('v');
    if !version_start(v) {
        return Spec::Unsupported(format!("`{s}` isn't a version or a range"));
    }
    let v = &v[..v.find(char::is_whitespace).unwrap_or(v.len())];
    match op {
        "<" | "<=" => {
            let blocks = if op == "<" { cmp(to, v) != Ordering::Less } else { cmp(to, v) == Ordering::Greater };
            if blocks {
                Spec::Unsupported(format!("the upper bound `{s}` excludes the fixed version {to}"))
            } else {
                Spec::Keep
            }
        }
        "!=" => Spec::Keep,
        "^" | "~" | ">=" | ">" | "~>" | "=" | "==" | "" => {
            if cmp(v, to) != Ordering::Less {
                return Spec::Allowed;
            }
            let start = spec.find(v).unwrap_or(0);
            let mut out = spec.to_string();
            out.replace_range(start..start + v.len(), to);
            Spec::Edit(out)
        }
        _ => Spec::Unsupported(format!("`{s}` isn't a range fixes are written for")),
    }
}

    /// PEP 440 clauses: exact pins move to the fix, lower bounds rise to it, upper bounds must allow it.
fn pep440(spec: &str, to: &str) -> Spec {
    let s = spec.trim();
    if s.is_empty() || s == "*" {
        return Spec::Edit(format!(">={to}"));
    }
    let mut out = spec.to_string();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let (mut lower, mut allowed) = (false, false);
    let mut offset = 0;
    for clause in spec.split(',') {
        let start = offset + (clause.len() - clause.trim_start().len());
        offset += clause.len() + 1;
        let c = clause.trim();
        let op_len = c.find(|ch: char| !"=<>!~ ".contains(ch)).unwrap_or(c.len());
        let op: String = c[..op_len].chars().filter(|ch| !ch.is_whitespace()).collect();
        let v = c[op_len..].trim();
        let v_at = start + c.find(v).unwrap_or(op_len);
        if !version_start(v) {
            return Spec::Unsupported(format!("`{c}` isn't a version clause"));
        }
        match op.as_str() {
            "==" | "===" | "~=" => {
                if v.contains('*') {
                    return Spec::Unsupported(format!("the wildcard pin `{c}`"));
                }
                lower = true;
                if (op != "~=" && cmp(v, to) == Ordering::Equal) || (op == "~=" && cmp(v, to) != Ordering::Less) {
                    allowed = true;
                } else {
                    edits.push((v_at..v_at + v.len(), to.to_string()));
                }
            }
            ">=" | ">" => {
                lower = true;
                if cmp(v, to) != Ordering::Less && !(op == ">" && cmp(v, to) == Ordering::Equal) {
                    allowed = true;
                } else {
                    let op_at = start + c.find(&op[..1]).unwrap_or(0);
                    edits.push((op_at..v_at + v.len(), format!(">={to}")));
                }
            }
            "<" | "<=" => {
                let blocks = if op == "<" { cmp(to, v) != Ordering::Less } else { cmp(to, v) == Ordering::Greater };
                if blocks {
                    return Spec::Unsupported(format!("the upper bound `{c}` excludes the fixed version {to}"));
                }
            }
            "!=" => {}
            _ => return Spec::Unsupported(format!("`{c}` isn't a version clause")),
        }
    }
    if !lower {
        return Spec::Edit(format!(">={to},{}", spec.trim_start()));
    }
    if edits.is_empty() {
        return if allowed { Spec::Allowed } else { Spec::Keep };
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (r, t) in edits {
        out.replace_range(r, &t);
    }
    Spec::Edit(out)
}

// ---------------------------------------------------------------- JSON (package.json, composer.json)

/// The top-level objects of a JSON document: (key, span of the object's inside).
fn top_level_objects(s: &str) -> Vec<(String, Range<usize>)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut i) = (0usize, 0usize);
    let mut last_key: Option<String> = None;
    let mut open_at: Option<(String, usize)> = None;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if depth == 1 {
                    let mut j = i + 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < b.len() && b[j] == b':' {
                        last_key = Some(s[start..i.min(b.len())].to_string());
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                if depth == 2 && b[i] == b'{' {
                    open_at = last_key.take().map(|k| (k, i + 1));
                }
            }
            b'}' | b']' => {
                if depth == 2 {
                    if let Some((k, start)) = open_at.take() {
                        out.push((k, start..i));
                    }
                }
                depth = depth.saturating_sub(1);
            }
            b',' if depth == 1 => last_key = None,
            _ => {}
        }
        i += 1;
    }
    out
}

fn json_deps(content: &str, package: &str, sections: &[&str], ignore_case: bool) -> Vec<Version> {
    let mut out = Vec::new();
    let re = Regex::new(r#""((?:[^"\\]|\\.)*)"\s*:\s*"((?:[^"\\]|\\.)*)""#).unwrap();
    for (key, span) in top_level_objects(content) {
        if !sections.contains(&key.as_str()) {
            continue;
        }
        let body = &content[span.clone()];
        for c in re.captures_iter(body) {
            let (name, spec) = (c.get(1).unwrap(), c.get(2).unwrap());
            let same = if ignore_case { name.as_str().eq_ignore_ascii_case(package) } else { name.as_str() == package };
            if same {
                let r = span.start + spec.start()..span.start + spec.end();
                let s = spec.as_str();
                if s.starts_with("npm:") || s.contains("://") || s.starts_with("git") || s.starts_with("file:") || s.starts_with("workspace:") || s.contains('/') {
                    out.push(Version::Unsupported(format!("{package} is declared as `{s}`, not a version")));
                } else {
                    out.push(Version::At(r, Style::Range));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Python

fn pep503(name: &str) -> String {
    normalize_name("PyPI", name).replace("--", "-")
}

/// A PEP 508 requirement's name and the span of its version clauses (relative to `req`); `None` when it isn't one.
fn pep508(req: &str) -> Option<(String, Range<usize>)> {
    let re = Regex::new(r"^\s*([A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?)\s*(\[[^\]]*\])?\s*").unwrap();
    let m = re.captures(req)?;
    let name = m.get(1)?.as_str().to_string();
    let after = m.get(0)?.end();
    let rest = &req[after..];
    let end = rest.find([';', '#']).unwrap_or(rest.len());
    let spec = &rest[..end];
    if spec.trim_start().starts_with('@') {
        return Some((name, usize::MAX..usize::MAX));
    }
    let trimmed = spec.trim_end();
    let (s, e) = if trimmed.trim_start().starts_with('(') && trimmed.ends_with(')') {
        let open = trimmed.find('(').unwrap();
        (after + open + 1, after + trimmed.len() - 1)
    } else {
        (after, after + trimmed.len())
    };
    let clauses = &req[s..e];
    if !clauses.trim().is_empty() && !clauses.trim_start().starts_with(['=', '<', '>', '!', '~']) {
        return None;
    }
    Some((name, s..e))
}

/// A requirement at `base` in the file: its version clauses, or an insertion when it has none.
fn requirement_version(content: &str, base: usize, req: &str, package: &str) -> Option<Version> {
    let (name, r) = pep508(req)?;
    if pep503(&name) != pep503(package) {
        return None;
    }
    if r.start == usize::MAX {
        return Some(Version::Unsupported(format!("{package} is installed from a URL")));
    }
    let _ = content;
    if req[r.clone()].trim().is_empty() {
        // `requests` or `requests ; python_version > "3"`: the bound goes right after the name (and extras).
        return Some(Version::Insert(base + r.start, ">={}".into()));
    }
    Some(Version::At(base + r.start..base + r.end, Style::Pep440))
}

fn requirements(content: &str, package: &str) -> Vec<Version> {
    let mut out = Vec::new();
    let mut offset = 0;
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    for (i, raw) in lines.iter().enumerate() {
        let base = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let code = line.split(" #").next().unwrap_or(line);
        let code = code.split(" \\").next().unwrap_or(code).trim_end_matches('\\');
        if code.trim_start().starts_with(['-', '#']) || code.trim().is_empty() {
            continue;
        }
        let req = code.split(" --").next().unwrap_or(code);
        let Some(v) = requirement_version(content, base, req, package) else { continue };
        let hashed = line.contains("--hash") || (line.trim_end().ends_with('\\') && lines.get(i + 1).is_some_and(|n| n.contains("--hash")));
        out.push(if hashed { Version::Unsupported("the requirements are pinned with hashes: regenerate them with pip-compile --generate-hashes".into()) } else { v });
    }
    out
}

/// The quoted strings of a TOML line: (span of the inside).
fn quoted(line: &str) -> Vec<Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            break;
        }
        if b[i] == b'"' || b[i] == b'\'' {
            let q = b[i];
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != q {
                if b[i] == b'\\' && q == b'"' {
                    i += 1;
                }
                i += 1;
            }
            out.push(start..i.min(b.len()));
        }
        i += 1;
    }
    out
}

/// A TOML table header's name (`[tool.poetry.dependencies]` → `tool.poetry.dependencies`), quotes removed.
fn table(line: &str) -> Option<String> {
    let t = line.trim();
    let t = t.split(" #").next().unwrap_or(t).trim();
    if !t.starts_with('[') || !t.ends_with(']') {
        return None;
    }
    Some(t.trim_matches(['[', ']']).trim().replace(['"', '\''], ""))
}

/// A TOML `key = value` line: the key (unquoted) and the value's offset in the line.
fn key_value(line: &str) -> Option<(String, usize)> {
    let re = Regex::new(r#"^\s*("[^"]+"|'[^']+'|[A-Za-z0-9_.-]+)\s*=\s*"#).unwrap();
    let m = re.captures(line)?;
    Some((m.get(1)?.as_str().trim_matches(['"', '\'']).to_string(), m.get(0)?.end()))
}

/// Lines of a TOML file with their offsets and the table they're in.
fn toml_lines(content: &str) -> Vec<(usize, &str, String)> {
    let mut out = Vec::new();
    let (mut offset, mut current) = (0, String::new());
    for raw in content.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        if let Some(t) = table(line) {
            current = t;
        } else {
            out.push((offset, line, current.clone()));
        }
        offset += raw.len();
    }
    out
}

/// `name = "spec"`, `name = { version = "spec", … }` in a dependency table, or `version = "spec"` in the
/// package's own table (`[dependencies.serde]`); `name` compared by `same`.
fn toml_table_deps(content: &str, is_deps: impl Fn(&str) -> bool, same: impl Fn(&str) -> bool, style: Style) -> Vec<Version> {
    let mut out = Vec::new();
    let package_re = Regex::new(r#"package\s*=\s*["']([^"']+)["']"#).unwrap();
    let workspace_re = Regex::new(r"workspace\s*=\s*true").unwrap();
    let version_re = Regex::new(r#"\bversion\s*=\s*["']([^"']*)["']"#).unwrap();
    for (base, line, t) in toml_lines(content) {
        let Some((key, at)) = key_value(line) else { continue };
        let value = &line[at..];
        if let Some((parent, name)) = t.rsplit_once('.') {
            if is_deps(parent) && same(name) && key == "version" {
                if let Some(q) = quoted(value).first() {
                    out.push(Version::At(base + at + q.start..base + at + q.end, style));
                }
                continue;
            }
        }
        if !is_deps(&t) {
            continue;
        }
        let renamed = package_re.captures(value).map(|c| c[1].to_string());
        let name = key.strip_suffix(".workspace").unwrap_or(&key);
        if !same(renamed.as_deref().unwrap_or(name)) {
            continue;
        }
        if key.ends_with(".workspace") || workspace_re.is_match(value) {
            out.push(Version::Unsupported("the version is inherited from the workspace's manifest".into()));
        } else if value.trim_start().starts_with(['"', '\'']) {
            let q = quoted(value)[0].clone();
            out.push(Version::At(base + at + q.start..base + at + q.end, style));
        } else if value.trim_start().starts_with('{') {
            match version_re.captures(value).and_then(|c| c.get(1)) {
                Some(m) => out.push(Version::At(base + at + m.start()..base + at + m.end(), style)),
                None => out.push(Version::Unsupported("it comes from git or a path, not a version".into())),
            }
        }
    }
    out
}

fn pyproject(content: &str, package: &str) -> Vec<Version> {
    let mut out = Vec::new();
    // PEP 621 / PEP 735 arrays of requirement strings.
    let mut in_array = false;
    for (base, line, t) in toml_lines(content) {
        let pep621 = t == "project" || t == "project.optional-dependencies" || t == "dependency-groups";
        let mut from = 0;
        if !in_array && pep621 {
            if let Some((key, at)) = key_value(line) {
                if line[at..].trim_start().starts_with('[') && (t != "project" || key == "dependencies") {
                    in_array = true;
                    from = at;
                }
            }
        }
        if !in_array {
            continue;
        }
        let rest = &line[from..];
        for q in quoted(rest) {
            let req = &rest[q.clone()];
            if let Some(v) = requirement_version(content, base + from + q.start, req, package) {
                out.push(v);
            }
        }
        // The array ends at a `]` outside the strings.
        let mut stripped = rest.to_string();
        for q in quoted(rest).into_iter().rev() {
            stripped.replace_range(q, "");
        }
        if stripped.split('#').next().unwrap_or("").contains(']') {
            in_array = false;
        }
    }
    let poetry = |t: &str| t == "tool.poetry.dependencies" || t == "tool.poetry.dev-dependencies" || (t.starts_with("tool.poetry.group.") && t.ends_with(".dependencies"));
    out.extend(toml_table_deps(content, poetry, |n| pep503(n) == pep503(package), Style::Range));
    out
}

fn pipfile(content: &str, package: &str) -> Vec<Version> {
    toml_table_deps(content, |t| t == "packages" || t == "dev-packages", |n| pep503(n) == pep503(package), Style::Pep440)
}

// ---------------------------------------------------------------- Rust

fn cargo(content: &str, package: &str) -> Vec<Version> {
    let deps = |t: &str| {
        let last = t.rsplit('.').next().unwrap_or(t);
        matches!(last, "dependencies" | "dev-dependencies" | "build-dependencies") && (t == last || t == "workspace.dependencies" || t.starts_with("target."))
    };
    toml_table_deps(content, deps, |n| n == package, Style::Range)
}

// ---------------------------------------------------------------- Go

fn go_mod(content: &str, package: &str) -> Vec<Version> {
    let mut out = Vec::new();
    let (mut offset, mut block) = (0, "");
    for raw in content.split_inclusive('\n') {
        let base = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let code = line.split("//").next().unwrap_or(line);
        let words: Vec<&str> = code.split_whitespace().collect();
        match words.as_slice() {
            [kw, "("] => block = if *kw == "require" { "require" } else if *kw == "replace" { "replace" } else { "other" },
            [")"] => block = "",
            ["replace", m, ..] | [m, ..] if (block == "replace" || words[0] == "replace") && (*m == package) => {
                return vec![Version::Unsupported(format!("{package} is replaced by a replace directive"))];
            }
            ["require", m, v] | [m, v] if (words[0] == "require" || block == "require") && *m == package => {
                let at = base + code.rfind(v).unwrap_or(0);
                out.push(Version::At(at..at + v.len(), Style::Exact));
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------- Maven, Gradle

fn pom(content: &str, package: &str) -> Vec<Version> {
    let Some((group, artifact)) = package.split_once(':') else { return vec![] };
    let mut out = Vec::new();
    let dep = Regex::new(r"(?s)<dependency>(.*?)</dependency>").unwrap();
    let props = Regex::new(r"(?s)<properties>(.*?)</properties>").unwrap();
    let el = |body: &str, tag: &str| Regex::new(&format!(r"(?s)<{tag}>\s*(.*?)\s*</{tag}>")).unwrap().captures(body).and_then(|c| c.get(1)).map(|m| (m.start(), m.as_str().to_string()));
    for c in dep.captures_iter(content) {
        let body = c.get(1).unwrap();
        let (Some((_, g)), Some((_, a))) = (el(body.as_str(), "groupId"), el(body.as_str(), "artifactId")) else { continue };
        if g != group || a != artifact {
            continue;
        }
        match el(body.as_str(), "version") {
            None => out.push(Version::Unsupported("its version is managed by a parent POM or an imported BOM".into())),
            Some((at, v)) => {
                if let Some(prop) = v.strip_prefix("${").and_then(|p| p.strip_suffix('}')) {
                    let found = props.captures(content).and_then(|p| {
                        let pb = p.get(1).unwrap();
                        el(pb.as_str(), &regex::escape(prop)).map(|(s, v)| (pb.start() + s, v))
                    });
                    match found {
                        Some((s, pv)) => out.push(Version::At(s..s + pv.len(), Style::Exact)),
                        None => out.push(Version::Unsupported(format!("its version comes from the property {prop}, defined in a parent POM"))),
                    }
                } else {
                    let s = body.start() + at;
                    out.push(Version::At(s..s + v.len(), Style::Exact));
                }
            }
        }
    }
    out
}

fn gradle(content: &str, package: &str) -> Vec<Version> {
    let Some((group, artifact)) = package.split_once(':') else { return vec![] };
    let (g, a) = (regex::escape(group), regex::escape(artifact));
    let mut out = Vec::new();
    let coords = Regex::new(&format!(r#"["']{g}:{a}:([^"'@:]+)"#)).unwrap();
    let map = Regex::new(&format!(r#"group\s*[:=]\s*["']{g}["']\s*,\s*name\s*[:=]\s*["']{a}["']\s*,\s*version\s*[:=]\s*["']([^"']+)["']"#)).unwrap();
    for m in coords.captures_iter(content).chain(map.captures_iter(content)).filter_map(|c| c.get(1)) {
        let v = m.as_str();
        if let Some(var) = v.strip_prefix("${").and_then(|x| x.strip_suffix('}')).or_else(|| v.strip_prefix('$')) {
            let def = Regex::new(&format!(r#"(?m)^\s*(?:def\s+|val\s+|var\s+|ext\.|extra\[)?["']?{}["']?\]?\s*=\s*["']([^"']+)["']"#, regex::escape(var))).unwrap();
            match def.captures(content).and_then(|c| c.get(1)) {
                Some(d) => out.push(Version::At(d.range(), Style::Exact)),
                None => out.push(Version::Unsupported(format!("its version comes from {var}, defined outside this file (gradle.properties or a version catalog)"))),
            }
        } else {
            out.push(Version::At(m.range(), Style::Exact));
        }
    }
    out
}

// ---------------------------------------------------------------- RubyGems, NuGet

fn gemfile(content: &str, package: &str) -> Vec<Version> {
    let mut out = Vec::new();
    let head = Regex::new(r#"^\s*gem\s*\(?\s*(["'])([^"']+)["']"#).unwrap();
    let constraint = Regex::new(r#"^\s*,\s*(["'])([^"']*)["']"#).unwrap();
    let mut offset = 0;
    for raw in content.split_inclusive('\n') {
        let base = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let Some(h) = head.captures(line) else { continue };
        if !h[2].eq_ignore_ascii_case(package) {
            continue;
        }
        let quote = h[1].to_string();
        let mut at = h.get(0).unwrap().end();
        let mut ranges = Vec::new();
        while let Some(c) = constraint.captures(&line[at..]) {
            let m = c.get(2).unwrap();
            ranges.push(base + at + m.start()..base + at + m.end());
            at += c.get(0).unwrap().end();
        }
        let options = &line[at..];
        if ["git:", "path:", "github:", ":git", ":path", ":github"].iter().any(|o| options.contains(o)) {
            out.push(Version::Unsupported(format!("{package} comes from git or a path")));
        } else if ranges.is_empty() {
            let end = base + h.get(0).unwrap().end();
            out.push(Version::Insert(end, format!(", {quote}>= {{}}{quote}")));
        } else {
            out.push(Version::All(ranges));
        }
    }
    out
}

fn nuget(content: &str, package: &str) -> Vec<Version> {
    let mut out = Vec::new();
    let p = regex::escape(package);
    let tag = Regex::new(&format!(r#"(?is)<(PackageReference|PackageVersion|package)\s[^>]*?\b(?:Include|Update|id)\s*=\s*"{p}"[^>]*?(/?)>"#)).unwrap();
    let attr = Regex::new(r#"(?i)\bVersion\s*=\s*"([^"]*)""#).unwrap();
    let child = Regex::new(r"(?is)^(.*?)<Version>\s*([^<]*?)\s*</Version>").unwrap();
    for c in tag.captures_iter(content) {
        let whole = c.get(0).unwrap();
        if let Some(v) = attr.captures(whole.as_str()).and_then(|a| a.get(1)) {
            out.push(nuget_version(whole.start() + v.start()..whole.start() + v.end(), v.as_str()));
        } else if &c[2] != "/" {
            let after = &content[whole.end()..];
            let close = after.to_ascii_lowercase().find(&format!("</{}>", c[1].to_ascii_lowercase())).unwrap_or(after.len());
            match child.captures(&after[..close]).and_then(|x| x.get(2)) {
                Some(v) => out.push(nuget_version(whole.end() + v.start()..whole.end() + v.end(), v.as_str())),
                None => out.push(Version::Unsupported("its version is set centrally (Directory.Packages.props)".into())),
            }
        } else {
            out.push(Version::Unsupported("its version is set centrally (Directory.Packages.props)".into()));
        }
    }
    out
}

fn nuget_version(r: Range<usize>, v: &str) -> Version {
    if v.contains("$(") {
        Version::Unsupported(format!("its version comes from the MSBuild property {v}"))
    } else {
        Version::At(r, Style::Exact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(path: &str, before: &str, package: &str, to: &str) -> String {
        bump(path, before, package, to).unwrap_or_else(|e| panic!("{path}: {e:?}"))
    }

    #[test]
    fn manifests_are_known_by_name_and_lockfiles_point_to_them() {
        assert_eq!(format_of("services/api/requirements-dev.txt"), Some(Format::Requirements));
        assert_eq!(format_of("src/Storefront.Api/Storefront.Api.csproj"), Some(Format::Nuget));
        assert_eq!(format_of("build.gradle.kts"), Some(Format::Gradle));
        assert_eq!(format_of("yarn.lock"), None);
        assert_eq!(manifest_candidates("web/package-lock.json"), vec!["web/package.json"]);
        assert_eq!(manifest_candidates("go.sum"), vec!["go.mod"]);
        assert_eq!(manifest_candidates("poetry.lock"), vec!["pyproject.toml"]);
        assert_eq!(manifest_candidates("./api/Gemfile.lock"), vec!["api/Gemfile"]);
        assert_eq!(manifest_candidates("src/App/packages.lock.json"), vec!["src/App/*.csproj"]);
        assert_eq!(manifest_candidates("pom.xml"), vec!["pom.xml"]);
        assert!(manifest_candidates("Dockerfile").is_empty());
        assert!(lockfile_note("package.json").unwrap().contains("Regenerate the lockfile"));
        assert!(lockfile_note("go.mod").unwrap().contains("go mod tidy"));
        assert_eq!(lockfile_note("pom.xml"), None);
    }

    #[test]
    fn package_json_keeps_the_range_operator_and_formatting() {
        let before = "{\n  \"name\": \"storefront-web\",\n  \"dependencies\": {\n    \"lodash\": \"^4.17.20\",\n    \"express\":   \"4.18.1\"\n  },\n  \"devDependencies\": { \"lodash-es\": \"~4.17.0\" }\n}\n";
        assert_eq!(ok("package.json", before, "lodash", "4.17.21"), before.replace("^4.17.20", "^4.17.21"));
        assert_eq!(ok("package.json", before, "express", "4.19.2"), before.replace("\"4.18.1\"", "\"4.19.2\""));
        assert_eq!(ok("package.json", before, "lodash-es", "4.17.21"), before.replace("~4.17.0", "~4.17.21"));
        assert_eq!(bump("package.json", before, "minimist", "1.2.6"), Err(BumpError::NotDeclared));
        assert_eq!(bump("package.json", before, "storefront-web", "1.0.0"), Err(BumpError::NotDeclared), "the name isn't a dependency");
        let fixed = "{\"dependencies\": {\"lodash\": \"^4.17.21\"}}";
        assert_eq!(bump("package.json", fixed, "lodash", "4.17.21"), Err(BumpError::AlreadyAllowed));
        let ranged = "{\"dependencies\": {\"lodash\": \">=4.0.0 <5\"}}";
        assert!(matches!(bump("package.json", ranged, "lodash", "4.17.21"), Err(BumpError::Unsupported(_))));
        let git = "{\"dependencies\": {\"lodash\": \"github:lodash/lodash\"}}";
        assert!(matches!(bump("package.json", git, "lodash", "4.17.21"), Err(BumpError::Unsupported(_))));
        let scoped = "{\"dependencies\": {\"@babel/traverse\": \"7.22.5\"}}";
        assert_eq!(ok("package.json", scoped, "@babel/traverse", "7.23.2"), "{\"dependencies\": {\"@babel/traverse\": \"7.23.2\"}}");
        let star = "{\"dependencies\": {\"lodash\": \"*\"}}";
        assert_eq!(ok("package.json", star, "lodash", "4.17.21"), "{\"dependencies\": {\"lodash\": \"^4.17.21\"}}");
    }

    #[test]
    fn composer_json_compares_names_without_case() {
        let before = "{\n    \"require\": {\n        \"php\": \">=8.1\",\n        \"guzzlehttp/guzzle\": \"^7.4.0\"\n    }\n}\n";
        assert_eq!(ok("composer.json", before, "GuzzleHttp/Guzzle", "7.4.5"), before.replace("^7.4.0", "^7.4.5"));
        let either = "{\"require\": {\"guzzlehttp/guzzle\": \"^6.5|^7.0\"}}";
        assert!(matches!(bump("composer.json", either, "guzzlehttp/guzzle", "7.4.5"), Err(BumpError::Unsupported(_))));
    }

    #[test]
    fn requirements_pins_bounds_and_names_follow_pep_440_and_503() {
        let before = "# API\nDjango==4.2.1  # web\nrequests >= 2.20, < 3\nPyYAML\nurllib3[socks]==1.26.5 ; python_version >= \"3.8\"\n-r base.txt\n";
        let after = ok("requirements.txt", before, "django", "4.2.7");
        assert_eq!(after, before.replace("Django==4.2.1", "Django==4.2.7"));
        assert_eq!(ok("requirements.txt", before, "requests", "2.31.0"), before.replace(">= 2.20", ">=2.31.0"));
        assert_eq!(ok("requirements.txt", before, "pyyaml", "5.4"), before.replace("PyYAML\n", "PyYAML>=5.4\n"));
        assert_eq!(ok("requirements.txt", before, "urllib3", "1.26.18"), before.replace("1.26.5", "1.26.18"));
        let capped = "requests>=2.20,<2.30\n";
        assert!(matches!(bump("requirements.txt", capped, "requests", "2.31.0"), Err(BumpError::Unsupported(w)) if w.contains("upper bound")));
        let hashed = "requests==2.28.0 \\\n    --hash=sha256:abc\n";
        assert!(matches!(bump("requirements.txt", hashed, "requests", "2.31.0"), Err(BumpError::Unsupported(w)) if w.contains("hashes")));
        assert_eq!(bump("requirements.txt", "requests==2.31.0\n", "requests", "2.31.0"), Err(BumpError::AlreadyAllowed));
        assert_eq!(bump("requirements.txt", "flask==2.0\n", "requests", "2.31.0"), Err(BumpError::NotDeclared));
        assert_eq!(ok("requirements/prod.txt", "zope.interface==5.0\n", "zope-interface", "5.4"), "zope.interface==5.4\n");
    }

    #[test]
    fn pyproject_edits_pep_621_arrays_and_poetry_tables() {
        let pep621 = "[project]\nname = \"requests-tools\"\ndependencies = [\n    \"requests>=2.25\",\n    'jinja2==3.1.2',\n]\n\n[project.optional-dependencies]\ndev = [\"pytest==7.0.0\"]\n\n[tool.black]\nline-length = 100\n";
        assert_eq!(ok("pyproject.toml", pep621, "requests", "2.31.0"), pep621.replace("requests>=2.25", "requests>=2.31.0"));
        assert_eq!(ok("pyproject.toml", pep621, "Jinja2", "3.1.3"), pep621.replace("3.1.2", "3.1.3"));
        assert_eq!(ok("pyproject.toml", pep621, "pytest", "7.2.0"), pep621.replace("7.0.0", "7.2.0"));
        assert_eq!(bump("pyproject.toml", pep621, "black", "24.0"), Err(BumpError::NotDeclared));
        let poetry = "[tool.poetry]\nname = \"billing\"\n\n[tool.poetry.dependencies]\npython = \"^3.11\"\nrequests = \"^2.28.0\"\ncelery = { version = \"~5.2.0\", extras = [\"redis\"] }\n\n[tool.poetry.group.dev.dependencies]\npytest = \"7.1.0\"\n\n[tool.poetry.dependencies.Pillow]\nversion = \"9.0.0\"\n";
        assert_eq!(ok("pyproject.toml", poetry, "requests", "2.31.0"), poetry.replace("^2.28.0", "^2.31.0"));
        assert_eq!(ok("pyproject.toml", poetry, "celery", "5.2.2"), poetry.replace("~5.2.0", "~5.2.2"));
        assert_eq!(ok("pyproject.toml", poetry, "pytest", "7.2.0"), poetry.replace("7.1.0", "7.2.0"));
        assert_eq!(ok("pyproject.toml", poetry, "pillow", "10.0.1"), poetry.replace("9.0.0", "10.0.1"));
    }

    #[test]
    fn pipfile_uses_pip_specifiers() {
        let before = "[[source]]\nurl = \"https://pypi.org/simple\"\n\n[packages]\nrequests = \"==2.28.0\"\nflask = \"*\"\ndjango = {version = \">=4.0\", extras = [\"bcrypt\"]}\n\n[dev-packages]\npytest = \"*\"\n";
        assert_eq!(ok("Pipfile", before, "requests", "2.31.0"), before.replace("==2.28.0", "==2.31.0"));
        assert_eq!(ok("Pipfile", before, "flask", "2.3.2"), before.replace("flask = \"*\"", "flask = \">=2.3.2\""));
        assert_eq!(ok("Pipfile", before, "Django", "4.2.7"), before.replace(">=4.0", ">=4.2.7"));
    }

    #[test]
    fn go_mod_edits_require_lines_and_refuses_replaced_modules() {
        let before = "module github.com/example-org/order-service\n\ngo 1.22\n\nrequire github.com/gin-gonic/gin v1.9.0\n\nrequire (\n\tgolang.org/x/net v0.17.0 // indirect\n\tgoogle.golang.org/grpc v1.58.2\n)\n";
        assert_eq!(ok("go.mod", before, "golang.org/x/net", "0.23.0"), before.replace("v0.17.0", "v0.23.0"));
        assert_eq!(ok("go.mod", before, "google.golang.org/grpc", "v1.58.3"), before.replace("v1.58.2", "v1.58.3"));
        assert_eq!(ok("go.mod", before, "github.com/gin-gonic/gin", "1.9.1"), before.replace("v1.9.0", "v1.9.1"));
        assert_eq!(bump("go.mod", before, "github.com/example-org/order-service", "1.0.0"), Err(BumpError::NotDeclared));
        let replaced = format!("{before}\nreplace golang.org/x/net => golang.org/x/net v0.20.0\n");
        assert!(matches!(bump("go.mod", &replaced, "golang.org/x/net", "0.23.0"), Err(BumpError::Unsupported(w)) if w.contains("replace")));
    }

    #[test]
    fn pom_edits_the_version_or_the_property_it_names() {
        let before = "<project>\n  <properties>\n    <jackson.version>2.13.0</jackson.version>\n  </properties>\n  <dependencies>\n    <dependency>\n      <groupId>org.apache.logging.log4j</groupId>\n      <artifactId>log4j-core</artifactId>\n      <version>2.14.1</version>\n    </dependency>\n    <dependency>\n      <groupId>com.fasterxml.jackson.core</groupId>\n      <artifactId>jackson-databind</artifactId>\n      <version>${jackson.version}</version>\n    </dependency>\n    <dependency>\n      <groupId>org.springframework</groupId>\n      <artifactId>spring-web</artifactId>\n    </dependency>\n  </dependencies>\n</project>\n";
        assert_eq!(ok("pom.xml", before, "org.apache.logging.log4j:log4j-core", "2.17.1"), before.replace("2.14.1", "2.17.1"));
        assert_eq!(ok("pom.xml", before, "com.fasterxml.jackson.core:jackson-databind", "2.13.4.2"), before.replace("<jackson.version>2.13.0", "<jackson.version>2.13.4.2"));
        assert!(matches!(bump("pom.xml", before, "org.springframework:spring-web", "6.0.0"), Err(BumpError::Unsupported(w)) if w.contains("parent POM")));
        assert_eq!(bump("pom.xml", before, "org.yaml:snakeyaml", "2.0"), Err(BumpError::NotDeclared));
    }

    #[test]
    fn gradle_edits_coordinates_maps_and_version_variables() {
        let groovy = "def jacksonVersion = '2.13.0'\ndependencies {\n    implementation 'org.apache.logging.log4j:log4j-core:2.14.1'\n    implementation \"com.fasterxml.jackson.core:jackson-databind:$jacksonVersion\"\n    testImplementation group: 'junit', name: 'junit', version: '4.12'\n}\n";
        assert_eq!(ok("build.gradle", groovy, "org.apache.logging.log4j:log4j-core", "2.17.1"), groovy.replace("2.14.1", "2.17.1"));
        assert_eq!(ok("build.gradle", groovy, "com.fasterxml.jackson.core:jackson-databind", "2.13.4"), groovy.replace("'2.13.0'", "'2.13.4'"));
        assert_eq!(ok("build.gradle", groovy, "junit:junit", "4.13.1"), groovy.replace("4.12", "4.13.1"));
        let kts = "val netty = \"4.1.90.Final\"\ndependencies {\n    implementation(\"io.netty:netty-codec-http:${netty}\")\n    implementation(libs.guava)\n}\n";
        assert_eq!(ok("build.gradle.kts", kts, "io.netty:netty-codec-http", "4.1.100.Final"), kts.replace("4.1.90.Final", "4.1.100.Final"));
        let elsewhere = "dependencies { implementation \"org.yaml:snakeyaml:$snakeyamlVersion\" }\n";
        assert!(matches!(bump("build.gradle", elsewhere, "org.yaml:snakeyaml", "2.0"), Err(BumpError::Unsupported(w)) if w.contains("gradle.properties")));
    }

    #[test]
    fn gemfile_raises_constraints_or_adds_one() {
        let before = "source 'https://rubygems.org'\n\ngem 'rails', '~> 7.0.4'\ngem \"nokogiri\", \">= 1.13\", \"< 2\"\ngem 'rack'\ngem 'sidekiq', git: 'https://github.com/example-org/sidekiq'\n";
        assert_eq!(ok("Gemfile", before, "rails", "7.0.8"), before.replace("~> 7.0.4", "~> 7.0.8"));
        assert_eq!(ok("Gemfile", before, "nokogiri", "1.16.5"), before.replace(">= 1.13", ">= 1.16.5"));
        assert_eq!(ok("Gemfile", before, "rack", "2.2.8.1"), before.replace("gem 'rack'", "gem 'rack', '>= 2.2.8.1'"));
        assert!(matches!(bump("Gemfile", before, "sidekiq", "7.0.0"), Err(BumpError::Unsupported(_))));
        assert!(matches!(bump("Gemfile", before, "nokogiri", "2.1.0"), Err(BumpError::Unsupported(w)) if w.contains("upper bound")));
    }

    #[test]
    fn cargo_toml_edits_strings_inline_tables_and_dependency_tables() {
        let before = "[package]\nname = \"order-worker\"\nversion = \"0.1.0\"\n\n[dependencies]\ntokio = { version = \"1.28\", features = [\"full\"] }\nh2 = \"0.3.20\"\nrustls = { workspace = true }\n\n[dev-dependencies.openssl]\nversion = \"0.10.50\"\n\n[target.'cfg(unix)'.dependencies]\nnix = \"0.26\"\n";
        assert_eq!(ok("Cargo.toml", before, "tokio", "1.29.1"), before.replace("\"1.28\"", "\"1.29.1\""));
        assert_eq!(ok("Cargo.toml", before, "h2", "0.3.24"), before.replace("0.3.20", "0.3.24"));
        assert_eq!(ok("Cargo.toml", before, "openssl", "0.10.55"), before.replace("0.10.50", "0.10.55"));
        assert_eq!(ok("Cargo.toml", before, "nix", "0.27.1"), before.replace("\"0.26\"", "\"0.27.1\""));
        assert!(matches!(bump("Cargo.toml", before, "rustls", "0.21.0"), Err(BumpError::Unsupported(w)) if w.contains("workspace")));
        assert_eq!(bump("Cargo.toml", before, "order-worker", "1.0.0"), Err(BumpError::NotDeclared));
    }

    #[test]
    fn csproj_edits_attributes_and_child_elements() {
        let before = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"12.0.1\" />\n    <PackageReference Include=\"System.Text.Encodings.Web\">\n      <Version>4.5.0</Version>\n    </PackageReference>\n    <PackageReference Include=\"Serilog\" Version=\"$(SerilogVersion)\" />\n  </ItemGroup>\n</Project>\n";
        assert_eq!(ok("Api.csproj", before, "newtonsoft.json", "13.0.1"), before.replace("12.0.1", "13.0.1"));
        assert_eq!(ok("Api.csproj", before, "System.Text.Encodings.Web", "4.5.1"), before.replace("4.5.0", "4.5.1"));
        assert!(matches!(bump("Api.csproj", before, "Serilog", "3.0.0"), Err(BumpError::Unsupported(w)) if w.contains("MSBuild")));
        let central = "<Project>\n  <ItemGroup>\n    <PackageVersion Include=\"Newtonsoft.Json\" Version=\"12.0.1\" />\n  </ItemGroup>\n</Project>\n";
        assert_eq!(ok("Directory.Packages.props", central, "Newtonsoft.Json", "13.0.1"), central.replace("12.0.1", "13.0.1"));
    }
}
