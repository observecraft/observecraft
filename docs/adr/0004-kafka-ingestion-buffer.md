# ADR-0004: Kafka as the durable ingestion buffer (replacing NATS)

Status: accepted (2026-10-30)

## Context

Before this change, the intake forwarded every accepted request synchronously over HTTP: OTLP and the Datadog
formats it converts to OTLP went to the OpenTelemetry Collector, JSON logs to the Vector aggregator. The intake
answered the sender with the Collector's or Vector's answer. Durability came from what sat behind it:

- the Collector's exporters keep a file-backed sending queue (`file_storage/queue`);
- Vector's sinks keep disk buffers;
- NATS ran with JetStream enabled (`-js`), but nothing used JetStream: its only use was core (fire-and-forget)
  publish/subscribe of normalized logs for live tail (`oc.logs.live.<org>`).

This has three weaknesses at scale:

1. **Coupling.** A slow or restarting Collector or Vector makes the intake answer 503, and senders (the Datadog
   Agent, OTel SDKs) retry or drop. Capacity of the processing tier bounds what the intake can accept.
2. **Durability is per pod.** Queued data lives on the disk of one Collector or Vector replica. Losing that pod's
   volume loses it, and another replica can't take the work over.
3. **No replay.** Once processed, a request can't be reprocessed (a bad pipeline change, a ClickHouse outage longer
   than the queues).

Datadog's own intake works the other way round: the intake only authenticates, validates and writes each payload
to Kafka, partitioned by organization, and acknowledges the sender once Kafka has it. Separate consumers then
process and store the data, can fall behind and catch up, and can be replayed.

## Decision

Kafka (Apache Kafka, KRaft mode, Apache-2.0) sits between the intake and processing:

```
senders ─► oc-intake ──produce (acks=all)──► Kafka ─┬─► OTel Collector (kafka receiver) ─► ClickHouse otel.*
                                                     └─► Vector aggregator (kafka source) ─► ClickHouse oc.logs_raw,
                                                                                            server log indexer,
                                                                                            Kafka oc.logs.live ─► server (live tail SSE)
```

- **Topics** (one per signal and encoding, so consumers never sniff payloads):
  `oc.otlp.traces`, `oc.otlp.metrics`, `oc.otlp.logs` (OTLP protobuf), `oc.otlp-json.traces|metrics|logs` (OTLP
  JSON), `oc.logs.json` (Datadog/JSON log events), and `oc.logs.live` (normalized logs for live tail, short
  retention). The intake decompresses request bodies before producing; Kafka compresses batches itself (zstd).
- **Tenancy.** The intake sets the same headers it sent over HTTP (`x-oc-org`, the org's retention settings,
  `x-oc-series-known`) as Kafka record headers. The Collector's Kafka receiver propagates record headers as request
  metadata, so `resource/org` keeps reading `metadata.x-oc-org` unchanged; Vector reads them from the record's
  headers. A sender can't set them: only the intake produces.
- **Partitioning.** Records are keyed by organization: an org's data stays ordered and lands on a stable set of
  partitions, and a noisy org can't reorder another's. Topics have enough partitions (default 12) to spread
  orgs over consumers.
- **Acknowledgement.** The intake answers the sender only after Kafka acknowledged the write with `acks=all`. If
  Kafka can't take it, the sender gets 503 and retries, as Datadog's intake does.
- **Consumer commits.** The Collector marks a record consumed only after export succeeds
  (`message_marking.after: true`, `sending_queue.wait_for_result: true`, no batch processor in front of the
  exporters). Collector 0.163 does not support waiting for a persistent queue, so the exporters use bounded
  memory queues (32 MiB each) for batching; Kafka remains responsible for unexported records. Transient export
  errors retry without a deadline (`max_elapsed_time: 0s`). Vector uses end-to-end acknowledgements and bounded
  memory buffers for both durable log sinks: a local disk write must not release Kafka/OTLP ownership. Its
  live-tail sink stays best effort and cannot back-pressure storage. A consumer crash replays from the last
  commit: delivery is at-least-once, not exactly-once.
  Collector records returning permanent pipeline errors remain uncommitted and need operator intervention;
  no dead-letter policy is implied for either consumer.
