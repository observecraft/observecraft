//! Code Security's pure model (no I/O, tested here): package ecosystems and versions, OSV advisories
//! and their affected ranges, CVSS 3 scores, and the payloads Datadog's tools send — tracer
//! telemetry (libraries a service loads), `datadog-ci sbom upload`, `datadog-ci sarif upload` — and
//! the IAST vulnerabilities tracers put on spans (`_dd.iast.json`).
//!
//! Payload shapes follow Datadog's open-source senders (`datadog-ci`, `dd-trace-*`, Apache-2.0); no
//! code is copied from them.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

// ================================================================ ecosystems

/// OSV ecosystem of a tracer's `language_name` or an SBOM's `language`.
pub fn ecosystem_of_language(language: &str) -> Option<&'static str> {
    Some(match language.to_ascii_lowercase().as_str() {
        "java" | "jvm" | "kotlin" | "scala" => "Maven",
        "nodejs" | "node" | "javascript" | "js" | "npm" => "npm",
        "python" => "PyPI",
        "dotnet" | ".net" | "csharp" => "NuGet",
        "go" | "golang" => "Go",
        "ruby" => "RubyGems",
        "php" => "Packagist",
        "rust" => "crates.io",
        _ => return None,
    })
}

/// Display language of an ecosystem.
pub fn language_of_ecosystem(ecosystem: &str) -> &'static str {
    match ecosystem {
        "Maven" => "java",
        "npm" => "nodejs",
        "PyPI" => "python",
        "NuGet" => "dotnet",
        "Go" => "go",
        "RubyGems" => "ruby",
        "Packagist" => "php",
        "crates.io" => "rust",
        _ => "",
    }
}

/// `pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@11.0.9` → (Maven, `org.apache.tomcat.embed:tomcat-embed-core`, 11.0.9).
pub fn parse_purl(purl: &str) -> Option<(&'static str, String, String)> {
    let rest = purl.strip_prefix("pkg:")?;
    let rest = rest.split(['?', '#']).next()?;
    let (kind, rest) = rest.split_once('/')?;
    let (path, version) = rest.rsplit_once('@').map_or((rest, ""), |(p, v)| (p, v));
    let path = percent_decode(path);
    let version = percent_decode(version);
    let (eco, name) = match kind {
        "maven" => ("Maven", path.replacen('/', ":", 1)),
        "npm" => ("npm", path),
        "pypi" => ("PyPI", path.to_lowercase().replace('_', "-")),
        "nuget" => ("NuGet", path),
        "golang" => ("Go", path),
        "gem" => ("RubyGems", path),
        "composer" => ("Packagist", path),
        "cargo" => ("crates.io", path),
        _ => return None,
    };
    Some((eco, name, version))
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A package name as its ecosystem compares names (PyPI normalizes case, `_` and `.`).
pub fn normalize_name(ecosystem: &str, name: &str) -> String {
    match ecosystem {
        "PyPI" => name.to_lowercase().replace(['_', '.'], "-"),
        "NuGet" | "Packagist" => name.to_lowercase(),
        _ => name.to_string(),
    }
}

/// `git@github.com:example-org/storefront.git`, `https://github.com/example-org/storefront` → `github.com/example-org/storefront`.
pub fn normalize_repo(url: &str) -> String {
    let mut s = url.trim().trim_end_matches('/').trim_end_matches(".git").to_string();
    for p in ["https://", "http://", "ssh://", "git://"] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    if let Some((_, r)) = s.split_once('@') {
        s = r.to_string();
    }
    if let Some((host, path)) = s.split_once(':') {
        if !path.starts_with(|c: char| c.is_ascii_digit()) {
            s = format!("{host}/{path}");
        }
    }
    s
}

// ================================================================ versions

#[derive(Debug, PartialEq)]
enum Tok {
    Num(u64),
    Word(String),
}

fn tokens(v: &str) -> Vec<Tok> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    // Build metadata never orders versions.
    let v = v.split('+').next().unwrap_or(v);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut digits = false;
    let flush = |cur: &mut String, digits: bool, out: &mut Vec<Tok>| {
        if !cur.is_empty() {
            out.push(if digits { Tok::Num(cur.parse().unwrap_or(u64::MAX)) } else { Tok::Word(cur.to_ascii_lowercase()) });
            cur.clear();
        }
    };
    for c in v.chars() {
        if c.is_ascii_alphanumeric() {
            let d = c.is_ascii_digit();
            if !cur.is_empty() && d != digits {
                flush(&mut cur, digits, &mut out);
            }
            digits = d;
            cur.push(c);
        } else {
            flush(&mut cur, digits, &mut out);
        }
    }
    flush(&mut cur, digits, &mut out);
    out
}

/// Rank of a qualifier against a release (0): pre-releases below, patches above.
fn qualifier(w: &str) -> i32 {
    match w {
        "dev" | "snapshot" => -6,
        "alpha" | "a" => -5,
        "beta" | "b" => -4,
        "milestone" | "m" => -3,
        "rc" | "cr" | "c" | "pre" | "preview" => -2,
        "final" | "ga" | "release" | "r" => 0,
        "sp" | "post" | "patch" | "p" => 1,
        _ => -1,
    }
}

