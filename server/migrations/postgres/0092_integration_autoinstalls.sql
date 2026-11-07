-- What the server installed by itself when an integration's logs first arrived (`routes::integration_autoinstall`):
-- its log pipeline from the library (`pipeline:<library id>`) and the dashboards of integrations that only send logs
-- (`dashboards:<catalog id>`). Each is installed once per org, so one the org removes stays removed.
CREATE TABLE IF NOT EXISTS integration_autoinstalls (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    item         TEXT NOT NULL,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, item)
);
