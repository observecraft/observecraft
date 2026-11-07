//! The NetBird integration: the server polls a NetBird account's management API (NetBird Cloud or a self-hosted
//! management server) and sends its audit events (peers added and removed, policies, routes, networks, setup keys,
//! users, logins) as logs (`source:netbird`) and its peers' state (connected, last seen, login expired, client
//! version, OS) as `netbird.peer.*` metrics, through Kafka. No inbound access to ObserveCraft is needed, and it
//! works with self-hosted NetBird, where NetBird's own event streaming (a NetBird Cloud feature) isn't available.
//!
//! - [`api`]: NetBird's API requests, sent through the SSRF guard.
//! - [`logs`]: an audit event as a log with Datadog's standard attributes.
//! - [`peers`]: the peers as metric points.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).

pub mod api;
pub mod crawler;
pub mod logs;
pub mod peers;

#[cfg(test)]
pub mod fixtures;
