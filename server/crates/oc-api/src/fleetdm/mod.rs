//! The FleetDM integration: the server polls a Fleet server's API (self-hosted or Fleet's cloud) and sends its
//! activities (logins, policies, queries, scripts, software installs, MDM commands, hosts locked or wiped, users and
//! teams changed) as logs (`source:fleetdm`), and its hosts (status, last seen, disk, memory, failing policies,
//! vulnerabilities, MDM enrollment), host counts and policies (passing and failing hosts) as `fleetdm.*` metrics,
//! through Kafka. No inbound access to ObserveCraft is needed (Fleet's own webhooks and log destinations aren't used).
//!
//! - [`api`]: Fleet's API requests, sent through the SSRF guard.
//! - [`logs`]: an activity as a log with Datadog's standard attributes.
//! - [`hosts`]: the hosts, the host counts and the policies as metric points.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).

pub mod api;
pub mod crawler;
pub mod hosts;
pub mod logs;

#[cfg(test)]
pub mod fixtures;