- **Reliability settings.** Production (Helm `values-ha.yaml`): 3 brokers, replication factor 3,
  `min.insync.replicas=2`, so one broker can fail without losing acknowledged data or stopping writes. Retention
  72 hours (subject to sufficient disk capacity and catch-up throughput), `oc.logs.live` 1 hour. Development (compose,
  Helm defaults): one broker, replication 1.
- **Client.** The intake and the server use `rskafka` (MIT/Apache-2.0, pure Rust: no librdkafka C build). The
  intake only produces; the server only reads `oc.logs.live` from the latest offset of the partitions its live
  tail sessions need, with no consumer group.
- **NATS is removed.** Its only use, live tail, moves to `oc.logs.live`.

Uploads the intake passes to the server's internal listener (profiles, feature flag evaluation counts, APM
telemetry, SARIF reports) keep that path for now: they are small, rare, and stored by the server itself. They can
move to Kafka topics the same way later.

## Consequences

- The intake no longer depends on the Collector or Vector being up: only on Kafka. Processing can be scaled,
  restarted or upgraded without senders seeing errors, and falls behind instead of rejecting data.
- Accepted data survives the loss of any processing pod and of one broker (HA).
- Data can be replayed by resetting a consumer group's offsets, within retention.
- Kafka is one more stateful service to run (brokers with persistent volumes). KRaft mode avoids ZooKeeper.
- End-to-end latency grows by the consumer poll interval (well under a second at load).
- The intake's answer no longer contains the Collector's partial-success details: it answers an empty
  `Export*ServiceResponse` once the data is durable, as Datadog's intake answers 202 without processing.

## Recovery and verification

JSON logs (`/api/v2/logs`) now receive a server-generated UUIDv7 per event and an acceptance timestamp
before being serialized to Kafka. The reserved `_oc_ingest` envelope is overwritten at the authenticated intake;
Vector consumes and removes it before normalization. A replay keeps the same ID and, for logs without a valid
event timestamp, the same fallback time. Equal events in one batch or separate requests receive distinct IDs.
Invalid JSON and arrays containing non-objects are rejected before Kafka acceptance. Compressed requests are
decoded before stamping on both the Kafka and direct HTTP paths.

OTLP logs (`/v1/logs`, HTTP JSON/protobuf and gRPC, including gzip/zstd) also receive a UUIDv7
before Kafka, in reserved log attributes `oc.ingest_id` and `oc.ingest_received_at`. Sender values
are replaced. Vector extracts these before parsing the body and removes them from user attributes.
The intake preserves unknown protobuf fields, resource/scope schema URLs, trace/span IDs and
structured bodies without re-encoding them through its partial Datadog conversion model.
When event time is missing or zero, it sets it to the supplied observed time, or to acceptance time
if both are absent. The persisted timestamp therefore survives replay, including logs without a
timestamp. OTLP metadata expansion is bounded by the intake body limit (default 16 MiB); oversized
batches receive HTTP 413 / gRPC ResourceExhausted before Kafka and must be split.

New writes go to `oc.logs_dedup` and `oc.logs_raw_dedup`, both ReplacingMergeTree tables.
Their sort key includes org, service, timestamp and ID; the shard key is `cityHash64(org_id, id)`.
`oc.logs_read` and `oc.logs_raw_read` apply FINAL, so search, monitors, index volumes and archives
see one copy even before merges. Indexed error/facet aggregates use `uniqExactState(id)` and merge those
states at read time. Retried inserts with different batching and order therefore do not inflate these counts.
This is logical deduplication of immutable normalized events; it does not promise one physical row per event.
Aggregate read views finalize the exact states on each shard and use ordered aggregation, so only counts
and sample states cross the network. Unique counts from the shards are additive because an identity always
belongs to one shard. Query clients explicitly send `optimize_aggregation_in_order=1`: a SETTINGS clause
inside a view alone does not enable it for the caller. Direct SQL clients should send it too.
On the local development stack (2026-10-31), a whole-history facet count representing about 23.1 million
logs exceeded a 1 GiB query limit without that setting. With it, the count completed in about one second
with 438 MiB peak query memory. This checks a failure mode on demo data; it is not a capacity benchmark.

