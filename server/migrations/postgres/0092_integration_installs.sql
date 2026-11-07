-- Integrations installed from Integrations → Install (`routes::integration_installs`): the integration's log pipelines
-- and dashboards are installed with it, and its dashboards are listed while it stays installed.
CREATE TABLE IF NOT EXISTS integration_installs (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    integration  TEXT NOT NULL,
    installed_by UUID REFERENCES users(id) ON DELETE SET NULL,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, integration)
);
