//! Vulnerabilities as Vanta `vulnerable_component` and `package_vulnerability` resources, as Vanta reads Cloud
//! Security's library and host/container vulnerabilities from Datadog: Cloud Security's hosts and container images
//! (`cloud_security::infra_vulnerabilities`) and Code Security's libraries, per service and repository that has them
//! (`code_security::library_vulnerabilities`). Only open ones (not muted or resolved), most severe first; a component
//! is sent once, before the vulnerabilities that name it.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::error::ApiError;
use crate::routes::cloud_security::{self, InfraVulnerability};
use crate::routes::code_security::{self, LibraryVulnerability};
use crate::state::AppState;

/// The two resource types, ready to send.
#[derive(Debug, Default, PartialEq)]
pub struct Evidence {
    pub components: Vec<Value>,
    pub vulnerabilities: Vec<Value>,
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Vanta's severity (0–10, a tenth): the CVSS score when the advisory has one, else the label's lower bound.
pub fn severity_score(severity: &str, score: Option<f64>) -> f64 {
    if let Some(s) = score {
        return (s.clamp(0.0, 10.0) * 10.0).round() / 10.0;
    }
    match severity {
        "critical" => 9.0,
        "high" => 7.0,
        "medium" => 4.0,
        "low" => 0.1,
        _ => 0.0,
    }
}

/// A vulnerability as Vanta's `package_vulnerability` of the component `component`.
#[allow(clippy::too_many_arguments)]
fn package_vulnerability(unique_id: String, url: &str, advisory: &str, cve: Option<&str>, title: &str, package: &str, version: &str, fixed: Option<&str>, severity: f64, score: Option<f64>, component: &str) -> Value {
    let mut v = json!({
        "uniqueId": unique_id,
        "displayName": format!("{} in {package} {version}", cve.unwrap_or(advisory)),
        "externalUrl": url,
        "packageName": package,
        "packageVersion": version,
        "severity": severity,
        "vulnerableComponentUniqueId": component,
        "description": title,
        "isResolvable": fixed.is_some(),
        "remediationInstructions": match fixed {
            Some(f) => format!("Upgrade {package} to {f} or later."),
            None => "No fixed version has been published yet.".to_string(),
        },
    });
    if let Some(c) = cve {
        v["cveId"] = json!(c);
    }
    if let Some(s) = score {
        v["cvss3Score"] = json!((s.clamp(0.0, 10.0) * 10.0).round() / 10.0);
    }
    v
}

fn component(unique_id: &str, url: &str, name: &str, description: String, target_type: &str, collected: i64) -> Value {
    json!({
        "uniqueId": unique_id,
        "displayName": name,
        "externalUrl": url,
        "collectedTimestamp": rfc3339(collected),
        "name": name,
        "description": description,
        "targetType": target_type,
    })
}

/// The components and vulnerabilities of open host, image and library vulnerabilities (`ui`: the public URL).
pub fn evidence(infra: &[InfraVulnerability], libraries: &[LibraryVulnerability], ui: &str) -> Evidence {
    let (infra_url, library_url) = (format!("{ui}/security/cloud/vulnerabilities"), format!("{ui}/code-security/vulnerabilities/libraries"));
    // Components by id in the order first met, with their latest sighting.
    let mut order: Vec<String> = Vec::new();
    let mut components: BTreeMap<String, Value> = BTreeMap::new();
    let mut seen = |id: &str, make: &dyn Fn() -> Value, at: i64, order: &mut Vec<String>| {
        match components.get_mut(id) {
            Some(c) => {
                if at > chrono::DateTime::parse_from_rfc3339(c["collectedTimestamp"].as_str().unwrap_or_default()).map(|t| t.timestamp_millis()).unwrap_or(0) {
                    c["collectedTimestamp"] = json!(rfc3339(at));
                }
            }
            None => {
                order.push(id.to_string());
                components.insert(id.to_string(), make());
            }
        }
    };
    let mut vulnerabilities: Vec<(f64, Value)> = Vec::new();
    for v in infra.iter().filter(|v| v.status == "open") {
        let (id, target, description) = match v.resource_kind.as_str() {
            "host" => (format!("host:{}", v.resource_id), "HOST", format!("Host {} ({}, env {})", v.resource_name, v.os, v.env)),
            kind => (format!("{kind}:{}", v.resource_id), "CONTAINER_REPOSITORY_IMAGE", format!("Container image {}", v.resource_name)),
        };
        seen(&id, &|| component(&id, &infra_url, &v.resource_name, description.clone(), target, v.last_seen), v.last_seen, &mut order);
        let severity = severity_score(&v.severity, v.score);
        vulnerabilities.push((severity, package_vulnerability(v.key.clone(), &infra_url, &v.advisory_id, v.cve.as_deref(), &v.title, &v.package, &v.version, v.fixed.as_deref(), severity, v.score, &id)));
    }
    for v in libraries.iter().filter(|v| v.status == "open") {
        let severity = severity_score(&v.severity, v.score);
        let mut targets: Vec<(String, String, String, &str, i64)> = Vec::new();
        for o in &v.occurrences {
            let (id, name, description, target) = if !o.service.is_empty() {
                (format!("service:{}", o.service), o.service.clone(), format!("Service {} (env {})", o.service, o.env), "OTHER")
            } else if !o.repository.is_empty() {
                (format!("repository:{}", o.repository), o.repository.clone(), format!("Repository {}", o.repository), "CODE_REPOSITORY")
            } else {
                continue;
            };
            match targets.iter_mut().find(|t| t.0 == id) {
                Some(t) => t.4 = t.4.max(o.last_seen),
                None => targets.push((id, name, description, target, o.last_seen)),
            }
        }
        for (id, name, description, target, at) in targets {
            seen(&id, &|| component(&id, &library_url, &name, description.clone(), target, at), at, &mut order);
            vulnerabilities.push((severity, package_vulnerability(format!("lib:{}:{id}", v.key), &library_url, &v.advisory_id, v.cve.as_deref(), &v.title, &v.package, &v.version, v.fixed.as_deref(), severity, v.score, &id)));
        }
    }
    // Most severe first (Vanta takes 10,000 per type), stable otherwise.
    vulnerabilities.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Evidence { components: order.iter().filter_map(|id| components.remove(id)).collect(), vulnerabilities: vulnerabilities.into_iter().map(|(_, v)| v).collect() }
}

/// The org's vulnerabilities, read from Cloud Security and Code Security.
pub async fn collect(st: &AppState, org: &str) -> Result<Evidence, ApiError> {
    let (infra, libraries) = tokio::try_join!(cloud_security::infra_vulnerabilities(st, org), code_security::library_vulnerabilities(st, org))?;
    Ok(evidence(&infra, &libraries, &st.config.public_url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::code_security::Occurrence;

    const UI: &str = "https://observecraft.example.com";

    fn infra(key: &str, severity: &str, score: Option<f64>, resource: (&str, &str, &str), status: &str) -> InfraVulnerability {
        InfraVulnerability {
            key: key.into(),
            advisory_id: "GHSA-xxxx-yyyy-zzzz".into(),
            cve: Some("CVE-2026-1001".into()),
            aliases: vec!["CVE-2026-1001".into()],
            title: "Heap overflow in libexample".into(),
            severity: severity.into(),
            score,
            cwes: vec![],
            ecosystem: "deb".into(),
            package: "libexample".into(),
            version: "1.2.3".into(),
            fixed: Some("1.2.4".into()),
            resource_id: resource.0.into(),
            resource_name: resource.1.into(),
            resource_kind: resource.2.into(),
            host: "i-0a1b2c3d4e5f60001".into(),
            env: "prod".into(),
            os: "debian 12".into(),
            running: true,
            exploit_available: false,
            status: status.into(),
            first_seen: 1_790_000_000_000,
            last_seen: 1_795_000_000_000,
        }
    }

    fn occurrence(service: &str, repository: &str) -> Occurrence {
        Occurrence { source: if service.is_empty() { "static".into() } else { "runtime".into() }, service: service.into(), env: "prod".into(), repository: repository.into(), file: "package-lock.json".into(), dev: false, first_seen: 1_790_000_000_000, last_seen: 1_795_100_000_000 }
    }

    fn library(status: &str, fixed: Option<&str>, occurrences: Vec<Occurrence>) -> LibraryVulnerability {
        LibraryVulnerability {
            key: "GHSA-aaaa-bbbb-cccc|npm|lodash|4.17.20".into(),
            advisory_id: "GHSA-aaaa-bbbb-cccc".into(),
            cve: None,
            aliases: vec![],
            title: "Prototype pollution in lodash".into(),
            severity: "high".into(),
            score: None,
            cwes: vec![],
            ecosystem: "npm".into(),
            language: "javascript".into(),
            package: "lodash".into(),
            version: "4.17.20".into(),
            fixed: fixed.map(str::to_string),
            services: vec![],
            repositories: vec![],
            envs: vec![],
            in_production: true,
            direct: Some(true),
            first_seen: 1_790_000_000_000,
            last_seen: 1_795_100_000_000,
            status: status.into(),
            status_reason: String::new(),
            sources: vec![],
            url: String::new(),
            occurrences,
        }
    }

    #[test]
    fn severities_use_the_cvss_score_else_the_label() {
        assert_eq!(severity_score("high", Some(9.84)), 9.8);
        assert_eq!(severity_score("critical", None), 9.0);
        assert_eq!(severity_score("high", None), 7.0);
        assert_eq!(severity_score("medium", None), 4.0);
        assert_eq!(severity_score("low", None), 0.1);
        assert_eq!(severity_score("unknown", None), 0.0);
        assert_eq!(severity_score("low", Some(12.0)), 10.0);
    }

    #[test]
    fn a_host_and_its_vulnerability() {
        let e = evidence(&[infra("csm-vuln:GHSA-xxxx-yyyy-zzzz:h1:libexample", "high", Some(8.1), ("h1", "i-0a1b2c3d4e5f60001", "host"), "open")], &[], UI);
        assert_eq!(
            e.components,
            [json!({
                "uniqueId": "host:h1",
                "displayName": "i-0a1b2c3d4e5f60001",
                "externalUrl": "https://observecraft.example.com/security/cloud/vulnerabilities",
                "collectedTimestamp": "2026-11-18T11:06:40Z",
                "name": "i-0a1b2c3d4e5f60001",
                "description": "Host i-0a1b2c3d4e5f60001 (debian 12, env prod)",
                "targetType": "HOST",
            })]
        );
        assert_eq!(
            e.vulnerabilities,
            [json!({
                "uniqueId": "csm-vuln:GHSA-xxxx-yyyy-zzzz:h1:libexample",
                "displayName": "CVE-2026-1001 in libexample 1.2.3",
                "externalUrl": "https://observecraft.example.com/security/cloud/vulnerabilities",
                "packageName": "libexample",
                "packageVersion": "1.2.3",
                "severity": 8.1,
                "vulnerableComponentUniqueId": "host:h1",
                "description": "Heap overflow in libexample",
                "isResolvable": true,
                "remediationInstructions": "Upgrade libexample to 1.2.4 or later.",
                "cveId": "CVE-2026-1001",
                "cvss3Score": 8.1,
            })]
        );
    }

    #[test]
    fn images_services_and_repositories_are_components_and_only_open_ones_are_sent() {
        let image = infra("csm-vuln:GHSA-xxxx-yyyy-zzzz:img1:libexample", "critical", None, ("img1", "registry.example.com/storefront-api:1.4", "container_image"), "open");
        let muted = infra("csm-vuln:GHSA-xxxx-yyyy-zzzz:h2:libexample", "critical", None, ("h2", "i-0a1b2c3d4e5f60002", "host"), "muted");
        let lib = library("open", None, vec![occurrence("checkout-api", ""), occurrence("", "example-org/storefront")]);
        let resolved = library("resolved", Some("4.17.21"), vec![occurrence("billing", "")]);
        let e = evidence(&[image, muted], &[lib, resolved], UI);
        let components: Vec<(&str, &str)> = e.components.iter().map(|c| (c["uniqueId"].as_str().unwrap(), c["targetType"].as_str().unwrap())).collect();
        assert_eq!(components, [("container_image:img1", "CONTAINER_REPOSITORY_IMAGE"), ("service:checkout-api", "OTHER"), ("repository:example-org/storefront", "CODE_REPOSITORY")]);
        // One vulnerability per component that has it, most severe first; a library without a CVE keeps its advisory.
        let vulns: Vec<(&str, &str, f64)> = e.vulnerabilities.iter().map(|v| (v["vulnerableComponentUniqueId"].as_str().unwrap(), v["displayName"].as_str().unwrap(), v["severity"].as_f64().unwrap())).collect();
        assert_eq!(
            vulns,
            [
                ("container_image:img1", "CVE-2026-1001 in libexample 1.2.3", 9.0),
                ("service:checkout-api", "GHSA-aaaa-bbbb-cccc in lodash 4.17.20", 7.0),
                ("repository:example-org/storefront", "GHSA-aaaa-bbbb-cccc in lodash 4.17.20", 7.0),
            ]
        );
        let lib_vuln = &e.vulnerabilities[1];
        assert_eq!(lib_vuln["uniqueId"], "lib:GHSA-aaaa-bbbb-cccc|npm|lodash|4.17.20:service:checkout-api");
        assert_eq!(lib_vuln["externalUrl"], "https://observecraft.example.com/code-security/vulnerabilities/libraries");
        assert_eq!((lib_vuln["isResolvable"].as_bool(), lib_vuln["remediationInstructions"].as_str()), (Some(false), Some("No fixed version has been published yet.")));
        assert!(lib_vuln.get("cveId").is_none() && lib_vuln.get("cvss3Score").is_none());
        assert_eq!(e.components[0]["description"], "Container image registry.example.com/storefront-api:1.4");
        assert_eq!(e.components[1]["collectedTimestamp"], "2026-11-19T14:53:20Z");
    }

    #[test]
    fn a_component_is_sent_once_however_many_vulnerabilities_it_has() {
        let a = infra("csm-vuln:A:h1:libexample", "high", None, ("h1", "i-0a1b2c3d4e5f60001", "host"), "open");
        let b = infra("csm-vuln:B:h1:libother", "low", None, ("h1", "i-0a1b2c3d4e5f60001", "host"), "open");
        let e = evidence(&[b, a], &[], UI);
        assert_eq!(e.components.len(), 1);
        assert_eq!(e.vulnerabilities.iter().map(|v| v["uniqueId"].as_str().unwrap()).collect::<Vec<_>>(), ["csm-vuln:A:h1:libexample", "csm-vuln:B:h1:libother"]);
    }
}
