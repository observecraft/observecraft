//! The GCP integration's collection: the Cloud Monitoring crawler (`crawler`), the Google APIs it calls
//! (`api`) and how its metrics are named and filtered (`naming`).

pub mod naming;
pub mod api;
pub mod crawler;