/// Compares two versions the way package managers do, closely enough for advisory ranges: numeric
/// parts numerically, pre-release qualifiers (`-rc1`, `a1`, `.dev0`, `-SNAPSHOT`) below the release,
/// patch qualifiers (`.post1`, `-SP1`) above it.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (ta, tb) = (tokens(a), tokens(b));
    let n = ta.len().max(tb.len());
    for i in 0..n {
        let ord = match (ta.get(i), tb.get(i)) {
            (Some(Tok::Num(x)), Some(Tok::Num(y))) => x.cmp(y),
            (Some(Tok::Word(x)), Some(Tok::Word(y))) => {
                let (qx, qy) = (qualifier(x), qualifier(y));
                if qx != qy {
                    qx.cmp(&qy)
                } else {
                    x.cmp(y)
                }
            }
            (Some(Tok::Num(_)), Some(Tok::Word(w))) => 0.cmp(&qualifier(w)).then(Ordering::Greater),
            (Some(Tok::Word(w)), Some(Tok::Num(_))) => qualifier(w).cmp(&0).then(Ordering::Less),
            // A missing part is 0 for numbers ("1.0" == "1.0.0"); a trailing qualifier is compared to a release.
            (Some(Tok::Num(x)), None) => x.cmp(&0),
            (None, Some(Tok::Num(y))) => 0.cmp(y),
            (Some(Tok::Word(w)), None) => qualifier(w).cmp(&0),
            (None, Some(Tok::Word(w))) => 0.cmp(&qualifier(w)),
            (None, None) => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// One affected interval of a package: `[introduced, fixed)` or `[introduced, last_affected]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AffectedRange {
    pub introduced: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fixed: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_affected: String,
}

/// Whether `version` is affected: in one of the ranges, or listed.
pub fn is_affected(version: &str, ranges: &[AffectedRange], versions: &[String]) -> bool {
    is_affected_in("", version, ranges, versions)
}

/// [`is_affected`] with the versions ordered as `ecosystem` orders them.
pub fn is_affected_in(ecosystem: &str, version: &str, ranges: &[AffectedRange], versions: &[String]) -> bool {
    let cmp = |a: &str, b: &str| compare_versions_in(ecosystem, a, b);
    if versions.iter().any(|v| cmp(v, version) == Ordering::Equal) {
        return true;
    }
    ranges.iter().any(|r| {
        let after = r.introduced.is_empty() || r.introduced == "0" || cmp(version, &r.introduced) != Ordering::Less;
        let before = if !r.fixed.is_empty() {
            cmp(version, &r.fixed) == Ordering::Less
        } else if !r.last_affected.is_empty() {
            cmp(version, &r.last_affected) != Ordering::Greater
        } else {
            true
        };
        after && before
    })
}

/// The fixed version the remediation points to: the lowest fix above `version` (or above any fix).
pub fn fix_for(version: &str, ranges: &[AffectedRange]) -> Option<String> {
    fix_for_in("", version, ranges)
}

/// [`fix_for`] with the versions ordered as `ecosystem` orders them.
pub fn fix_for_in(ecosystem: &str, version: &str, ranges: &[AffectedRange]) -> Option<String> {
    let cmp = |a: &str, b: &str| compare_versions_in(ecosystem, a, b);
    let mut fixes: Vec<&str> = ranges.iter().map(|r| r.fixed.as_str()).filter(|f| !f.is_empty()).collect();
    fixes.sort_by(|a, b| cmp(a, b));
    fixes.iter().find(|f| cmp(f, version) == Ordering::Greater).or(fixes.last()).map(|s| s.to_string())
}

/// Compares versions as the ecosystem's package manager does: dpkg (Debian, Ubuntu), rpm (AlmaLinux,
/// Rocky Linux, Red Hat, SUSE, …), apk (Alpine, Wolfi, Chainguard); others with [`compare_versions`].
pub fn compare_versions_in(ecosystem: &str, a: &str, b: &str) -> Ordering {
    match ecosystem.split(':').next().unwrap_or("") {
        "Debian" | "Ubuntu" => dpkg_compare(a, b),
        "Alpine" | "Wolfi" | "Chainguard" | "MinimOS" => apk_compare(a, b),
        "AlmaLinux" | "Rocky Linux" | "Red Hat" | "SUSE" | "openSUSE" | "Mageia" | "Photon OS" | "openEuler" | "Azure Linux" | "Amazon Linux" | "CentOS" | "Oracle Linux"
        | "Fedora" => rpm_compare(a, b),
        _ => compare_versions(a, b),
    }
}

/// `epoch:rest` → (epoch, rest); no or a non-numeric epoch is 0.
fn split_epoch(v: &str) -> (u64, &str) {
    match v.split_once(':') {
        Some((e, rest)) if !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit()) => (e.parse().unwrap_or(0), rest),
        _ => (0, v),
    }
}

/// dpkg's version order (deb-version(7)): epoch, then upstream and revision with `verrevcmp`.
fn dpkg_compare(a: &str, b: &str) -> Ordering {
    let ((ea, a), (eb, b)) = (split_epoch(a.trim()), split_epoch(b.trim()));
    let (ua, ra) = a.rsplit_once('-').unwrap_or((a, ""));
    let (ub, rb) = b.rsplit_once('-').unwrap_or((b, ""));
    ea.cmp(&eb).then_with(|| verrevcmp(ua, ub)).then_with(|| verrevcmp(ra, rb))
}

