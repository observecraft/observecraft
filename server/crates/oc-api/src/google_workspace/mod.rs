//! The Google Workspace integration: the server reads a Workspace account's audit activities (login, admin, drive,
//! token, saml, groups, mobile, user_accounts, …) through the Admin SDK Reports API, with a service account acting as
//! an administrator (domain-wide delegation, the read-only audit scope), and sends each event as a log
//! (`source:gsuite`, `service:` the application, as Datadog's Google Workspace integration does) through Kafka. No
//! inbound access to ObserveCraft is needed.
//!
//! - [`api`]: the key, the JWT assertion and token exchange, the activity requests.
//! - [`logs`]: an activity's events as logs with Datadog's standard attributes.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`), one job per application.

pub mod api;
pub mod crawler;
pub mod logs;

#[cfg(test)]
pub mod fixtures;
