-- Host metadata the Datadog Agent reports (`POST /intake/` on the intake, every 30 minutes and at
-- startup): OS, hardware, agent version, cloud identity and host tags. One row per (org, host),
-- replaced on every report; the host list and host page read it next to the host's metrics.
CREATE TABLE datadog_hosts (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    host          TEXT NOT NULL,
    agent_version TEXT NOT NULL DEFAULT '',
    os            TEXT NOT NULL DEFAULT '',
    -- Distribution and version, e.g. "ubuntu 24.04".
    platform      TEXT NOT NULL DEFAULT '',
    kernel        TEXT NOT NULL DEFAULT '',
    arch          TEXT NOT NULL DEFAULT '',
    cpu_cores     INTEGER,
    memory_bytes  BIGINT,
    -- aws, gcp, azure or empty.
    cloud         TEXT NOT NULL DEFAULT '',
    instance_id   TEXT NOT NULL DEFAULT '',
    -- "key:value" host tags (the Agent's `tags`, cloud and system tags).
    host_tags     TEXT[] NOT NULL DEFAULT '{}',
    payload       JSONB NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, host)
);