fn verrevcmp(a: &str, b: &str) -> Ordering {
    // `~` before everything (even the end), letters before other characters; digits and the end are 0.
    let order = |c: Option<u8>| -> i32 {
        match c {
            None => 0,
            Some(b'~') => -1,
            Some(c) if c.is_ascii_digit() => 0,
            Some(c) if c.is_ascii_alphabetic() => c as i32,
            Some(c) => c as i32 + 256,
        }
    };
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let (ac, bc) = (order(a.get(i).copied()), order(b.get(j).copied()));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while a.get(i) == Some(&b'0') {
            i += 1;
        }
        while b.get(j) == Some(&b'0') {
            j += 1;
        }
        let mut first = Ordering::Equal;
        while i < a.len() && a[i].is_ascii_digit() && j < b.len() && b[j].is_ascii_digit() {
            if first == Ordering::Equal {
                first = a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
        if i < a.len() && a[i].is_ascii_digit() {
            return Ordering::Greater;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            return Ordering::Less;
        }
        if first != Ordering::Equal {
            return first;
        }
    }
    Ordering::Equal
}

/// rpm's version order: epoch, then version and release with `rpmvercmp`.
fn rpm_compare(a: &str, b: &str) -> Ordering {
    let ((ea, a), (eb, b)) = (split_epoch(a.trim()), split_epoch(b.trim()));
    let (va, ra) = a.rsplit_once('-').unwrap_or((a, ""));
    let (vb, rb) = b.rsplit_once('-').unwrap_or((b, ""));
    ea.cmp(&eb).then_with(|| rpmvercmp(va, vb)).then_with(|| if ra.is_empty() || rb.is_empty() { Ordering::Equal } else { rpmvercmp(ra, rb) })
}

fn rpmvercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let sep = |c: u8| !c.is_ascii_alphanumeric() && c != b'~' && c != b'^';
    loop {
        while i < a.len() && sep(a[i]) {
            i += 1;
        }
        while j < b.len() && sep(b[j]) {
            j += 1;
        }
        // `~` sorts before everything, `^` after the end but before anything else.
        let (ca, cb) = (a.get(i).copied(), b.get(j).copied());
        if ca == Some(b'~') || cb == Some(b'~') {
            if ca != Some(b'~') {
                return Ordering::Greater;
            }
            if cb != Some(b'~') {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if ca == Some(b'^') || cb == Some(b'^') {
            if ca.is_none() {
                return Ordering::Less;
            }
            if cb.is_none() {
                return Ordering::Greater;
            }
            if ca != Some(b'^') {
                return Ordering::Greater;
            }
            if cb != Some(b'^') {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if ca.is_none() || cb.is_none() {
            break;
        }
        let numeric = a[i].is_ascii_digit();
        let take = |s: &[u8], mut k: usize| {
            let start = k;
            while k < s.len() && if numeric { s[k].is_ascii_digit() } else { s[k].is_ascii_alphabetic() } {
                k += 1;
            }
            (start, k)
        };
        let ((sa, ea), (sb, eb)) = (take(a, i), take(b, j));
        if sb == eb {
            // Numbers are newer than letters.
            return if numeric { Ordering::Greater } else { Ordering::Less };
        }
        let (mut x, mut y) = (&a[sa..ea], &b[sb..eb]);
        let ord = if numeric {
            while x.first() == Some(&b'0') {
                x = &x[1..];
            }
            while y.first() == Some(&b'0') {
                y = &y[1..];
            }
            x.len().cmp(&y.len()).then_with(|| x.cmp(y))
        } else {
            x.cmp(y)
        };
        if ord != Ordering::Equal {
            return ord;
        }
        (i, j) = (ea, eb);
    }
    // Whichever has characters left is newer.
    (i < a.len()).cmp(&(j < b.len()))
}

/// apk's version order: numbers, an optional letter (newer), `_suffix` (`_rc` older, `_p` newer), `-rN`.
fn apk_compare(a: &str, b: &str) -> Ordering {
    enum Part {
        Num(u64),
        Letter(u8),
        Suffix(i32, u64),
        Rev(u64),
    }
    fn parts(v: &str) -> Vec<Part> {
        let v = v.trim();
        let (v, rev) = match v.rsplit_once("-r") {
            Some((v, r)) if r.bytes().all(|c| c.is_ascii_digit()) && !r.is_empty() => (v, r.parse().unwrap_or(0)),
            _ => (v, 0),
        };
        let (main, suffixes) = v.split_once('_').unwrap_or((v, ""));
        let mut out = Vec::new();
        for n in main.split('.') {
            let digits: String = n.chars().take_while(char::is_ascii_digit).collect();
            out.push(Part::Num(digits.parse().unwrap_or(0)));
            if let Some(l) = n[digits.len()..].bytes().next() {
                out.push(Part::Letter(l));
            }
        }
        for s in suffixes.split('_').filter(|s| !s.is_empty()) {
            let name: String = s.chars().take_while(char::is_ascii_alphabetic).collect();
            let rank = match name.as_str() {
                "alpha" => -4,
                "beta" => -3,
                "pre" => -2,
                "rc" => -1,
                "cvs" => 1,
                "svn" => 2,
                "git" => 3,
                "hg" => 4,
                "p" => 5,
                _ => 0,
            };
            out.push(Part::Suffix(rank, s[name.len()..].parse().unwrap_or(0)));
        }
        out.push(Part::Rev(rev));
        out
    }
    let (pa, pb) = (parts(a), parts(b));
    for k in 0..pa.len().max(pb.len()) {
        let ord = match (pa.get(k), pb.get(k)) {
            (Some(Part::Num(x)), Some(Part::Num(y))) => x.cmp(y),
            (Some(Part::Letter(x)), Some(Part::Letter(y))) => x.cmp(y),
            (Some(Part::Suffix(x, n)), Some(Part::Suffix(y, m))) => x.cmp(y).then(n.cmp(m)),
            (Some(Part::Rev(x)), Some(Part::Rev(y))) => x.cmp(y),
            // More numbers, or a letter, is newer; a suffix is newer or older by its kind.
            (Some(Part::Num(_) | Part::Letter(_)), Some(_)) => Ordering::Greater,
            (Some(_), Some(Part::Num(_) | Part::Letter(_))) => Ordering::Less,
            (Some(Part::Suffix(x, _)), Some(Part::Rev(_))) => x.cmp(&0),
            (Some(Part::Rev(_)), Some(Part::Suffix(y, _))) => 0.cmp(y),
            _ => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

// ================================================================ severity

pub fn severity_of_score(score: f64) -> &'static str {
    match score {
        s if s >= 9.0 => "critical",
        s if s >= 7.0 => "high",
        s if s >= 4.0 => "medium",
        s if s > 0.0 => "low",
        _ => "unknown",
    }
}

pub fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// CVSS 3.0/3.1 base score of a vector (`CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H` → 9.8), from
/// the formula in the CVSS v3.1 specification.
pub fn cvss3_score(vector: &str) -> Option<f64> {
    if !vector.starts_with("CVSS:3") {
        return None;
    }
    let m: std::collections::HashMap<&str, &str> = vector.split('/').skip(1).filter_map(|p| p.split_once(':')).collect();
    let changed = *m.get("S")? == "C";
    let av = match *m.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.2,
        _ => return None,
    };
    let ac = match *m.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let pr = match (*m.get("PR")?, changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.5,
        _ => return None,
    };
    let ui = match *m.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let cia = |k: &str| -> Option<f64> {
        Some(match *m.get(k)? {
            "H" => 0.56,
            "L" => 0.22,
            "N" => 0.0,
            _ => return None,
        })
    };
    let iss = 1.0 - (1.0 - cia("C")?) * (1.0 - cia("I")?) * (1.0 - cia("A")?);
    let impact = if changed { 7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15) } else { 6.42 * iss };
    let exploitability = 8.22 * av * ac * pr * ui;
    if impact <= 0.0 {
        return Some(0.0);
    }
    let raw = if changed { (1.08 * (impact + exploitability)).min(10.0) } else { (impact + exploitability).min(10.0) };
    Some(roundup(raw))
}

fn roundup(x: f64) -> f64 {
    let i = (x * 100_000.0).round() as i64;
    if i % 10_000 == 0 {
        i as f64 / 100_000.0
    } else {
        ((i / 10_000) + 1) as f64 / 10.0
    }
}

// ================================================================ OSV advisories

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AdvisoryRef {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AdvisoryPackage {
    pub ecosystem: String,
    pub package: String,
    pub ranges: Vec<AffectedRange>,
    pub versions: Vec<String>,
    pub fixed: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Advisory {
    pub id: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub details: String,
    pub severity: String,
    pub score: Option<f64>,
    pub vector: String,
    pub cwes: Vec<String>,
    pub refs: Vec<AdvisoryRef>,
    pub published: Option<i64>,
    pub modified: Option<i64>,
    pub withdrawn: Option<i64>,
    pub packages: Vec<AdvisoryPackage>,
}

fn ts(v: &Value) -> Option<i64> {
    v.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp_millis())
}

/// OSV ecosystems whose advisories are per distribution release (`Debian:12`, `Ubuntu:22.04:LTS`, `Alpine:v3.19`).
pub const DISTRO_ECOSYSTEMS: &[&str] =
    &["Debian", "Ubuntu", "Alpine", "AlmaLinux", "Rocky Linux", "Red Hat", "SUSE", "openSUSE", "Mageia", "Photon OS", "openEuler", "Azure Linux", "Wolfi", "Chainguard", "Echo", "MinimOS"];

/// An affected package's ecosystem as packages are keyed: OS distributions keep their release (an SBOM's
/// Debian 12 packages are `Debian:12`), language ecosystems drop a suffix (`Maven:<repository>` → `Maven`).
pub fn osv_ecosystem(eco: &str) -> String {
    let base = eco.split(':').next().unwrap_or("");
    if DISTRO_ECOSYSTEMS.contains(&base) {
        eco.to_string()
    } else {
        base.to_string()
    }
}

/// Whether OSV.dev knows the ecosystem (its querybatch rejects unknown ones): language ecosystems and
/// the distributions OSV covers. Amazon Linux, CentOS, Oracle Linux and Fedora packages are only listed.
pub fn osv_queryable(eco: &str) -> bool {
    let base = eco.split(':').next().unwrap_or("");
    DISTRO_ECOSYSTEMS.contains(&base) || !language_of_ecosystem(base).is_empty()
}

/// An OSV `querybatch` response: each advisory's `modified` and the queries (packages, by index) it affects.
pub fn osv_batch_advisories(res: &Value) -> std::collections::BTreeMap<String, (String, Vec<usize>)> {
    let mut ids: std::collections::BTreeMap<String, (String, Vec<usize>)> = Default::default();
    for (k, r) in res.get("results").and_then(Value::as_array).into_iter().flatten().enumerate() {
        for v in r.get("vulns").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = v.get("id").and_then(Value::as_str) {
                let e = ids.entry(id.to_string()).or_insert_with(|| (v.get("modified").and_then(Value::as_str).unwrap_or("").to_string(), vec![]));
                e.1.push(k);
            }
        }
    }
    ids
}

/// An OSV record (`https://ossf.github.io/osv-schema/`) as an advisory.
pub fn parse_osv(v: &Value) -> Option<Advisory> {
    let id = v.get("id")?.as_str()?.to_string();
    let strs = |k: &str| -> Vec<String> { v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let mut vector = String::new();
    let mut score = None;
    for s in v.get("severity").and_then(Value::as_array).into_iter().flatten() {
        let (kind, val) = (s.get("type").and_then(Value::as_str).unwrap_or(""), s.get("score").and_then(Value::as_str).unwrap_or(""));
        if kind == "CVSS_V3" {
            vector = val.to_string();
            score = cvss3_score(val);
        } else if vector.is_empty() {
            vector = val.to_string();
        }
    }
    let db = v.get("database_specific");
    let db_severity = db.and_then(|d| d.get("severity")).and_then(Value::as_str).map(|s| match s.to_ascii_uppercase().as_str() {
        "CRITICAL" => "critical",
        "HIGH" => "high",
        "MODERATE" | "MEDIUM" => "medium",
        "LOW" => "low",
        _ => "unknown",
    });
    let severity = score.map(severity_of_score).or(db_severity).unwrap_or("unknown").to_string();
    let cwes = db.and_then(|d| d.get("cwe_ids")).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let refs = v
        .get("references")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|r| AdvisoryRef { kind: r.get("type").and_then(Value::as_str).unwrap_or("WEB").to_string(), url: r.get("url").and_then(Value::as_str).unwrap_or("").to_string() }).collect())
        .unwrap_or_default();
    let mut packages: Vec<AdvisoryPackage> = Vec::new();
    for a in v.get("affected").and_then(Value::as_array).into_iter().flatten() {
        let Some(pkg) = a.get("package") else { continue };
        let ecosystem = osv_ecosystem(pkg.get("ecosystem").and_then(Value::as_str).unwrap_or(""));
        let name = pkg.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        if ecosystem.is_empty() || name.is_empty() {
            continue;
        }
        let mut ranges = Vec::new();
        for r in a.get("ranges").and_then(Value::as_array).into_iter().flatten() {
            if r.get("type").and_then(Value::as_str) == Some("GIT") {
                continue;
            }
            // Events are ordered: each `introduced` opens an interval the next `fixed` / `last_affected` closes.
            let mut open: Option<AffectedRange> = None;
            for e in r.get("events").and_then(Value::as_array).into_iter().flatten() {
                if let Some(i) = e.get("introduced").and_then(Value::as_str) {
                    if let Some(o) = open.take() {
                        ranges.push(o);
                    }
                    open = Some(AffectedRange { introduced: i.to_string(), ..Default::default() });
                } else if let Some(f) = e.get("fixed").and_then(Value::as_str) {
                    let mut o = open.take().unwrap_or_default();
                    o.fixed = f.to_string();
                    ranges.push(o);
                } else if let Some(l) = e.get("last_affected").and_then(Value::as_str) {
                    let mut o = open.take().unwrap_or_default();
                    o.last_affected = l.to_string();
                    ranges.push(o);
                }
            }
            if let Some(o) = open {
                ranges.push(o);
            }
        }
        let versions: Vec<String> = a.get("versions").and_then(Value::as_array).map(|x| x.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let mut fixes: Vec<&str> = ranges.iter().map(|r| r.fixed.as_str()).filter(|f| !f.is_empty()).collect();
        fixes.sort_by(|a, b| compare_versions_in(&ecosystem, a, b));
        let fixed = fixes.first().map(|s| s.to_string()).unwrap_or_default();
        let name = normalize_name(&ecosystem, &name);
        match packages.iter_mut().find(|p| p.ecosystem == ecosystem && p.package == name) {
            Some(p) => {
                p.ranges.extend(ranges);
                p.versions.extend(versions);
            }
            None => packages.push(AdvisoryPackage { ecosystem, package: name, ranges, versions, fixed }),
        }
    }
    Some(Advisory {
        id,
        aliases: strs("aliases"),
        summary: v.get("summary").and_then(Value::as_str).unwrap_or("").to_string(),
        details: v.get("details").and_then(Value::as_str).unwrap_or("").to_string(),
        severity,
        score,
        vector,
        cwes,
        refs,
        published: v.get("published").and_then(ts),
        modified: v.get("modified").and_then(ts),
        withdrawn: v.get("withdrawn").and_then(ts),
        packages,
    })
}

// ================================================================ libraries

/// A library a service loads or a repository declares.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
    pub direct: Option<bool>,
    pub dev: bool,
    pub file: String,
    pub licenses: Vec<String>,
}

/// Where a tracer runs, from its telemetry envelope.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelemetryApp {
    pub service: String,
    pub env: String,
    pub version: String,
    pub language: String,
    pub host: String,
}

/// The libraries in a tracer's instrumentation telemetry (`app-dependencies-loaded`, also inside
/// `message-batch`; v1 `app-started` listed them too). `None` when the message carries none.
pub fn parse_telemetry(v: &Value) -> Option<(TelemetryApp, Vec<Library>)> {
    let app = v.get("application")?;
    let s = |k: &str| app.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let language = s("language_name");
    let ecosystem = ecosystem_of_language(&language)?;
    let info = TelemetryApp {
        service: s("service_name"),
        env: s("env"),
        version: s("service_version"),
        language: language.to_lowercase(),
        host: v.pointer("/host/hostname").and_then(Value::as_str).unwrap_or("").to_string(),
    };
    let mut libs = Vec::new();
    let mut collect = |kind: &str, payload: &Value| {
        if kind == "app-dependencies-loaded" || kind == "app-started" {
            for d in payload.get("dependencies").and_then(Value::as_array).into_iter().flatten() {
                let (Some(name), Some(version)) = (d.get("name").and_then(Value::as_str), d.get("version").and_then(Value::as_str)) else { continue };
                if name.is_empty() || version.is_empty() {
                    continue;
                }
                libs.push(Library { ecosystem: ecosystem.into(), name: normalize_name(ecosystem, name), version: version.into(), ..Default::default() });
            }
        }
    };
    let kind = v.get("request_type").and_then(Value::as_str).unwrap_or("");
    let payload = v.get("payload").cloned().unwrap_or(Value::Null);
    if kind == "message-batch" {
        for m in payload.as_array().into_iter().flatten() {
            collect(m.get("request_type").and_then(Value::as_str).unwrap_or(""), m.get("payload").unwrap_or(&Value::Null));
        }
    } else {
        collect(kind, &payload);
    }
    (!libs.is_empty()).then_some((info, libs))
}

/// Git and CI context of a static upload (datadog-ci's span tags).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanContext {
    pub repository: String,
    pub branch: String,
    pub commit_sha: String,
    pub pr_number: String,
    pub base_branch: String,
    pub service: String,
    pub env: String,
}

impl ScanContext {
    fn from_tags(tags: &Value) -> Self {
        let t = |k: &str| tags.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        ScanContext {
            repository: normalize_repo(&t("git.repository_url")),
            branch: t("git.branch").trim_start_matches("refs/heads/").to_string(),
            commit_sha: t("git.commit.sha"),
            pr_number: t("pr.number"),
            base_branch: t("git.pull_request.base_branch").trim_start_matches("refs/heads/").to_string(),
            service: t("service"),
            env: t("env"),
        }
    }
}

/// A `datadog-ci sbom upload` request (`{"data": {"type": "scarequests", "attributes": …}}`).
pub fn parse_sbom_request(v: &Value) -> Result<(ScanContext, Vec<Library>), String> {
    let data = v.get("data").ok_or("missing data")?;
    if data.get("type").and_then(Value::as_str) != Some("scarequests") {
        return Err("data.type must be scarequests".into());
    }
    let a = data.get("attributes").ok_or("missing data.attributes")?;
    let mut ctx = ScanContext::from_tags(a.get("tags").unwrap_or(&Value::Null));
    if let Some(u) = a.pointer("/repository/url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
        ctx.repository = normalize_repo(u);
    }
    if let Some(b) = a.pointer("/commit/branch").and_then(Value::as_str).filter(|b| !b.is_empty()) {
        ctx.branch = b.trim_start_matches("refs/heads/").to_string();
    }
    if let Some(s) = a.pointer("/commit/sha").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        ctx.commit_sha = s.to_string();
    }
    if let Some(s) = a.get("service").and_then(Value::as_str) {
        ctx.service = s.to_string();
    }
    if let Some(e) = a.get("env").and_then(Value::as_str) {
        ctx.env = e.to_string();
    }
    let mut libs = Vec::new();
    for d in a.get("dependencies").and_then(Value::as_array).into_iter().flatten() {
        let parsed = d.get("purl").and_then(Value::as_str).and_then(parse_purl);
        let (ecosystem, name, purl_version) = match parsed {
            Some(p) => p,
            None => {
                let Some(eco) = d.get("language").and_then(Value::as_str).and_then(ecosystem_of_language) else { continue };
                let name = d.get("name").and_then(Value::as_str).unwrap_or("");
                let name = match d.get("group").and_then(Value::as_str).filter(|g| !g.is_empty()) {
                    Some(g) if eco == "Maven" => format!("{g}:{name}"),
                    Some(g) => format!("{g}/{name}"),
                    None => name.to_string(),
                };
                (eco, normalize_name(eco, &name), String::new())
            }
        };
        let version = d.get("version").and_then(Value::as_str).map(str::to_string).filter(|v| !v.is_empty()).unwrap_or(purl_version);
        if name.is_empty() || version.is_empty() {
            continue;
        }
        let file = d.pointer("/locations/0/block/file_name").or_else(|| d.pointer("/locations/0/file_name")).and_then(Value::as_str).unwrap_or("").to_string();
        let licenses = d.get("licenses").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
        libs.push(Library {
            ecosystem: ecosystem.into(),
            name,
            version,
            direct: d.get("is_direct").and_then(Value::as_bool),
            dev: d.get("is_dev").and_then(Value::as_bool).unwrap_or(false),
            file,
            licenses,
        });
    }
    Ok((ctx, libs))
}

