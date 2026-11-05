# ADR-0005: The server ingests logs from Kafka itself

Status: accepted (2026-11-05). Amends [ADR-0004](0004-kafka-ingestion-buffer.md) for logs.

## Context

After ADR-0004, application logs took this path:

```
intake ─► Kafka oc.otlp.logs ─► OTel Collector ─► Vector aggregator (normalize.vrl) ─┬─► ClickHouse oc.logs_raw_dedup
                                                                                      ├─► server log indexer ─► oc.logs_dedup
         Kafka oc.logs.json ──────────────────────────────────┘                      └─► Kafka oc.logs.live
```

On VictoriaMetrics' logs benchmark (docs/benchmarks/2026-11-05-logs-benchmark.md) this stored 3–16k lines/s on a
laptop where ClickStack (also on ClickHouse) stored ~92k and VictoriaLogs ~99k. Kafka accepted everything; the
ceiling was the chain after it. Each hop waited for the next to acknowledge (the Collector for Vector, Vector for its
three sinks, the indexer for ClickHouse), so a batch's latency, not CPU, bounded throughput, and every hop decoded and
re-encoded the logs. Datadog keeps Vector at the edge (Observability Pipelines); its backend after Kafka is its own.

## Decision

The server reads the log topics itself (`OC_LOG_INGEST=kafka`, the default; `server.logIngest` in the chart):

```
intake ─► Kafka oc.otlp.logs, oc.otlp-json.logs, oc.logs.json ─► server (log_ingest) ─┬─► ClickHouse oc.logs_dedup
                                                                                       ├─► ClickHouse oc.logs_raw_dedup (unindexed only)
                                                                                       ├─► Kafka oc.logs.live (open sessions only)
                                                                                       └─► Kafka oc.otlp.signals (other signals)
intake ─► Kafka oc.otlp.signals ─► OTel Collector ─► ClickHouse otel.* (events, RUM, replay, network, Kubernetes objects)
```

- **Normalization in Rust** (`log_normalize`): row for row what `normalize.vrl` produced; the tests compare 110
  inputs with the rows Vector 0.42 itself made from them (`scripts/log-normalize-golden.mjs`).
- **Decoding** (`log_records`): OTLP protobuf and JSON, and JSON logs, shaped as the Collector's `resource/org`
  processor and Vector's sources shaped them. The organization is the record's `x-oc-org` header.
- **Partitions and offsets in Postgres** (`log_ingest_partitions`, heartbeats in `log_ingest_servers`): servers lease
  their fair share, and an offset moves only after the partition's logs are stored. rskafka has no consumer groups
  and the workspace avoids librdkafka. Delivery is at least once; ids come from the intake, so a replay rewrites
  the same rows (ReplacingMergeTree).
- **Batches**: 1 MiB fetches, accumulated per partition up to 24 MiB or 1 s; at most `OC_LOG_INGEST_CONCURRENCY`
  (by default the CPUs it may use, 4–16) batches decoded and stored at once, which bounds memory.
- **The indexer runs on typed rows** (`logproc::IndexRow`): pipelines and standard attributes on the attributes
  normalization kept, admission without `serde_json::Value`. Vector's JSON rows still go through the same code.
- **Attributes are stored once**, in `attr` (strings). `attributes` (typed JSON) is empty for these rows; readers
  nest `attr` again (`pipelines::log::attributes_json`). Search, facets and aggregations always read `attr`.
- **Maps in key order**: ClickHouse stores a `Map` as arrays in the given order; the same order in every row
  compresses and merges far better.
- **The 72-hour copy holds only the logs no index keeps**, after pipelines. Archives and today's ingested volume
  read every log once (`archives::every_log`: indexed + unindexed, `LIMIT 1 BY id`, so Vector's rows, which have
  every log in the copy, count once). Archives now hold processed logs, as Datadog's do.
- **Live tail on demand**: an org's logs go to `oc.logs.live` only while one of its sessions is open
  (`live_tail_watchers`, renewed by the sessions).
- **Other signals** the intake converts to OTLP logs (`oc.signal`) have their own topic, `oc.otlp.signals`, which
  the Collector reads; resources marked with a signal that arrive on the logs topics (a cluster collector's
  Kubernetes objects) are passed on to it before the offset moves.
- **The intake doesn't limit an organization by default** (`OC_INTAKE_ORG_MBPS` / `OC_INTAKE_ORG_RPS` = 0); instance
  admins can limit one. The former 50 MiB/s default capped an org at ~55k lines/s of 1 KB logs.

`OC_LOG_INGEST=vector` (with `--profile vector` in Compose) keeps the earlier path, which an intake without Kafka needs.

## Consequences

On the benchmark (same laptop, docs/benchmarks/2026-11-06-kafka-ingester.md): ~89k lines/s stored with no lag,
everything the generator sent, at ~97 millicores per 1k lines/s for the whole default stack (ClickHouse, Kafka,
intake, server, Collector) and 123 MiB per million lines on disk, against 3.1k lines/s and 361 MiB before. ClickStack measured ~40 millicores and 79 MiB, VictoriaLogs
~10 millicores and 58 MiB.

- Details of new logs show attribute values as text (`"200"`), as custom destinations receive them.
- A log becomes searchable up to ~1 s later (batching); live tail shows logs once stored.
- Logs that arrive without a host no longer get the Collector's own hostname (`resourcedetection`).
- `normalize.vrl` and `log_normalize` must change together while the Vector path exists.
