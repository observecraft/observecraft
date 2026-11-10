//! User-agent parsing for the log pipelines' user-agent parser, as Datadog's: ua-parser's regexes (uap-core v0.18.0,
//! Apache-2.0, `server/seeds/useragent`) read the browser, OS and device, and the device gets Datadog's category
//! (Desktop, Mobile, Tablet, Bot or Other).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

const REGEXES: &str = include_str!("../../../../seeds/useragent/regexes.yaml");

#[derive(Deserialize)]
struct Rules {
    user_agent_parsers: Vec<RuleDef>,
    os_parsers: Vec<RuleDef>,
    device_parsers: Vec<RuleDef>,
}

/// One uap-core rule: its regex and the replacements of the fields it reads (`$1` is the first group).
#[derive(Deserialize)]
struct RuleDef {
    regex: String,
    #[serde(default)]
    regex_flag: Option<String>,
    #[serde(default, alias = "os_replacement", alias = "device_replacement")]
    family_replacement: Option<String>,
    #[serde(default, alias = "os_v1_replacement", alias = "brand_replacement")]
    v1_replacement: Option<String>,
    #[serde(default, alias = "os_v2_replacement", alias = "model_replacement")]
    v2_replacement: Option<String>,
    #[serde(default, alias = "os_v3_replacement")]
    v3_replacement: Option<String>,
    #[serde(default, alias = "os_v4_replacement")]
    v4_replacement: Option<String>,
}

struct Rule {
    re: Regex,
    replacements: [Option<String>; 5],
}

struct Parsers {
    agents: Vec<Rule>,
    oses: Vec<Rule>,
    devices: Vec<Rule>,
}

fn parsers() -> &'static Parsers {
    static PARSERS: OnceLock<Parsers> = OnceLock::new();
    PARSERS.get_or_init(|| {
        let rules: Rules = serde_yaml_ng::from_str(REGEXES).expect("uap-core regexes");
        // A rule Rust's regex engine can't compile is left out (none at v0.18.0).
        let compile = |defs: Vec<RuleDef>| -> Vec<Rule> {
            defs.into_iter()
                .filter_map(|d| {
                    let flags = if d.regex_flag.as_deref() == Some("i") { "(?i)" } else { "" };
                    let re = Regex::new(&format!("{flags}{}", d.regex)).ok()?;
                    Some(Rule { re, replacements: [d.family_replacement, d.v1_replacement, d.v2_replacement, d.v3_replacement, d.v4_replacement] })
                })
                .collect()
        };
        Parsers { agents: compile(rules.user_agent_parsers), oses: compile(rules.os_parsers), devices: compile(rules.device_parsers) }
    })
}

/// The first matching rule's five fields: each its replacement (with `$1`… substituted) or, for `defaults`' fields,
/// the matching group; empty ones are None.
fn fields(rules: &[Rule], ua: &str, defaults: [Option<usize>; 5]) -> Option<[Option<String>; 5]> {
    rules.iter().find_map(|rule| {
        let caps = rule.re.captures(ua)?;
        let group = |i: usize| caps.get(i).map(|m| m.as_str().to_string());
        Some(std::array::from_fn(|k| {
            let value = match &rule.replacements[k] {
                Some(r) => Some((1..=9).rev().fold(r.clone(), |s, i| s.replace(&format!("${i}"), caps.get(i).map_or("", |m| m.as_str())))),
                None => defaults[k].and_then(group),
            };
            value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
        }))
    })
}

/// Datadog's device category: bots (uap's Spider), tablets and phones, desktop operating systems, else Other.
fn category(device: &str, os: &str) -> &'static str {
    if device == "Spider" {
        "Bot"
    } else if ["iPad", "Kindle", "Tablet", "Nexus 7", "Nexus 9", "Nexus 10"].iter().any(|t| device.contains(t)) {
        "Tablet"
    } else if ["iOS", "Android", "Windows Phone", "BlackBerry OS", "KaiOS", "Tizen", "Symbian OS"].iter().any(|m| os.starts_with(m)) {
        "Mobile"
    } else if os.starts_with("Windows") || ["Mac OS X", "Linux", "Ubuntu", "Chrome OS", "Fedora", "Debian", "FreeBSD", "OpenBSD", "NetBSD", "Solaris", "Red Hat", "SUSE", "Mint", "Gentoo", "Arch Linux"].contains(&os) {
        "Desktop"
    } else {
        "Other"
    }
}

