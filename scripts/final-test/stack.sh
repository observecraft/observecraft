#!/usr/bin/env bash
# One isolated ObserveCraft stack of the before/after test (README.md here), side by side with the other.
#
#   scripts/final-test/stack.sh <old|new> containers   ClickHouse 26.8 (2 vCPU / 6 GiB, chart tuning), NATS
#                                                      (versions before ADR-0004), Kafka with its topics,
#                                                      Vector and the OTel Collector 0.163.2 of that version
#   scripts/final-test/stack.sh <old|new> server       release observecraft-server (applies migrations)
#   scripts/final-test/stack.sh <old|new> intake       release oc-intake
#   scripts/final-test/stack.sh <old|new> stop <server|intake>   stop that process (by its pid file)
#   scripts/final-test/stack.sh <old|new> rm           remove the containers, network and volumes
#
# OLD_SRC is the checkout of the old version (default ../oc-old next to this worktree); NEW is this
# worktree. Ports: old 182xx, new 183xx (ClickHouse HTTP x01, collector x03, Vector JSON logs x04,
# NATS x05, Kafka x06, intake x10 / gRPC x11, server x20, log indexer x21). Postgres: the existing ocv-pg
# container (DB oc_final_<version>); NEW's shared cache: ocfin-valkey on :16393.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
state="$root/.ft.local"
version="${1:?old or new}"
action="${2:?action}"
case "$version" in
  old) p=182; src="${OLD_SRC:-$(cd "$root/.." && pwd)/oc-old}" ;;
  new) p=183; src="$root" ;;
  *) echo "version must be old or new" >&2; exit 2 ;;
esac
bin="$src/server/target/release"
name="ocfin-$version"
net="$name-net"
mkdir -p "$state"

pg_url="postgres://oc:oc@localhost:25433/oc_final_$version"

start() { # name, command...
  local what="$1"; shift
  local pidfile="$state/$version-$what.pid"
  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "$version $what already running (pid $(cat "$pidfile"))"; return
  fi
  nohup "$@" >>"$state/$version-$what.log" 2>&1 &
  echo $! >"$pidfile"
  echo "$version $what started (pid $!)"
}

