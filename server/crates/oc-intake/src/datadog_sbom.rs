//! Software bills of materials from the Datadog Agent (`POST /api/v2/sbom`, the Agent's `sbom`
//! event platform track, enabled with `sbom.host.enabled` / `sbom.container_image.enabled`).
//!
//! The body is an agent-payload `SBOMPayload` protobuf (zstd, gzip or zlib): per entity (a host's
//! file system or a container image) a CycloneDX 1.4 BOM. Only what Cloud Security uses is decoded:
//! the entity, whether it is in use, and its components' purls and source packages. The intake passes them to the server
//! as JSON ([`to_upload`]), which matches them against advisories.
//!
//! The message definitions below are the fields used of agent-payload `proto/sbom/sbom.proto` and its
//! CycloneDX `bom-1.4.proto` (Copyright (c) 2017 Datadog, Inc., BSD-3-Clause), at their field numbers.

use prost::Message;
use serde_json::{json, Value};

pub mod pb {
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
        /// SBOMSourceType: 1 CONTAINER_IMAGE_LAYERS, 2 CONTAINER_FILE_SYSTEM, 3 HOST_FILE_SYSTEM, 5 HOST_IMAGE, …
        #[prost(int32, tag = "1")]
        pub r#type: i32,
        #[prost(string, tag = "2")]
        pub id: String,
        #[prost(string, repeated, tag = "4")]
        pub repo_tags: Vec<String>,
        #[prost(bool, tag = "5")]
        pub in_use: bool,
        #[prost(string, repeated, tag = "7")]
        pub dd_tags: Vec<String>,
        #[prost(bool, tag = "8")]
        pub heartbeat: bool,
        #[prost(message, optional, tag = "10")]
        pub cyclonedx: Option<Bom>,
        #[prost(string, optional, tag = "12")]
        pub error: Option<String>,
        /// SBOMStatus: 0 SUCCESS, 1 FAILED, 2 PENDING.
        #[prost(int32, tag = "11")]
        pub status: i32,
        #[prost(string, repeated, tag = "14")]
        pub repo_digests: Vec<String>,
        #[prost(string, tag = "15")]
        pub kernel_version: String,
        #[prost(string, tag = "16")]
        pub cpu_architecture: String,
    }

    /// cyclonedx.v1_4.Bom
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Bom {
        #[prost(string, tag = "1")]
        pub spec_version: String,
        #[prost(message, repeated, tag = "5")]
        pub components: Vec<Component>,
    }

    /// cyclonedx.v1_4.Component
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Component {
        #[prost(string, tag = "8")]
        pub name: String,
        #[prost(string, tag = "9")]
        pub version: String,
        #[prost(string, optional, tag = "16")]
        pub purl: Option<String>,
        /// Trivy's `aquasecurity:trivy:*` properties (the source package of an OS package).
        #[prost(message, repeated, tag = "22")]
        pub properties: Vec<Property>,
    }

    /// cyclonedx.v1_4.Property
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Property {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, optional, tag = "2")]
        pub value: Option<String>,
    }
}

pub fn decode(raw: &[u8]) -> Result<pb::SbomPayload, String> {
    pb::SbomPayload::decode(raw).map_err(|e| format!("invalid SBOM payload: {e}"))
}

/// The server's upload: entities by kind with their components' purls. Failed and pending scans
/// are left out.
pub fn to_upload(p: &pb::SbomPayload) -> Value {
    let entities: Vec<Value> = p
        .entities
        .iter()
        .filter(|e| e.status == 0 && e.error.is_none())
        .filter_map(|e| {
            let kind = match e.r#type {
                1 | 2 => "container_image",
                3 | 5 => "host",
                _ => return None,
            };
            let components: Vec<Value> = e
                .cyclonedx
                .iter()
                .flat_map(|b| &b.components)
                .filter_map(component)
                .collect();
            Some(json!({
                "kind": kind, "id": e.id, "repoTags": e.repo_tags, "repoDigests": e.repo_digests, "inUse": e.in_use,
                "kernel": e.kernel_version, "arch": e.cpu_architecture, "tags": e.dd_tags, "heartbeat": e.heartbeat, "components": components,
            }))
        })
        .collect();
    json!({ "host": p.host, "env": p.dd_env.clone().unwrap_or_default(), "entities": entities })
}

