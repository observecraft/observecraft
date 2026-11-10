-- What a connector package created for an org when its first connection was saved (crates/oc-api/src/connectors/
-- assets.rs): its monitors, Cloud SIEM rules and log-based metrics, one row per asset file of the package. A row
-- means "installed once": an asset the org deleted isn't created again, and removing the org's last connection
-- deletes what the rows name (the package's log pipeline too; its dashboards stay).
CREATE TABLE IF NOT EXISTS connector_assets (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    connector_id TEXT NOT NULL,
    -- monitor | security_rule | log_metric
    kind         TEXT NOT NULL,
    -- The asset's file in the package (`assets/monitors/errors.json`).
    asset        TEXT NOT NULL,
    -- The monitor's id, the rule's id or the metric's name.
    ref          TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, connector_id, kind, asset)
);
