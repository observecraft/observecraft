# Implementation plan: logs from Kafka to ClickHouse in one Rust process

On VictoriaMetrics' logs-benchmark ingest workload, ObserveCraft stores ~13-16k lines/s where ClickStack (also on
ClickHouse) stores ~92k and VictoriaLogs ~99k on the same machine (docs/benchmarks/2026-11-05-logs-benchmark.md).
Kafka accepts every line; the ceiling is the chain after it (Collector → Vector → raw ClickHouse + HTTP indexer →
indexed ClickHouse), each stage waiting for the next. Datadog keeps Vector at the edge (Observability Pipelines)
and its backend after Kafka its own; ObserveCraft does the same: the server's log indexer reads Kafka itself and
writes ClickHouse, Vector and the Collector stay for agents, metrics and traces.

## Stage 1: Normalization in Rust
Goal: `log_normalize` turns an OTLP log record or a JSON log (with its Kafka org header and ingest identity) into
the indexed row exactly as `deploy/vector/normalize.vrl` does: reserved attributes, levels (klog, logfmt, dashed,
leading words, the Datadog Agent's stream status), timestamps, trace ids, Datadog tags from resources, unified
service tagging, FireLens, ddtags, the flattened `attr` and the `attributes` JSON.
Success Criteria: every case of `deploy/vector/tests.yaml` as a Rust test; golden rows produced by Vector itself
from the same inputs (`scripts/log-normalize-golden.mjs`, committed fixtures) match the Rust rows field by field,
for the test cases, edge cases and a sample of the benchmark generator's lines.
Status: Complete (110 cases match Vector 0.42.0's rows)

## Stage 2: The ingester reads Kafka
Goal: the server consumes `oc.otlp.logs` (OTLP protobuf) and `oc.logs.json` with a consumer group (rdkafka),
normalizes, runs the existing indexer (pipelines, standard attributes, index routing, exclusion filters, quotas,
generated metrics, forwarding, live tail) and commits offsets only after storage; `OC_LOG_INGEST=kafka|vector`
chooses the path, `vector` keeping today's.
Success Criteria: the indexer's existing tests; restart and rebalance tests with a real Kafka (no loss, no
duplicates beyond the ReplacingMergeTree's); the benchmark.
Status: Complete (Postgres leases and offsets, no consumer groups: rskafka has none and the workspace avoids
librdkafka; 1 MiB fetches, batches of up to 24 MiB / 1 s, at most OC_LOG_INGEST_CONCURRENCY stored at once)

## Stage 3: Binary inserts
Goal: rows go to ClickHouse as RowBinary in large batches instead of JSONEachRow.
Success Criteria: the indexer's storage tests; the benchmark (stored lines/s, CPU).
Status: Dropped: ClickHouse spends ~2 s of CPU parsing a run's JSON inserts, against ~85 s merging; the server's
indexed path instead runs pipelines on typed attributes (prepare_normalized) and sends larger batches

## Stage 4: Storage schema
Goal: one table per indexed log (the 72-hour raw copy optional), attributes stored once, rows ordered by stream
(org, service, host, time).
Success Criteria: Logs Explorer, facets, histogram, live tail and archives on the new table (their tests); the
benchmark's disk per million lines.
Status: Complete: attributes stored once (`attr`), maps in key order, the 72-hour copy holds only unindexed logs
(archives read indexed + unindexed), live tail on demand; 123 MiB per million lines. Stream ordering was not needed
(ORDER BY org, service, time already groups similar rows).

## Stage 5: Switch over
Goal: Compose and the Helm chart run the Kafka ingester by default, Vector off the log path, with the documented
way back; docs and CLAUDE.md updated.
Success Criteria: the full benchmark ramp, a run on the dev cluster.
Status: Complete in Compose and Helm (OC_LOG_INGEST / server.logIngest, default kafka; other signals on
oc.otlp.signals for the Collector; loadgen writes to Kafka; docs/adr/0005-kafka-log-ingester.md). A run on the dev
cluster remains.
