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

This establishes event identity, not storage deduplication: `oc.logs_raw`, indexed logs, incremental views,
quota counters and generated log metrics can still count a replay again. OTLP logs and legacy Kafka records
without the envelope still receive new IDs at normalization. A sender retry accepted as a new request also gets
new IDs; there is no client idempotency-key contract. The next storage change must use these identities across
changed batch boundaries and protect aggregates and both durable sinks. Replacing rows during background merges
alone does not retract duplicates already counted by incremental materialized views.

Before upgrading from file-backed Collector/Vector queues, stop new ingestion, leave the old consumers running
with healthy destinations until every exporter queue and Vector disk buffer is empty, and check Kafka consumer lag. Do not remove a volume
that still contains queued data: older consumers may already have committed those records. The new configuration
does not open or drain old file-backed queues. Resume ingestion after the upgraded consumers are healthy.

`node scripts/kafka-durability.mjs` starts a disposable Kafka, Collector, Vector and ClickHouse on their own network.
It holds the storage/indexer unavailable for 310 seconds, verifies that offsets remain uncommitted, destroys both
consumers and their local queues, then checks that replacements export traces, metrics and OTLP/JSON logs with
the correct tenant before committing. The indexer is an HTTP test endpoint; raw log storage uses real ClickHouse.
`OC_DURABILITY_OUTAGE_SECONDS=15` is a shorter smoke test, not verification of the former five-minute retry limit.
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
