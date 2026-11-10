//! The Microsoft Entra ID integration's logs: the server reads a tenant's sign-ins, directory audits and Identity
//! Protection risk detections from Microsoft Graph with the Azure integration's app registration, and sends each as
//! a log shaped as Azure's diagnostic settings export it (`source:azure.activedirectory`), through Kafka, so no Event
//! Hub or forwarder is needed.
//!
//! - [`api`]: the streams and their Graph requests.
//! - [`logs`]: an event as a log.
//! - [`crawler`]: the poller, on the cloud crawlers' jobs (`crate::cloud::jobs`).

pub mod api;
pub mod crawler;
pub mod logs;

#[cfg(test)]
pub mod fixtures;