case "$action" in
  containers)
    docker network inspect "$net" >/dev/null 2>&1 || docker network create "$net" >/dev/null
    docker run -d --name "$name-ch" --network "$net" --network-alias ch \
      --cpus 2 --memory 6g --ulimit nofile=262144:262144 \
      -e CLICKHOUSE_SKIP_USER_SETUP=1 \
      -v "$here/clickhouse-config.xml:/etc/clickhouse-server/config.d/observecraft.xml:ro" \
      -v "$here/clickhouse-users.xml:/etc/clickhouse-server/users.d/observecraft-users.xml:ro" \
      -v "$name-ch-data:/var/lib/clickhouse" \
      -p "127.0.0.1:${p}01:8123" clickhouse/clickhouse-server:26.8 >/dev/null
    docker run -d --name "$name-nats" --network "$net" --network-alias nats \
      -p "127.0.0.1:${p}05:4222" nats:2.10-alpine -js >/dev/null
    docker run -d --name "$name-kafka" --network "$net" --network-alias kafka --user 0:0 \
      -e KAFKA_NODE_ID=1 -e KAFKA_PROCESS_ROLES=broker,controller \
      -e KAFKA_LISTENERS=INTERNAL://0.0.0.0:9092,HOST://0.0.0.0:19092,CONTROLLER://0.0.0.0:9093 \
      -e "KAFKA_ADVERTISED_LISTENERS=INTERNAL://kafka:9092,HOST://localhost:${p}06" \
      -e KAFKA_LISTENER_SECURITY_PROTOCOL_MAP=INTERNAL:PLAINTEXT,HOST:PLAINTEXT,CONTROLLER:PLAINTEXT \
      -e KAFKA_INTER_BROKER_LISTENER_NAME=INTERNAL -e KAFKA_CONTROLLER_LISTENER_NAMES=CONTROLLER \
      -e KAFKA_CONTROLLER_QUORUM_VOTERS=1@kafka:9093 -e KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR=1 \
      -e KAFKA_TRANSACTION_STATE_LOG_REPLICATION_FACTOR=1 -e KAFKA_TRANSACTION_STATE_LOG_MIN_ISR=1 \
      -e KAFKA_AUTO_CREATE_TOPICS_ENABLE=false -e KAFKA_LOG_DIRS=/var/lib/kafka/data \
      -e KAFKA_MESSAGE_MAX_BYTES=67108864 -e KAFKA_REPLICA_FETCH_MAX_BYTES=67108864 \
      -v "$name-kafka-data:/var/lib/kafka/data" -p "127.0.0.1:${p}06:19092" apache/kafka:4.1.0 >/dev/null
    if [[ -f "$src/deploy/kafka/topics.sh" ]]; then
      docker run --rm --network "$net" -e OC_KAFKA_BROKERS=kafka:9092 \
        -v "$src/deploy/kafka/topics.sh:/topics.sh:ro" --entrypoint /bin/sh apache/kafka:4.1.0 /topics.sh
    fi
    docker run -d --name "$name-vector" --network "$net" --network-alias vector-aggregator \
      -e CLICKHOUSE_USER=observecraft_ingest -e CLICKHOUSE_PASSWORD=observecraft \
      -e OC_CLICKHOUSE_HTTP_ENDPOINT=http://ch:8123 -e OC_NATS_URL=nats://nats:4222 -e OC_KAFKA_BROKERS=kafka:9092 \
      -e "OC_LOGPROC_URL=http://host.docker.internal:${p}21/ingest/logs" \
      -v "$src/deploy/vector:/etc/vector:ro" -v "$name-vector-data:/var/lib/vector" \
      -p "127.0.0.1:${p}04:8687" timberio/vector:0.42.0-debian --config /etc/vector/aggregator.yaml >/dev/null
    docker run -d --name "$name-collector" --network "$net" --user 0:0 \
      -e CLICKHOUSE_USER=observecraft_ingest -e CLICKHOUSE_PASSWORD=observecraft \
      -e "OC_CLICKHOUSE_TCP_ENDPOINT=tcp://ch:9000?dial_timeout=10s" \
      -e OC_VECTOR_OTLP_ENDPOINT=vector-aggregator:4317 -e OC_COLLECTOR_MEMORY_LIMIT_MIB=1024 -e OC_KAFKA_BROKERS=kafka:9092 \
      -v "$src/deploy/otel/collector.yaml:/etc/otelcol-contrib/config.yaml:ro" \
      -v "$name-collector-queue:/var/lib/otelcol/queue" \
      -p "127.0.0.1:${p}03:4318" otel/opentelemetry-collector-contrib:0.163.2 --config=/etc/otelcol-contrib/config.yaml >/dev/null
    docker ps --filter "name=$name-" --format '{{.Names}} {{.Status}}'
    ;;
  server)
    env_cache=()
    [[ "$version" == new ]] && env_cache=(OC_CACHE_URL=redis://localhost:16393/0)
    mkdir -p "$state/$version-data"
    start server env ${env_cache[@]+"${env_cache[@]}"} \
      OC_HTTP_ADDR="127.0.0.1:${p}20" OC_LOGPROC_ADDR="0.0.0.0:${p}21" \
      OC_CLICKHOUSE_URL="http://localhost:${p}01" OC_CLICKHOUSE_USER=observecraft OC_CLICKHOUSE_PASSWORD=observecraft \
      OC_POSTGRES_URL="$pg_url" OC_NATS_URL="nats://localhost:${p}05" OC_KAFKA_BROKERS="localhost:${p}06" \
      OC_SEED_DEMO=false OC_DATA_DIR="$state/$version-data" OC_PUBLIC_URL="http://localhost:${p}20" \
      OC_CORS_ORIGINS="http://localhost:${p}20" RUST_LOG="${RUST_LOG:-info}" \
      "$bin/observecraft-server"
    ;;
  intake)
    start intake env \
      OC_INTAKE_ADDR="127.0.0.1:${p}10" OC_INTAKE_GRPC_ADDR="127.0.0.1:${p}11" OC_POSTGRES_URL="$pg_url" \
      OC_INTAKE_COLLECTOR_URL="http://localhost:${p}03" OC_INTAKE_LOGS_URL="http://localhost:${p}04" \
      OC_KAFKA_BROKERS="localhost:${p}06" \
      OC_INTAKE_PROFILES_URL="http://localhost:${p}21/ingest/profiles" \
      OC_INTAKE_FLAG_EVALUATIONS_URL="http://localhost:${p}21/ingest/feature-flag-evaluations" \
      RUST_LOG="${RUST_LOG:-info}" "$bin/oc-intake"
    ;;
  stop)
    what="${3:?server or intake}"
    pidfile="$state/$version-$what.pid"
    if [[ -f "$pidfile" ]]; then
      pid="$(cat "$pidfile")"
      kill "$pid" 2>/dev/null || true
      for _ in $(seq 1 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
      rm -f "$pidfile"
      echo "$version $what stopped (pid $pid)"
    fi
    ;;
  rm)
    for c in ch nats kafka vector collector; do docker rm -f "$name-$c" >/dev/null 2>&1 || true; done
    for v in ch-data kafka-data vector-data collector-queue; do docker volume rm "$name-$v" >/dev/null 2>&1 || true; done
    docker network rm "$net" >/dev/null 2>&1 || true
    echo "$name removed"
    ;;
  *) echo "unknown action $action" >&2; exit 2 ;;
esac