// ================================================================ SARIF findings

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rule {
    pub kind: &'static str,
    pub tool: String,
    pub rule_id: String,
    pub name: String,
    pub description: String,
    pub severity: String,
    pub cwes: Vec<String>,
    pub help_uri: String,
    pub language: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Finding {
    pub id: String,
    pub kind: &'static str,
    pub tool: String,
    pub rule_id: String,
    pub severity: String,
    pub message: String,
    pub file: String,
    pub line: i32,
    pub snippet: String,
}

const SECRET_TOOLS: &[&str] = &["gitleaks", "trufflehog", "detect-secrets", "ggshield", "datadog-secrets"];
const IAC_TOOLS: &[&str] = &["checkov", "tfsec", "kics", "terrascan", "trivy-config", "datadog-iac", "iac"];

fn rule_kind(tool: &str, tags: &[String]) -> &'static str {
    if tags.iter().any(|t| t == "DATADOG_RULE_TYPE:SECRET") {
        return "secret";
    }
    if tags.iter().any(|t| t == "DATADOG_RULE_TYPE:IAC" || t == "DATADOG_RULE_TYPE:IAC_SCANNING") {
        return "iac";
    }
    if tags.iter().any(|t| t == "DATADOG_RULE_TYPE:STATIC_ANALYSIS") {
        return "sast";
    }
    let t = tool.to_ascii_lowercase();
    if SECRET_TOOLS.iter().any(|s| t.contains(s)) {
        "secret"
    } else if IAC_TOOLS.iter().any(|s| t.contains(s)) {
        "iac"
    } else {
        "sast"
    }
}