/// A component with a purl: its version and purl, and for an OS package the source package Trivy names
/// (`srcName`, `srcVersion` as `epoch:version-release`), which distributions' advisories are keyed by.
fn component(c: &pb::Component) -> Option<Value> {
    let purl = c.purl.as_ref().filter(|p| !p.is_empty())?;
    let prop = |k: &str| c.properties.iter().find(|p| p.name == format!("aquasecurity:trivy:{k}")).and_then(|p| p.value.as_deref()).filter(|v| !v.is_empty());
    let mut out = json!({ "version": c.version, "purl": purl });
    if let Some(name) = prop("SrcName") {
        out["srcName"] = json!(name);
        if let Some(v) = prop("SrcVersion") {
            let mut version = v.to_string();
            if let Some(r) = prop("SrcRelease") {
                version = format!("{version}-{r}");
            }
            if let Some(e) = prop("SrcEpoch").filter(|e| *e != "0") {
                version = format!("{e}:{version}");
            }
            out["srcVersion"] = json!(version);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_become_uploads() {
        let p = pb::SbomPayload {
            version: 1,
            host: "i-0a1b2c3d4e5f60001".into(),
            source: Some("agent".into()),
            dd_env: Some("prod".into()),
            entities: vec![
                pb::SbomEntity {
                    r#type: 1,
                    id: "sha256:abc".into(),
                    repo_tags: vec!["registry.example.com/storefront-api:1.42.0".into()],
                    in_use: true,
                    cyclonedx: Some(pb::Bom {
                        spec_version: "1.4".into(),
                        components: vec![
                            pb::Component { name: "example-musl".into(), version: "1.2.4-r2".into(), purl: Some("pkg:apk/alpine/example-musl@1.2.4-r2?distro=3.19.1".into()), properties: vec![] },
                            pb::Component { name: "no-purl".into(), version: "1".into(), purl: None, properties: vec![] },
                        ],
                    }),
                    ..Default::default()
                },
                pb::SbomEntity { r#type: 3, id: "i-0a1b2c3d4e5f60001".into(), status: 1, ..Default::default() },
                pb::SbomEntity { r#type: 4, id: "ci".into(), ..Default::default() },
            ],
        };
        let decoded = decode(&p.encode_to_vec()).unwrap();
        let up = to_upload(&decoded);
        assert_eq!(up["env"], "prod");
        assert_eq!(up["entities"].as_array().unwrap().len(), 1);
        assert_eq!(up["entities"][0]["kind"], "container_image");
        assert_eq!(up["entities"][0]["components"].as_array().unwrap().len(), 1);
        assert!(decode(b"\xff\xff\xff").is_err());
    }

    #[test]
    fn os_packages_carry_their_source_package() {
        // Trivy names the source package of a binary one (Debian's OSV advisories are by source package).
        let prop = |n: &str, v: &str| pb::Property { name: n.into(), value: Some(v.into()) };
        let c = pb::Component {
            name: "libssl3".into(),
            version: "3.0.11-1~deb12u2".into(),
            purl: Some("pkg:deb/debian/libssl3@3.0.11-1~deb12u2?arch=amd64&distro=debian-12.5".into()),
            properties: vec![
                prop("aquasecurity:trivy:SrcName", "openssl"),
                prop("aquasecurity:trivy:SrcVersion", "3.0.11"),
                prop("aquasecurity:trivy:SrcRelease", "1~deb12u2"),
                prop("aquasecurity:trivy:PkgType", "debian"),
            ],
        };
        assert_eq!(component(&c).unwrap(), json!({ "version": "3.0.11-1~deb12u2", "purl": c.purl, "srcName": "openssl", "srcVersion": "3.0.11-1~deb12u2" }));
        let mut e = c.clone();
        e.properties.push(prop("aquasecurity:trivy:SrcEpoch", "1"));
        assert_eq!(component(&e).unwrap()["srcVersion"], "1:3.0.11-1~deb12u2");
        let mut z = c.clone();
        z.properties = vec![prop("aquasecurity:trivy:SrcName", "musl"), prop("aquasecurity:trivy:SrcVersion", "1.2.4"), prop("aquasecurity:trivy:SrcEpoch", "0")];
        assert_eq!(component(&z).unwrap()["srcVersion"], "1.2.4");
        let mut plain = c.clone();
        plain.properties.clear();
        assert_eq!(component(&plain).unwrap(), json!({ "version": "3.0.11-1~deb12u2", "purl": c.purl }));
        // Through the protobuf: properties are CycloneDX field 22.
        let bom = pb::Bom { spec_version: "1.4".into(), components: vec![c.clone()] };
        assert_eq!(pb::Bom::decode(bom.encode_to_vec().as_slice()).unwrap().components[0].properties.len(), 4);
    }
}
