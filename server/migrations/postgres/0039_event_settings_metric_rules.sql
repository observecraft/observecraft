-- Event Management settings (correlators, maintenance windows, work item attributes), Metrics
-- settings (derived metrics, ingestion and indexing rules) and the latest message of each service
-- check. Managed through routes/event_settings.rs, routes/derived_metrics.rs and
-- routes/metric_rules.rs; the intake reads metric_rules (ingestion) and writes service_check_messages.

-- Derived metrics: a name that metric queries resolve to a formula over other metric queries
-- (`sum:a{*} by {host} / sum:b{*} by {host} * 100`), evaluated at query time.
CREATE TABLE derived_metrics (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- The metric queries by letter, in order: ["sum:a{*} by {host}", "sum:b{*} by {host}"] are a and b.
    queries     JSONB NOT NULL DEFAULT '[]',
    -- Arithmetic over the letters: `a / b * 100`.
    expression  TEXT NOT NULL,
    -- The expression with each letter replaced by its query: what queries resolve the name to.
    formula     TEXT NOT NULL,
    created_by  TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

-- Metrics rules. Ingestion controls run at the intake (drop matching metrics, or strip tags from
-- them); indexing controls limit the tags metric queries may filter or group the metrics by.
CREATE TABLE metric_rules (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('ingestion', 'indexing')),
    -- Metric names it applies to; `*` matches any run of characters (`kubernetes.*`).
    metric_pattern TEXT NOT NULL,
    -- ingestion: 'drop' or 'strip_tags'; indexing: 'allow_tags'.
    action         TEXT NOT NULL CHECK (action IN ('drop', 'strip_tags', 'allow_tags')),
    -- Tag keys stripped (strip_tags) or queryable (allow_tags).
    tags           TEXT[] NOT NULL DEFAULT '{}',
    enabled        BOOLEAN NOT NULL DEFAULT true,
    updated_by     TEXT NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((kind = 'ingestion' AND action IN ('drop', 'strip_tags')) OR (kind = 'indexing' AND action = 'allow_tags'))
);
CREATE UNIQUE INDEX metric_rules_name ON metric_rules (org_id, lower(name));
CREATE INDEX metric_rules_org_kind ON metric_rules (org_id, kind);

-- Event correlators: warning and error events matching `query` are grouped into one work item per
-- distinct `group_by` values, while they keep arriving within `window_minutes` of each other.
CREATE TABLE event_correlators (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    -- Event search syntax (`source:kubernetes env:prod`); '' = every event.
    query          TEXT NOT NULL DEFAULT '',
    -- `source`, `host`, `status`, `title` or tag keys (`kube_namespace`, `service`).
    group_by       TEXT[] NOT NULL DEFAULT '{}',
    window_minutes INTEGER NOT NULL CHECK (window_minutes BETWEEN 1 AND 1440),
    enabled        BOOLEAN NOT NULL DEFAULT true,
    -- Correlators are tried in this order; the first match groups the event.
    position       INTEGER NOT NULL DEFAULT 0,
    updated_by     TEXT NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX event_correlators_name ON event_correlators (org_id, lower(name));

-- Maintenance windows: events matching `query` between starts_at and ends_at are marked as
-- expected; work items made only of such events are suppressed in the Triage Inbox.
CREATE TABLE event_maintenance_windows (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    query      TEXT NOT NULL DEFAULT '',
    starts_at  TIMESTAMPTZ NOT NULL,
    ends_at    TIMESTAMPTZ NOT NULL,
    created_by TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at)
);
CREATE INDEX event_maintenance_windows_org ON event_maintenance_windows (org_id, ends_at);

-- Custom fields of work items (Triage Inbox): a select of values or free text.
CREATE TABLE event_work_item_attributes (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('select', 'text')),
    -- The values a select offers.
    options    TEXT[] NOT NULL DEFAULT '{}',
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX event_work_item_attributes_name ON event_work_item_attributes (org_id, lower(name));

-- A work item's attribute values (work item ids are stable hashes of what they group).
CREATE TABLE event_work_item_values (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    work_item_id TEXT NOT NULL,
    attribute_id UUID NOT NULL REFERENCES event_work_item_attributes(id) ON DELETE CASCADE,
    value        TEXT NOT NULL,
    updated_by   TEXT NOT NULL DEFAULT '',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, work_item_id, attribute_id)
);

-- The latest message a service check reported per host (the status itself is the
-- `service_check.status` gauge). The intake writes a row only when the status or message changes.
CREATE TABLE service_check_messages (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    check_name TEXT NOT NULL,
    host       TEXT NOT NULL,
    status     SMALLINT NOT NULL,
    message    TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, check_name, host)
);
