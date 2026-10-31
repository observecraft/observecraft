# JSON log ingestion benchmark

Measured examples and identified capacity gaps: [2026-10-31 results](2026-10-31-ingestion.md).
First optimization and paired comparison: [Vector normalization](2026-10-31-normalization.md).

Run `node scripts/ingestion-benchmark.mjs` after building the current application image
(`observecraft:local` by default). Docker is required. The harness creates a private network,
fresh databases and capped containers; it never connects to, stops or resets the local application.
It removes its containers, anonymous volumes and network on completion, including failed runs.
The JSON report defaults to the operating system's temporary directory and its path is printed.

This measures the real JSON path:

```text
rate-controlled sender → authenticated intake → Kafka → shipped Vector configuration
                                                   ├→ raw ClickHouse storage
                                                   └→ two API indexers → indexed ClickHouse storage
                                                                      ├→ quota metadata in Postgres
                                                                      └→ metric outbox → native metric storage
```

The test proxy streams requests and distributes indexer requests round-robin. Faults delay or
hold only ClickHouse INSERTs, allowing observation queries to continue. Generated metrics use
the API's real outbox workers and native event-bearing metric storage. No benchmark endpoint,
mock indexer or direct telemetry insert substitutes for these components.

## Default workload

The matrix compares 1,000 and 5,000 offered logs/s, without and with quota coordination, followed
by quota plus one generated count metric, a 250 ms storage delay, and a short storage pause.
Each phase has fresh organizations, a warmup of one batch per organization, and a fifteen-second
offered window. Warmup rows are excluded from reported measured counts. All organizations use a
catch-all index, no exclusions, seven-day indexed retention, and a quota high enough to keep every
log. Quota scenarios measure coordination cost, not intentional quota rejection.

Defaults are four organizations, two API replicas, batches of 500 events and at most sixteen
concurrent sender requests. Events use a repetitive 128-character message suffix, four commerce
services plus a probe service, eight hosts, and five percent error severity. Each batch contains
one probe with a unique marker. The authenticated intake assigns the actual event identities.
High-entropy messages, more fields, high-cardinality metrics, pipelines and historical datasets
can change the results substantially.

Container limits, also included in the report:

| Component | CPUs | Memory |
|---|---:|---:|
| Kafka | 2 | 2 GiB |
| ClickHouse | 4 | 3 GiB |
| Postgres | 2 | 1 GiB |
| Each API replica | 2 | 768 MiB |
| Vector | 2 | 512 MiB |
| Intake | 2 | 512 MiB |
| Proxy / sender, each | 1 | 256 MiB |
| Kafka observation CLI | 1 | 512 MiB |
| Valkey | 0.5 | 256 MiB |

Kafka has one broker and twelve partitions; ClickHouse has one node. The report records application
revision, exact container image IDs, harness hash, Docker host CPU/RAM and the configured limits.
New reports also include `vectorConfigSha256` for the mounted aggregator and normalization files:
the application image revision alone does not identify a change to the external Vector configuration.
Other workloads can share the host; limits do not reserve dedicated CPUs or eliminate contention.
Kafka's Java observation CLI runs in its own capped container, so its startup CPU is not charged
to the broker's budget or misreported as broker saturation.

## Run configuration

For example, in PowerShell:

```powershell
$env:OC_BENCH_SECONDS = '60'
$env:OC_BENCH_RATES = '1000,5000,10000'
$env:OC_BENCH_OUTPUT = "$env:TEMP\oc-ingestion-benchmark.json"
node scripts/ingestion-benchmark.mjs
```