Migration 0016 preserves the legacy tables, installs forwarding views for legacy writers, and backfills
both new tables after their aggregate views exist. Backfill can be retried without inflating reads or
aggregates. Pause ingestion, drain old consumers and stop them before upgrading; on a cluster, sync the
legacy tables' replicas before backfill. Budget disk space and migration time for copying the existing
indexed/raw logs and building the exact aggregate states. Legacy copies expire under their original TTL;
new writes use only the new tables. Retention changes and organization deletion cover both generations.
The new raw table uses the maximum supported raw TTL (30 days) during backfill; startup then applies the
configured raw retention, so installations keeping more than 72 hours retain their older copied logs.
Do not roll back to an older server that reads only the legacy tables after new writes have started.

The deduplication contract requires the same org, ID, timestamp, service, routing dimensions and retention
on replay. Reprocessing with changed pipelines/index rules is a reindex/rebuild operation, not an immutable
retry: old aggregate groups must be rebuilt as well. Changing the number of shards also requires moving
existing identities consistently. Exact UUID states consume memory/storage proportional to distinct events,
and FINAL adds read work; benchmark these costs before sizing a large installation. External ClickHouse
installations must set `insert_distributed_sync=1`, `wait_for_async_insert=1` and
`http_wait_end_of_query=1` for the ingestion user's profile, as the bundled chart does.

Remaining boundaries: legacy Kafka records and trusted agents bypassing intake without identities still receive new IDs at
normalization, so their replay is not deduplicated by identity. A sender retry accepted as a new request
also gets new IDs; there is no client idempotency-key contract.

For organizations with daily index quotas, every quota-bearing chunk (at most 512 events) acquires
transaction-scoped Postgres advisory locks in organization order. The indexer loads shared daily
counters and admitted IDs, routes the chunk, and commits new reservations and ID/retention metadata
atomically **before** writing ClickHouse. `synchronous_commit=on` requires WAL durability for the
admission transaction. No log bodies enter this journal. Unlimited organizations skip quota storage;
within mixed organizations, unlimited indexes do not create quota receipts.

A failed, cancelled or ambiguously acknowledged storage write keeps its durable reservation. It
must not release a slot that another replica could spend while the first write may have succeeded.
A retry/restart reuses the admitted identity, including when a ClickHouse replica has not seen the
write yet. Storage and the generated metric outbox must still succeed before source acknowledgement;
otherwise HTTP 503 leaves ownership with Kafka. Per-org local locks protect the process's working
copy; Postgres advisory locks, counters and receipts are the cross-replica source of truth. There is
no periodic whole-history ClickHouse counter reconciliation. Locks have a five-second timeout,
admission has a twenty-second deadline, and storage HTTP writes have a thirty-second deadline.

The first use of each quota-bearing index bootstraps its current-day count once from deduplicated
ClickHouse rows. Stop old writers, drain Kafka/outboxes, and synchronize ClickHouse replicas before
cutover: old writers do not participate in shared admission. This cutover baseline uses event
timestamps, so it cannot reconstruct historical ingestion-day counts for late/future logs. Later
quota days start at zero and count first admissions on the configured UTC reset schedule, regardless
of event timestamp. A replay of an admitted prior-day event spends no new-day slot. A newly admitted
log consumes capacity even if delivery is pending or its old event timestamp is already outside TTL.
The UI's indexed volume continues to report stored event-time volume; pending reservations can make
available quota smaller than that volume suggests during an outage.

