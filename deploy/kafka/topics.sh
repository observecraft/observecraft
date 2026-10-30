#!/bin/sh
# Creates (or updates) ObserveCraft's Kafka topics (docs/adr/0004-kafka-ingestion-buffer.md).
# Env: OC_KAFKA_BROKERS (kafka:9092), OC_KAFKA_PARTITIONS (12), OC_KAFKA_REPLICATION (1),
#      OC_KAFKA_MIN_INSYNC (1), OC_KAFKA_RETENTION_HOURS (72), OC_KAFKA_LIVE_RETENTION_MINUTES (60).
# Idempotent: existing topics keep their partitions (more can be added, never fewer) and get the configs.
set -eu
BROKERS="${OC_KAFKA_BROKERS:-kafka:9092}"
PARTITIONS="${OC_KAFKA_PARTITIONS:-12}"
REPLICATION="${OC_KAFKA_REPLICATION:-1}"
MIN_INSYNC="${OC_KAFKA_MIN_INSYNC:-1}"
RETENTION_MS=$(( ${OC_KAFKA_RETENTION_HOURS:-72} * 3600000 ))
LIVE_RETENTION_MS=$(( ${OC_KAFKA_LIVE_RETENTION_MINUTES:-60} * 60000 ))
BIN="${KAFKA_BIN:-/opt/kafka/bin}"

until "$BIN/kafka-broker-api-versions.sh" --bootstrap-server "$BROKERS" >/dev/null 2>&1; do
  echo "waiting for Kafka at $BROKERS"; sleep 2
done

topic() { # name retention_ms
  "$BIN/kafka-topics.sh" --bootstrap-server "$BROKERS" --create --if-not-exists --topic "$1" \
    --partitions "$PARTITIONS" --replication-factor "$REPLICATION" \
    --config "retention.ms=$2" --config "min.insync.replicas=$MIN_INSYNC" \
    --config compression.type=producer --config max.message.bytes=67108864
  "$BIN/kafka-configs.sh" --bootstrap-server "$BROKERS" --alter --entity-type topics --entity-name "$1" \
    --add-config "retention.ms=$2,min.insync.replicas=$MIN_INSYNC,max.message.bytes=67108864" >/dev/null
  current=$("$BIN/kafka-topics.sh" --bootstrap-server "$BROKERS" --describe --topic "$1" | sed -n 's/.*PartitionCount: *\([0-9]*\).*/\1/p' | head -1)
  if [ -n "$current" ] && [ "$current" -lt "$PARTITIONS" ]; then
    "$BIN/kafka-topics.sh" --bootstrap-server "$BROKERS" --alter --topic "$1" --partitions "$PARTITIONS"
  fi
}

for signal in traces metrics logs; do
  topic "oc.otlp.$signal" "$RETENTION_MS"
  topic "oc.otlp-json.$signal" "$RETENTION_MS"
done
topic oc.logs.json "$RETENTION_MS"
topic oc.logs.live "$LIVE_RETENTION_MS"
echo "Kafka topics ready"
