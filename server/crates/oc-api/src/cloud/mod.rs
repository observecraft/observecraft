//! What the cloud integrations' crawlers (AWS, Azure, GCP) share: their jobs, their metric names and
//! points, and where the points go.

pub mod jobs;
pub mod kafka;
pub mod naming;

#[cfg(test)]
pub mod testing;
