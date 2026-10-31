# Indexed ingestion batching and concurrency, 2026-11-01

At 40,000 offered JSON logs/s in fifteen-second bursts, indexed probe visibility p95 fell
from **66.30 to 2.10 seconds** with shared quotas, and from **55.34 to 4.22 seconds** with
quotas plus a generated count metric. This combines code changes and a second Vector consumer;
the intermediate runs below separate those steps. A sixty-second run exposes a remaining
capacity limit: two consumers reached **22.91 seconds** p95 in the metric scenario. Short-burst
results must not be used as a steady-state SLO.

## Changes and delivery guarantees

- Record bounded, fixed-label ingestion histograms for requests, request slots, organization gates,
  persisted identity lookup, admission, quota locks, indexed INSERTs and metric outbox enqueue.
  Their elapsed sums overlap; they are not CPU time. The benchmark subtracts warmup counters.
- Admit quota-only chunks of up to 2,048 events; metric expansion remains bounded to 512 source
  events. Coalesce indexed writes up to 8,192 events or 8 MiB, flushing before crossing either
  bound. An oversized individual chunk progresses alone. Await the final flush before ACK.
- Reuse storage connections within each API's application state. This also keeps clients scoped
  to the correct Tokio runtime in tests, rather than sharing a process-global connection pool.
- Export a bounded prefix of up to 32 durable metric-outbox entries or 8 MiB per grouped write.
  Preserve event identities and failed payloads across partial sum/histogram export failures.
  Each organization gets at most eight grouped rounds per pass; four organizations run concurrently.
- Hold local organization gates only while admitting each chunk. Release them after the durable
  Postgres reservation, before storage and outbox I/O. Rollback of uncommitted working counters
  still runs while their guards are held. Postgres advisory locks and receipts coordinate replicas.
- Limit each API to four active decoded indexer requests before JSON preparation, bounding the
  additional concurrency and its per-request buffers. Queued work stays owned upstream.
- Preserve the upstream RUM/replay Collector exporters with unlimited retry elapsed time, alongside
  the existing durability settings, when rebasing onto main.

Kafka ownership, stable replay identities, tenant checks, durable quotas and metric deduplication
remain required. Tests exercise partial buffered storage failures, replay without spending quota
again, same-organization admission while an earlier storage write is blocked, oversized buffers,
connection reuse and late ClickHouse errors, and mixed outbox export failures across restarts.
See [ADR-0004](../adr/0004-kafka-ingestion-buffer.md) for the contract and its limits.

## Fifteen-second comparison

All four trials used the same four organizations, two API replicas, 500-event sender batches,
repetitive 128-character messages, one broker with twelve partitions and one ClickHouse node.
Each scenario accepted 600,000 measured logs, plus 2,000 warmup logs. These are sequential single
trials on a shared Docker Desktop host, not randomized repetitions. Vector normalization was
already optimized in the baseline. Each Vector instance gets 2 CPUs and 512 MiB; adding replicas
adds resources. Other caps are listed in the [benchmark instructions](ingestion.md).
Reports retain image IDs, application revisions and mounted configuration hashes. The baseline
image records the pre-final-rebase revision `42429fd`; its application source matches the rebased
instrumented baseline `dcc46b6`. The later main changes moved Helm/release metadata, and the
Rust, web and Vector source trees were checked unchanged across that final rebase.

| Version | Vector replicas | No quota p95, s | Quota p95, s | Quota + metric p95, s |
|---|---:|---:|---:|---:|
| Rebased baseline | 1 | 7.079 | 66.302 | 55.344 |
| Indexed/outbox batching and connection pools | 1 | 7.351 | 18.312 | 19.676 |
| Same batching code | 2 | 1.157 | 5.360 | 10.956 |
| Per-chunk gates and four request slots | 2 | 1.143 | 2.102 | 4.215 |

With one consumer, quota-only indexed INSERT calls fell from 1,200 to 120 and admission calls
from 1,200 to 300. The metric scenario kept 1,200 bounded admissions but reduced indexed INSERTs
from 1,200 to 120. In the final two-consumer trial, accumulated gate wait was 0.34 seconds for
quota-only admission and 0.46 seconds for metric admission. The earlier two-consumer version
held gates over storage I/O and accumulated much more wait. Gate operation counts changed from
one per request to one per admission chunk, so compare elapsed sums rather than mean values.

The one-consumer batching quota trial had an ACK p95 of 254.6 ms, versus approximately 11–13 ms
in most other cases. Keep this variation visible; it is not evidence that every latency metric
improved monotonically. Probe visibility includes 500 ms polling and query time.

Reports: [baseline](results/2026-11-01-batching-before.json),
[batching, one Vector](results/2026-11-01-batching-one-vector.json),
[batching, two Vectors](results/2026-11-01-batching-two-vectors.json),
[per-chunk gates, two Vectors](results/2026-11-01-parallel-two-vectors.json).

## Longer run and consumer sizing

The [sixty-second, two-consumer run](results/2026-11-01-steady-two-vectors.json) accepted
2,400,000 measured logs per scenario. ACK p95 remained 11.1–12.3 ms, but downstream behavior differed:

