-- Notebooks (Dashboards → Notebooks, Logs → Notebooks): documents of ordered cells (Markdown text and
-- live query cells: timeseries, query value, top list, log stream). The cells are the JSON the web app
-- edits (web/src/api/notebooks/types.ts), in the shape of dashboard widget definitions; the server
-- checks their types and sizes (routes/notebooks.rs). Deleting is soft: `deleted_at` lists the
-- notebook under Recently Deleted until it is restored.

CREATE TABLE notebooks (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    author_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    teams       TEXT[] NOT NULL DEFAULT '{}',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    status      TEXT NOT NULL DEFAULT 'published' CHECK (status IN ('published', 'draft')),
    -- The notebook's relative time range (`1h`, `1d`, …); a cell may override it with its own `time`.
    time_range  TEXT NOT NULL DEFAULT '1h',
    -- [{ "id": "c1", "definition": { "type": "markdown", "text": "…" }, "time": "4h" }, …]
    cells       JSONB NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ,
    -- Target of the per-user tables' (org, notebook) keys.
    UNIQUE (org_id, id)
);
CREATE INDEX notebooks_org_modified ON notebooks (org_id, modified_at DESC) WHERE deleted_at IS NULL;

-- A user's favorite notebooks (the star in the list).
CREATE TABLE notebook_favorites (
    org_id      TEXT NOT NULL,
    notebook_id TEXT NOT NULL,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (notebook_id, user_id),
    FOREIGN KEY (org_id, notebook_id) REFERENCES notebooks(org_id, id) ON DELETE CASCADE
);

-- When a user last opened a notebook (Recents).
CREATE TABLE notebook_views (
    org_id      TEXT NOT NULL,
    notebook_id TEXT NOT NULL,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    viewed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (notebook_id, user_id),
    FOREIGN KEY (org_id, notebook_id) REFERENCES notebooks(org_id, id) ON DELETE CASCADE
);
CREATE INDEX notebook_views_user ON notebook_views (user_id, viewed_at DESC);
