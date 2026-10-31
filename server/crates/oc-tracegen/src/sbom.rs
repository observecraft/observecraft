//! Software bills of materials, as the Datadog Agent sends them (`/api/v2/sbom`, agent-payload
//! `SBOMPayload` protobuf with CycloneDX 1.4 BOMs): each demo host's Ubuntu packages and the demo
//! services' container images (Debian and Alpine based). The vulnerable packages are fictional
//! (`libexample-ssl3`, `example-musl`, …) and match the demo's fictional advisories
//! (`oc-api` `cloud_security::seed_demo_advisories`); the rest are ordinary package names.

use prost::Message;

use crate::hosts::HOSTS;

/// agent-payload `proto/sbom/sbom.proto` and CycloneDX `bom-1.4.proto` (Copyright (c) 2017 Datadog,
/// Inc., BSD-3-Clause): the fields the intake reads.
mod pb {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SbomPayload {
        #[prost(int32, tag = "1")]
        pub version: i32,
        #[prost(string, tag = "2")]
        pub host: String,
        #[prost(string, optional, tag = "3")]
        pub source: Option<String>,
        #[prost(message, repeated, tag = "4")]
        pub entities: Vec<SbomEntity>,
        #[prost(string, optional, tag = "5")]
        pub dd_env: Option<String>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SbomEntity {
        #[prost(int32, tag = "1")]
        pub r#type: i32,
        #[prost(string, tag = "2")]
        pub id: String,
        #[prost(string, repeated, tag = "4")]
        pub repo_tags: Vec<String>,
        #[prost(bool, tag = "5")]
        pub in_use: bool,
        #[prost(message, optional, tag = "10")]
        pub cyclonedx: Option<Bom>,
        #[prost(string, tag = "15")]
        pub kernel_version: String,
        #[prost(string, tag = "16")]
        pub cpu_architecture: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Bom {
        #[prost(string, tag = "1")]
        pub spec_version: String,
        #[prost(message, repeated, tag = "5")]
        pub components: Vec<Component>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Component {
        #[prost(string, tag = "8")]
        pub name: String,
        #[prost(string, tag = "9")]
        pub version: String,
        #[prost(string, optional, tag = "16")]
        pub purl: Option<String>,
    }
}

const HOST_PACKAGES: &[(&str, &str)] = &[
    ("bash", "5.1-6ubuntu1.1"),
    ("coreutils", "8.32-4.1ubuntu1.2"),
    ("openssh-server", "1:8.9p1-3ubuntu0.10"),
    ("example-sudo", "1.9.9-1ubuntu2.4"),
    ("example-kernel-tools", "5.15.0-119.129"),
    ("ca-certificates", "20240203~22.04.1"),
];

/// (image, tag, digest suffix, base, in use, extra libraries as (purl, version))
type Image = (&'static str, &'static str, &'static str, &'static str, bool, &'static [(&'static str, &'static str)]);
const IMAGES: &[Image] = &[
    ("storefront-api", "1.42.0", "a1", "debian", true, &[("pkg:maven/com.example.json/json-core@2.15.0", "2.15.0")]),
    ("storefront-api", "1.41.3", "a0", "debian", false, &[("pkg:maven/com.example.json/json-core@2.15.0", "2.15.0")]),
    ("order-service", "3.8.1", "b1", "debian", true, &[]),
    ("catalog-service", "2.4.0", "c1", "debian", true, &[]),
    ("billing", "5.0.2", "d1", "debian", true, &[]),
    ("api-gateway", "0.19.4", "e1", "alpine", true, &[]),
    ("notification-worker", "1.7.0", "f1", "alpine", true, &[]),
];

fn deb(name: &str, version: &str, distro: &str) -> pb::Component {
    let vendor = distro.split('-').next().unwrap_or("debian");
    pb::Component { name: name.into(), version: version.into(), purl: Some(format!("pkg:deb/{vendor}/{name}@{version}?arch=amd64&distro={distro}")) }
}

fn apk(name: &str, version: &str) -> pb::Component {
    pb::Component { name: name.into(), version: version.into(), purl: Some(format!("pkg:apk/alpine/{name}@{version}?arch=x86_64&distro=3.19.1")) }
}

fn image_components(base: &str, extra: &[(&str, &str)]) -> Vec<pb::Component> {
    let mut c = match base {
        "alpine" => vec![apk("example-musl", "1.2.4-r2"), apk("example-busybox", "1.36.1-r15"), apk("ca-certificates", "20240226-r0"), apk("libcrypto3", "3.1.4-r5")],
        _ => vec![
            deb("libexample-ssl3", "3.0.11-1~deb12u2", "debian-12"),
            deb("example-zlib", "1.2.13.dfsg-1", "debian-12"),
            deb("examplecurl", "7.88.1-10+deb12u5", "debian-12"),
            deb("libc6", "2.36-9+deb12u7", "debian-12"),
            deb("tzdata", "2024a-0+deb12u1", "debian-12"),
        ],
    };
    c.extend(extra.iter().map(|(purl, v)| pb::Component { name: purl.rsplit('/').next().unwrap_or(purl).split('@').next().unwrap_or_default().into(), version: (*v).into(), purl: Some((*purl).into()) }));
    c
}

/// The payloads of one round: one per host (its file system and the images it runs).
pub fn payloads() -> Vec<Vec<u8>> {
    HOSTS
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let mut entities = vec![pb::SbomEntity {
                r#type: 3,
                id: h.name.into(),
                in_use: true,
                cyclonedx: Some(pb::Bom { spec_version: "1.4".into(), components: HOST_PACKAGES.iter().map(|(n, v)| deb(n, v, "ubuntu-22.04")).collect() }),
                kernel_version: "5.15.0-119-generic".into(),
                cpu_architecture: "amd64".into(),
                ..Default::default()
            }];
            // Each host runs a share of the images.
            for (name, tag, digest, base, in_use, extra) in IMAGES.iter().skip(i % 2).step_by(2) {
                entities.push(pb::SbomEntity {
                    r#type: 1,
                    id: format!("sha256:{digest}{}", "0".repeat(62)),
                    repo_tags: vec![format!("registry.example.com/{name}:{tag}")],
                    in_use: *in_use,
                    cyclonedx: Some(pb::Bom { spec_version: "1.4".into(), components: image_components(base, extra) }),
                    ..Default::default()
                });
            }
            pb::SbomPayload { version: 1, host: h.name.into(), source: Some("agent".into()), entities, dd_env: Some("prod".into()) }.encode_to_vec()
        })
        .collect()
}

pub async fn send(http: &reqwest::Client, url: &str) {
    for body in payloads() {
        match http.post(url).header("content-type", "application/x-protobuf").body(body).send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!(status = %r.status(), "SBOM rejected"),
            Err(e) => tracing::warn!(error = %e, "SBOM not sent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_host_sends_its_file_system_and_images() {
        let p = payloads();
        assert_eq!(p.len(), HOSTS.len());
        let first = pb::SbomPayload::decode(p[0].as_slice()).unwrap();
        assert_eq!(first.entities[0].r#type, 3);
        assert!(first.entities.iter().skip(1).all(|e| e.repo_tags[0].starts_with("registry.example.com/")));
        assert!(first.entities.iter().flat_map(|e| &e.cyclonedx).flat_map(|b| &b.components).all(|c| c.purl.as_deref().is_some_and(|p| p.starts_with("pkg:"))));
    }
}
