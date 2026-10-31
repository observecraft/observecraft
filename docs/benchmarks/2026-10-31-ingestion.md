# JSON ingestion measurements, 2026-10-31

Kafka absorbed the tested bursts and short storage faults without missing acknowledged logs.
Downstream indexing accumulated substantial backlog at higher rates: a 40,000 logs/s burst with
quotas and one generated metric had 61.30 seconds p95 indexed visibility despite 11.3 ms p95
intake acknowledgement. This is evidence of buffering and a downstream capacity gap; it does
not establish sustainable throughput at 40,000 logs/s.

## Reproduction and scope

See [the harness instructions](ingestion.md) for configuration, resource caps and measurement
definitions. These runs used application revision `2ba12cc6a15a680b8d0f19e51a66db14e2c52366`,
four organizations, two API replicas, 500-event batches and fifteen-second sending windows.
The Docker Desktop 29.8.2 host reported 32 logical CPUs and 15,994,150,912 bytes of RAM; the
main development application remained running on the same host. Each run used fresh isolated
databases, one Kafka broker with twelve partitions and one ClickHouse node. Payloads were small,
repetitive and low cardinality. Container caps are limits, not dedicated host reservations.

The complete reports contain image IDs, samples, acknowledged batch markers and harness SHA-256:

- [Default matrix](results/2026-10-31-baseline.json): default environment, mode `all`.
- [Higher offered rates](results/2026-10-31-high.json): `OC_BENCH_RATES=10000,40000`,
  `OC_BENCH_MODE=throughput`; other settings default.

Only the report destination path was replaced with its portable basename. Earlier calibration
trials are excluded: the final harness runs Kafka's Java observation CLI in a separate container
so its CPU cannot be mistaken for broker CPU. The two published runs used the same harness hash.

## Results

`unlimited` has no daily quota; `quota` enables durable shared quota coordination; `metrics`
adds one generated count metric to quota coordination. `slow` delays each storage INSERT by
250 ms, and `paused` temporarily holds INSERTs. All values below are observations from one
trial per scenario. Visibility includes 500 ms probe polling; drain includes observation and
Kafka commit overhead. Lag counts batched Kafka records, not logs. WAL includes background work
and full-page images in the isolated Postgres cluster.

| Scenario | Acknowledged logs | ACK p95, ms | Indexed visibility p95, s | Drain after sender, s | Sampled peak Kafka lag | WAL bytes / acknowledged log |
|---|---:|---:|---:|---:|---:|---:|
| unlimited-1000 | 15,000 | 12.6 | 1.53 | 4.0 | 11 | 23.5 |
| quota-1000 | 15,000 | 17.2 | 1.63 | 4.4 | 10 | 377.7 |
| unlimited-5000 | 75,000 | 12.0 | 1.47 | 4.9 | 50 | 1.7 |
| quota-5000 | 75,000 | 11.8 | 20.78 | 25.8 | 72 | 361.5 |
| metrics-5000 | 75,000 | 11.3 | 3.94 | 8.2 | 83 | 397.8 |
| slow-5000 | 75,000 | 11.2 | 12.90 | 16.9 | 102 | 363.1 |
| paused-5000 | 75,000 | 11.2 | 7.18 | 10.9 | 98 | 361.9 |
| unlimited-10000 | 150,000 | 11.7 | 1.54 | 3.4 | 108 | 2.2 |
| quota-10000 | 150,000 | 11.5 | 13.44 | 18.3 | 218 | 361.9 |
| unlimited-40000 | 600,000 | 11.1 | 13.58 | 17.9 | 580 | 0.3 |
| quota-40000 | 600,000 | 10.9 | 58.30 | 64.5 | 1097 | 360.6 |
| metrics-40000 | 600,000 | 11.3 | 61.30 | 73.3 | 960 | 399.1 |

Across both runs, all **2,505,000 measured acknowledged logs**, plus 24,000 warmup logs, were
present in raw and indexed deduplicated reads. Every acknowledged batch's full event count was
verified. Sender failures, missing accepted probes, probe read failures, sampling errors and
stored-but-unacknowledged events were all zero. Quota receipts and reservations matched indexed
counts. Generated counts matched every corresponding indexed event, and outboxes and Kafka lag
were empty at completion. During the storage pause, committed source offsets stayed unchanged
until writes resumed. These checks cover finite runs, not arbitrary outages or long-term retention.

## Implications and next work

1. **Keep the Kafka buffer and durable acknowledgement rules.** Fast intake acknowledgements
   remained separate from storage progress, and the paused sink did not advance committed source
   offsets. Monitor indexed freshness and backlog age alongside acceptance rate: low ACK latency
   alone would hide the minute-long search delay in the highest-rate scenarios.
2. **Profile Vector and the indexer path before adding another state store.** In unlimited-40000,
   Vector reached 197.74% sampled CPU against its two-CPU cap while Kafka reached 51.24% against
   its two-CPU cap. This makes Vector CPU a concrete capacity suspect for that workload. Repeat
   with more Vector CPU and then multiple consumers to measure improvement; do not infer a
   scaling factor from a single CPU sample. The quota scenario also reached about 459 MiB Vector
   working set against a 512 MiB cap, so longer overload runs must include memory/backpressure.
3. **Measure quota and outbox waits before changing coordination.** Quota phases produced about
   361–378 WAL bytes per acknowledged event, and metric phases about 398–399. At 40,000 offered
   logs/s, quotas raised observed visibility from 13.58 to 58.30 seconds. The runs do not attribute
   this delay to a particular lock, fsync, ClickHouse call or batch boundary. Instrument those
   durations, repeat scenarios in varied order, then tune batching/concurrency while preserving
   replay receipts and global limits. The 5,000/s metric case being faster than quota-only also
   demonstrates variability; these single trials do not isolate the incremental metric cost.
4. **Include native metric delivery in capacity planning.** The metric outbox reached a sampled
   16.4 MiB at 40,000/s and drained fully; ClickHouse reached 322.10% sampled CPU against a
   four-CPU cap. Indexed logs, metric writes and deduplicating reads share that budget. Repeat
   with realistic metric cardinality, retained data and concurrent user queries.
5. **Establish sustained capacity and HA separately.** Run multi-minute steady loads, longer
   sink outages and recovery, broker/node failures with replicated storage, and tenant-skewed
   workloads. This experiment covers JSON logs only, not OTLP/Collector, direct metrics/traces,
   multi-node ClickHouse or production query traffic. It supplies no equivalent Datadog benchmark
   and no evidence that introducing RocksDB would fix the measured delays.
