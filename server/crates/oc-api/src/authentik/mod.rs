//! The authentik integration: the server polls an authentik instance's events (logins, failed logins, applications
//! opened, users and groups written, access requests, system exceptions, …) through its API and sends each as a log
//! (`source:authentik`) through Kafka, like a log shipper would. No inbound access to ObserveCraft is needed.
//!
//! - [`api`]: authentik's API requests, sent through the SSRF guard.
//! - [`logs`]: an event as a log with Datadog's standard attributes.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).

pub mod api;
pub mod crawler;
pub mod logs;

#[cfg(test)]
pub mod fixtures;