fn level_severity(level: &str) -> &'static str {
    match level {
        "error" => "high",
        "warning" => "medium",
        "note" => "low",
        _ => "unknown",
    }
}

/// Severity from a rule's properties: Datadog's `DATADOG_SEVERITY:…` tag, the `security-severity`
/// score code scanners publish, or the SARIF level.
fn rule_severity(props: &Value, tags: &[String], level: &str) -> String {
    if let Some(s) = tags.iter().find_map(|t| t.strip_prefix("DATADOG_SEVERITY:")) {
        let s = s.to_ascii_lowercase();
        return match s.as_str() {
            "error" => "high".into(),
            "warning" => "medium".into(),
            "notice" | "note" => "low".into(),
            _ => s,
        };
    }
    if let Some(score) = props.get("security-severity").and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64())) {
        return severity_of_score(score).into();
    }
    level_severity(level).into()
}

/// The rules and results of a SARIF 2.1.0 report.
pub fn parse_sarif(v: &Value, repository: &str) -> (Vec<Rule>, Vec<Finding>) {
    let mut rules = Vec::new();
    let mut findings = Vec::new();
    for run in v.get("runs").and_then(Value::as_array).into_iter().flatten() {
        let tool = run.pointer("/tool/driver/name").and_then(Value::as_str).unwrap_or("third-party-analyzer").to_string();
        let mut by_id: std::collections::HashMap<String, Rule> = std::collections::HashMap::new();
        for r in run.pointer("/tool/driver/rules").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = r.get("id").and_then(Value::as_str) else { continue };
            let props = r.get("properties").cloned().unwrap_or(Value::Null);
            let tags: Vec<String> = props.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let level = r.pointer("/defaultConfiguration/level").and_then(Value::as_str).unwrap_or("warning");
            let text = |p: &str| r.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
            let rule = Rule {
                kind: rule_kind(&tool, &tags),
                tool: tool.clone(),
                rule_id: id.to_string(),
                name: [text("/shortDescription/text"), text("/name")].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| id.to_string()),
                description: [text("/fullDescription/text"), text("/help/text")].into_iter().find(|s| !s.is_empty()).unwrap_or_default(),
                severity: rule_severity(&props, &tags, level),
                cwes: tags.iter().filter_map(|t| t.strip_prefix("CWE:").or_else(|| t.strip_prefix("external/cwe/cwe-"))).map(|c| format!("CWE-{}", c.trim_start_matches("CWE-"))).collect(),
                help_uri: text("/helpUri"),
                language: tags.iter().find_map(|t| t.strip_prefix("DATADOG_LANGUAGE:")).unwrap_or("").to_lowercase(),
            };
            by_id.insert(id.to_string(), rule.clone());
            rules.push(rule);
        }
        for res in run.get("results").and_then(Value::as_array).into_iter().flatten() {
            let rule_id = res.get("ruleId").and_then(Value::as_str).unwrap_or("").to_string();
            if rule_id.is_empty() {
                continue;
            }
            let rule = by_id.get(&rule_id).cloned().unwrap_or_else(|| Rule { kind: rule_kind(&tool, &[]), tool: tool.clone(), rule_id: rule_id.clone(), name: rule_id.clone(), severity: "unknown".into(), ..Default::default() });
            let loc = res.pointer("/locations/0/physicalLocation");
            let file = loc.and_then(|l| l.pointer("/artifactLocation/uri")).and_then(Value::as_str).unwrap_or("").to_string();
            let line = loc.and_then(|l| l.pointer("/region/startLine")).and_then(Value::as_i64).unwrap_or(0) as i32;
            let snippet = loc.and_then(|l| l.pointer("/region/snippet/text")).and_then(Value::as_str).unwrap_or("").to_string();
            let severity = match res.get("level").and_then(Value::as_str) {
                Some(l) if rule.severity == "unknown" => level_severity(l).to_string(),
                _ => rule.severity.clone(),
            };
            let fingerprint = res
                .get("partialFingerprints")
                .and_then(Value::as_object)
                .and_then(|o| o.values().find_map(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| format!("{line}:{snippet}"));
            let id = hex(&Sha256::digest(format!("{}|{tool}|{rule_id}|{repository}|{file}|{fingerprint}", rule.kind).as_bytes()))[..32].to_string();
            findings.push(Finding {
                id,
                kind: rule.kind,
                tool: tool.clone(),
                rule_id,
                severity,
                message: res.pointer("/message/text").and_then(Value::as_str).unwrap_or("").to_string(),
                file,
                line,
                snippet,
            });
        }
    }
    (rules, findings)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The context of a SARIF upload's `event` part (datadog-ci's span tags, plus `service`).
