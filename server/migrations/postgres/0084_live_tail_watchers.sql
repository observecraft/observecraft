-- Live tail sessions, any server's (`live_feed::watch`): the Kafka log ingester publishes an org's logs to
-- `oc.logs.live` only while one of its sessions is open, renewed every few seconds.
CREATE TABLE IF NOT EXISTS live_tail_watchers (
    org_id TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    until  TIMESTAMPTZ NOT NULL
);
