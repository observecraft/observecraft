//! Vanta: an org's compliance evidence sent to its private Vanta app (Build Integrations API), as Vanta's own Datadog
//! integration reads Datadog's (`docs/vanta.md`). [`api`] is Vanta's API, [`users`] and the other modules turn
//! ObserveCraft's records into Vanta's resources, [`sync`] sends them every hour on the cloud crawlers' jobs.

pub mod api;
pub mod custom;
pub mod findings;
pub mod sync;
pub mod users;
pub mod vulnerabilities;
