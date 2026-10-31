# Vector normalization optimization, 2026-10-31

Skipping unused resource lookups and UUID generation reduced observed p95 indexed visibility
from **15.55 to 7.65 seconds** in the quota-free 40,000 logs/s burst. Shared quota and metric
delivery still dominated the other scenarios: their p95 values remained around one minute.

## Change and validation

`deploy/vector/normalize.vrl` now avoids constructing and scanning the Kubernetes/cloud alias
mapping, environment/version resource lookups and resource service fallback list when the resource
map is empty after removing tenant metadata. FireLens attributes and explicit tags are still
processed. Events carrying a valid intake-assigned UUIDv7 reuse it directly; UUID generation runs
only for events that need an identity. Normalized fields and delivery acknowledgement rules are
unchanged.

All 19 Vector tests passed with Vector 0.42.0, covering OTLP and JSON replay identities,
Kubernetes, FireLens, level preprocessing and tenant-header precedence. Two added cases check
resource service/version/environment priorities and FireLens fallback with tenant-only resources.
The 422 JavaScript tests, TypeScript check and benchmark script formatting also passed. No Rust
application code or schema changed in this experiment.

## Paired pipeline runs

The [before report](results/2026-10-31-normalize-before.json) and
[after report](results/2026-10-31-normalize-after.json) used the same application image at revision
`2ba12cc6a15a680b8d0f19e51a66db14e2c52366`, harness, aggregator configuration, container caps and
workload. The reports record separate SHA-256 hashes for the externally mounted Vector files,
which are not identified by the application image revision. Only report destination paths were
replaced with portable basenames in the saved JSON files. The harness subsequently received a
whitespace-only Prettier formatting change.

Both runs used `OC_BENCH_RATES=40000`, `OC_BENCH_MODE=throughput` and otherwise default settings:
four organizations, two API replicas, 500-event batches and a fifteen-second offered window per
scenario. Each run created fresh isolated databases. Vector retained its two-CPU, 512 MiB cap.
The same shared Docker Desktop host and small, repetitive JSON payloads described in the
[benchmark instructions](ingestion.md) apply. The baseline ran first, then the candidate; these
are single sequential trials, not randomized repetitions or a steady-state capacity estimate.

| Scenario | Version | Accepted logs/s | ACK p95, ms | Indexed probe p95, s | Drain after sender, s | Sampled peak lag, records | Vector peak working set, MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| No quota | Before | 40,002 | 15.2 | 15.55 | 24.08 | 854 | 344.1 |
| No quota | After | 40,000 | 12.1 | 7.65 | 11.89 | 522 | 274.5 |
| Shared quota | Before | 40,002 | 13.4 | 65.95 | 74.67 | 1120 | 396.0 |
| Shared quota | After | 40,002 | 11.4 | 61.15 | 67.94 | 1060 | 392.4 |
| Shared quota + count metric | Before | 39,607 | 211.6 | 73.91 | 88.44 | 1060 | 491.7 |
| Shared quota + count metric | After | 40,002 | 12.3 | 64.06 | 87.01 | 1000 | 422.1 |

The sender slightly exceeded its planned window in the baseline metric case; its p95 scheduling
lag was 183.6 ms. Thus its lower achieved send rate and high ACK latency should not be treated as
a controlled measurement of the normalizer alone. CPU samples still approached Vector's cap in
both versions. The improvement is observed backlog, visibility and completion time, not evidence
that CPU saturation disappeared. Probe visibility includes 500 ms polling. Drain includes metric
delivery, Kafka commit intervals and observation overhead, so it can remain long even when log
visibility improves.

All **3,600,000 acknowledged measured logs**, plus 12,000 warmup logs, reached raw and indexed
deduplicated storage. Every acknowledged batch was verified in full. Both metric scenarios ended
with exactly 602,000 generated contributions including warmup, empty metric outboxes and zero
Kafka lag. Quota receipts and reserved counts matched indexed events. Sender failures, missing
accepted probes, failed probe reads, sampling errors and unacknowledged stored events were zero.
The private benchmark containers and networks were removed after each run.

## Remaining bottleneck

This optimization removes work from the normalizer; it does not resolve durable admission and
storage batching. In `logproc::ingest`, quota or metric batches are divided into 512-event chunks.
Each chunk awaits quota admission, an indexed ClickHouse INSERT, and metric-outbox enqueue before
the next chunk. The process-local quota organization gates span this sequence; PostgreSQL advisory
locks cover admission transactions only. A 10,000-event indexer request therefore crosses many
sequential transaction/storage boundaries even when CPU is available.

The next change should first record wait and execution durations for those gates, PostgreSQL
admission, indexed INSERTs and outbox enqueue. That will distinguish coordinator contention from
storage round trips and durable commit latency. Then evaluate larger quota-only admission batches
or bounded coalescing of indexed writes, keeping metric expansion bounded and checking concurrent
replica caps, cancellation, partial storage failures and replay. The paired results do not isolate
which boundary dominates, and the small quota-only improvement could include run-to-run variation.
