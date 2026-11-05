# OTLP log ingestion against VictoriaLogs and ClickStack, 2026-11-05

ObserveCraft against VictoriaLogs and ClickStack on VictoriaMetrics'
[logs-benchmark](https://github.com/VictoriaMetrics/logs-benchmark) (MIT): its writer, generator and seed, with
an `observecraft` system type added locally (OTLP protobuf to the intake's `/v1/logs`, `OC-API-Key`).

## Method

- One machine for every system, one system at a time: Apple M4 Max, Docker (OrbStack) with 16 CPUs and 64 GiB.
  The published results use one 8-vCPU node per system, so absolute numbers differ; the comparison is between
  the systems run here.
- The workload of the published ingestion test: 5,000 streams (125 hosts x 40 apps), 30-40 fields per line
  (~930 bytes of JSON), seed 1, 32 workers, 2,000 lines per request, lines dropped when a system can't keep up.
- Ramp: 10k lines/s, +10k every 3 minutes up to 100k (the published test holds each step 10 minutes, up to
  180k). Experiments: a fixed 60k lines/s for 6 minutes, measured over minutes 2 to 6.
- Stored lines are counted in the storage (ObserveCraft `oc.logs_dedup`, ClickStack `default.otel_logs`,
  VictoriaLogs `* | stats count()`), CPU and memory are the sum over the system's containers (`docker stats`),
  disk is read once the merges are done.
- ObserveCraft runs its Compose stack (intake, Kafka, Collector, Vector, server, ClickHouse, Postgres, Valkey);
  ClickStack its ingest path (HyperDX's collector 2.39.1 in standalone mode, ClickHouse 25.7); VictoriaLogs
  v1.53.0 at its defaults.

## Results (November 2026)

| | VictoriaLogs 1.53 | ClickStack 2.39 | ObserveCraft 0.12.1 |
|---|---|---|---|
| Stored lines/s at the ceiling | >= 98.7k (not reached) | ~92k | ~11-12k |
| CPU at 10k lines/s | 0.1 cores | 0.4 cores | 2.8 cores |
| CPU at the ceiling | 2.0 cores at 99k | 5.1 cores at 92k | ~4 cores at 11k |
| Peak memory | 1.4 GiB | 4.4 GiB | 6.4 GiB |
| Disk per million lines | 58 MiB | 79 MiB | 191 MiB (361 MiB for 72 h with `oc.logs_raw_dedup`) |

ObserveCraft's Kafka accepted every line (77 million); its ceiling is after Kafka. Its disk goes mostly to the
attributes stored twice (`attr` 79 MiB, `attributes` 65 MiB per million lines) and the random `id` (12 MiB).

## Experiments on the pipeline (fixed 60k lines/s, `main` at 0.13.0)

| Change | Stored lines/s | CPU |
|---|---|---|
| Baseline | 3.1k | 1.9 |
| Vector sinks at a fixed concurrency of 16 | 3.1k | 1.6 |
| An org's records on 12 Kafka partitions instead of 4 (`OC_KAFKA_ORG_SPREAD`) | **13.4k** | 5.4 |
| + 32 Collector export consumers | 13.4k | 5.6 |
| + 4 MiB Kafka fetches in the Collector | 0 (memory limiter refuses) | 4.5 |
| + a 2 GiB Collector memory limit | 13.7k | 5.5 |
| The Collector's earlier fetch settings with 12 partitions | 0 (memory limiter refuses) | 4.2 |

The Collector reads a partition's records one fetch at a time (`max_buffered_batches: 1`, bounded memory), so
throughput follows the number of partitions an org uses; with 12, Vector is the most expensive stage (1.7 to 3.3
cores at ~13.5k lines/s: the VRL normalization and three JSON encodings).
