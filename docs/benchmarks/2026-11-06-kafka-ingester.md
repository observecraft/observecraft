# Logs and metrics ingestion after the Kafka log ingester (2026-11-06)

Follows [2026-11-05-logs-benchmark.md](2026-11-05-logs-benchmark.md); design in
[ADR-0005](../adr/0005-kafka-log-ingester.md).

## Setup

One laptop (Apple M4 Max, 16 cores, 64 GiB; Docker in OrbStack), the repository's Compose stack (`deploy/`) with the
branch's server and intake: ClickHouse, Postgres, Kafka (one broker, 12 partitions), Valkey, the OTel Collector,
server, intake. Defaults unless noted: `OC_LOG_INGEST=kafka`, the raw copy on, no intake limits, an org's records
spread over 12 partitions. Per-container CPU from `docker stats` every 10 s, averaged after the first minute. The
load generators share the laptop's CPU.

Generators must run inside the Docker network: from the host, a few requests through OrbStack's port forwarding never
complete (1 in 200 under 50 concurrent requests), which stalls a generator that waits for every request of a second.

## Logs

VictoriaMetrics' [logs-benchmark](https://github.com/VictoriaMetrics/logs-benchmark) writer, OTLP protobuf to
`/v1/logs`, 2,000-line batches of ~1 KB lines, 32 workers.

| Run | Sent lines/s | Stored lines/s | Kafka lag | Stack CPU (cores) | Millicores per 1k lines/s |
|---|---|---|---|---|---|
| 0.13.0, Collector → Vector → indexer (2026-11-05) | 60k target | 3.1k | growing | 1.9 | — |
| Kafka ingester, an org on 4 partitions | 98.3k | 79.0k | growing | 6.3 | — |
| Kafka ingester, an org on 12 partitions | 89.1k | **89.2k** | **0** | 8.7 | **97** |

Per container on the last run: server 4.5, ClickHouse 3.2, intake 0.7, Kafka 0.6, Collector 0.01 (it reads only
`oc.otlp.signals` now). Disk: 123 MiB per million lines (the raw copy is empty when every log is indexed), against
361 MiB with the 0.13.0 path. The generator dropped ~9.5k lines/s: the laptop's CPU, shared with it, was full.

Earlier limits found on the way: the intake's 50 MiB/s per-org default (now unlimited) capped an org at ~55k lines/s,
and with 4 partitions per org a partition stores ~19k lines/s (one core's decoding, pipelines and admission).

On the same hardware the first benchmark measured ClickStack at ~92k lines/s (~40 millicores per 1k lines/s,
79 MiB per million lines) and VictoriaLogs at ~99k (~10 millicores, 58 MiB).

## Metrics

`oc-tracegen load` (Datadog Agent-like `/api/v2/series` and 10% OTLP, 20 labels, each series every 10 s), one org.

| Run | Series | Stored samples/s | Stack CPU (cores) | Millicores per 1k samples/s |
|---|---|---|---|---|
| Collector at 512 MiB (Compose's former default) | 1M | 0 after 40 s (memory limiter refusing) | 4.8 | — |
| Collector at 2 GiB | 1M | **100.5k** (all sent), lag ~0 | 2.4 | **24** |
| series cache 2M (former default) | 3M | 115k | 5.1 | 44 |
| series cache 6M (new default) | 3M | **~200k** steady | 6.1 | **31** |

Disk: ~8–11 MiB per million samples, ~11 MiB per million tag rows (written about once per series and hour). With 3M
series the generator itself (one thread) was near one core, so ~200k samples/s is its ceiling as much as ours.

Fixed defaults: Compose's Collector memory limit is the chart's 1536 MiB (with `GOMEMLIMIT`), and the intake's series
cache holds 6M series × hour entries (`OC_INTAKE_SERIES_CACHE`, `intake.seriesCache`); past the active series a
replica sees, every point rewrites its series and ~20 tag rows.