| Environment variable | Default | Meaning |
|---|---|---|
| `OC_BENCH_IMAGE` | `observecraft:local` | Current application image, migrations included |
| `OC_BENCH_SECONDS` | `15` | Offered seconds per measured phase, 5–300 |
| `OC_BENCH_RATES` | `1000,5000` | Up to four distinct offered rates, 100–100,000 logs/s |
| `OC_BENCH_MODE` | `all` | `throughput` skips faults; `faults` runs only delayed/paused storage |
| `OC_BENCH_ORGS` | `4` | Organizations per phase, 1–16 |
| `OC_BENCH_API_REPLICAS` | `2` | Independent API processes, 1–4 |
| `OC_BENCH_VECTOR_REPLICAS` | `1` | Independent consumers sharing `oc-vector`, 1–4; each gets the listed Vector CPU/RAM cap |
| `OC_BENCH_BATCH` | `500` | Logs per authenticated sender request, 10–2,000 |
| `OC_BENCH_CONCURRENCY` | `16` | Maximum in-flight sender requests, 1–128 |
| `OC_BENCH_MESSAGE_BYTES` | `128` | Repetitive message suffix length, 32–4,096 |
| `OC_BENCH_STORAGE_DELAY_MS` | `250` | Artificial delay per INSERT in the slow phase, 1–2,000 ms |
| `OC_BENCH_OUTPUT` | unique temporary file | JSON result destination |

The sender does not retry failed requests. It reports HTTP/error counts and successful versus
offered logs. Its scheduling lag distinguishes a load-generator/intake limit from downstream
backlog. There is a maximum of 100,000 sender requests per phase to bound latency sample storage.
The pause targets at most five seconds, then snapshots Kafka offsets before releasing writes;
the actual release timestamp includes observation overhead and is recorded.

## What the measurements mean

- **Intake ACK latency** is measured in the sender process and ends after the response body is
  read. It measures Kafka acceptance, not ClickHouse visibility. The send window includes the
  final scheduled batch interval to avoid overstating rates in small runs.
- **Indexed probe visibility** runs from a batch's send start until its marker first appears in
  `logs_read`. It includes 500 ms polling granularity and query time. Percentiles use nearest-rank
  observations, not interpolation. Missing accepted probes and failed observation reads are reported.
- **Completion** requires every acknowledged log in raw and indexed deduplicated reads, an empty
  metric outbox, zero Kafka lag, and the matching generated count where enabled. Quota receipts and
  reserved counts must match distinct indexed events. These checks include warmup rows, while
  measured delivered counts subtract warmup. Completion time is an observed upper bound, including
  Kafka's commit interval and polling/CLI overhead; it is not a per-event latency percentile.
  Every acknowledged batch's marker and full event count are checked individually, so a stored
  but unacknowledged request cannot hide a missing acknowledged batch in aggregate totals. Events
  stored after an uncertain sender outcome are reported as `unacknowledgedStored` rather than
  silently classifying them as loss or treating client retries as deduplication.
- **Kafka lag** counts Kafka records (batched requests), not individual log events. Partition
  offsets are retained in the report. For these freshly created, earliest-offset topics only,
  an uncommitted partition starts at zero. Missing consumer metadata remains unknown. During the
  storage pause, committed offsets must remain at their pre-fault values.
- **Resource peaks and spool size** are sampled; they are not continuous high-water marks.
  Docker CPU percent uses 100% per CPU. Memory is Docker's reported working set, not Rust heap
  size. The sampler itself runs Docker/Kafka/SQL commands and adds observation overhead.
- **Stage timings**, on application images exposing `oc_log_ingest_stage_seconds`, subtract
  post-warmup counters from the final scrape and aggregate API replicas. Counts, elapsed sums and
  means include failed/cancelled operations. Request/admission stages contain other stages, so
  their sums overlap and are not CPU time. Older images without these metrics report no stages.
- **Postgres WAL bytes** cover the isolated database server over the phase, including receipt,
  counter and outbox work, full-page images, checkpoints and background jobs. They are not solely
  receipt payload bytes. Table growth includes page allocation, indexes and churn; short phases
  should not be extrapolated as exact long-term disk requirements.

Normal phases should have no sender failures, missing probes or count mismatches. Fault phases
should accumulate lag while memory stays within caps, then drain without logical loss or inflated
generated counts. A run that fails an invariant exits nonzero and saves its completed phases and
failure details before cleanup. Rate limiting is reported separately from acknowledged data loss.

These are bounded local experiments, not a capacity ceiling or a production SLO. The benchmark
does not measure OTLP/Collector, metrics/traces ingestion, query traffic, object storage, multi-node
replication, resharding, receipt expiration at steady state, long outages or a multi-day soak. Use
longer runs on dedicated representative hardware, realistic payloads and retention, concurrent reads,
and HA failure tests before sizing a large installation. There is no Datadog performance comparison
without an equivalent workload and deployment.