pub fn sarif_context(event: &Value) -> ScanContext {
    ScanContext::from_tags(event)
}

// ================================================================ IAST

/// A runtime code vulnerability a tracer reported on a span (`_dd.iast.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IastVulnerability {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub severity: String,
    pub cwe: String,
    pub hash: String,
    pub path: String,
    pub method: String,
    pub line: i64,
    pub evidence: String,
}

/// Title, severity and CWE of an IAST vulnerability type.
pub fn iast_type(kind: &str) -> (&'static str, &'static str, &'static str) {
    match kind {
        "SQL_INJECTION" => ("SQL Injection", "critical", "CWE-89"),
        "COMMAND_INJECTION" => ("Command Injection", "critical", "CWE-77"),
        "CODE_INJECTION" => ("Code Injection", "critical", "CWE-94"),
        "LDAP_INJECTION" => ("LDAP Injection", "high", "CWE-90"),
        "NOSQL_MONGODB_INJECTION" => ("NoSQL Injection", "critical", "CWE-943"),
        "PATH_TRAVERSAL" => ("Path Traversal", "high", "CWE-22"),
        "SSRF" => ("Server-Side Request Forgery", "high", "CWE-918"),
        "XSS" | "REFLECTION_INJECTION" => ("Cross-Site Scripting", "high", "CWE-79"),
        "UNVALIDATED_REDIRECT" => ("Unvalidated Redirect", "medium", "CWE-601"),
        "XPATH_INJECTION" => ("XPath Injection", "high", "CWE-643"),
        "HEADER_INJECTION" => ("Header Injection", "medium", "CWE-113"),
        "UNTRUSTED_DESERIALIZATION" => ("Untrusted Deserialization", "critical", "CWE-502"),
        "HARDCODED_SECRET" | "HARDCODED_PASSWORD" => ("Hardcoded Secret", "high", "CWE-798"),
        "WEAK_HASH" => ("Weak Hashing Algorithm", "low", "CWE-328"),
        "WEAK_CIPHER" => ("Weak Cipher", "low", "CWE-327"),
        "WEAK_RANDOMNESS" => ("Weak Randomness", "low", "CWE-330"),
        "INSECURE_COOKIE" => ("Insecure Cookie", "low", "CWE-614"),
        "NO_HTTPONLY_COOKIE" => ("No HttpOnly Cookie", "low", "CWE-1004"),
        "NO_SAMESITE_COOKIE" => ("No SameSite Cookie", "low", "CWE-1275"),
        "HSTS_HEADER_MISSING" => ("HSTS Header Missing", "low", "CWE-319"),
        "XCONTENTTYPE_HEADER_MISSING" => ("X-Content-Type-Options Header Missing", "low", "CWE-693"),
        "TRUST_BOUNDARY_VIOLATION" => ("Trust Boundary Violation", "medium", "CWE-501"),
        "STACKTRACE_LEAK" => ("Stack Trace Leak", "low", "CWE-209"),
        _ => ("Code Vulnerability", "medium", ""),
    }
}

