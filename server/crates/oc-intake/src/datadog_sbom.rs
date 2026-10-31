//! Software bills of materials from the Datadog Agent (`POST /api/v2/sbom`, the Agent's `sbom`
//! event platform track, enabled with `sbom.host.enabled` / `sbom.container_image.enabled`).
//!
//! The body is an agent-payload `SBOMPayload` protobuf (zstd, gzip or zlib): per entity (a host's
//! file system or a container image) a CycloneDX 1.4 BOM. Only what Cloud Security uses is decoded:
//! the entity, whether it is in use, and its components' purls. The intake passes them to the server
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
                .filter_map(|c| c.purl.as_ref().filter(|p| !p.is_empty()).map(|purl| json!({ "version": c.version, "purl": purl })))
                .collect();
            Some(json!({
                "kind": kind, "id": e.id, "repoTags": e.repo_tags, "repoDigests": e.repo_digests, "inUse": e.in_use,
                "kernel": e.kernel_version, "arch": e.cpu_architecture, "tags": e.dd_tags, "heartbeat": e.heartbeat, "components": components,
            }))
        })
        .collect();
    json!({ "host": p.host, "env": p.dd_env.clone().unwrap_or_default(), "entities": entities })
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
                            pb::Component { name: "example-musl".into(), version: "1.2.4-r2".into(), purl: Some("pkg:apk/alpine/example-musl@1.2.4-r2?distro=3.19.1".into()) },
                            pb::Component { name: "no-purl".into(), version: "1".into(), purl: None },
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
}
