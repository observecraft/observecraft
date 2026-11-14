-- Sensitive Data Scanner (sds/, routes/sensitive_data_scanner.rs, datadog_api/sensitive_data_scanner.rs): scanning
-- groups and their rules as Datadog's API and Terraform provider manage them (`datadog_sensitive_data_scanner_group`,
-- `_rule`, `_group_order`); the UI edits the same rows. `attributes` holds a document's attributes as stored
-- (Datadog's field names). The log ingester scans logs with each org's enabled groups (sds::Scanner).

-- One configuration per org: its version moves on every change (Datadog's `meta.version`), and the
-- ingester recompiles an org's scanner when it does.
CREATE TABLE sds_configs (
    org_id      TEXT PRIMARY KEY,
    version     BIGINT NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Scanning groups, in the configuration's order (`position`).
CREATE TABLE sds_groups (
    org_id      TEXT NOT NULL,
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    position    INT NOT NULL,
    attributes  JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- Scanning rules, in their group's order. `standard_pattern_id` names a pattern of our library (sds::patterns)
-- instead of the rule's own `pattern`.
CREATE TABLE sds_rules (
    org_id      TEXT NOT NULL,
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    group_id    UUID NOT NULL,
    position    INT NOT NULL,
    standard_pattern_id TEXT,
    attributes  JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, group_id) REFERENCES sds_groups (org_id, id) ON DELETE CASCADE
);
CREATE INDEX sds_rules_group ON sds_rules (org_id, group_id, position);
