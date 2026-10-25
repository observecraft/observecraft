-- Monitors: definitions (extending the 0001 table), per-group state, state transitions and downtimes.
-- `muted` is superseded by downtimes (a muted monitor is one with an active downtime).

ALTER TABLE monitors
    DROP COLUMN muted,
    ADD COLUMN type             TEXT NOT NULL DEFAULT 'metric' CHECK (type IN ('metric', 'apm')),
    ADD COLUMN message          TEXT NOT NULL DEFAULT '',
    ADD COLUMN priority         SMALLINT CHECK (priority BETWEEN 1 AND 5),
    ADD COLUMN tags             TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN teams            TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN draft            BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN created_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN state_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN evaluated_at     TIMESTAMPTZ;
CREATE INDEX monitors_org ON monitors (org_id);

-- One row per monitor group (`*` when the query has no `by`, else `host:a,service:b`).
CREATE TABLE monitor_groups (
    monitor_id       BIGINT NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
    group_key        TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('ok', 'warn', 'alert', 'nodata')),
    value            DOUBLE PRECISION,
    last_data_at     TIMESTAMPTZ,
    state_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (monitor_id, group_key)
);

-- State transitions. Notifications are recorded here; delivery needs a mail/chat/on-call integration.
CREATE TABLE monitor_events (
    id          BIGSERIAL PRIMARY KEY,
    monitor_id  BIGINT NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
    group_key   TEXT NOT NULL,
    from_status TEXT NOT NULL,
    to_status   TEXT NOT NULL,
    value       DOUBLE PRECISION,
    muted       BOOLEAN NOT NULL DEFAULT false,
    recipients  TEXT[] NOT NULL DEFAULT '{}',
    message     TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX monitor_events_monitor ON monitor_events (monitor_id, created_at DESC);

-- A downtime silences notifications for one monitor (monitor_id) or every monitor carrying all of
-- monitor_tags, limited to groups matching scope ('*' or tags like 'host:a'). Recurring downtimes
-- repeat their [start_at, end_at) window every day or week until recur_until.
CREATE TABLE downtimes (
    id           BIGSERIAL PRIMARY KEY,
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    monitor_id   BIGINT REFERENCES monitors(id) ON DELETE CASCADE,
    monitor_tags TEXT[] NOT NULL DEFAULT '{}',
    scope        TEXT[] NOT NULL DEFAULT '{}',
    message      TEXT NOT NULL DEFAULT '',
    start_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    end_at       TIMESTAMPTZ,
    recurrence   TEXT CHECK (recurrence IN ('daily', 'weekly')),
    recur_until  TIMESTAMPTZ,
    created_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    canceled_at  TIMESTAMPTZ,
    CHECK (recurrence IS NULL OR end_at IS NOT NULL)
);
CREATE INDEX downtimes_org ON downtimes (org_id) WHERE canceled_at IS NULL;
