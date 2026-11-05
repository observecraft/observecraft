-- The Kafka log ingester (`crate::log_ingest`, OC_LOG_INGEST=kafka): which server reads each partition of the log
-- topics and the next offset to read. A server holds a partition while its lease is current and moves the offset
-- only after the partition's logs are stored, so a partition another server takes over is read from there.
CREATE TABLE IF NOT EXISTS log_ingest_partitions (
    topic       TEXT NOT NULL,
    partition   INTEGER NOT NULL,
    -- NULL until first stored: the partition is then read from its earliest record.
    next_offset BIGINT,
    owner       TEXT,
    lease_until TIMESTAMPTZ NOT NULL DEFAULT 'epoch',
    PRIMARY KEY (topic, partition)
);

-- The servers reading them (heartbeats), so a server that holds nothing yet still counts in everyone's fair share.
CREATE TABLE IF NOT EXISTS log_ingest_servers (
    owner      TEXT NOT NULL PRIMARY KEY,
    topics     TEXT[] NOT NULL,
    seen_until TIMESTAMPTZ NOT NULL
);