/// `{browser, os, device}` of a user-agent string, as Datadog's user-agent parser writes them.
pub fn parse(ua: &str) -> Value {
    static CACHE: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(ua).cloned()) {
        return v;
    }
    let p = parsers();
    let versioned = |family: Option<String>, versions: &[Option<String>]| {
        let mut out = Map::new();
        out.insert("family".into(), Value::from(family.unwrap_or_else(|| "Other".into())));
        for (k, v) in ["major", "minor", "patch", "patch_minor"].iter().zip(versions) {
            if let Some(v) = v {
                out.insert((*k).into(), Value::from(v.as_str()));
            }
        }
        Value::Object(out)
    };
    let [family, v1, v2, v3, v4] = fields(&p.agents, ua, [Some(1), Some(2), Some(3), Some(4), Some(5)]).unwrap_or_default();
    let browser = versioned(family, &[v1, v2, v3, v4]);
    let [family, v1, v2, v3, v4] = fields(&p.oses, ua, [Some(1), Some(2), Some(3), Some(4), Some(5)]).unwrap_or_default();
    let os = versioned(family, &[v1, v2, v3, v4]);
    // A device rule: its family (else the first group), brand and model (else the first group).
    let [family, brand, model, _, _] = fields(&p.devices, ua, [Some(1), None, Some(1), None, None]).unwrap_or_default();
    let mut device = Map::new();
    let family = family.unwrap_or_else(|| "Other".into());
    device.insert("category".into(), Value::from(category(&family, os["family"].as_str().unwrap_or(""))));
    device.insert("family".into(), Value::from(family));
    for (k, v) in [("brand", brand), ("model", model)] {
        if let Some(v) = v {
            device.insert(k.into(), Value::from(v));
        }
    }
    let out = serde_json::json!({ "browser": browser, "os": os, "device": device });
    if let Ok(mut c) = cache.lock() {
        // A bounded cache: logs repeat a few user agents.
        if c.len() >= 4096 {
            c.clear();
        }
        c.insert(ua.to_string(), out.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browsers_systems_and_devices() {
        let chrome = parse("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.6099.109 Safari/537.36");
        assert_eq!(chrome["browser"], json!({"family": "Chrome", "major": "120", "minor": "0", "patch": "6099", "patch_minor": "109"}));
        assert_eq!(chrome["os"], json!({"family": "Mac OS X", "major": "10", "minor": "15", "patch": "7"}));
        assert_eq!(chrome["device"], json!({"family": "Mac", "brand": "Apple", "model": "Mac", "category": "Desktop"}));
        let windows = parse("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0");
        assert_eq!((windows["browser"]["family"].clone(), windows["os"]["family"].clone(), windows["device"].clone()), (json!("Firefox"), json!("Windows"), json!({"family": "Other", "category": "Desktop"})));
        let phone = parse("Mozilla/5.0 (iPhone; CPU iPhone OS 17_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Mobile/15E148 Safari/604.1");
        assert_eq!((phone["os"]["family"].clone(), phone["device"]["category"].clone()), (json!("iOS"), json!("Mobile")));
        let bot = parse("Mozilla/5.0 (compatible; YandexAccessibilityBot/3.0; +http://yandex.com/bots)");
        assert_eq!(bot["device"], json!({"family": "Spider", "brand": "Spider", "model": "Desktop", "category": "Bot"}));
        assert_eq!(parse("curl/8.4.0"), json!({"browser": {"family": "curl", "major": "8", "minor": "4", "patch": "0"}, "os": {"family": "Other"}, "device": {"family": "Other", "category": "Other"}}));
        assert_eq!(parse("")["browser"], json!({"family": "Other"}));
    }
}