Receipts default to seven days, exceeding the bundled 72-hour Kafka retention; counters retain eight
days and small per-index bootstrap markers remain for the organization lifetime. Cleanup discovers expired state,
deletes up to 10,000 receipts per transaction and performs at most sixteen passes per organization
per ten-second sweep. It uses the same org lock and never decrements a daily counter. Expiration is a
cleanup target, not a hard disk bound under sustained overload. Longer supported replay requires
extending the `log_quota_admissions.expires_at` default before accepting those records and sizing the
metadata store accordingly. After receipt expiry, retained ClickHouse rows still allow best-effort
admission reuse, but stale reads can charge a replay again. Source records without stable IDs and
newly accepted sender retries remain outside identity reuse.

For a fixed loaded policy, healthy durable Postgres and retained receipts, replicas cannot allocate
more fresh slots than the shared daily cap. Policies still reload every ten seconds; changes are not
an atomic barrier across replicas. Rejected events have no receipt and can be admitted if replayed
on a later day or after policy changes. Postgres unavailability fails quota admission closed, without
acknowledging those source requests. Restoring/failing over Postgres must preserve committed quota
metadata, as well as the generated metric outbox. The cost is one small receipt per distinct admitted
quota event plus WAL/index/vacuum work; benchmark this traffic and cleanup before sizing a large
installation. This is a deliberate durability cost restricted to organizations that enable quotas,
not evidence of Datadog-scale ingestion capacity.

`scripts/bench-quota-metadata.sql` measures receipt storage in a temporary table and rolls back.
On local Postgres 16 (2026-10-31), 100,000 fresh receipts with 36-character org IDs and a short
index name used 18,735,104 bytes including heap/indexes, about 187 bytes per receipt. A linear
extrapolation is roughly 1.3 GB for seven days at one million admissions per day, before WAL,
replicas, vacuum/bloat and operational headroom. This is a storage microbenchmark, not an ingestion
capacity result; high-volume quota tenants need measured metadata throughput and cleanup capacity.

Generated log metrics retain one contribution per source event, metric, series and 10-second
bucket, including logs excluded from indexes. Duplicate identities inside a request contribute
once. Before acknowledging the source request, the indexer commits these metric contributions
to the bounded Postgres `log_metric_outbox`; no raw log bodies or permanent per-event receipt
journal are added to Postgres. A background worker exports through `metrics_log_ingest_sum` /
`metrics_log_ingest_exponential_histogram`, feeding the existing normalized OpenTelemetry points,
series and tag catalog. The reserved `oc.log_event_id` attribute does not become a series label.

ClickHouse stores event-bearing scalar/distribution samples in `metrics_log_samples` /
`metrics_log_dist_samples` (ReplacingMergeTree, deterministically routed by series). Shard-local
read views apply FINAL before aggregating unique contributions into 10-second points; histogram
bucket indexes are aligned before combining their counts. The common `metrics_samples_read` /
`metrics_dist_samples_read` views combine these with ordinary metric samples. Explorers, dashboards,
other metric readers and rollups use these common views. Arrival markers on the new tables schedule
late rollup repair just as ordinary samples do. Correctness does not depend on background merges,
outbox batch boundaries or exporter deduplication windows: a lost insert acknowledgement, worker
restart or source replay may write more physical rows, but retained identities contribute once.

Outbox row locks with SKIP LOCKED share work across replicas. Successful sink payloads are cleared
even when another fails. Each sink attempt is bounded to 30 seconds, and up to four organizations
export concurrently per API replica. The spool remains bounded per organization to 64 MiB of pending
payloads and 4,096 batches. A full spool or unavailable Postgres returns 503, retaining source ownership
with Kafka. Per-event payloads cost more than the former per-request aggregates: benchmark outbox/WAL,
ClickHouse event-state volume, FINAL/grouping reads and Vector batch sizes at the intended log rate.
If one request expands beyond the spool cap, reduce its batch size or generated metric definitions;
a Kafka-backed derived stream is a scaling option for replacing the Postgres retry spool.
Requests for organizations with generated metrics are processed in chunks of 512 events so event contributions
do not accumulate for an entire Vector batch. Completed prefixes can safely be replayed after a
later failure. Source acknowledgement still waits for the whole request, and the 1,000-series budget
per metric is shared across all its chunks. Requests without generated metrics retain one bulk write.

