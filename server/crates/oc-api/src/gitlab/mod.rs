//! Software Delivery's GitLab CI integration: the server polls GitLab's API (GitLab.com or a
//! self-managed instance; no inbound access needed) and sends each finished pipeline as a CI/CD trace
//! through Kafka, like a CI provider's OpenTelemetry exporter would (`routes::ci` reads them).
//!
//! - [`api`]: GitLab's REST requests, sent through the SSRF guard.
//! - [`otlp`]: a pipeline and its jobs as one OTLP trace.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).
//! - [`security`]: Code Security's import of GitLab's Vulnerability Report (the same connection, when asked).

pub mod api;
pub mod crawler;
pub mod otlp;
pub mod security;

#[cfg(test)]
pub mod fixtures;