| Scenario | Indexed probe p95, s | Drain after sender, s |
|---|---:|---:|
| No quota | 1.204 | 5.802 |
| Shared quota | 5.102 | 5.293 |
| Shared quota + count metric | 22.910 | 23.090 |

The [same sixty-second workload with three consumers](results/2026-11-01-steady-three-vectors.json)
used an additional 2 CPUs and 512 MiB for Vector. Visibility p95 was 1.195 seconds without
quotas, 5.764 seconds with quotas and 11.597 seconds with quotas plus the count metric; final
drain times were 4.985, 8.729 and 18.494 seconds respectively. This supports consumer sizing
as one part of the remedy, but does not establish a low-latency sustained 40,000 logs/s profile.
The HA chart already configures three Vector replicas; the benchmark now supports one to four
independent members of the same consumer group without modifying the shipped pipeline.

In the metric phase, one Vector reached its two-CPU cap while the other had spare capacity.
Organizations are Kafka keys: their partitions are indivisible between consumers. This workload's
metric organizations occupy partitions 0, 4 and 8, and two consumers can receive an uneven share.
More consumers help only when hot partitions can move to different owners; more replicas cannot
split a single hot partition or organization. Watch per-consumer CPU and partition lag, rather
than sizing from total Kafka lag or aggregate CPU alone.

ClickHouse also approached its four-CPU cap. Its sampled working set reached 1,609 MiB,
Postgres 653 MiB, the two APIs 360/372 MiB, and Vector 193/355 MiB, all within configured memory
caps. Observation queries count canonical `FINAL` rows repeatedly; this read work grows with
retained data and competes with ingestion. These runs do not isolate consumer skew from storage
or observation-query cost. Raising consumers alone is not a general remedy for storage saturation.
Size ClickHouse for the combined read/write load, and validate partition ownership after scaling.
Keep Kafka retention and disk headroom sufficient for the measured backlog and planned outages.

At a lower offered rate, the [sixty-second, 20,000 logs/s run with two consumers](results/2026-11-01-steady-20k-two-vectors.json)
accepted 1,200,000 measured logs per scenario. Visibility p95 was **1.572 seconds** without
quotas, **2.012 seconds** with shared quotas and **1.669 seconds** with quotas plus the count
metric. ACK p95 was 10.9–11.6 ms; observed final drain was 4.821–5.809 seconds. This is a
measured operating point for these caps and this synthetic one-minute workload, not a multi-day
capacity guarantee. It provides a concrete lower-load profile while sizing storage and consumers
for higher sustained rates.

## Backpressure and correctness

The [fault run](results/2026-11-01-faults-two-vectors.json), at 5,000 logs/s with two consumers,
accepted 75,000 measured logs per scenario. With 250 ms added to each storage INSERT, visibility
p95 was 1.842 seconds. With writes held for a nominal five seconds, including recorded release
observation overhead, it was 6.932 seconds. ACK p95 stayed approximately 11.6 ms. Kafka offsets
did not advance for the held work before storage was released; both scenarios drained completely.

Every completed report verifies each acknowledged batch's marker and full event count, not just
aggregate totals. Raw/indexed canonical counts match, quota receipts and reservations match
distinct indexed events, and generated counts match source contributions. Final outboxes are
empty and Kafka lag is zero. Sender failures, missing accepted probes, failed probe reads,
sampling errors and unacknowledged stored events are zero. Saved report destination paths are
portable basenames; the remaining machine-readable observations are preserved.
Across the eight runs above, **25,350,000 measured logs plus 46,000 warmup logs** were verified.

## Validation and upgrade

The exact final application source passed 1,120 Rust tests (three ignored) against both a real
single-node ClickHouse and a two-shard, two-replica ClickHouse cluster with Keeper. The 424
JavaScript regression tests, TypeScript check, production web build and 19 Vector cases passed.
Clippy completed with existing upstream warnings and no new warnings from this change.
`cargo deny check licenses` also passed.

The branch was rebased onto main at `32a67e1` (v0.7.1). Its unpublished migrations were renumbered
to Postgres 0066–0068 and ClickHouse 0018–0020 to preserve main's merged migration history.
A private upgrade rehearsal restored a backup of the earlier local database, checked the exact
migration checksums, remapped only the branch's old migration-history entries, applied main's
intervening migrations and verified an indexed sentinel survived. Existing published installations
apply the new numbered files normally. A developer database that already ran the unpublished old
numbers needs the guarded history remap; do not apply that remap to unrelated installations.
The local application was then upgraded using the same guarded procedure after draining Kafka
and taking a fresh private backup. All recorded Postgres migrations succeeded (highest version 68), the new main ClickHouse
migrations applied, and a pre-upgrade indexed event remained present exactly once. API readiness
reported ClickHouse/Postgres/Kafka healthy; intake and both local web endpoints returned HTTP 200.

Cutover still requires draining old consumers/outboxes, stopping old writers and synchronizing
replicas as described in ADR-0004. Metadata receipt retention and disk/WAL capacity must cover the
replay window. Longer dedicated-hardware soak tests, realistic payload/cardinality and concurrent
query traffic, replication failures and multi-signal workloads remain necessary for production
sizing. These measurements are not a Datadog capacity comparison.