Deduplication requires stable source IDs and immutable metric definitions, processed fields, labels,
event time and retention. Intake-issued JSON/OTLP UUIDv7 IDs also supply a stable acceptance clock for the
20-minute lateness policy, including delayed Kafka replay. Legacy records without stable
normalization identities still get new IDs on source replay; a newly accepted sender retry also
gets new IDs. Changing configuration is a rebuild operation, not immutable replay. The guarantee
lasts while the canonical event state is retained; resharding must move existing identities and
series consistently. It applies to contributions produced by this
version: historical aggregate samples and older pending payloads have no event IDs and cannot be
deduplicated retroactively. Drain Kafka and the old outbox before upgrading; replaying pre-upgrade
history across that boundary requires rebuilding the corresponding generated metric history.
Before downgrading, drain the new outbox and preserve/export the new sample tables; older binaries
read only the legacy sample tables. Outbox transport itself remains at least once, while the new
metric sample reads and rollups deduplicate stable contributions.
Raw metric samples, traces and live tail retain at-least-once semantics.

Before upgrading from file-backed Collector/Vector queues, stop new ingestion, leave the old consumers running
with healthy destinations until every exporter queue and Vector disk buffer is empty, and check Kafka consumer lag. Do not remove a volume
that still contains queued data: older consumers may already have committed those records. The new configuration
does not open or drain old file-backed queues. Resume ingestion after the upgraded consumers are healthy.

`node scripts/kafka-durability.mjs` starts a disposable Kafka, Collector, Vector and ClickHouse on their own network.
It holds the storage/indexer unavailable for 310 seconds, verifies that offsets remain uncommitted, destroys both
consumers and their local queues, then checks that replacements export traces, metrics and OTLP/JSON logs with
the correct tenant before committing. The indexer is an HTTP test endpoint; raw log storage uses real ClickHouse. The test also resets JSON
and OTLP consumer offsets and checks raw read deduplication, then inserts indexed events in changed batches to
verify search/error/facet counts before background merges, tenant isolation and repeated legacy backfill.
`OC_DURABILITY_OUTAGE_SECONDS=15` is a shorter smoke test, not verification of the former five-minute retry limit.
Set `OC_DURABILITY_INTAKE_IMAGE=observecraft:local` after building the current image to additionally
run the actual intake against disposable Postgres/Kafka. This sends both JSON and protobuf OTLP
logs through authentication, stamps two identical events independently, destroys/recreates consumers,
then resets both OTLP log topics and verifies stable identities and event/fallback timestamps in ClickHouse.
The test uses the shipped Collector configuration. It does not stop or reuse the application stack.

Metric replay also needs rollup repair. `otel.metrics_arrivals` records arrival minutes and event-time buckets
for both scalar and distribution samples. The rollup worker scans settled arrival minutes, transactionally
rewinds both tier frontiers for affected closed buckets, and then recalculates them from samples. Automatic
repair is bounded to the last 72 hours (the default Kafka replay window), within the supported raw retention
tiers (at least seven days). Older late data fails with a rebuild-required error: verify that the original raw
samples still exist before replacing historical aggregates. Processing
time is separate from sender event time. Markers expire after 30 days; an arrival scan older than that fails
closed with a rebuild-required error instead of silently advancing. The settling window is at least two minutes
and the existing replica-delay guard applies. This is eventual correction after arrival, not transactional
visibility of raw samples and aggregates. Query caches may briefly serve their existing cached result.

## Alternatives considered

- **NATS JetStream** (already deployed): durable streams and consumer acknowledgements would work, but neither
  the OTel Collector nor Vector has a production JetStream consumer that commits after delivery, and Datadog's
  model, tooling and operational knowledge are Kafka's.
- **Redpanda**: Kafka-compatible and simpler to run, but source-available (BSL), which the project's license
  rules exclude.
- **Keep HTTP forwarding with bigger queues**: doesn't remove the coupling or the per-pod durability.
