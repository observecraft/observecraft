//! Fixed-cardinality ingestion timings on the authenticated internal metrics endpoint.
//! Timers include failed/cancelled operations; stages overlap (request/admission contain substeps).

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BOUNDS_US: [u64; 11] = [1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000, 5_000_000, 15_000_000, 30_000_000, 60_000_000];

#[derive(Clone, Copy)]
pub enum Stage {
    Request,
    RequestSlot,
    OrgGate,
    PersistedLookup,
    Admission,
    QuotaLock,
    IndexedInsert,
    OutboxEnqueue,
}
const NAMES: [&str; 8] = ["request", "request_slot", "org_gate", "persisted_lookup", "admission", "quota_lock", "indexed_insert", "outbox_enqueue"];
static HISTOGRAMS: [Histogram; 8] = [const { Histogram::new() }; 8];

struct Histogram {
    count: AtomicU64,
    micros: AtomicU64,
    buckets: [AtomicU64; BOUNDS_US.len()],
}
impl Histogram {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            micros: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; BOUNDS_US.len()],
        }
    }
    fn observe(&self, duration: Duration) {
        let micros = duration.as_micros().min(u64::MAX as u128) as u64;
        self.micros.fetch_add(micros, Ordering::Relaxed);
        for (bound, bucket) in BOUNDS_US.iter().zip(&self.buckets) {
            if micros <= *bound {
                bucket.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.count.fetch_add(1, Ordering::Relaxed);
    }
    fn render(&self, stage: &str, out: &mut String) {
        for (bound, bucket) in BOUNDS_US.iter().zip(&self.buckets) {
            let _ = writeln!(
                out,
                "oc_log_ingest_stage_seconds_bucket{{stage=\"{stage}\",le=\"{}\"}} {}",
                *bound as f64 / 1e6,
                bucket.load(Ordering::Relaxed)
            );
        }
        let count = self.count.load(Ordering::Relaxed);
        let _ = writeln!(out, "oc_log_ingest_stage_seconds_bucket{{stage=\"{stage}\",le=\"+Inf\"}} {count}");
        let _ = writeln!(out, "oc_log_ingest_stage_seconds_count{{stage=\"{stage}\"}} {count}");
        let _ = writeln!(out, "oc_log_ingest_stage_seconds_sum{{stage=\"{stage}\"}} {}", self.micros.load(Ordering::Relaxed) as f64 / 1e6);
    }
}

pub struct Timer {
    stage: Stage,
    start: Instant,
}
pub fn timer(stage: Stage) -> Timer {
    Timer { stage, start: Instant::now() }
}
impl Drop for Timer {
    fn drop(&mut self) {
        HISTOGRAMS[self.stage as usize].observe(self.start.elapsed());
    }
}

pub fn render() -> String {
    let mut out = String::from("# HELP oc_log_ingest_stage_seconds Elapsed ingestion stage time, including failures and cancellation.\n# TYPE oc_log_ingest_stage_seconds histogram\n");
    for (name, histogram) in NAMES.iter().zip(&HISTOGRAMS) {
        histogram.render(name, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_buckets_include_boundaries_and_slow_observations() {
        let histogram = Histogram::new();
        for micros in [1_000, 1_001, 70_000_000] {
            histogram.observe(Duration::from_micros(micros));
        }
        let mut text = String::new();
        histogram.render("admission", &mut text);
        assert!(text.contains("le=\"0.001\"} 1\n"));
        assert!(text.contains("le=\"0.005\"} 2\n"));
        assert!(text.contains("le=\"60\"} 2\n"));
        assert!(text.contains("le=\"+Inf\"} 3\n"));
        assert!(text.contains("_count{stage=\"admission\"} 3\n"));
        assert!(text.contains("_sum{stage=\"admission\"} 70.002001\n"));
    }
}
