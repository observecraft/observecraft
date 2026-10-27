//! Datadog host tags on the host's series. The Agent sends a host's tags (`DD_TAGS`, cloud and
//! Kubernetes tags such as `kube_cluster_name`) once, in its host metadata, not on every series;
//! Datadog attaches them to every series from that host, and so do we, before the series are
//! converted to OTLP (`kube_cluster_name` then becomes `k8s.cluster.name` like a series' own tag).
//!
//! The tags are cached per (org, host). An org's hosts are loaded from Postgres `datadog_hosts` on
//! its first series (so a restarted intake doesn't wait for the next host metadata, sent every
//! 30 minutes) and reloaded every [`TTL`] (picking up host metadata other replicas received);
//! host metadata received here updates the cache at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sqlx::PgPool;

/// How long an org's loaded host tags are used before they are reloaded.
const TTL: Duration = Duration::from_secs(300);
/// After a failed (or running) load, how long until another request tries again.
const RETRY: Duration = Duration::from_secs(10);

/// One org's hosts → their `key:value` tags.
pub type Hosts = Arc<HashMap<String, Vec<String>>>;

struct Entry {
    hosts: Hosts,
    refresh_at: Instant,
}

/// Per-org host tags, keyed by the org of the sender's API key.
#[derive(Default)]
pub struct HostTags {
    orgs: Mutex<HashMap<String, Entry>>,
}

impl HostTags {
    /// The org's cached hosts, and whether the caller should (re)load them: an unknown org always
    /// loads; a stale one is reloaded by the first caller while the others keep the cached tags.
    fn snapshot(&self, org: &str) -> (Hosts, bool) {
        let now = Instant::now();
        let mut orgs = self.orgs.lock().unwrap();
        match orgs.get_mut(org) {
            Some(e) if e.refresh_at > now => (e.hosts.clone(), false),
            Some(e) => {
                e.refresh_at = now + RETRY;
                (e.hosts.clone(), true)
            }
            None => (Hosts::default(), true),
        }
    }

    /// Replaces the org's hosts with a fresh load.
    fn replace(&self, org: &str, hosts: HashMap<String, Vec<String>>) {
        self.orgs.lock().unwrap().insert(org.to_string(), Entry { hosts: Arc::new(hosts), refresh_at: Instant::now() + TTL });
    }

    /// The host's tags from its host metadata. An org not loaded yet is left alone: its first load
    /// reads them from Postgres.
    pub fn set(&self, org: &str, host: &str, tags: Vec<String>) {
        if let Some(e) = self.orgs.lock().unwrap().get_mut(org) {
            Arc::make_mut(&mut e.hosts).insert(host.to_string(), tags);
        }
    }

    /// The org's hosts and their tags, loaded from Postgres when not cached (or stale). A failed
    /// load keeps what is cached: series are then sent without host tags rather than refused.
    pub async fn hosts(&self, pg: &PgPool, org: &str) -> Hosts {
        let (cached, load) = self.snapshot(org);
        if !load {
            return cached;
        }
        let loaded = sqlx::query_as::<_, (String, Vec<String>)>("SELECT host, host_tags FROM datadog_hosts WHERE org_id = $1").bind(org).fetch_all(pg).await;
        match loaded {
            Ok(rows) => {
                self.replace(org, rows.into_iter().filter(|(_, tags)| !tags.is_empty()).collect());
                self.snapshot(org).0
            }
            Err(e) => {
                tracing::warn!(error = %e, org, "loading Datadog host tags failed");
                // Keep what is cached and retry after RETRY (an unknown org starts empty).
                self.orgs.lock().unwrap().entry(org.to_string()).or_insert_with(|| Entry { hosts: cached.clone(), refresh_at: Instant::now() + RETRY });
                cached
            }
        }
    }
}

/// Adds a host's tags to a series' tags. Tags already on the series win: a host tag whose key the
/// series has (`kube_cluster_name:a` vs `kube_cluster_name:b`) is left out.
pub fn merge(tags: &mut Vec<String>, host_tags: &[String]) {
    let key = |t: &str| t.split_once(':').map_or(t, |(k, _)| k).to_string();
    let present: std::collections::HashSet<String> = tags.iter().map(|t| key(t)).collect();
    tags.extend(host_tags.iter().filter(|t| !present.contains(&key(t))).cloned());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn merge_adds_the_hosts_tags() {
        let mut tags = strings(&["kube_namespace:shop", "env:prod"]);
        merge(&mut tags, &strings(&["kube_cluster_name:prod-eu", "kube_node:node-1", "env:prod"]));
        assert_eq!(tags, strings(&["kube_namespace:shop", "env:prod", "kube_cluster_name:prod-eu", "kube_node:node-1"]));
    }

    #[test]
    fn merge_keeps_the_series_tag_on_conflict() {
        let mut tags = strings(&["kube_cluster_name:from-series", "canary"]);
        merge(&mut tags, &strings(&["kube_cluster_name:from-host", "canary", "team:web"]));
        assert_eq!(tags, strings(&["kube_cluster_name:from-series", "canary", "team:web"]));
    }

    #[test]
    fn other_orgs_host_tags_never_apply() {
        let cache = HostTags::default();
        cache.replace("org-a", HashMap::from([("node-1".to_string(), strings(&["kube_cluster_name:a"]))]));
        let (hosts_b, load_b) = cache.snapshot("org-b");
        assert!(load_b, "org-b was never loaded");
        assert!(hosts_b.get("node-1").is_none());
        cache.set("org-b", "node-1", strings(&["kube_cluster_name:b"]));
        let (hosts_a, load_a) = cache.snapshot("org-a");
        assert!(!load_a);
        assert_eq!(hosts_a["node-1"], strings(&["kube_cluster_name:a"]));
    }

    #[test]
    fn host_metadata_updates_a_loaded_org() {
        let cache = HostTags::default();
        // Not loaded yet: the first load reads the host from Postgres.
        cache.set("org-a", "node-1", strings(&["env:prod"]));
        assert!(cache.snapshot("org-a").1);
        cache.replace("org-a", HashMap::new());
        cache.set("org-a", "node-2", strings(&["kube_cluster_name:a"]));
        let (hosts, load) = cache.snapshot("org-a");
        assert!(!load);
        assert_eq!(hosts["node-2"], strings(&["kube_cluster_name:a"]));
    }

    #[test]
    fn a_stale_org_is_reloaded_by_one_caller() {
        let cache = HostTags::default();
        cache.replace("org-a", HashMap::from([("node-1".to_string(), strings(&["env:prod"]))]));
        cache.orgs.lock().unwrap().get_mut("org-a").unwrap().refresh_at = Instant::now() - Duration::from_secs(1);
        let (first, reload) = cache.snapshot("org-a");
        assert!(reload);
        assert_eq!(first["node-1"], strings(&["env:prod"]));
        let (second, again) = cache.snapshot("org-a");
        assert!(!again, "the reload is already claimed");
        assert_eq!(second["node-1"], strings(&["env:prod"]));
    }
}