/// The vulnerabilities of a span's `_dd.iast.json`.
pub fn parse_iast(json: &str) -> Vec<IastVulnerability> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    v.get("vulnerabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| {
            let kind = x.get("type").and_then(Value::as_str).unwrap_or("").to_string();
            let (title, severity, cwe) = iast_type(&kind);
            let evidence = x
                .pointer("/evidence/value")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    x.pointer("/evidence/valueParts")
                        .and_then(Value::as_array)
                        .map(|p| p.iter().filter_map(|part| part.get("value").and_then(Value::as_str)).collect::<String>())
                })
                .unwrap_or_default();
            IastVulnerability {
                title: title.to_string(),
                severity: severity.to_string(),
                cwe: cwe.to_string(),
                hash: x.get("hash").map(|h| h.to_string().trim_matches('"').to_string()).unwrap_or_default(),
                path: x.pointer("/location/path").and_then(Value::as_str).unwrap_or("").to_string(),
                method: x.pointer("/location/method").and_then(Value::as_str).unwrap_or("").to_string(),
                line: x.pointer("/location/line").and_then(Value::as_i64).unwrap_or(0),
                evidence,
                kind,
            }
        })
        .filter(|x| !x.kind.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_order_like_package_managers() {
        use Ordering::*;
        assert_eq!(compare_versions("11.0.9", "11.0.21"), Less);
        assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
        assert_eq!(compare_versions("v1.2.3", "1.2.3"), Equal);
        assert_eq!(compare_versions("2.0.0-rc1", "2.0.0"), Less);
        assert_eq!(compare_versions("2.0.0-SNAPSHOT", "2.0.0-rc1"), Less);
        assert_eq!(compare_versions("1.0.0a1", "1.0.0b2"), Less);
        assert_eq!(compare_versions("1.0.0.dev0", "1.0.0a1"), Less);
        assert_eq!(compare_versions("1.0.0.post1", "1.0.0"), Greater);
        assert_eq!(compare_versions("2.19.2", "2.19.10"), Less);
        assert_eq!(compare_versions("4.17.21", "4.17.21+build.5"), Equal);
    }

    #[test]
    fn distro_versions_order_as_their_package_managers() {
        use Ordering::*;
        // dpkg: epoch, upstream, revision; `~` sorts before anything, letters before other characters.
        let deb = |a, b| compare_versions_in("Debian:12", a, b);
        assert_eq!(deb("3.0.11-1~deb12u2", "3.0.15-1~deb12u1"), Less);
        assert_eq!(deb("1.0~rc1-1", "1.0-1"), Less);
        assert_eq!(deb("1:1.0-1", "2.0-1"), Greater);
        assert_eq!(deb("2.36-9+deb12u4", "2.36-9+deb12u10"), Less);
        assert_eq!(deb("1.2.13.dfsg-1", "1.2.13.dfsg-1+deb12u1"), Less);
        assert_eq!(deb("1.0a-1", "1.0+a-1"), Less);
        assert_eq!(compare_versions_in("Ubuntu:22.04:LTS", "3.0.2-0ubuntu1.15", "3.0.2-0ubuntu1.9"), Greater);
        // rpm: epoch:version-release, alphanumeric segments, numbers newer than letters, `~` older.
        let rpm = |a, b| compare_versions_in("Rocky Linux:9", a, b);
        assert_eq!(rpm("1:3.0.7-24.el9", "1:3.0.7-27.el9"), Less);
        assert_eq!(rpm("3.0.7-27.el9", "1:3.0.1-1.el9"), Less);
        assert_eq!(rpm("1.0~rc1-1", "1.0-1"), Less);
        assert_eq!(rpm("1.0a-1", "1.0.1-1"), Less);
        assert_eq!(rpm("2.28-225.el9_4", "2.28-225.el9"), Greater);
        // apk: a letter after the numbers is newer, `_rc` older, `_p` newer, then `-rN`.
        let apk = |a, b| compare_versions_in("Alpine:v3.19", a, b);
        assert_eq!(apk("3.1.1w-r1", "3.1.1-r9"), Greater);
        assert_eq!(apk("1.36.1-r15", "1.36.1-r16"), Less);
        assert_eq!(apk("1.2.4_rc1-r0", "1.2.4-r0"), Less);
        assert_eq!(apk("1.2.4_p1-r0", "1.2.4-r5"), Greater);
        assert_eq!(apk("1.2.10-r0", "1.2.9-r0"), Greater);
        assert_eq!(compare_versions_in("Wolfi", "2.38-r10", "2.38-r9"), Greater);
        // Ranges and fixes follow the ecosystem's order.
        let r = vec![AffectedRange { introduced: "0".into(), fixed: "1:3.0.15-1~deb12u1".into(), ..Default::default() }];
        assert!(is_affected_in("Debian:12", "1:3.0.11-1~deb12u2", &r, &[]));
        assert!(!is_affected_in("Debian:12", "1:3.0.15-1~deb12u1", &r, &[]));
        assert!(!is_affected_in("Debian:12", "2:1.0-1", &r, &[]));
        assert_eq!(fix_for_in("Debian:12", "1:3.0.11-1~deb12u2", &r).as_deref(), Some("1:3.0.15-1~deb12u1"));
        let a = vec![AffectedRange { introduced: "0".into(), fixed: "1.2.4-r3".into(), ..Default::default() }];
        assert!(is_affected_in("Alpine:v3.19", "1.2.4-r2", &a, &[]));
        assert!(!is_affected_in("Alpine:v3.19", "1.2.4_p1-r2", &a, &[]));
    }

    #[test]
    fn ranges_decide_affected_versions_and_fixes() {
        let ranges = vec![
            AffectedRange { introduced: "10.1.0-M1".into(), fixed: "10.1.55".into(), ..Default::default() },
            AffectedRange { introduced: "11.0.0-M1".into(), fixed: "11.0.22".into(), ..Default::default() },
        ];
        assert!(is_affected("11.0.9", &ranges, &[]));
        assert!(!is_affected("11.0.22", &ranges, &[]));
        assert!(is_affected("10.1.40", &ranges, &[]));
        assert!(!is_affected("9.0.100", &ranges, &[]));
        assert_eq!(fix_for("11.0.9", &ranges).as_deref(), Some("11.0.22"));
        let last = vec![AffectedRange { introduced: "0".into(), last_affected: "2.3.1".into(), ..Default::default() }];
        assert!(is_affected("2.3.1", &last, &[]));
        assert!(!is_affected("2.3.2", &last, &[]));
        assert!(is_affected("5.0.0", &[], &["5.0.0".into()]));
    }

    #[test]
    fn cvss3_base_scores_follow_the_specification() {
        assert_eq!(cvss3_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"), Some(9.8));
        assert_eq!(cvss3_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"), Some(10.0));
        assert_eq!(cvss3_score("CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:N/A:N"), Some(5.9));
        assert_eq!(cvss3_score("CVSS:3.0/AV:L/AC:L/PR:L/UI:N/S:U/C:N/I:N/A:N"), Some(0.0));
        assert_eq!(cvss3_score("CVSS:4.0/AV:N"), None);
        assert_eq!(severity_of_score(7.5), "high");
    }

    #[test]
    fn osv_records_become_advisories() {
        let v = json!({
            "id": "GHSA-aaaa-bbbb-cccc", "aliases": ["CVE-2026-0001"], "summary": "Example flaw",
            "published": "2026-05-19T01:35:00Z",
            "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"}],
            "database_specific": {"severity": "MODERATE", "cwe_ids": ["CWE-287"]},
            "references": [{"type": "ADVISORY", "url": "https://example.com/a"}],
            "affected": [
                {"package": {"ecosystem": "Maven", "name": "org.example:core"},
                 "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"fixed": "1.2.0"}, {"introduced": "2.0.0"}, {"fixed": "2.0.3"}]},
                            {"type": "GIT", "events": [{"introduced": "abc"}]}]},
                {"package": {"ecosystem": "PyPI", "name": "Example_Lib"}, "versions": ["0.9"]}
            ]
        });
        let a = parse_osv(&v).unwrap();
        assert_eq!((a.severity.as_str(), a.score), ("critical", Some(9.8)));
        assert_eq!(a.cwes, vec!["CWE-287"]);
        assert_eq!(a.packages[0].ranges.len(), 2);
        assert_eq!(a.packages[0].fixed, "1.2.0");
        assert_eq!(a.packages[1].package, "example-lib");
        assert!(a.published.is_some());
    }

    #[test]
    fn osv_distro_ecosystems_keep_their_release() {
        // OS advisories are per release (DSA/DLA, USN, Alpine secdb): SBOM packages are keyed `Debian:12`, so must they.
        let v = json!({
            "id": "DEBIAN-CVE-2026-0002",
            "affected": [
                {"package": {"ecosystem": "Debian:12", "name": "openssl"}, "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"fixed": "3.0.15-1~deb12u1"}]}]},
                {"package": {"ecosystem": "Debian:11", "name": "openssl"}, "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"fixed": "1.1.1w-0+deb11u2"}]}]},
                {"package": {"ecosystem": "Ubuntu:22.04:LTS", "name": "openssl"}, "versions": ["3.0.2-0ubuntu1.15"]},
                {"package": {"ecosystem": "Alpine:v3.19", "name": "openssl"}, "versions": ["3.1.4-r0"]},
                {"package": {"ecosystem": "Rocky Linux:9", "name": "openssl"}, "versions": ["1:3.0.7-24.el9"]},
                {"package": {"ecosystem": "Maven:https://repo.example.com/", "name": "org.example:core"}, "versions": ["1.0"]}
            ]
        });
        let a = parse_osv(&v).unwrap();
        let ecos: Vec<&str> = a.packages.iter().map(|p| p.ecosystem.as_str()).collect();
        assert_eq!(ecos, ["Debian:12", "Debian:11", "Ubuntu:22.04:LTS", "Alpine:v3.19", "Rocky Linux:9", "Maven"]);
        assert_eq!(a.packages[0].fixed, "3.0.15-1~deb12u1");
        assert_eq!(osv_ecosystem("Debian:12"), "Debian:12");
        assert_eq!(osv_ecosystem("PyPI:extra"), "PyPI");
    }

    #[test]
    fn osv_batch_results_map_advisories_to_their_packages() {
        let res = json!({ "results": [
            { "vulns": [{ "id": "DSA-5801-1", "modified": "2026-10-01T00:00:00Z" }, { "id": "DEBIAN-CVE-2026-1" }] },
            {},
            { "vulns": [{ "id": "DSA-5801-1", "modified": "2026-10-01T00:00:00Z" }] }
        ]});
        let ids = osv_batch_advisories(&res);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids["DSA-5801-1"], ("2026-10-01T00:00:00Z".to_string(), vec![0, 2]));
        assert_eq!(ids["DEBIAN-CVE-2026-1"], (String::new(), vec![0]));
        assert!(osv_batch_advisories(&json!({})).is_empty());
    }

    #[test]
    fn purls_and_repositories_normalize() {
        assert_eq!(parse_purl("pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@11.0.9"), Some(("Maven", "org.apache.tomcat.embed:tomcat-embed-core".into(), "11.0.9".into())));
        assert_eq!(parse_purl("pkg:npm/%40example/session@1.2.0"), Some(("npm", "@example/session".into(), "1.2.0".into())));
        assert_eq!(normalize_repo("git@github.com:example-org/storefront.git"), "github.com/example-org/storefront");
        assert_eq!(normalize_repo("https://user@github.com/example-org/storefront/"), "github.com/example-org/storefront");
    }

    #[test]
    fn telemetry_lists_loaded_libraries() {
        let v = json!({
            "api_version": "v2", "request_type": "message-batch",
            "application": {"service_name": "storefront-api", "env": "prod", "service_version": "2.14.3", "language_name": "jvm"},
            "host": {"hostname": "i-0a1b2c3d4e5f60002"},
            "payload": [
                {"request_type": "app-heartbeat", "payload": {}},
                {"request_type": "app-dependencies-loaded", "payload": {"dependencies": [{"name": "org.example:core", "version": "1.1.0"}, {"name": "x", "version": ""}]}}
            ]
        });
        let (app, libs) = parse_telemetry(&v).unwrap();
        assert_eq!((app.service.as_str(), app.env.as_str()), ("storefront-api", "prod"));
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].ecosystem, "Maven");
        assert!(parse_telemetry(&json!({"request_type": "app-heartbeat", "application": {"language_name": "go"}})).is_none());
    }

    #[test]
    fn sbom_uploads_list_declared_libraries() {
        let v = json!({"data": {"type": "scarequests", "attributes": {
            "commit": {"sha": "abc123", "branch": "main"}, "repository": {"url": "https://github.com/example-org/storefront.git"},
            "tags": {"git.repository_url": "https://github.com/example-org/storefront.git"}, "service": "datadog-sbom-generator", "env": "ci",
            "dependencies": [
                {"name": "core", "group": "org.example", "version": "1.1.0", "language": "jvm", "purl": "pkg:maven/org.example/core@1.1.0", "is_direct": true,
                 "locations": [{"block": {"file_name": "pom.xml"}}]},
                {"name": "requests", "version": "2.31.0", "language": "python", "purl": "pkg:pypi/requests@2.31.0", "is_dev": true}
            ]}}});
        let (ctx, libs) = parse_sbom_request(&v).unwrap();
        assert_eq!((ctx.repository.as_str(), ctx.branch.as_str(), ctx.commit_sha.as_str()), ("github.com/example-org/storefront", "main", "abc123"));
        assert_eq!(libs[0].name, "org.example:core");
        assert_eq!((libs[0].direct, libs[0].file.as_str()), (Some(true), "pom.xml"));
        assert!(libs[1].dev);
        assert!(parse_sbom_request(&json!({"data": {"type": "other"}})).is_err());
    }

    #[test]
    fn sarif_results_become_findings_of_their_kind() {
        let v = json!({"runs": [{
            "tool": {"driver": {"name": "datadog-static-analyzer", "rules": [
                {"id": "python-security/sql-injection", "shortDescription": {"text": "Potential SQL injection"},
                 "properties": {"tags": ["DATADOG_RULE_TYPE:STATIC_ANALYSIS", "DATADOG_SEVERITY:CRITICAL", "CWE:89", "DATADOG_LANGUAGE:PYTHON"]}},
                {"id": "secrets/example-api-key", "properties": {"tags": ["DATADOG_RULE_TYPE:SECRET"]}, "defaultConfiguration": {"level": "error"}}
            ]}},
            "results": [
                {"ruleId": "python-security/sql-injection", "level": "error", "message": {"text": "Query built from input"},
                 "locations": [{"physicalLocation": {"artifactLocation": {"uri": "coupons/validators.py"}, "region": {"startLine": 138, "snippet": {"text": "q = f'SELECT …'"}}}}],
                 "partialFingerprints": {"DATADOG_FINGERPRINT": "fp1"}},
                {"ruleId": "secrets/example-api-key", "message": {"text": "Key"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "README.md"}, "region": {"startLine": 54}}}]}
            ]}]});
        let (rules, findings) = parse_sarif(&v, "github.com/example-org/storefront");
        assert_eq!(rules[0].kind, "sast");
        assert_eq!((rules[0].severity.as_str(), rules[0].cwes[0].as_str(), rules[0].language.as_str()), ("critical", "CWE-89", "python"));
        assert_eq!((findings[0].kind, findings[0].line, findings[0].severity.as_str()), ("sast", 138, "critical"));
        assert_eq!((findings[1].kind, findings[1].severity.as_str()), ("secret", "high"));
        // The same result rescanned keeps its id.
        assert_eq!(parse_sarif(&v, "github.com/example-org/storefront").1[0].id, findings[0].id);
        let checkov = json!({"runs": [{"tool": {"driver": {"name": "Checkov"}}, "results": [{"ruleId": "CKV_AWS_28", "level": "error", "message": {"text": "x"}}]}]});
        assert_eq!(parse_sarif(&checkov, "r").1[0].kind, "iac");
    }

    #[test]
    fn iast_spans_carry_code_vulnerabilities() {
        let j = r#"{"vulnerabilities":[{"type":"SQL_INJECTION","hash":1234,"location":{"path":"com.example.orders.OrderDao","method":"findById","line":44},
                    "evidence":{"valueParts":[{"value":"SELECT * FROM orders WHERE id = "},{"value":"1 OR 1=1","source":0}]}}]}"#;
        let v = parse_iast(j);
        assert_eq!((v[0].title.as_str(), v[0].severity.as_str(), v[0].cwe.as_str(), v[0].line), ("SQL Injection", "critical", "CWE-89", 44));
        assert_eq!(v[0].evidence, "SELECT * FROM orders WHERE id = 1 OR 1=1");
        assert_eq!(v[0].hash, "1234");
        assert!(parse_iast("not json").is_empty());
    }
}
