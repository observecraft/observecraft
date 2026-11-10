//! The GitLab Audit Events integration: the server polls a GitLab instance's audit events (GitLab.com or
//! self-managed, Premium or Ultimate) through its REST API, for the whole instance (an administrator's token) or a
//! group or project, and sends each as a log (`source:gitlab-audit-events`, `service:gitlab`) through Kafka, like a
//! log shipper would. No inbound access to ObserveCraft is needed.
//!
//! - [`api`]: the audit event requests (GitLab's client is the GitLab CI poller's, `crate::gitlab::api`).
//! - [`logs`]: an audit event as a log with Datadog's standard attributes.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).

pub mod api;
pub mod crawler;
pub mod logs;

#[cfg(test)]
pub mod fixtures;
