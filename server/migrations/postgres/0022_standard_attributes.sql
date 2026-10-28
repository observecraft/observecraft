-- Standard attributes (Logs → Configuration → Standard Attributes): the default list ships with the
-- server (server/seeds/standard_attributes.json); this table keeps only each org's changes, so an
-- org that changed nothing follows the defaults. A row is an attribute the org added, a default it
-- edited (same path), or a default it removed (`hidden`). "Reset to defaults" deletes the org's rows.
-- The indexer copies alias values to the standard attribute after pipelines, before index routing.

CREATE TABLE log_standard_attributes (
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- The attribute's path (`http.status_code`), without `@`.
    path            TEXT NOT NULL,
    -- A default the org removed; the other columns are unused then.
    hidden          BOOLEAN NOT NULL DEFAULT false,
    type            TEXT NOT NULL DEFAULT 'string' CHECK (type IN ('string', 'integer', 'double', 'boolean')),
    description     TEXT NOT NULL DEFAULT '',
    -- ["status_code", "http.response.status_code"]: tried in order, the first present wins.
    aliases         JSONB NOT NULL DEFAULT '[]',
    -- Keep the alias attribute after copying its value (otherwise it is moved).
    preserve_source BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, path)
);
