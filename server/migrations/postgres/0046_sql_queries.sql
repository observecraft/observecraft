-- SQL Editor: saved queries. Managed through routes/sql_editor.rs; queries run on the editor's
-- org-scoped virtual tables, so a saved query holds only its text.
CREATE TABLE sql_queries (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    sql        TEXT NOT NULL,
    -- Owning team handle ('' for none).
    team       TEXT NOT NULL DEFAULT '',
    -- Display name of who saved it and who changed it last.
    author     TEXT NOT NULL DEFAULT '',
    updated_by TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sql_queries_org ON sql_queries (org_id, updated_at DESC);
